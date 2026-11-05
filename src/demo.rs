//! Test-Setup ohne echte Laufwerke.
//!
//! * `--demo`: Komplett erfundene Laufwerke im Speicher. Es wird keine einzige Nutzerdatei gelesen.
//!   Dient zum Ausprobieren und zum Pruefen der Oberflaeche bei realistischer Datenmenge.
//! * `--make-fixture`: Legt einen kleinen Testordner mit bekanntem Inhalt an. Zusammen mit
//!   `--sandbox` prueft das den echten Scan, das echte Hashen und das echte Loeschen.

use crate::analysis::now_secs;
use crate::dupes::DupGroup;
use crate::guard::{self, Guard};
use crate::history;
use crate::index::*;
use crate::paths;
use crate::platform::{self, DriveInfo};
use crate::scan;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, SystemTime};

const KB: u64 = 1 << 10;
const MB: u64 = 1 << 20;
const GB: u64 = 1 << 30;

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, a: u64, b: u64) -> u64 {
        a + self.next() % (b - a + 1).max(1)
    }
}

#[derive(Default)]
struct T {
    name: String,
    dir: bool,
    size: u64,
    age: u32,
    acc: u32,
    flags: u16,
    file_id: u64,
    kids: Vec<T>,
}

impl T {
    fn d(name: &str) -> T {
        T { name: name.into(), dir: true, ..Default::default() }
    }
    fn f(name: &str, size: u64, age: u32) -> T {
        T { name: name.into(), size, age, acc: age, ..Default::default() }
    }
    fn at(&mut self, path: &str) -> &mut T {
        let mut cur = self;
        for part in path.split(['\\', '/']) {
            let pos = cur.kids.iter().position(|k| k.dir && k.name == part);
            let i = match pos {
                Some(i) => i,
                None => {
                    cur.kids.push(T::d(part));
                    cur.kids.len() - 1
                }
            };
            cur = &mut cur.kids[i];
        }
        cur
    }
    fn add(&mut self, f: T) -> &mut Self {
        self.kids.push(f);
        self
    }
    /// Verteilt `total` Bytes auf `count` Dateien: wenige grosse, viele kleine.
    fn mass(&mut self, r: &mut Rng, prefix: &str, ext: &str, count: u64, total: u64, age: (u32, u32)) -> &mut Self {
        if count == 0 {
            return self;
        }
        let weights: Vec<f64> = (0..count).map(|_| r.f().powi(6) + 0.002).collect();
        let sum: f64 = weights.iter().sum();
        for (i, w) in weights.iter().enumerate() {
            let size = (total as f64 * w / sum) as u64 + r.range(0, 4000);
            let a = r.range(age.0 as u64, age.1 as u64) as u32;
            let mut f = T::f(&format!("{prefix}{:05}.{ext}", i + 1), size, a);
            f.acc = r.range(0, a.max(1) as u64) as u32;
            self.kids.push(f);
        }
        self
    }
}

fn flatten(ix: &mut Index, id: u32, t: &T, now: u64) {
    let base = ix.nodes.len() as u32;
    ix.nodes[id as usize].first_child = base;
    ix.nodes[id as usize].child_count = t.kids.len() as u32;
    for k in &t.kids {
        let off = ix.names.len() as u32;
        ix.names.extend_from_slice(k.name.as_bytes());
        let stamp = |d: u32| now.saturating_sub(d as u64 * 86_400) as u32;
        let mut n = Node {
            parent: id,
            name_off: off,
            name_len: k.name.len() as u16,
            flags: k.flags,
            file_id: k.file_id,
            mtime: stamp(k.age),
            atime: stamp(k.acc),
            ctime: stamp(k.age + 30),
            newest: stamp(k.age),
            avg_mtime: stamp(k.age),
            ..Default::default()
        };
        if k.dir {
            n.flags |= F_DIR;
            n.newest = 0;
        } else {
            n.size = k.size;
            n.alloc = if k.flags & F_CLOUD != 0 { 0 } else { (k.size + 4095) & !4095 };
        }
        ix.nodes.push(n);
    }
    for (i, k) in t.kids.iter().enumerate() {
        if k.dir {
            flatten(ix, base + i as u32, k, now);
        }
    }
}

fn scaled(n: u64, scale: u64) -> u64 {
    n * scale.max(1)
}

const APPS: &[&str] = &[
    "Bildstudio",
    "TextPro",
    "Tonwerk",
    "Klickbuero",
    "Rechenmeister",
    "Schnittplatz",
    "Mailhafen",
    "Zeichenbrett",
    "Datenfuchs",
    "Notizwerk",
    "Netzwerkzeug",
    "Paketmanager",
];
const GAMES: &[(&str, u64)] = &[
    ("Weltraum-Saga", 70),
    ("Rennfieber 7", 45),
    ("Burgenbauer Online", 28),
    ("Pixelabenteuer", 18),
    ("Tiefsee-Expedition", 12),
    ("Schach Deluxe", 9),
];
const PROJECTS: &[&str] = &[
    "webshop",
    "foto-app",
    "rechnungstool",
    "spiel-prototyp",
    "ki-training",
    "wetterdienst",
    "blog-engine",
    "chat-server",
    "kalender",
    "bastelkiste",
    "uni-projekt",
    "homepage",
];

