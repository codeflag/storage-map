//! Lokaler Server fuer die Oberflaeche. Lauscht nur auf 127.0.0.1 und verlangt bei jedem
//! API-Aufruf das Start-Token. So kann keine Webseite im Browser an die Loeschfunktion heran.

use crate::analysis::{self, now_secs, FileQuery};
use crate::cleanup::{self, Cart, ItemResult, Mode};
use crate::config::{self, Level};
use crate::dupes::{self, DupEngine, DupFolderGroup, DupGroup, DupParams};
use crate::guard::{self, Guard};
use crate::history;
use crate::index::*;
use crate::junk::{self, JunkReport};
use crate::paths::{self, inside};
use crate::scan::{self, ScanProgress};
use crate::{assets, demo, mcp, platform};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};
use tiny_http::{Header, Method, Request, Response, Server};

#[derive(Default)]
pub struct CleanupState {
    pub running: bool,
    pub mode: Option<Mode>,
    pub total: usize,
    pub done: usize,
    pub results: Vec<ItemResult>,
    pub last: Option<Value>,
}

/// In welchem Betrieb laeuft das Programm? Nur `Normal` fasst echte Laufwerke an.
#[derive(Clone, Debug, PartialEq)]
pub enum RunMode {
    Normal,
    /// Erfundene Laufwerke im Speicher. Kein Zugriff auf Nutzerdateien.
    Demo {
        scale: u64,
    },
    /// Nur dieser Testordner darf gelesen und veraendert werden.
    Sandbox(String),
}

impl RunMode {
    pub fn name(&self) -> &'static str {
        match self {
            RunMode::Normal => "normal",
            RunMode::Demo { .. } => "demo",
            RunMode::Sandbox(_) => "sandbox",
        }
    }
    pub fn is_normal(&self) -> bool {
        *self == RunMode::Normal
    }
    /// Darf dieser Pfad angefasst werden? Im Sandbox-Betrieb nur innerhalb des Testordners.
    pub fn allows(&self, path: &str) -> bool {
        match self {
            RunMode::Normal => true,
            RunMode::Demo { .. } => demo::drives().iter().any(|d| d.root == path),
            RunMode::Sandbox(root) => inside(path, root),
        }
    }
}

pub const REFUSED: &str = "Im Test-Betrieb ist nur der Testordner erlaubt.";

pub struct App {
    pub token: String,
    pub port: u16,
    pub ui_dir: Option<PathBuf>,
    pub mode: RunMode,
    pub index: RwLock<Option<Index>>,
    /// Zaehlt jede Aenderung am Index, damit Caches wissen, wann sie veraltet sind.
    pub gen: AtomicU64,
    pub scan: Mutex<Option<Arc<ScanProgress>>>,
    pub scan_error: Mutex<Option<String>>,
    pub junk: Mutex<Option<(u64, Arc<JunkReport>)>>,
    pub summary: Mutex<Option<(u64, Arc<Value>)>>,
    pub dup: Arc<DupEngine>,
    pub dup_folders: Mutex<Option<(u64, Arc<Vec<DupFolderGroup>>)>>,
    pub cart: Mutex<Cart>,
    pub cleanup: Mutex<CleanupState>,
    pub health: Mutex<HashMap<String, Value>>,
    pub live_free: AtomicU64,
    pub last_ping: AtomicU64,
    pub quitting: AtomicBool,
    pub started: Instant,
    /// Ohne Oberflaeche (MCP-Betrieb): keine Dialoge auf dem Bildschirm des Nutzers.
    pub headless: AtomicBool,
    /// Stand der Dateien, die sich auch ein anderer Prozess teilt (Oberflaeche und MCP-Server).
    index_seen: Mutex<Option<SystemTime>>,
    cart_seen: Mutex<Option<SystemTime>>,
    dups_seen: Mutex<Option<SystemTime>>,
    session_codes: Mutex<Vec<(String, Instant)>>,
}

fn modified(p: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(p).ok()?.modified().ok()
}

pub type Res = Result<Value, (u16, String)>;

fn err<T>(code: u16, m: impl Into<String>) -> Result<T, (u16, String)> {
    Err((code, m.into()))
}

impl App {
    pub fn new(token: String, port: u16, ui_dir: Option<PathBuf>, mode: RunMode) -> Arc<App> {
        Arc::new(App {
            token,
            port,
            ui_dir,
            mode,
            index: RwLock::new(None),
            gen: AtomicU64::new(1),
            scan: Mutex::new(None),
            scan_error: Mutex::new(None),
            junk: Mutex::new(None),
            summary: Mutex::new(None),
            dup: Arc::new(DupEngine::default()),
            dup_folders: Mutex::new(None),
            cart: Mutex::new(Cart::default()),
            cleanup: Mutex::new(CleanupState::default()),
            health: Mutex::new(HashMap::new()),
            live_free: AtomicU64::new(0),
            last_ping: AtomicU64::new(now_secs()),
            quitting: AtomicBool::new(false),
            started: Instant::now(),
            headless: AtomicBool::new(false),
            index_seen: Mutex::new(None),
            cart_seen: Mutex::new(None),
            dups_seen: Mutex::new(None),
            session_codes: Mutex::new(vec![]),
        })
    }

    fn is_scanning(&self) -> bool {
        self.scan.lock().unwrap().is_some()
    }

    pub fn is_busy(&self) -> bool {
        self.is_scanning() || self.dup.running.load(Relaxed) || self.cleanup.lock().unwrap().running
    }

    /// Setzt einen neuen Index und alles, was davon abhaengt, zurueck.
    pub fn install_index(&self, ix: Index) {
        let root = ix.meta.root.clone();
        self.live_free.store(ix.meta.volume.free, Relaxed);
        *self.index.write().unwrap() = Some(ix);
        self.gen.fetch_add(1, Relaxed);
        self.cart.lock().unwrap().ids.clear();
        if let Some(ix) = self.index.read().unwrap().as_ref() {
            self.load_cart(ix);
        }
        let dir = history::scan_dir(&root);
        *self.index_seen.lock().unwrap() = modified(&dir.join("latest.smidx"));
        *self.dups_seen.lock().unwrap() = modified(&dir.join("dups.json"));
        *self.cleanup.lock().unwrap() = CleanupState::default();
        self.dup.reset();
        // Duplikate eines frueheren Laufs auf genau diesem Scan wiederherstellen.
        if let Ok(t) = std::fs::read_to_string(history::scan_dir(&root).join("dups.json")) {
            if let Ok(g) = serde_json::from_str::<Vec<DupGroup>>(&t) {
                *self.dup.groups.lock().unwrap() = g;
                self.dup.done.store(true, Relaxed);
            }
        }
    }

    /// Schreibt die Aufraeumliste auf die Platte. So bleibt sie erhalten und ein zweiter Prozess sieht sie.
    pub fn persist_cart(&self, ix: &Index) {
        let paths: Vec<String> = self.cart.lock().unwrap().ids.iter().map(|i| ix.path(*i)).collect();
        let dir = history::scan_dir(&ix.meta.root);
        let file = dir.join("cart.json");
        let _ = std::fs::create_dir_all(&dir);
        if platform::write_private(&file, json!({ "paths": paths }).to_string().as_bytes()).is_ok() {
            *self.cart_seen.lock().unwrap() = modified(&file);
        }
    }

    fn load_cart(&self, ix: &Index) {
        let file = history::scan_dir(&ix.meta.root).join("cart.json");
        let ids: Vec<u32> = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v["paths"].as_array().map(|a| a.iter().filter_map(|p| p.as_str().and_then(|x| ix.find_path(x))).collect()))
            .unwrap_or_default();
        let mut cart = self.cart.lock().unwrap();
        cart.ids.clear();
        cart.add(ix, &ids);
        *self.cart_seen.lock().unwrap() = modified(&file);
    }

    /// Nimmt Aenderungen eines anderen Prozesses auf, etwa wenn ein MCP-Client die Aufraeumliste bearbeitet
    /// oder Dateien entfernt hat, waehrend die Oberflaeche offen ist.
    pub fn sync_from_disk(&self) {
        if self.is_busy() {
            return;
        }
        let root = match self.index.try_read() {
            Ok(g) => g.as_ref().map(|ix| ix.meta.root.clone()),
            Err(_) => None,
        };
        let Some(root) = root else { return };
        let dir = history::scan_dir(&root);
        let now = modified(&dir.join("latest.smidx"));
        if now.is_some() && now != *self.index_seen.lock().unwrap() {
            if let Ok(ix) = history::load_latest(&root) {
                self.install_index(ix);
            }
            return;
        }
        let dups = modified(&dir.join("dups.json"));
        if dups.is_some() && dups != *self.dups_seen.lock().unwrap() {
            *self.dups_seen.lock().unwrap() = dups;
            let groups = std::fs::read_to_string(dir.join("dups.json")).ok().and_then(|t| serde_json::from_str::<Vec<DupGroup>>(&t).ok());
            if let Some(g) = groups {
                *self.dup.groups.lock().unwrap() = g;
                self.dup.done.store(true, Relaxed);
                self.gen.fetch_add(1, Relaxed);
            }
        }
        let cart = modified(&dir.join("cart.json"));
        if cart.is_some() && cart != *self.cart_seen.lock().unwrap() {
            if let Some(ix) = self.index.read().unwrap().as_ref() {
                self.load_cart(ix);
            }
            self.gen.fetch_add(1, Relaxed);
        }
    }

    /// Einmal-Code fuer ein neues Fenster. Er taugt nur einmal und nur eine Minute lang.
    pub fn new_session_code(&self) -> String {
        let code = platform::random_hex();
        let mut codes = self.session_codes.lock().unwrap();
        codes.retain(|(_, t)| t.elapsed() < Duration::from_secs(60));
        codes.push((code.clone(), Instant::now()));
        code
    }

    fn redeem_session_code(&self, code: &str) -> bool {
        let mut codes = self.session_codes.lock().unwrap();
        codes.retain(|(_, t)| t.elapsed() < Duration::from_secs(60));
        match codes.iter().position(|(c, _)| c == code) {
            Some(i) => {
                codes.remove(i);
                true
            }
            None => false,
        }
    }

    fn junk_report(&self, ix: &Index) -> Arc<JunkReport> {
        let gen = self.gen.load(Relaxed);
        let mut slot = self.junk.lock().unwrap();
        if let Some((g, r)) = slot.as_ref() {
            if *g == gen {
                return r.clone();
            }
        }
        let r = Arc::new(junk::analyze(ix));
        *slot = Some((gen, r.clone()));
        r
    }

    fn dup_totals(&self, ix: &Index) -> (Option<u64>, bool) {
        if !self.dup.done.load(Relaxed) {
            return (None, true);
        }
        let groups = self.dup.groups.lock().unwrap();
        let mut sum = 0u64;
        let mut all_verified = true;
        for g in groups.iter() {
            let alive = g.ids.iter().filter(|i| !ix.nodes[**i as usize].is_deleted()).count() as u64;
            sum += g.alloc * alive.saturating_sub(1);
            all_verified &= g.verified;
        }
        (Some(sum), all_verified)
    }
}

/// Fuehrt einen Scan vollstaendig aus: lesen, auswerten, speichern.
pub fn run_scan(root: &str, progress: &ScanProgress, drive: Option<platform::DriveInfo>, mode: &RunMode) -> Result<Index, String> {
    if !mode.allows(root) {
        return Err(REFUSED.into());
    }
    let t0 = Instant::now();
    let mut ix = scan::scan(root, progress)?;
    *progress.phase.lock().unwrap() = "analyze".into();
    scan::finalize(&mut ix);
    ix.meta.scanned_at = now_secs();
    ix.meta.duration_ms = t0.elapsed().as_millis() as u64;
    ix.meta.elevated = platform::is_elevated();
    // Freien Platz erst nach dem Scan lesen, damit er zum Ergebnis passt.
    if mode.is_normal() {
        let drive = drive.or_else(|| platform::volume_root(root).map(|r| platform::volume_info(&r)));
        if let Some(d) = drive {
            let fresh = platform::volume_info(&d.root);
            ix.meta.volume =
                VolumeMeta { total: fresh.total, free: fresh.free, label: fresh.label, fs: fresh.fs, kind: fresh.kind, media: fresh.media };
        }
    } else {
        ix.meta.volume = VolumeMeta { kind: "fixed".into(), media: "ssd".into(), fs: "TEST".into(), ..Default::default() };
    }
    if ix.meta.is_drive {
        let v = &ix.meta.volume;
        ix.meta.unaccounted = v.total.saturating_sub(v.free).saturating_sub(ix.nodes[0].alloc);
    }
    *progress.phase.lock().unwrap() = "save".into();
    if let Err(e) = history::save_scan(&ix) {
        eprintln!("Scan konnte nicht gespeichert werden: {e}");
    }
    Ok(ix)
}

pub fn start_scan(app: &Arc<App>, raw: &str) -> Res {
    let root = paths::normalize(raw).map_err(|e| (400, e))?;
    if app.is_busy() {
        return err(409, "Es läuft noch ein anderer Vorgang.");
    }
    if !app.mode.allows(&root) {
        return err(403, REFUSED);
    }
    if let RunMode::Demo { scale } = app.mode {
        return simulate_scan(app, root, scale);
    }
    let meta = std::fs::metadata(platform::long_pathbuf(std::path::Path::new(&root))).map_err(|e| (400, format!("Pfad nicht lesbar: {e}")))?;
    if !meta.is_dir() {
        return err(400, "Das ist kein Ordner.");
    }
    let drive = if app.mode.is_normal() { platform::volume_root(&root).map(|r| platform::volume_info(&r)) } else { None };
    let media = if app.mode.is_normal() { drive.as_ref().map(|d| d.media.clone()).unwrap_or_else(|| "net".into()) } else { "ssd".to_string() };
    let is_drive = platform::is_volume_root(&root);
    let expected = match &drive {
        Some(d) if is_drive => d.total.saturating_sub(d.free),
        _ => 0,
    };
    let progress = Arc::new(ScanProgress::new(&root, expected, scan::threads_for(&media), &media));
    *app.scan.lock().unwrap() = Some(progress.clone());
    *app.scan_error.lock().unwrap() = None;
    let app2 = app.clone();
    std::thread::spawn(move || {
        let res = run_scan(&progress.root, &progress, drive, &app2.mode);
        match res {
            Ok(ix) => app2.install_index(ix),
            Err(e) => {
                if !progress.cancel.load(Relaxed) {
                    *app2.scan_error.lock().unwrap() = Some(e);
                }
            }
        }
        *app2.scan.lock().unwrap() = None;
    });
    Ok(json!({"ok": true, "root": root}))
}

/// Demo: Zeigt einen Scan-Ablauf mit Fortschritt, ohne etwas zu lesen.
fn simulate_scan(app: &Arc<App>, root: String, scale: u64) -> Res {
    let d = demo::drives().into_iter().find(|d| d.root == root).unwrap_or_default();
    let progress = Arc::new(ScanProgress::new(&root, d.total.saturating_sub(d.free), 16, &d.media));
    *app.scan.lock().unwrap() = Some(progress.clone());
    *app.scan_error.lock().unwrap() = None;
    let app2 = app.clone();
    std::thread::spawn(move || {
        let ix = demo::generate(&root, scale);
        let kids = ix.children_by_size(0);
        const STEPS: u64 = 36;
        for step in 1..=STEPS {
            if progress.cancel.load(Relaxed) {
                *app2.scan.lock().unwrap() = None;
                return;
            }
            std::thread::sleep(Duration::from_millis(140));
            let bytes = ix.nodes[0].alloc / STEPS * step;
            progress.bytes.store(bytes, Relaxed);
            progress.files.store(ix.nodes[0].files as u64 / STEPS * step, Relaxed);
            progress.dirs.store(ix.nodes[0].dirs as u64 / STEPS * step, Relaxed);
            let sample = kids[(step as usize) % kids.len().max(1)];
            *progress.current.lock().unwrap() = ix.path(sample);
            let mut tops = progress.tops.lock().unwrap();
            tops.clear();
            for k in kids.iter().take(14) {
                let n = &ix.nodes[*k as usize];
                tops.push(scan::Top { name: ix.name(*k).to_string(), bytes: n.alloc / STEPS * step, files: n.files as u64 / STEPS * step });
            }
        }
        *progress.phase.lock().unwrap() = "analyze".into();
        let _ = history::save_scan(&ix);
        app2.install_index(ix);
        *app2.scan.lock().unwrap() = None;
    });
    Ok(json!({"ok": true, "root": d.root}))
}