/// Was in jedem Profil liegt: Fotos, Filme, Dokumente, Downloads und Entwicklungsprojekte.
fn user_files(u: &mut T, r: &mut Rng, s: u64) {
    u.at(".cache\\huggingface\\hub\\models--demo--bildmodell\\blobs").add(T::f("modell-a.safetensors", 9 * GB + 500 * MB, 200)).add(T::f(
        "modell-b.safetensors",
        9 * GB + 500 * MB,
        200,
    ));
    u.at(".cache\\huggingface\\hub\\models--demo--sprachmodell\\blobs").add(T::f("sprache.gguf", 13 * GB, 90));
    u.at("Documents\\VirtualBox\\Testsystem").add(T::f("testsystem.vdi", 24 * GB, 500));

    for y in 2017..=2026u32 {
        let age = (2026 - y) * 365 + 20;
        u.at(&format!("Pictures\\{y}")).mass(r, "IMG_", "jpg", scaled(2000, s), 3 * GB + (y as u64 - 2017) * 200 * MB, (age, age + 300));
        u.at(&format!("Pictures\\{y}\\RAW")).mass(r, "RAW_", "dng", 150, GB, (age, age + 300));
    }
    u.at("Videos\\Urlaub Italien").mass(r, "clip", "mp4", 40, 22 * GB, (900, 1000));
    u.at("Videos\\Hochzeit").mass(r, "clip", "mov", 18, 18 * GB, (1600, 1700));
    u.at("Videos\\Bildschirmaufnahmen").mass(r, "aufnahme", "mkv", 60, 20 * GB, (2, 300));
    u.at("Music\\Sammlung").mass(r, "titel", "flac", scaled(2500, s), 18 * GB, (800, 3000));
    u.at("Documents\\Rechnungen").mass(r, "rechnung", "pdf", scaled(2000, s), 4 * GB, (30, 3000));
    u.at("Documents\\Projekte").mass(r, "bericht", "docx", 1500, 6 * GB, (30, 2500));
    u.at("Documents\\Tabellen").mass(r, "tabelle", "xlsx", 800, 3 * GB, (30, 2500));
    u.at("Documents\\Scans").mass(r, "scan", "pdf", 700, 17 * GB, (500, 3500));

    let dl = u.at("Downloads");
    dl.add(T::f("Installer-Bildstudio-2021.exe", 2 * GB, 1500));
    dl.add(T::f("Betriebssystem-Abbild.iso", 6 * GB, 800));
    dl.add(T::f("Treiberpaket-alt.zip", 900 * MB, 700));
    dl.add(T::f("Setup-Tonwerk.msi", 400 * MB, 400));
    dl.add(T::f("Fotos-Sommer.zip", 7 * GB + 123_456, 200));
    dl.add(T::f("Film-Kopie.mkv", 5 * GB + 777_001, 90));
    dl.add(T::f("aktuell.pdf", 3 * MB, 1));
    dl.mass(r, "datei", "pdf", 300, 2 * GB, (5, 900));
    dl.add(T::f("abgebrochen.iso.crdownload", 3 * GB, 60));
    u.at("Videos\\Filme").add(T::f("Film-Original.mkv", 5 * GB + 777_001, 300));
    u.at("Pictures\\Sicherung").add(T::f("Fotos-Sommer-Kopie.zip", 7 * GB + 123_456, 100));
    u.at("Leerer Ordner");
    u.at("Noch ein leerer Ordner");
    for (i, p) in PROJECTS.iter().enumerate() {
        let d = u.at(&format!("Entwicklung\\{p}"));
        let idle = 5 + (i as u32) * 90;
        d.add(T::f("README.md", 4 * KB, idle));
        d.at("src").mass(r, "modul", "ts", 200, 40 * MB, (idle, idle + 90));
        if i % 2 == 0 {
            d.add(T::f("package.json", 2 * KB, idle));
            for pkg in 0..30u64 {
                let n = d.at(&format!("node_modules\\paket-{pkg:03}\\lib"));
                n.mass(r, "datei", "js", scaled(100, s), 20 * MB, (idle, idle + 200));
            }
            d.at(".next\\cache").mass(r, "build", "pack", 80, 800 * MB, (idle, idle + 5));
        } else {
            d.add(T::f("Cargo.toml", 2 * KB, idle));
            d.at("target\\debug\\deps").mass(r, "lib", "rlib", 800, (2 + i as u64) * GB, (idle, idle + 5));
            d.at("__pycache__").mass(r, "modul", "pyc", 60, 50 * MB, (idle, idle + 5));
        }
        if i % 3 == 0 {
            let v = d.at("venv");
            v.add(T::f("pyvenv.cfg", 300, idle));
            v.at("Lib\\site-packages").mass(r, "paket", "py", 2000, 2 * GB, (idle, idle + 5));
        }
    }
    u.at("Entwicklung\\ki-training\\ausgaben").mass(r, "lauf", "pt", 30, 18 * GB, (3, 20));
}