pub fn status(app: &App) -> Value {
    app.last_ping.store(now_secs(), Relaxed);
    let scan = app.scan.lock().unwrap().clone();
    let scan_json = scan.map(|p| {
        let tops = p.tops.lock().unwrap();
        let mut rows: Vec<Value> =
            tops.iter().filter(|t| t.bytes > 0 || t.files > 0).map(|t| json!({"name": t.name, "bytes": t.bytes, "files": t.files})).collect();
        rows.sort_by_key(|r| std::cmp::Reverse(r["bytes"].as_u64().unwrap_or(0)));
        rows.truncate(14);
        json!({
            "root": p.root,
            "phase": *p.phase.lock().unwrap(),
            "files": p.files.load(Relaxed),
            "dirs": p.dirs.load(Relaxed),
            "bytes": p.bytes.load(Relaxed),
            "errors": p.errors.load(Relaxed),
            "expected": p.expected,
            "threads": p.threads,
            "media": p.media,
            "elapsedMs": p.started.elapsed().as_millis() as u64,
            "current": *p.current.lock().unwrap(),
            "tops": rows,
        })
    });

    // Die Sperre nur kurz versuchen: Waehrend des Aufraeumens soll der Status nicht haengen.
    let guard = app.index.try_read();
    let index_json = match &guard {
        Ok(g) => g.as_ref().map(|ix| {
            json!({
                "root": ix.meta.root,
                "isDrive": ix.meta.is_drive,
                "scannedAt": ix.meta.scanned_at,
                "alloc": ix.nodes[0].alloc,
                "files": ix.nodes[0].files,
                "total": ix.meta.volume.total,
                "free": app.live_free.load(Relaxed),
                "media": ix.meta.volume.media,
                "kind": ix.meta.volume.kind,
            })
        }),
        Err(_) => None,
    };
    let (cart_count, cart_bytes) = match &guard {
        Ok(g) => match g.as_ref() {
            Some(ix) => {
                let c = app.cart.lock().unwrap();
                (c.ids.len(), c.bytes(ix))
            }
            None => (0, 0),
        },
        Err(_) => (0, 0),
    };
    drop(guard);

    let d = &app.dup;
    let cl = app.cleanup.lock().unwrap();
    json!({
        "gen": app.gen.load(Relaxed),
        "mode": app.mode.name(),
        "platform": platform::PLATFORM,
        "version": env!("CARGO_PKG_VERSION"),
        "exe": std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default(),
        "mcpLevel": config::load().0.mcp.level,
        "elevated": platform::is_elevated(),
        "canElevate": platform::CAN_RESTART_ELEVATED && app.mode.is_normal(),
        "index": index_json,
        "scan": scan_json,
        "scanError": *app.scan_error.lock().unwrap(),
        "dups": {
            "running": d.running.load(Relaxed),
            "done": d.done.load(Relaxed),
            "phase": *d.phase.lock().unwrap(),
            "totalFiles": d.total_files.load(Relaxed),
            "doneFiles": d.done_files.load(Relaxed),
            "totalBytes": d.total_bytes.load(Relaxed),
            "doneBytes": d.done_bytes.load(Relaxed),
            "skipped": d.skipped.load(Relaxed),
            "groups": d.groups.lock().unwrap().len(),
        },
        "cart": {"count": cart_count, "bytes": cart_bytes},
        "cleanup": {
            "running": cl.running,
            "mode": cl.mode,
            "total": cl.total,
            "done": cl.done,
            "last": cl.last,
        },
    })
}

fn query_map(q: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for part in q.split('&') {
        if part.is_empty() {
            continue;
        }
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        m.insert(url_decode(k), url_decode(v));
    }
    m
}

fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hi = (b[i + 1] as char).to_digit(16);
                let lo = (b[i + 2] as char).to_digit(16);
                if let (Some(h), Some(l)) = (hi, lo) {
                    out.push((h * 16 + l) as u8);
                    i += 2;
                } else {
                    out.push(b'%');
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn num<T: std::str::FromStr>(m: &HashMap<String, String>, k: &str, d: T) -> T {
    m.get(k).and_then(|v| v.parse().ok()).unwrap_or(d)
}

fn file_query(m: &HashMap<String, String>) -> FileQuery {
    FileQuery {
        under: num(m, "under", 0u32),
        q: m.get("q").cloned().unwrap_or_default(),
        cats: m.get("cat").map(|c| c.split(',').filter_map(|x| x.parse().ok()).collect()).unwrap_or_default(),
        ext: m.get("ext").cloned().unwrap_or_default(),
        min: num(m, "min", 0u64),
        max: num(m, "max", 0u64),
        mod_before: num(m, "mb", 0u32),
        mod_after: num(m, "ma", 0u32),
        acc_before: num(m, "ab", 0u32),
        kind: m.get("kind").cloned().unwrap_or_else(|| "file".into()),
        sort: m.get("sort").cloned().unwrap_or_else(|| "alloc".into()),
        desc: m.get("dir").map(|d| d != "asc").unwrap_or(true),
        offset: num(m, "offset", 0usize),
        limit: num(m, "limit", 200usize).min(5000),
    }
}

pub fn with_index<T>(app: &App, f: impl FnOnce(&Index) -> Result<T, (u16, String)>) -> Result<T, (u16, String)> {
    let g = app.index.read().unwrap();
    match g.as_ref() {
        Some(ix) => f(ix),
        None => err(404, "Noch kein Scan geladen."),
    }
}

fn check_id(ix: &Index, id: u32) -> Result<u32, (u16, String)> {
    match ix.nodes.get(id as usize) {
        Some(n) if !n.is_deleted() => Ok(id),
        _ => err(404, "Eintrag gibt es nicht mehr."),
    }
}

fn browse(app: &App, path: &str) -> Res {
    match &app.mode {
        RunMode::Normal => {}
        RunMode::Demo { .. } => {
            // Zum Testen des Ziel-Dialogs: erfundene Ordner, keine echten.
            if path.is_empty() {
                let rows: Vec<Value> = demo::drives().iter().map(|d| json!({"name": format!("{} ({})", d.root, d.label), "path": d.root})).collect();
                return Ok(json!({"path": "", "parent": Value::Null, "dirs": rows}));
            }
            let root = paths::normalize(path).map_err(|e| (400, e))?;
            let is_drive = demo::drives().iter().any(|d| d.root == root);
            let dirs = if is_drive { vec![json!({"name": "Auslagerung", "path": paths::join(&root, "Auslagerung")})] } else { vec![] };
            return Ok(json!({"path": root, "parent": paths::parent_of(&root).unwrap_or_default(), "dirs": dirs}));
        }
        RunMode::Sandbox(sb) => {
            if path.is_empty() {
                return Ok(json!({"path": "", "parent": Value::Null, "dirs": [{"name": "Testordner", "path": sb}]}));
            }
            let root = paths::normalize(path).map_err(|e| (400, e))?;
            if !inside(&root, sb) {
                return err(403, REFUSED);
            }
        }
    }
    if path.is_empty() {
        let drives = platform::list_drives(Duration::from_millis(1200));
        let rows: Vec<Value> = drives
            .iter()
            .filter(|d| d.ready)
            .map(|d| json!({"name": if d.label.is_empty() { d.root.clone() } else { format!("{} ({})", d.root, d.label) }, "path": d.root}))
            .collect();
        return Ok(json!({"path": "", "parent": Value::Null, "dirs": rows}));
    }
    let root = paths::normalize(path).map_err(|e| (400, e))?;
    let rd = std::fs::read_dir(platform::long_pathbuf(std::path::Path::new(&root))).map_err(|e| (400, format!("Ordner nicht lesbar: {e}")))?;
    let mut rows = vec![];
    for e in rd.flatten() {
        let Ok(t) = e.file_type() else { continue };
        if !t.is_dir() || t.is_symlink() {
            continue;
        }
        let name = e.file_name().to_string_lossy().to_string();
        rows.push(json!({"path": paths::join(&root, &name), "name": name}));
    }
    rows.sort_by_key(|r| r["name"].as_str().unwrap_or("").to_lowercase());
    Ok(json!({"path": root, "parent": paths::parent_of(&root).unwrap_or_default(), "dirs": rows}))
}

pub fn drives_json(app: &App) -> Value {
    let drives = match &app.mode {
        RunMode::Normal => platform::list_drives(Duration::from_millis(1500)),
        RunMode::Demo { .. } => demo::drives(),
        RunMode::Sandbox(sb) => vec![platform::DriveInfo {
            root: sb.clone(),
            label: "Testordner".into(),
            fs: "TEST".into(),
            kind: "fixed".into(),
            media: "ssd".into(),
            ready: true,
            ..Default::default()
        }],
    };
    let known = history::known_scans();
    let rows: Vec<Value> = drives
        .iter()
        .map(|d| {
            let last = known.iter().find(|k| k["root"] == d.root.as_str());
            let mut v = serde_json::to_value(d).unwrap();
            v["lastScan"] = last.map(|l| l["ts"].clone()).unwrap_or(Value::Null);
            v
        })
        .collect();
    let folders: Vec<Value> = known.into_iter().filter(|k| k["isDrive"] != true).collect();
    json!({"drives": rows, "folders": if app.mode.is_normal() { folders } else { vec![] }, "elevated": platform::is_elevated()})
}

pub fn start_dups(app: &Arc<App>, body: &Value) -> Res {
    if app.is_busy() {
        return err(409, "Es läuft noch ein anderer Vorgang.");
    }
    let min_size = body["minSize"].as_u64().unwrap_or(1 << 20);
    let quick = body["quick"].as_bool().unwrap_or(false);
    let include_system = body["includeSystem"].as_bool().unwrap_or(false);
    if matches!(app.mode, RunMode::Demo { .. }) {
        return simulate_dups(app, min_size, include_system);
    }
    let (media, cands_exist) = with_index(app, |ix| Ok((ix.meta.volume.media.clone(), !ix.nodes.is_empty())))?;
    if !cands_exist {
        return err(400, "Nichts zu prüfen.");
    }
    let threads = match media.as_str() {
        "hdd" => 2,
        "net" => 4,
        _ => std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(12),
    };
    let params = DupParams { min_size, quick, include_system, threads };
    app.dup.reset();
    app.dup.running.store(true, Relaxed);
    app.dup.quick.store(quick, Relaxed);
    app.dup.started.store(now_secs(), Relaxed);
    *app.dup.phase.lock().unwrap() = "collect".into();
    let app2 = app.clone();
    std::thread::spawn(move || {
        let eng = app2.dup.clone();
        let root = app2.index.read().unwrap().as_ref().map(|ix| ix.meta.root.clone());
        dupes::run(
            |p| {
                let g = app2.index.read().unwrap();
                match g.as_ref() {
                    Some(ix) => dupes::candidates(ix, p),
                    None => vec![],
                }
            },
            params,
            &eng,
        );
        eng.finished.store(now_secs(), Relaxed);
        eng.running.store(false, Relaxed);
        eng.done.store(true, Relaxed);
        if let Some(r) = root {
            if !eng.cancel.load(Relaxed) {
                if let Ok(t) = serde_json::to_string(&*eng.groups.lock().unwrap()) {
                    let file = history::scan_dir(&r).join("dups.json");
                    if std::fs::write(&file, t).is_ok() {
                        *app2.dups_seen.lock().unwrap() = modified(&file);
                    }
                }
            }
        }
        app2.gen.fetch_add(1, Relaxed);
    });
    Ok(json!({"ok": true}))
}

/// Demo: Fortschritt zeigen und ein vorbereitetes Ergebnis liefern. Es gibt keine Inhalte zum Vergleichen.
fn simulate_dups(app: &Arc<App>, min_size: u64, include_system: bool) -> Res {
    let groups = with_index(app, |ix| Ok(demo::dup_groups(ix, min_size, include_system)))?;
    app.dup.reset();
    app.dup.running.store(true, Relaxed);
    let app2 = app.clone();
    std::thread::spawn(move || {
        let eng = &app2.dup;
        let bytes: u64 = groups.iter().map(|g| g.size * g.ids.len() as u64).sum();
        let files: u64 = groups.iter().map(|g| g.ids.len() as u64).sum();
        eng.total_bytes.store(bytes, Relaxed);
        eng.total_files.store(files, Relaxed);
        for (phase, steps) in [("partial", 8u64), ("full", 20)] {
            *eng.phase.lock().unwrap() = phase.into();
            for i in 1..=steps {
                if eng.cancel.load(Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(120));
                eng.done_bytes.store(bytes / steps * i, Relaxed);
                eng.done_files.store(files / steps * i, Relaxed);
            }
        }
        *eng.groups.lock().unwrap() = groups;
        eng.running.store(false, Relaxed);
        eng.done.store(true, Relaxed);
        app2.gen.fetch_add(1, Relaxed);
    });
    Ok(json!({"ok": true}))
}

fn dups_json(app: &App, m: &HashMap<String, String>) -> Res {
    let offset = num(m, "offset", 0usize);
    let limit = num(m, "limit", 50usize).min(500);
    with_index(app, |ix| {
        let groups = app.dup.groups.lock().unwrap();
        let mut alive: Vec<(&DupGroup, Vec<u32>)> = groups
            .iter()
            .map(|g| (g, g.ids.iter().copied().filter(|i| !ix.nodes[*i as usize].is_deleted()).collect::<Vec<_>>()))
            .filter(|(_, ids)| ids.len() >= 2)
            .collect();
        alive.sort_by_key(|(g, ids)| std::cmp::Reverse(g.alloc * (ids.len() as u64 - 1)));
        let total_wasted: u64 = alive.iter().map(|(g, ids)| g.alloc * (ids.len() as u64 - 1)).sum();
        let total_files: usize = alive.iter().map(|(_, ids)| ids.len()).sum();
        let rows: Vec<Value> = alive
            .iter()
            .skip(offset)
            .take(limit)
            .map(|(g, ids)| {
                json!({
                    "size": g.size,
                    "alloc": g.alloc,
                    "wasted": g.alloc * (ids.len() as u64 - 1),
                    "verified": g.verified,
                    "files": ids.iter().map(|i| analysis::row(ix, *i)).collect::<Vec<_>>(),
                })
            })
            .collect();
        Ok(json!({"total": alive.len(), "wasted": total_wasted, "files": total_files, "rows": rows}))
    })
}

fn dup_folders_json(app: &App, m: &HashMap<String, String>) -> Res {
    let min = num(m, "min", 50u64 << 20);
    let system = m.get("system").map(|s| s == "1").unwrap_or(false);
    with_index(app, |ix| {
        let gen = app.gen.load(Relaxed);
        let groups = {
            let mut slot = app.dup_folders.lock().unwrap();
            match slot.as_ref() {
                Some((g, v)) if *g == gen && min == 50 << 20 && !system => v.clone(),
                _ => {
                    let v = Arc::new(dupes::duplicate_folders(ix, min, system));
                    if min == 50 << 20 && !system {
                        *slot = Some((gen, v.clone()));
                    }
                    v
                }
            }
        };
        let wasted: u64 = groups.iter().map(|g| g.alloc * (g.ids.len() as u64 - 1)).sum();
        let rows: Vec<Value> = groups
            .iter()
            .take(100)
            .map(|g| {
                json!({
                    "alloc": g.alloc,
                    "fileCount": g.files,
                    "wasted": g.alloc * (g.ids.len() as u64 - 1),
                    "folders": g.ids.iter().map(|i| analysis::row(ix, *i)).collect::<Vec<_>>(),
                })
            })
            .collect();
        Ok(json!({"total": groups.len(), "wasted": wasted, "rows": rows}))
    })
}

fn junk_json(app: &App) -> Res {
    with_index(app, |ix| {
        let rep = app.junk_report(ix);
        let rules: Vec<Value> = junk::rules()
            .iter()
            .zip(&rep.results)
            .filter(|(_, r)| !r.items.is_empty())
            .map(|(rule, r)| {
                json!({
                    "id": rule.id,
                    "group": rule.group,
                    "title": rule.title,
                    "why": rule.why,
                    "risk": rule.risk,
                    "action": rule.action,
                    "alloc": r.alloc,
                    "files": r.files,
                    "items": r.items.len(),
                })
            })
            .collect();
        let bin = match app.mode {
            RunMode::Normal => platform::volume_root(&ix.meta.root).and_then(|r| platform::recycle_bin_info(&r)),
            RunMode::Demo { .. } => {
                if ix.meta.volume.kind == "network" {
                    None
                } else {
                    let totals = ["recycle", "trash", "trash-volume"]
                        .iter()
                        .filter_map(|r| junk::rule_by_id(r))
                        .map(|ri| (rep.results[ri].alloc, rep.results[ri].files));
                    Some(totals.fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1)))
                }
            }
            RunMode::Sandbox(_) => None,
        };
        Ok(json!({
            "rules": rules,
            "emptyDirs": rep.empty_dirs.len(),
            "recycleBin": bin.map(|(s, n)| json!({"bytes": s, "items": n})),
            "safeBytes": rep.safe_bytes(),
        }))
    })
}