fn system_tree(r: &mut Rng, s: u64) -> T {
    let mut root = T::d("");

    // Windows mit Hardlinks wie im echten WinSxS
    {
        let w = root.at("Windows");
        let mut id = 7000u64;
        for i in 0..scaled(3000, s) {
            let size = r.range(20 * KB, 3 * MB);
            let name = format!("sys{i:05}.dll");
            let age = r.range(30, 900) as u32;
            id += 1;
            let mut a = T::f(&name, size, age);
            a.file_id = id;
            let mut b = T::f(&name, size, age);
            b.file_id = id;
            w.at("System32").add(a);
            w.at(&format!("WinSxS\\amd64_paket_{:04}", i / 40)).add(b);
        }
        w.at("System32").mass(r, "treiber", "sys", scaled(900, s), 3 * GB, (60, 1400));
        w.at("Fonts").mass(r, "schrift", "ttf", 400, 600 * MB, (300, 2000));
        w.at("Installer").mass(r, "setup", "msi", 300, 3 * GB + 500 * MB, (60, 1500));
        w.at("Temp").mass(r, "win", "tmp", 200, 900 * MB, (2, 200));
        w.at("Logs\\CBS").mass(r, "cbs", "log", 80, 700 * MB, (2, 500));
        w.at("SoftwareDistribution\\Download").mass(r, "update", "cab", 60, 4 * GB, (5, 200));
        w.at("Minidump").mass(r, "absturz", "dmp", 12, 90 * MB, (20, 700));
        w.add(T::f("MEMORY.DMP", 2 * GB + 424_242, 160));
        w.at("assembly").mass(r, "net", "dll", scaled(2000, s), 8 * GB, (100, 1500));
    }

    // Programme und Spiele
    for (i, app) in APPS.iter().enumerate() {
        let total = (2 + (i as u64 * 3) % 14) * GB;
        let d = root.at(&format!("Program Files\\{app}"));
        d.mass(r, "modul", "dll", scaled(1200, s), total * 7 / 10, (40, 900));
        d.at("bin").mass(r, "tool", "exe", 30, total / 10, (40, 900));
        d.at("resources").mass(r, "daten", "pak", 200, total / 5, (40, 900));
    }
    for (game, gb) in GAMES {
        let d = root.at(&format!("Program Files (x86)\\Steam\\steamapps\\common\\{game}"));
        d.mass(r, "inhalt", "pak", 40, gb * GB * 9 / 10, (20, 1200));
        d.at("bin").mass(r, "spiel", "dll", 300, gb * GB / 10, (20, 1200));
    }
    root.at("Program Files (x86)\\Steam\\steamapps\\downloading").mass(r, "chunk", "part", 8, 700 * MB, (15, 40));
    root.at("ProgramData\\Paketspeicher").mass(r, "pkg", "msi", 200, 7 * GB, (30, 900));
    root.at("ProgramData\\Microsoft\\Windows\\WER\\ReportArchive").mass(r, "bericht", "wer", 200, 300 * MB, (10, 900));

    // Benutzerprofil
    {
        let u = root.at("Users\\Demo");
        let l = u.at("AppData\\Local");
        l.at("Docker\\wsl\\disk").add(T::f("docker_data.vhdx", 95 * GB, 2));
        l.at("wsl\\{demo-distro}").add(T::f("ext4.vhdx", 28 * GB, 1));
        l.at("Temp").mass(r, "tmp", "tmp", scaled(6000, s), 30 * GB, (0, 300));
        l.at("Temp\\installer-reste").mass(r, "rest", "dat", 300, 5 * GB, (30, 300));
        l.at("Google\\Chrome\\User Data\\Default\\Cache\\Cache_Data").mass(r, "f_", "", scaled(8000, s), 3 * GB, (0, 30));
        l.at("Google\\Chrome\\User Data\\Default\\Code Cache\\js").mass(r, "c_", "", 2000, 600 * MB, (0, 30));
        l.at("Microsoft\\Edge\\User Data\\Default\\Cache\\Cache_Data").mass(r, "f_", "", 3000, 1200 * MB, (0, 30));
        l.at("NVIDIA\\DXCache").mass(r, "shader", "bin", 800, 4 * GB, (1, 120));
        l.at("npm-cache").mass(r, "index", "tgz", scaled(4000, s), 6 * GB, (1, 400));
        l.at("pip\\Cache").mass(r, "wheel", "whl", 900, 5 * GB, (1, 400));
        l.at("CrashDumps").mass(r, "app", "dmp", 15, 3 * GB, (3, 300));
        l.at("Microsoft\\Windows\\Explorer").mass(r, "thumbcache_", "db", 8, 900 * MB, (0, 30));
        l.at("Bildstudio\\Cache").mass(r, "bild", "cache", 3000, 22 * GB, (1, 200));
        l.at("Schnittplatz\\Cache").mass(r, "vorschau", "cache", 800, 38 * GB, (1, 200));
        l.at("Datenfuchs\\Daten").mass(r, "db", "sqlite", 30, 6 * GB, (1, 200));
        u.at("AppData\\Roaming\\Mailhafen\\Profile").add(T::f("archiv.pst", 11 * GB, 3));

        user_files(u, r, s);
        let od = u.at("OneDrive\\Geteilt");
        for i in 0..60 {
            let mut f = T::f(&format!("nur-online-{i:03}.mp4"), 900 * MB, 300);
            f.flags = F_CLOUD;
            od.add(f);
        }
    }

    backups(&mut root);

    root.at("$Recycle.Bin\\S-1-5-21-demo").flags = F_HIDDEN | F_SYSTEM;
    root.at("$Recycle.Bin\\S-1-5-21-demo").mass(r, "$R", "mp4", 30, 8 * GB, (3, 60));
    root.at("$Recycle.Bin").flags = F_HIDDEN | F_SYSTEM;
    let mut hib = T::f("hiberfil.sys", 13 * GB + 600 * MB, 1);
    hib.flags = F_HIDDEN | F_SYSTEM;
    let mut pf = T::f("pagefile.sys", 12 * GB + 800 * MB, 1);
    pf.flags = F_HIDDEN | F_SYSTEM;
    root.add(hib).add(pf);
    let denied = root.at("System Volume Information");
    denied.flags = F_DENIED | F_HIDDEN | F_SYSTEM;
    root
}