fn junk_items(app: &App, m: &HashMap<String, String>) -> Res {
    let rule = m.get("rule").cloned().unwrap_or_default();
    let offset = num(m, "offset", 0usize);
    let limit = num(m, "limit", 100usize).min(1000);
    with_index(app, |ix| {
        let rep = app.junk_report(ix);
        let items: &[u32] = if rule == "empty" {
            &rep.empty_dirs
        } else {
            let Some(ri) = junk::rule_by_id(&rule) else { return err(404, "Unbekannte Regel") };
            &rep.results[ri].items
        };
        let rows: Vec<Value> = items.iter().skip(offset).take(limit).map(|i| analysis::row(ix, *i)).collect();
        Ok(json!({"total": items.len(), "rows": rows}))
    })
}

fn cart_json(app: &App, m: &HashMap<String, String>) -> Res {
    let limit = num(m, "limit", 1000usize).min(5000);
    with_index(app, |ix| {
        let c = app.cart.lock().unwrap();
        let mut ids: Vec<u32> = c.ids.iter().copied().collect();
        ids.sort_by_key(|i| std::cmp::Reverse(ix.nodes[*i as usize].alloc));
        let rows: Vec<Value> = ids.iter().take(limit).map(|i| analysis::row(ix, *i)).collect();
        let warn = ids.iter().filter(|i| matches!(guard::check(&ix.path(**i)), Guard::Warn(_))).count();
        Ok(json!({"count": ids.len(), "bytes": c.bytes(ix), "warnings": warn, "rows": rows}))
    })
}

fn cart_add(app: &App, body: &Value) -> Res {
    with_index(app, |ix| {
        let mut ids: Vec<u32> = body["ids"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64().map(|x| x as u32)).collect()).unwrap_or_default();
        if let Some(rule) = body["rule"].as_str() {
            let rep = app.junk_report(ix);
            if rule == "empty" {
                ids.extend(&rep.empty_dirs);
            } else if let Some(ri) = junk::rule_by_id(rule) {
                ids.extend(&rep.results[ri].items);
            }
        }
        if body["safeRules"].as_bool().unwrap_or(false) {
            let rep = app.junk_report(ix);
            for (rule, res) in junk::rules().iter().zip(&rep.results) {
                if rule.risk == junk::Risk::Safe && rule.action == junk::Action::None {
                    ids.extend(&res.items);
                }
            }
        }
        let reply = {
            let mut cart = app.cart.lock().unwrap();
            let before = cart.ids.len();
            let r = cart.add(ix, &ids);
            json!({
                "added": r.added,
                "count": cart.ids.len(),
                "countBefore": before,
                "bytes": cart.bytes(ix),
                "blocked": r.blocked.iter().map(|(p, w)| json!({"path": p, "why": w})).collect::<Vec<_>>(),
            })
        };
        app.persist_cart(ix);
        Ok(reply)
    })
}

pub fn start_cleanup(app: &Arc<App>, body: &Value) -> Res {
    if app.is_busy() {
        return err(409, "Es läuft noch ein anderer Vorgang.");
    }
    let mode = match body["mode"].as_str() {
        Some("recycle") => Mode::Recycle,
        Some("permanent") => Mode::Permanent,
        Some("move") => Mode::Move,
        _ => return err(400, "Unbekannter Modus."),
    };
    if mode == Mode::Permanent && body["confirm"].as_str() != Some("LÖSCHEN") {
        return err(400, "Endgültiges Löschen muss mit dem Wort LÖSCHEN bestätigt werden.");
    }
    let dest = body["dest"].as_str().unwrap_or("").to_string();
    if mode == Mode::Move {
        let d = paths::normalize(&dest).map_err(|e| (400, e))?;
        if let RunMode::Sandbox(sb) = &app.mode {
            if !inside(&d, sb) {
                return err(403, REFUSED);
            }
        }
        if !matches!(app.mode, RunMode::Demo { .. }) && !std::path::Path::new(&d).is_dir() {
            return err(400, "Zielordner gibt es nicht.");
        }
    }
    let items = with_index(app, |ix| {
        let ids: Vec<u32> = app.cart.lock().unwrap().ids.iter().copied().collect();
        Ok(cleanup::snapshot(ix, &ids))
    })?;
    if items.is_empty() {
        return err(400, "Die Aufräumliste ist leer.");
    }
    if let RunMode::Sandbox(sb) = &app.mode {
        if items.iter().any(|i| !inside(&i.display, sb)) {
            return err(403, REFUSED);
        }
    }
    if mode == Mode::Recycle {
        // Netzlaufwerke haben keinen Papierkorb. Dort wuerde "Papierkorb" in Wahrheit endgueltig loeschen.
        let (root, kind) = with_index(app, |ix| Ok((ix.meta.root.clone(), ix.meta.volume.kind.clone())))?;
        let has_bin = match app.mode {
            RunMode::Normal => platform::volume_root(&root).and_then(|r| platform::recycle_bin_info(&r)).is_some(),
            RunMode::Demo { .. } => kind != "network",
            RunMode::Sandbox(_) => true,
        };
        if !has_bin {
            return err(400, "Dieses Laufwerk hat keinen Papierkorb. Bitte verschieben oder endgültig löschen.");
        }
    }
    {
        let mut c = app.cleanup.lock().unwrap();
        *c = CleanupState { running: true, mode: Some(mode), total: items.len(), ..Default::default() };
    }
    let app2 = app.clone();
    std::thread::spawn(move || {
        let mut removed: Vec<u32> = vec![];
        let mut report = |r: ItemResult, id: Option<u32>| {
            if let Some(id) = id {
                removed.push(id);
            }
            let mut c = app2.cleanup.lock().unwrap();
            c.done += 1;
            c.results.push(r);
        };
        let d = paths::normalize(&dest).unwrap_or(dest.clone());
        let interactive = !app2.headless.load(Relaxed);
        let bytes_before: u64 = items.iter().map(|i| i.alloc).sum();
        match (&app2.mode, mode) {
            (RunMode::Demo { .. }, _) => cleanup::simulated_all(&items, &mut report),
            (RunMode::Sandbox(_), Mode::Recycle) => {
                cleanup::sandbox_recycle_all(&items, &history::data_dir().join("sandbox-papierkorb"), &mut report)
            }
            (RunMode::Sandbox(_), Mode::Move) => cleanup::sandbox_move_all(&items, std::path::Path::new(&d), &mut report),
            (_, Mode::Permanent) => cleanup::permanent_all(&items, &mut report),
            (RunMode::Normal, Mode::Recycle) => cleanup::recycle_all(&items, &mut report),
            (RunMode::Normal, Mode::Move) => cleanup::move_all(&items, &d, interactive, &mut report),
        }
        if matches!(app2.mode, RunMode::Demo { .. }) && mode != Mode::Recycle {
            app2.live_free.fetch_add(bytes_before, Relaxed);
        }
        {
            let mut g = app2.index.write().unwrap();
            if let Some(ix) = g.as_mut() {
                for id in &removed {
                    ix.mark_deleted(*id);
                }
                // Auch der gespeicherte Stand soll das wissen, sonst tauchen geloeschte Dateien beim naechsten Start wieder auf.
                let file = history::scan_dir(&ix.meta.root).join("latest.smidx");
                if ix.save(&file).is_ok() {
                    *app2.index_seen.lock().unwrap() = modified(&file);
                }
                if app2.mode.is_normal() {
                    if let Some(r) = platform::volume_root(&ix.meta.root) {
                        let v = platform::volume_info(&r);
                        app2.live_free.store(v.free, Relaxed);
                    }
                }
            }
        }
        {
            let mut cart = app2.cart.lock().unwrap();
            for id in &removed {
                cart.ids.remove(id);
            }
        }
        if let Some(ix) = app2.index.read().unwrap().as_ref() {
            app2.persist_cart(ix);
        }
        app2.gen.fetch_add(1, Relaxed);
        let mut c = app2.cleanup.lock().unwrap();
        let mut last = cleanup::results_json(&c.results);
        last["mode"] = json!(c.mode);
        c.last = Some(last);
        c.running = false;
        c.results.clear();
    });
    Ok(json!({"ok": true}))
}