/// Dieselbe Geschichte als Linux-System.
fn linux_tree(r: &mut Rng, s: u64) -> T {
    let mut root = T::d("");
    {
        let lib = root.at("usr/lib");
        let mut id = 9000u64;
        for i in 0..scaled(3000, s) {
            let size = r.range(20 * KB, 3 * MB);
            let name = format!("sys{i:05}.so");
            let age = r.range(30, 900) as u32;
            id += 1;
            let mut a = T::f(&name, size, age);
            a.file_id = id;
            let mut b = T::f(&name, size, age);
            b.file_id = id;
            lib.add(a);
            lib.at(&format!("debug/paket_{:04}", i / 40)).add(b);
        }
    }
    root.at("usr/share").mass(r, "daten", "dat", scaled(2000, s), 6 * GB, (30, 1500));
    root.at("usr/bin").mass(r, "tool", "", 800, 900 * MB, (30, 900));
    root.at("opt/schnittplatz").mass(r, "modul", "so", 600, 8 * GB, (40, 900));
    root.at("var/cache/apt/archives").mass(r, "paket", "deb", 300, 4 * GB, (5, 200));
    root.at("var/lib/docker/overlay2").mass(r, "schicht", "tar", 200, 42 * GB, (5, 400));
    root.at("var/lib/snapd/snaps").mass(r, "paket", "snap", 30, 7 * GB, (5, 300));
    root.at("var/lib/systemd/coredump").mass(r, "core", "zst", 12, 2 * GB, (5, 200));
    root.at("var/log/journal/demo").mass(r, "system", "journal", 40, 4 * GB, (1, 200));
    root.at("var/log").mass(r, "syslog", "gz", 40, 300 * MB, (10, 300));
    root.at("var/tmp").mass(r, "rest", "dat", 40, GB, (10, 300));
    let mut swap = T::f("swapfile", 16 * GB, 1);
    swap.flags = F_HIDDEN;
    root.add(swap);
    {
        let u = root.at("home/demo");
        u.at(".cache/thumbnails/large").mass(r, "t", "png", scaled(4000, s), 2 * GB, (1, 300));
        u.at(".cache/google-chrome/Default/Cache/Cache_Data").mass(r, "f_", "", scaled(8000, s), 3 * GB, (0, 30));
        u.at(".cache/mozilla/firefox/demo.default/cache2/entries").mass(r, "e", "", 3000, 1200 * MB, (0, 30));
        u.at(".cache/pip/http").mass(r, "wheel", "whl", 900, 5 * GB, (1, 400));
        u.at(".cache/mesa_shader_cache").mass(r, "shader", "bin", 800, 3 * GB, (1, 120));
        u.at(".cache/bildstudio").mass(r, "bild", "cache", 3000, 22 * GB, (1, 200));
        u.at(".npm/_cacache").mass(r, "index", "tgz", scaled(4000, s), 6 * GB, (1, 400));
        u.at(".local/share/Trash/info").add(T::f("alt-00001.mp4.trashinfo", 200, 20));
        u.at(".local/share/Trash/files").mass(r, "alt-", "mp4", 30, 8 * GB, (3, 60));
        u.at(".local/share/containers/storage").mass(r, "schicht", "tar", 90, 60 * GB, (2, 90));
        u.at(".config/mailhafen").add(T::f("archiv.mbox", 11 * GB, 3));
        for (game, gb) in GAMES {
            let d = u.at(&format!(".local/share/Steam/steamapps/common/{game}"));
            d.mass(r, "inhalt", "pak", 40, gb * GB * 9 / 10, (20, 1200));
            d.at("bin").mass(r, "spiel", "so", 300, gb * GB / 10, (20, 1200));
        }
        u.at("Downloads").add(T::f("schnittplatz-1.2.deb", 900 * MB, 700)).add(T::f("werkzeug.AppImage", 400 * MB, 500));
        user_files(u, r, s);
    }
    backups(&mut root);
    let denied = root.at("root");
    denied.flags = F_DENIED;
    root
}

fn backups(root: &mut T) {
    // Zwei Sicherungen mit gleichem Inhalt: doppelte Ordner
    let mut backup_seed = Rng::new(4242);
    for name in ["Backup-2021", "Backup-2021 - Kopie"] {
        let mut s2 = Rng::new(backup_seed.0);
        let d = root.at(name);
        d.at("Fotos").mass(&mut s2, "IMG_", "jpg", 600, 16 * GB, (1500, 1500));
        d.at("Dokumente").mass(&mut s2, "akte", "pdf", 200, 8 * GB, (1500, 1500));
    }
    let _ = backup_seed.next();
}

fn archive_tree(r: &mut Rng, s: u64) -> T {
    let mut root = T::d("");
    for i in 0..scaled(300, s).min(3000) {
        let y = 2008 + (i % 17);
        let size = r.range(4, 45) * GB + r.range(0, GB);
        let age = (2026 - y as u32) * 365 + r.range(0, 300) as u32;
        root.at(&format!("Filme\\{}", (b'A' + (i % 26) as u8) as char)).add(T::f(&format!("Spielfilm {i:04} ({y}).mkv"), size, age));
    }
    for show in 0..24u64 {
        for season in 1..=(1 + show % 6) {
            let age = 500 + (show as u32) * 120;
            root.at(&format!("Serien\\Serie {show:02}\\Staffel {season}")).mass(r, "Folge", "mkv", 12, 18 * GB, (age, age + 20));
        }
    }
    for y in 2005..=2022u32 {
        let age = (2026 - y) * 365;
        root.at(&format!("Fotos-Archiv\\{y}")).mass(r, "BILD", "cr2", scaled(1200, s), 40 * GB, (age, age + 200));
    }
    root.at("Musik").mass(r, "Stueck", "flac", scaled(6000, s), 220 * GB, (1500, 5000));
    root.at("VM-Images").mass(r, "maschine", "vdi", 14, 420 * GB, (1000, 2500));
    root.at("ISO").mass(r, "abbild", "iso", 80, 260 * GB, (1500, 4000));
    root.at("Backup-2019\\Systemabbild").mass(r, "teil", "wim", 20, 900 * GB, (2400, 2500));
    root.at("Backup-2019\\Dokumente").mass(r, "akte", "docx", 4000, 40 * GB, (2400, 3500));
    root
}

fn nas_tree(r: &mut Rng, _s: u64) -> T {
    let mut root = T::d("");
    root.at("Medien\\Filme").mass(r, "Film", "mkv", 90, 900 * GB, (300, 2500));
    root.at("Medien\\Musik").mass(r, "Titel", "mp3", 4000, 60 * GB, (300, 3000));
    root.at("Familie\\Fotos").mass(r, "IMG_", "heic", 5000, 180 * GB, (30, 2500));
    root
}