fn csv_escape(s: &str) -> String {
    // Fuehrende =,+,-,@ wuerden Excel als Formel ausfuehren.
    let guarded = if s.starts_with(['=', '+', '-', '@']) { format!("'{s}") } else { s.to_string() };
    format!("\"{}\"", guarded.replace('"', "\"\""))
}

pub fn fmt_date(ts: u32) -> String {
    if ts == 0 {
        return String::new();
    }
    let days = (ts / 86_400) as i64;
    // Datumsumrechnung nach Howard Hinnant (civil_from_days).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

fn export(app: &App, m: &HashMap<String, String>) -> Result<(String, String), (u16, String)> {
    let what = m.get("what").map(|s| s.as_str()).unwrap_or("files");
    with_index(app, |ix| {
        let mut out = String::from("\u{feff}Pfad;Typ;Belegt (Bytes);Größe (Bytes);Geändert;Geöffnet;Erstellt;Dateien\r\n");
        let mut push = |id: u32| {
            let n = &ix.nodes[id as usize];
            out.push_str(&format!(
                "{};{};{};{};{};{};{};{}\r\n",
                csv_escape(&ix.path(id)),
                if n.is_dir() { "Ordner" } else { "Datei" },
                n.alloc,
                n.size,
                fmt_date(if n.is_dir() { n.newest } else { n.mtime }),
                fmt_date(n.atime),
                fmt_date(n.ctime),
                if n.is_dir() { n.files } else { 1 },
            ));
        };
        let name = match what {
            "cart" => {
                let ids: Vec<u32> = app.cart.lock().unwrap().ids.iter().copied().collect();
                for id in ids {
                    push(id);
                }
                "storage-map-aufraeumliste.csv"
            }
            "dups" => {
                let groups = app.dup.groups.lock().unwrap();
                for g in groups.iter() {
                    for id in &g.ids {
                        if !ix.nodes[*id as usize].is_deleted() {
                            push(*id);
                        }
                    }
                }
                "storage-map-duplikate.csv"
            }
            _ => {
                let mut q = file_query(m);
                q.offset = 0;
                let (ids, _) = analysis::run_query(ix, &q);
                for id in ids.iter().take(1_000_000) {
                    push(*id);
                }
                "storage-map-dateien.csv"
            }
        };
        Ok((out, name.to_string()))
    })
}

fn mcp_info(app: &App) -> Value {
    let (cfg, error) = config::load();
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| "storage-map".into());
    let mut args = vec!["--mcp".to_string()];
    match &app.mode {
        RunMode::Normal => {}
        RunMode::Demo { .. } => args.push("--demo".to_string()),
        RunMode::Sandbox(sb) => {
            args.push("--sandbox".to_string());
            args.push(sb.clone());
        }
    }
    if history::has_custom_data_dir() {
        args.push("--data-dir".to_string());
        args.push(history::data_dir().to_string_lossy().to_string());
    }
    json!({
        "config": cfg.mcp,
        "error": error,
        "configPath": config::path().to_string_lossy(),
        "auditPath": mcp::audit_path().to_string_lossy(),
        "command": exe,
        "args": args,
        "tools": mcp::catalog(),
    })
}

/// Aendert die MCP-Rechte. Mehr Rechte gibt es nur mit ausdruecklicher Bestaetigung, auch die prueft das Backend.
fn mcp_set_config(body: &Value) -> Res {
    let (mut cfg, _) = config::load();
    let old = cfg.mcp.clone();
    if let Some(l) = body.get("level") {
        cfg.mcp.level = serde_json::from_value(l.clone()).map_err(|_| (400, "Unbekannte Stufe.".to_string()))?;
    }
    if let Some(v) = body["allowScan"].as_bool() {
        cfg.mcp.allow_scan = v;
    }
    if let Some(v) = body["allowPermanent"].as_bool() {
        cfg.mcp.allow_permanent_delete = v;
    }
    if let Some(a) = body["roots"].as_array() {
        cfg.mcp.roots = a.iter().filter_map(|v| v.as_str()).filter_map(|r| paths::normalize(r).ok()).collect();
    }
    if cfg.mcp.level < Level::Delete {
        cfg.mcp.allow_permanent_delete = false;
    }
    let raises_write = cfg.mcp.level >= Level::Write && old.level < Level::Write;
    let raises_delete = cfg.mcp.level >= Level::Delete && old.level < Level::Delete;
    let raises_permanent = cfg.mcp.allow_permanent_delete && !old.allow_permanent_delete;
    let confirm = body["confirm"].as_str().unwrap_or("");
    if (raises_delete || raises_permanent) && confirm != "LÖSCHEN" {
        return err(400, "Löschrechte müssen mit dem Wort LÖSCHEN bestätigt werden.");
    }
    if raises_write && !raises_delete && confirm != "SCHREIBEN" {
        return err(400, "Schreibrechte müssen mit dem Wort SCHREIBEN bestätigt werden.");
    }
    config::save(&cfg).map_err(|e| (500, format!("Einstellungen nicht gespeichert: {e}")))?;
    Ok(json!({"ok": true, "config": cfg.mcp}))
}