fn all_drives() -> Vec<DriveInfo> {
    let mk = |root: &str, label: &str, kind: &str, media: &str, total: u64, free: u64, fs: &str| DriveInfo {
        root: root.into(),
        label: label.into(),
        fs: fs.into(),
        kind: kind.into(),
        media: media.into(),
        total,
        free,
        ready: true,
        serial: 0,
    };
    vec![
        mk(r"C:\", "DEMO System", "fixed", "ssd", 1000 * GB, 21 * GB, "NTFS"),
        mk(r"E:\", "DEMO Archiv", "fixed", "hdd", 7452 * GB, 3100 * GB, "NTFS"),
        mk(r"Z:\", "DEMO NAS", "network", "net", 2000 * GB, 800 * GB, "NTFS"),
        mk("/", "DEMO System", "fixed", "ssd", 1000 * GB, 21 * GB, "ext4"),
        mk("/mnt/archiv", "DEMO Archiv", "fixed", "hdd", 7452 * GB, 3100 * GB, "ext4"),
        mk("/mnt/nas", "DEMO NAS", "network", "net", 2000 * GB, 800 * GB, "nfs4"),
    ]
}

/// Die erfundenen Laufwerke im Stil des Systems, auf dem das Programm laeuft.
pub fn drives() -> Vec<DriveInfo> {
    all_drives().into_iter().filter(|d| paths::is_unix_style(&d.root) != cfg!(windows)).collect()
}

/// Baut den erfundenen Index eines Demo-Laufwerks. Fasst keine Datei an.
pub fn generate(root: &str, scale: u64) -> Index {
    let mut r = Rng::new(match root {
        r"C:\" => 11,
        r"E:\" | "/mnt/archiv" => 22,
        "/" => 44,
        _ => 33,
    });
    let tree = match root {
        r"C:\" => system_tree(&mut r, scale),
        "/" => linux_tree(&mut r, scale),
        r"E:\" | "/mnt/archiv" => archive_tree(&mut r, scale),
        _ => nas_tree(&mut r, scale),
    };
    let mut ix = Index::default();
    ix.meta.root = root.to_string();
    ix.meta.is_drive = true;
    ix.names.extend_from_slice(root.as_bytes());
    ix.nodes.push(Node { parent: NONE, name_len: root.len() as u16, flags: F_DIR, ..Default::default() });
    let now = now_secs();
    flatten(&mut ix, 0, &tree, now);
    scan::finalize(&mut ix);
    ix.meta.scanned_at = now;
    ix.meta.duration_ms = 6_400;
    let primary = root == r"C:\" || root == "/";
    match root {
        r"C:\" => {
            ix.meta.denied =
                vec![Denied { path: r"C:\System Volume Information".into(), code: 5 }, Denied { path: r"C:\Users\Anderer Nutzer".into(), code: 5 }];
        }
        "/" => {
            ix.meta.denied = vec![Denied { path: "/root".into(), code: 13 }, Denied { path: "/var/lib/private".into(), code: 13 }];
        }
        _ => {}
    }
    let d = all_drives().into_iter().find(|d| d.root == root).unwrap_or_default();
    let unaccounted = if primary { 12 * GB } else { 2 * GB };
    let total = (ix.nodes[0].alloc + unaccounted + d.free).max(d.total);
    let free = total - ix.nodes[0].alloc - unaccounted;
    ix.meta.volume = VolumeMeta { total, free, label: d.label, fs: d.fs, kind: d.kind, media: d.media };
    ix.meta.unaccounted = unaccounted;
    ix
}

/// Der Ordner, der in der Demo den Papierkorb darstellt.
pub fn trash_node(ix: &Index) -> Option<u32> {
    if paths::is_unix_style(&ix.meta.root) {
        ix.find_path("/home/demo/.local/share/Trash")
    } else {
        ix.children_by_size(0).into_iter().find(|i| ix.name(*i).eq_ignore_ascii_case("$recycle.bin"))
    }
}

fn perturb(map: &mut HashMap<String, (u64, u32)>, path: &str, delta: i64) {
    let mut cur = path.to_string();
    loop {
        let e = map.entry(cur.clone()).or_insert((0, 1));
        e.0 = (e.0 as i64 + delta).max(0) as u64;
        match paths::parent_of(&cur) {
            Some(p) => cur = p,
            None => break,
        }
    }
}

/// Legt fuer die Demo einen Verlauf ueber mehrere Wochen an, nur im Demo-Datenordner.
pub fn seed(scale: u64) {
    for d in drives() {
        let ix = generate(&d.root, scale);
        let dir = history::scan_dir(&d.root);
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::remove_file(dir.join("history.jsonl"));
        if d.root == r"C:\" || d.root == "/" {
            let linux = d.root == "/";
            let home = if linux { "/home/demo" } else { r"C:\Users\Demo" };
            // Baut einen Pfad im Profil, die Teile werden mit dem passenden Trenner verbunden.
            let at = |rel: &str| rel.split('|').fold(home.to_string(), |a, b| paths::join(&a, b));
            let (grow_a, grow_b) = if linux {
                (".local|share|containers|storage", ".cache|bildstudio")
            } else {
                ("AppData|Local|Docker|wsl|disk", "AppData|Local|Schnittplatz|Cache")
            };
            let base = history::entry_of(&ix);
            let rows = history::dir_rows(&ix);
            for weeks in (1..=9u64).rev() {
                let mut map: HashMap<String, (u64, u32)> = rows.iter().map(|(p, a, f)| (p.clone(), (*a, *f))).collect();
                let w = weeks as i64;
                let g = GB as i64;
                let ki = at("Entwicklung|ki-training|ausgaben");
                let alt = map.get(&ki).map(|x| x.0 as i64).unwrap_or(0);
                perturb(&mut map, &ki, -alt);
                map.remove(&ki);
                perturb(&mut map, &at(grow_a), -3 * g * w);
                perturb(&mut map, &at("Downloads"), -g * w);
                perturb(&mut map, &at(grow_b), -2 * g * w);
                if weeks >= 2 {
                    perturb(&mut map, &at("Videos|Altes Projekt"), 14 * g);
                }
                let snap: Vec<(String, u64, u32)> = map.into_iter().map(|(p, (a, f))| (p, a, f)).collect();
                let ts = ix.meta.scanned_at - weeks * 7 * 86_400;
                let _ = history::write_snapshot(&d.root, ts, &snap);
                let growth = weeks * 6 * GB;
                let scanned = base.scanned.saturating_sub(growth);
                let mut e = base.clone();
                e.ts = ts;
                e.scanned = scanned;
                e.free = base.total.saturating_sub(scanned + 12 * GB);
                let factor = scanned as f64 / base.scanned.max(1) as f64;
                e.cats = base.cats.iter().map(|c| (*c as f64 * factor) as u64).collect();
                let _ = history::append_entry(&e);
            }
        }
        let _ = history::save_scan(&ix);
    }
}

/// Duplikat-Ergebnis fuer die Demo: Gleich grosse Dateien gelten als gleich, Inhalte gibt es ja nicht.
pub fn dup_groups(ix: &Index, min_size: u64, include_system: bool) -> Vec<DupGroup> {
    let mut by_size: HashMap<u64, Vec<u32>> = HashMap::new();
    for (i, n) in ix.nodes.iter().enumerate() {
        if !n.is_dir() && !n.is_deleted() && n.flags & (F_HARDLINK | F_CLOUD | F_LINK) == 0 && n.size >= min_size.max(1) {
            by_size.entry(n.size).or_default().push(i as u32);
        }
    }
    let mut out: Vec<DupGroup> = by_size
        .into_iter()
        .filter_map(|(size, ids)| {
            let keep: Vec<u32> = ids.into_iter().filter(|i| include_system || !matches!(guard::check(&ix.path(*i)), Guard::Block(_))).collect();
            if keep.len() < 2 {
                return None;
            }
            let alloc = ix.nodes[keep[0] as usize].alloc;
            Some(DupGroup { size, alloc, ids: keep, verified: true })
        })
        .collect();
    out.sort_by_key(|g| std::cmp::Reverse(g.wasted()));
    out
}

pub fn health(root: &str) -> Value {
    match root {
        r"C:\" | "/" => {
            json!({"model": "DEMO NVMe 1 TB", "media": "SSD", "bus": "NVMe", "health": "Healthy", "size": 1000u64 * GB, "temp": 41, "wear": 7, "hours": 8120})
        }
        r"E:\" | "/mnt/archiv" => {
            json!({"model": "DEMO Archivplatte 8 TB", "media": "HDD", "bus": "SATA", "health": "Warning", "size": 7452u64 * GB, "temp": 47, "hours": 41200})
        }
        _ => json!({"unavailable": true}),
    }
}