pub fn route(app: &Arc<App>, method: &Method, path: &str, m: &HashMap<String, String>, body: &Value) -> Res {
    let get = *method == Method::Get;
    match path {
        "/api/status" => Ok(status(app)),
        "/api/session/new" if !get => Ok(json!({"code": app.new_session_code()})),
        "/api/mcp" => Ok(mcp_info(app)),
        "/api/mcp/config" if !get => mcp_set_config(body),
        "/api/mcp/audit" => Ok(mcp::audit_tail(num(m, "limit", 200usize).min(2000))),
        "/api/drives" => Ok(drives_json(app)),
        "/api/browse" => browse(app, m.get("path").map(|s| s.as_str()).unwrap_or("")),
        "/api/scan" if !get => start_scan(app, body["path"].as_str().unwrap_or("")),
        "/api/scan/cancel" if !get => {
            if let Some(p) = app.scan.lock().unwrap().as_ref() {
                p.cancel.store(true, Relaxed);
            }
            Ok(json!({"ok": true}))
        }
        "/api/open-cached" if !get => {
            if app.is_busy() {
                return err(409, "Es läuft noch ein anderer Vorgang.");
            }
            let root = paths::normalize(body["path"].as_str().unwrap_or("")).map_err(|e| (400, e))?;
            if !app.mode.allows(&root) {
                return err(403, REFUSED);
            }
            let ix = history::load_latest(&root).map_err(|e| (404, format!("Kein gespeicherter Scan: {e}")))?;
            app.install_index(ix);
            // Live-Wert nach dem Laden aktualisieren: Der Scan kann alt sein.
            if app.mode.is_normal() {
                if let Some(r) = platform::volume_root(&root) {
                    let v = platform::volume_info(&r);
                    app.live_free.store(v.free, Relaxed);
                }
            }
            Ok(json!({"ok": true}))
        }
        "/api/close" if !get => {
            if app.is_busy() {
                return err(409, "Es läuft noch ein anderer Vorgang.");
            }
            *app.index.write().unwrap() = None;
            app.cart.lock().unwrap().ids.clear();
            app.dup.reset();
            app.gen.fetch_add(1, Relaxed);
            Ok(json!({"ok": true}))
        }
        "/api/summary" => with_index(app, |ix| {
            let gen = app.gen.load(Relaxed);
            let mut slot = app.summary.lock().unwrap();
            if let Some((g, v)) = slot.as_ref() {
                if *g == gen {
                    return Ok((**v).clone());
                }
            }
            let rep = app.junk_report(ix);
            let (w, verified) = app.dup_totals(ix);
            let v = analysis::summary(ix, &rep, w, verified);
            *slot = Some((gen, Arc::new(v.clone())));
            Ok(v)
        }),
        "/api/node" => with_index(app, |ix| {
            let id = check_id(ix, num(m, "id", 0u32))?;
            Ok(analysis::node_detail(ix, id))
        }),
        "/api/children" => with_index(app, |ix| {
            let id = check_id(ix, num(m, "id", 0u32))?;
            Ok(analysis::children(
                ix,
                id,
                m.get("sort").map(|s| s.as_str()).unwrap_or("alloc"),
                m.get("dir").map(|d| d != "asc").unwrap_or(true),
                num(m, "offset", 0usize),
                num(m, "limit", 500usize).min(5000),
            ))
        }),
        "/api/map" => with_index(app, |ix| {
            let id = check_id(ix, num(m, "id", 0u32))?;
            Ok(analysis::map(ix, id, num(m, "limit", 6000usize).min(30_000), num(m, "minfrac", 0.00002f64), num(m, "maxdepth", 64u32)))
        }),
        "/api/files" => with_index(app, |ix| {
            let q = file_query(m);
            check_id(ix, q.under)?;
            Ok(analysis::files_json(ix, &q))
        }),
        "/api/types" => with_index(app, |ix| Ok(analysis::types(ix))),
        "/api/stale" => with_index(app, |ix| {
            let cutoff = num(m, "cutoff", 0u32);
            Ok(analysis::stale_dirs(ix, cutoff, num(m, "min", 1u64 << 20), num(m, "limit", 100usize).min(1000)))
        }),
        "/api/denied" => {
            with_index(app, |ix| Ok(json!({"total": ix.meta.denied.len(), "rows": ix.meta.denied.iter().take(1000).collect::<Vec<_>>()})))
        }
        "/api/junk" => junk_json(app),
        "/api/junk/items" => junk_items(app, m),
        "/api/dups/start" if !get => start_dups(app, body),
        "/api/dups/cancel" if !get => {
            app.dup.cancel.store(true, Relaxed);
            Ok(json!({"ok": true}))
        }
        "/api/dups" => dups_json(app, m),
        "/api/dupfolders" => dup_folders_json(app, m),
        "/api/history" => with_index(app, |ix| {
            let e = history::read_entries(&ix.meta.root);
            Ok(json!({"entries": e, "catNames": crate::category::NAMES}))
        }),
        "/api/diff" => {
            with_index(app, |ix| history::diff(ix, num(m, "ts", 0u64)).ok_or((404, "Für diesen Scan gibt es keine Vergleichsdaten.".to_string())))
        }
        "/api/cart" => cart_json(app, m),
        "/api/cart/add" if !get => cart_add(app, body),
        "/api/cart/remove" if !get => with_index(app, |ix| {
            let count = {
                let mut c = app.cart.lock().unwrap();
                if let Some(a) = body["ids"].as_array() {
                    for v in a {
                        if let Some(x) = v.as_u64() {
                            c.ids.remove(&(x as u32));
                        }
                    }
                }
                c.ids.len()
            };
            app.persist_cart(ix);
            Ok(json!({ "count": count }))
        }),
        "/api/cart/clear" if !get => with_index(app, |ix| {
            app.cart.lock().unwrap().ids.clear();
            app.persist_cart(ix);
            Ok(json!({"ok": true}))
        }),
        "/api/cleanup/start" if !get => start_cleanup(app, body),
        "/api/cleanup/ack" if !get => {
            app.cleanup.lock().unwrap().last = None;
            Ok(json!({"ok": true}))
        }
        "/api/reveal" if !get && !app.mode.is_normal() => Ok(json!({"ok": true, "simulated": true})),
        "/api/open" if !get && !app.mode.is_normal() => Ok(json!({"ok": true, "simulated": true})),
        "/api/tool/cleanmgr" if !get && !app.mode.is_normal() => Ok(json!({"ok": true, "simulated": true})),
        "/api/elevate" if !get && !app.mode.is_normal() => err(400, "Im Test-Betrieb nicht verfügbar."),
        "/api/recycle/empty" if !get && matches!(app.mode, RunMode::Sandbox(_)) => Ok(json!({"ok": true, "simulated": true})),
        "/api/recycle/empty" if !get && matches!(app.mode, RunMode::Demo { .. }) => {
            let mut g = app.index.write().unwrap();
            let Some(ix) = g.as_mut() else { return err(404, "Noch kein Scan geladen.") };
            if let Some(b) = demo::trash_node(ix) {
                app.live_free.fetch_add(ix.nodes[b as usize].alloc, Relaxed);
                ix.mark_deleted(b);
                app.gen.fetch_add(1, Relaxed);
            }
            Ok(json!({"ok": true, "simulated": true}))
        }
        "/api/health" if matches!(app.mode, RunMode::Demo { .. }) => Ok(demo::health(m.get("root").map(|s| s.as_str()).unwrap_or(""))),
        "/api/health" if !app.mode.is_normal() => Ok(json!({"unavailable": true})),
        "/api/reveal" if !get => with_index(app, |ix| {
            let id = check_id(ix, body["id"].as_u64().unwrap_or(0) as u32)?;
            platform::reveal(&ix.path(id)).map_err(|e| (500, e.to_string()))?;
            Ok(json!({"ok": true}))
        }),
        "/api/open" if !get => with_index(app, |ix| {
            let id = check_id(ix, body["id"].as_u64().unwrap_or(0) as u32)?;
            let n = &ix.nodes[id as usize];
            let ext = ix.meta.exts.get(n.ext as usize).map(|s| s.as_str()).unwrap_or("");
            const RUNNABLE: &[&str] = &[
                "exe", "bat", "cmd", "com", "msi", "ps1", "vbs", "js", "scr", "lnk", "reg", "hta", "jar", "wsf", "sh", "bash", "zsh", "fish", "run",
                "appimage", "desktop", "deb", "rpm", "py", "pl", "rb",
            ];
            if !n.is_dir() && RUNNABLE.contains(&ext) {
                return err(400, "Programme werden hier aus Sicherheitsgründen nicht gestartet. Bitte im Dateimanager öffnen.");
            }
            if !platform::shell_open(&ix.path(id)) {
                return err(500, "Konnte nicht geöffnet werden.");
            }
            Ok(json!({"ok": true}))
        }),
        "/api/recycle/empty" if !get => {
            let root = with_index(app, |ix| Ok(platform::volume_root(&ix.meta.root)))?;
            let Some(r) = root else { return err(400, "Kein Laufwerk.") };
            let hr = platform::empty_recycle_bin(&r);
            app.live_free.store(platform::volume_info(&r).free, Relaxed);
            Ok(json!({"ok": hr == 0, "code": hr}))
        }
        "/api/tool/cleanmgr" if !get => {
            let root = with_index(app, |ix| Ok(platform::volume_root(&ix.meta.root)))?;
            let d = root.unwrap_or_else(|| "C:\\".into());
            let _ = std::process::Command::new("cleanmgr.exe").arg(format!("/d{}", &d[..1])).spawn();
            Ok(json!({"ok": true}))
        }
        "/api/health" => {
            let root = m.get("root").cloned().unwrap_or_default();
            let cached = app.health.lock().unwrap().get(&root).cloned();
            if let Some(v) = cached {
                return Ok(v);
            }
            let app2 = app.clone();
            app.health.lock().unwrap().insert(root.clone(), json!({"pending": true}));
            std::thread::spawn(move || {
                let v = platform::disk_health(&root).unwrap_or(json!({"unavailable": true}));
                app2.health.lock().unwrap().insert(root, v);
            });
            Ok(json!({"pending": true}))
        }
        "/api/settings" => Ok(json!({
            "lastAccessTracking": if app.mode.is_normal() {
                let root = app.index.read().unwrap().as_ref().map(|ix| ix.meta.root.clone());
                platform::last_access_tracking(root.as_deref())
            } else {
                Some(true)
            },
            "elevated": platform::is_elevated(),
            "version": env!("CARGO_PKG_VERSION"),
            "dataDir": history::data_dir().to_string_lossy(),
        })),
        "/api/elevate" if !get => {
            if app.is_busy() {
                return err(409, "Es läuft noch ein anderer Vorgang.");
            }
            if platform::restart_elevated("") {
                app.quitting.store(true, Relaxed);
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(400));
                    std::process::exit(0);
                });
                Ok(json!({"ok": true}))
            } else {
                err(400, "Adminrechte wurden nicht erteilt.")
            }
        }
        "/api/quit" if !get => {
            app.quitting.store(true, Relaxed);
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(200));
                std::process::exit(0);
            });
            Ok(json!({"ok": true}))
        }
        _ => err(404, "Unbekannt"),
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

fn respond_json(rq: Request, code: u16, v: &Value) {
    let body = serde_json::to_vec(v).unwrap_or_default();
    let r = Response::from_data(body)
        .with_status_code(code)
        .with_header(header("Content-Type", "application/json; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("X-Content-Type-Options", "nosniff"));
    let _ = rq.respond(r);
}

fn handle(app: &Arc<App>, mut rq: Request) {
    let url = rq.url().to_string();
    let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
    let method = rq.method().clone();

    // Schutz gegen DNS-Rebinding: Nur Anfragen an die eigene Adresse beantworten.
    let host_ok = rq.headers().iter().any(|h| {
        h.field.equiv("Host") && {
            let v = h.value.as_str();
            v == format!("127.0.0.1:{}", app.port) || v == format!("localhost:{}", app.port)
        }
    });
    if !host_ok {
        let _ = rq.respond(Response::from_string("Forbidden").with_status_code(403));
        return;
    }

    if path == "/health" {
        let _ = rq.respond(Response::from_string("storage-map"));
        return;
    }

    if path == "/session" && method == Method::Post {
        let mut raw = String::new();
        let _ = rq.as_reader().take(4096).read_to_string(&mut raw);
        let code = serde_json::from_str::<Value>(&raw).ok().and_then(|v| v["code"].as_str().map(String::from)).unwrap_or_default();
        if app.redeem_session_code(&code) {
            respond_json(rq, 200, &json!({"token": app.token}));
        } else {
            respond_json(rq, 401, &json!({"error": "Der Code ist abgelaufen. Bitte Storage Map neu starten."}));
        }
        return;
    }

    if path.starts_with("/api/") {
        // Das Token kommt als eigener Header. Fremde Webseiten koennen so einen Header
        // nur nach CORS-Freigabe senden, und die gibt es hier nicht.
        let qm = query_map(query);
        let token_ok = rq.headers().iter().any(|h| h.field.equiv("X-SM-Token") && h.value.as_str() == app.token)
            || (path == "/api/export" && qm.get("t").map(|t| *t == app.token).unwrap_or(false));
        if !token_ok {
            respond_json(rq, 401, &json!({"error": "Nicht angemeldet. Bitte Storage Map neu starten."}));
            return;
        }
        if path == "/api/export" {
            match export(app, &qm) {
                Ok((text, name)) => {
                    let r = Response::from_data(text.into_bytes())
                        .with_header(header("Content-Type", "text/csv; charset=utf-8"))
                        .with_header(header("Content-Disposition", &format!("attachment; filename=\"{name}\"")));
                    let _ = rq.respond(r);
                }
                Err((c, e)) => respond_json(rq, c, &json!({"error": e})),
            }
            return;
        }
        let mut raw = String::new();
        let body: Value = if method == Method::Post {
            let _ = rq.as_reader().take(32 * 1024 * 1024).read_to_string(&mut raw);
            serde_json::from_str(&raw).unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| route(app, &method, path, &qm, &body)));
        match res {
            Ok(Ok(v)) => respond_json(rq, 200, &v),
            Ok(Err((c, e))) => respond_json(rq, c, &json!({"error": e})),
            Err(_) => respond_json(rq, 500, &json!({"error": "Interner Fehler. Bitte erneut versuchen."})),
        }
        return;
    }

    // Statische Dateien der Oberflaeche
    let rel = if path == "/" { "index.html" } else { path.trim_start_matches('/') };
    if rel.contains("..") {
        let _ = rq.respond(Response::from_string("Forbidden").with_status_code(403));
        return;
    }
    let data: Option<Vec<u8>> = match &app.ui_dir {
        Some(d) => std::fs::read(d.join(rel)).ok(),
        None => assets::get(rel).map(|b| b.to_vec()),
    };
    match data {
        Some(bytes) => {
            let r = Response::from_data(bytes)
                .with_header(header("Content-Type", assets::content_type(rel)))
                .with_header(header("Cache-Control", "no-store"))
                .with_header(header("X-Content-Type-Options", "nosniff"))
                .with_header(header(
                    "Content-Security-Policy",
                    "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; frame-ancestors 'none'",
                ));
            let _ = rq.respond(r);
        }
        None => {
            let _ = rq.respond(Response::from_string("Nicht gefunden").with_status_code(404));
        }
    }
}

/// Hintergrundarbeit: Aenderungen eines MCP-Clients aufnehmen, freien Platz aktuell halten.
fn ticker(app: Arc<App>) {
    let mut tick = 0u32;
    loop {
        std::thread::sleep(Duration::from_secs(2));
        tick = tick.wrapping_add(1);
        app.sync_from_disk();
        if !app.mode.is_normal() || tick % 3 != 0 {
            continue;
        }
        let root = app.index.try_read().ok().and_then(|g| g.as_ref().and_then(|ix| platform::volume_root(&ix.meta.root)));
        if let Some(r) = root {
            let v = platform::volume_info(&r);
            if v.ready {
                app.live_free.store(v.free, Relaxed);
            }
        }
    }
}

pub fn serve(app: Arc<App>, server: Server) {
    let server = Arc::new(server);
    let t = app.clone();
    std::thread::spawn(move || ticker(t));
    let mut handles = vec![];
    for _ in 0..8 {
        let s = server.clone();
        let a = app.clone();
        handles.push(std::thread::spawn(move || {
            while let Ok(rq) = s.recv() {
                handle(&a, rq);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_modes_refuse_real_paths() {
        let foreign: &[&str] = if cfg!(windows) { &[r"C:\Users", r"D:\"] } else { &["/home", "/etc"] };
        let demo = RunMode::Demo { scale: 1 };
        assert!(demo.allows(&demo::drives()[0].root), "erfundenes Demo-Laufwerk");
        for f in foreign {
            assert!(!demo.allows(f), "kein echter Ordner: {f}");
        }
        let sb = RunMode::Sandbox(r"D:\Test\Fixture".into());
        assert!(sb.allows(r"D:\Test\Fixture\Filme"));
        assert!(!sb.allows(r"C:\"));
        assert!(!sb.allows(r"D:\"));
        let sb = RunMode::Sandbox("/mnt/test/fixture".into());
        assert!(sb.allows("/mnt/test/fixture/filme"));
        assert!(!sb.allows("/"));
        assert!(!sb.allows("/mnt/test"));
    }

    #[test]
    fn decodes_urls() {
        assert_eq!(url_decode("a%20b+c"), "a b c");
        assert_eq!(url_decode("%C3%BC"), "ü");
        assert_eq!(url_decode("100%"), "100%");
        assert_eq!(url_decode("%zz"), "%zz");
        assert_eq!(url_decode("%4"), "%4");
    }

    #[test]
    fn csv_does_not_allow_formula_injection() {
        assert_eq!(csv_escape("=cmd()"), "\"'=cmd()\"");
        assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
        assert_eq!(csv_escape(r"C:\x"), "\"C:\\x\"");
    }

    #[test]
    fn formats_dates() {
        assert_eq!(fmt_date(0), "");
        assert_eq!(fmt_date(1_704_067_200), "2024-01-01");
        assert_eq!(fmt_date(951_782_400), "2000-02-29");
    }
}