fn write_random(path: &Path, size: u64, seed: u64) -> std::io::Result<()> {
    let mut r = Rng::new(seed);
    let f = std::fs::File::create(path)?;
    let mut w = std::io::BufWriter::new(f);
    let mut left = size;
    while left > 0 {
        let b = r.next().to_le_bytes();
        let n = left.min(8) as usize;
        w.write_all(&b[..n])?;
        left -= n as u64;
    }
    w.flush()
}

fn set_age(path: &Path, days: u64) -> std::io::Result<()> {
    let t = SystemTime::now() - Duration::from_secs(days * 86_400);
    std::fs::OpenOptions::new().write(true).open(path)?.set_modified(t)
}

/// Legt den Testordner mit bekanntem Inhalt an, gerade so gross, dass er nicht stoert (rund 60 MB).
pub fn make_fixture(dir: &Path) -> std::io::Result<()> {
    let fx = |rel: &str, size: u64, seed: u64, age: u64| -> std::io::Result<()> {
        let p = dir.join(rel.replace('\\', std::path::MAIN_SEPARATOR_STR));
        std::fs::create_dir_all(p.parent().unwrap())?;
        write_random(&p, size, seed)?;
        if age > 0 {
            set_age(&p, age)?;
        }
        Ok(())
    };
    // Echte Duplikate und ein fast gleicher Zwilling: gleiche Groesse, gleicher Anfang und gleiches Ende.
    fx(r"Filme\urlaub-2019.mp4", 12 * MB, 100, 2200)?;
    fx(r"Filme\urlaub-kopie.mp4", 12 * MB, 100, 300)?;
    fx(r"Filme\fast-gleich.mp4", 12 * MB, 100, 300)?;
    {
        use std::io::{Seek, SeekFrom};
        let twin = dir.join("Filme").join("fast-gleich.mp4");
        let mut f = std::fs::OpenOptions::new().write(true).open(&twin)?;
        f.seek(SeekFrom::Start(6 * MB))?;
        f.write_all(b"HIER IST DIE DATEI ANDERS")?;
        drop(f);
        set_age(&twin, 300)?;
    }
    for year in [2019u64, 2021, 2023] {
        for i in 0..60u64 {
            fx(&format!(r"Fotos\{year}\img_{i:04}.jpg"), 40 * KB + i * 100, year * 1000 + i, (2026 - year) * 365)?;
        }
    }
    for b in ["Backup-A", "Backup-B"] {
        fx(&format!(r"{b}\Dokumente\vertrag.pdf"), 2 * MB, 501, 1500)?;
        fx(&format!(r"{b}\Dokumente\brief.docx"), 300 * KB, 502, 1500)?;
        fx(&format!(r"{b}\Dokumente\notiz.txt"), 5 * KB, 503, 1500)?;
    }
    for p in 0..40u64 {
        for i in 0..10u64 {
            fx(&format!(r"Code\app\node_modules\paket-{p:02}\datei{i}.js"), 2 * KB + i, p * 100 + i + 7000, 200)?;
        }
    }
    fx(r"Code\app\package.json", 500, 8001, 200)?;
    fx(r"Code\app\src\index.ts", 3 * KB, 8002, 200)?;
    fx(r"Code\rust\Cargo.toml", 400, 8003, 100)?;
    fx(r"Code\rust\target\debug\app.exe", 8 * MB, 8004, 100)?;
    fx(r"Code\rust\src\main.rs", 2 * KB, 8005, 100)?;
    fx(r"Code\anderes\target\wichtig.txt", 600, 8006, 100)?;
    fx(r"Code\py\__pycache__\modul.pyc", 40 * KB, 8007, 100)?;
    fx(r"Code\py\venv\pyvenv.cfg", 200, 8008, 100)?;
    fx(r"Code\py\venv\Lib\paket.py", 900 * KB, 8009, 100)?;
    fx(r"Daten\alt.log", 2 * MB, 9001, 400)?;
    fx(r"Daten\frisch.log", 100 * KB, 9002, 0)?;
    fx(r"Daten\absturz.dmp", 3 * MB, 9003, 40)?;
    fx(r"Daten\reste.tmp", MB, 9004, 30)?;
    fx(r"Daten\maschine.vhdx", 5 * MB, 9005, 600)?;
    fx(r"Daten\wichtig.xlsx", 700 * KB, 9006, 10)?;
    fx(r"Downloads\setup-alt.exe", 3 * MB, 9101, 500)?;
    fx(r"Downloads\neu.zip", 2 * MB, 9102, 1)?;
    fx(r"Umlaute\Übung größe.txt", 12 * KB, 9201, 50)?;
    std::fs::create_dir_all(dir.join("Leer"))?;
    // Pfad jenseits der alten 260-Zeichen-Grenze
    let mut deep = platform::long_pathbuf(dir);
    for _ in 0..14 {
        deep.push("ein_sehr_langer_ordnername_zum_testen");
    }
    std::fs::create_dir_all(&deep)?;
    write_random(&deep.join("tief.bin"), 10 * KB, 9301)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{analysis, junk};

    #[test]
    fn demo_index_is_consistent_and_realistic() {
        let ix = generate(r"C:\", 1);
        assert!(ix.nodes.len() > 100_000, "{}", ix.nodes.len());
        assert!(ix.nodes[0].alloc > 500 * GB);
        for (i, n) in ix.nodes.iter().enumerate() {
            if n.parent != NONE {
                assert!((n.parent as usize) < i, "Eltern müssen vor Kindern liegen");
            }
        }
        let sum: u64 = ix.nodes.iter().filter(|n| !n.is_dir()).map(|n| n.alloc).sum();
        assert_eq!(sum, ix.nodes[0].alloc, "Summe aller Dateien muss der Wurzel entsprechen");
        assert!(ix.meta.volume.total > ix.nodes[0].alloc);
    }

    #[test]
    fn demo_exercises_every_analysis() {
        let ix = generate(r"C:\", 1);
        let rep = junk::analyze(&ix);
        for id in [
            "user-temp",
            "browser",
            "node-modules",
            "build-rust",
            "pycache",
            "venv",
            "games",
            "vm-disks",
            "hibernate",
            "pagefile",
            "recycle",
            "old-installers",
            "incomplete",
        ] {
            assert!(rep.results[junk::rule_by_id(id).unwrap()].alloc > 0, "Regel {id} kommt in der Demo nicht vor");
        }
        assert_eq!(rep.empty_dirs.len(), 2);
        let s = analysis::summary(&ix, &rep, None, true);
        assert!(s["recs"].as_array().unwrap().len() > 5);
        assert!(ix.nodes.iter().any(|n| n.flags & F_HARDLINK != 0), "Hardlinks fehlen");
        let g = dup_groups(&ix, MB, false);
        assert!(g.iter().any(|x| x.size == 5 * GB + 777_001), "Duplikate fehlen");
        let df = crate::dupes::duplicate_folders(&ix, 50 * MB, false);
        assert!(df.iter().any(|g| g.ids.iter().any(|i| ix.name(*i) == "Backup-2021")), "Doppelte Ordner fehlen");
    }

    #[test]
    fn linux_demo_exercises_the_linux_rules() {
        let ix = generate("/", 1);
        assert!(ix.nodes.len() > 50_000, "{}", ix.nodes.len());
        assert_eq!(ix.nodes.iter().filter(|n| !n.is_dir()).map(|n| n.alloc).sum::<u64>(), ix.nodes[0].alloc);
        let rep = junk::analyze(&ix);
        for id in [
            "trash",
            "thumbs-linux",
            "browser-linux",
            "pkg-cache",
            "user-cache",
            "node-modules",
            "build-rust",
            "games",
            "pkg-downloads",
            "coredumps",
            "journal",
            "container-store",
            "snap-store",
            "swap-linux",
            "rotated-logs",
            "var-tmp",
        ] {
            assert!(rep.results[junk::rule_by_id(id).unwrap()].alloc > 0, "Regel {id} kommt in der Linux-Demo nicht vor");
        }
        assert!(ix.nodes.iter().any(|n| n.flags & F_HARDLINK != 0), "Hardlinks fehlen");
        assert!(trash_node(&ix).is_some());
        assert!(ix.path(1).starts_with('/'));
    }

    #[test]
    fn demo_is_deterministic() {
        let a = generate(r"E:\", 1);
        let b = generate(r"E:\", 1);
        assert_eq!(a.nodes.len(), b.nodes.len());
        assert_eq!(a.nodes[0].alloc, b.nodes[0].alloc);
    }
}
