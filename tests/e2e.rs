//! Prueft das fertige Programm von aussen: echter Scan, echtes Hashen, echtes Entfernen,
//! aber ausschliesslich in einem Testordner unter target/tmp. Echte Laufwerke sind tabu,
//! und die Tests pruefen auch, dass das Programm im Test-Betrieb Zugriffe darauf verweigert.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

const MB: u64 = 1 << 20;
const EXE: &str = env!("CARGO_BIN_EXE_storage-map");

/// Echte Pfade dieses Systems, die im Test-Betrieb tabu sind.
#[cfg(windows)]
const FOREIGN: &[&str] = &[r"C:\", r"D:\", r"E:\", r"C:\Users", r"C:\Windows"];
#[cfg(not(windows))]
const FOREIGN: &[&str] = &["/", "/home", "/etc", "/usr", "/var"];

/// Pfad im Testordner: Im Testcode steht `\`, auf der Platte der Trenner des Systems.
fn rel(base: &Path, s: &str) -> PathBuf {
    base.join(s.replace('\\', std::path::MAIN_SEPARATOR_STR))
}

fn native(s: &str) -> String {
    s.replace('\\', std::path::MAIN_SEPARATOR_STR)
}

struct Srv {
    child: Child,
    port: u16,
}

impl Drop for Srv {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn start(args: &[&str]) -> Srv {
    let port = free_port();
    let child = Command::new(EXE).args(args).args(["--no-open", "--token", "t", "--port", &port.to_string()]).spawn().unwrap();
    let s = Srv { child, port };
    let t0 = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(t0.elapsed() < Duration::from_secs(60), "Server startet nicht");
        std::thread::sleep(Duration::from_millis(50));
    }
    s
}

fn raw(port: u16, method: &str, path: &str, body: &str, token: Option<&str>, host: Option<&str>) -> (u16, String) {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let host = host.map(String::from).unwrap_or_else(|| format!("127.0.0.1:{port}"));
    let tok = token.map(|t| format!("X-SM-Token: {t}\r\n")).unwrap_or_default();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\n{tok}Connection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut buf = vec![];
    s.read_to_end(&mut buf).unwrap();
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4).unwrap_or(buf.len());
    let head = String::from_utf8_lossy(&buf[..split]).to_string();
    let code = head.split_whitespace().nth(1).unwrap_or("0").parse().unwrap_or(0);
    let mut payload = buf[split..].to_vec();
    if head.to_lowercase().contains("transfer-encoding: chunked") {
        payload = dechunk(&payload);
    }
    (code, String::from_utf8_lossy(&payload).to_string())
}

/// Grosse Antworten kommen in Stuecken. Browser koennen das von selbst, dieser kleine Testclient nicht.
fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    while let Some(i) = b.windows(2).position(|w| w == b"\r\n") {
        let len = usize::from_str_radix(String::from_utf8_lossy(&b[..i]).trim(), 16).unwrap_or(0);
        if len == 0 {
            break;
        }
        out.extend_from_slice(&b[i + 2..i + 2 + len]);
        b = &b[i + 2 + len + 2..];
    }
    out
}

fn get(s: &Srv, path: &str) -> Value {
    let (c, b) = raw(s.port, "GET", path, "", Some("t"), None);
    assert_eq!(c, 200, "GET {path}: {b}");
    serde_json::from_str(&b).unwrap()
}

fn post(s: &Srv, path: &str, body: Value) -> Value {
    let (c, b) = raw(s.port, "POST", path, &body.to_string(), Some("t"), None);
    assert_eq!(c, 200, "POST {path}: {b}");
    serde_json::from_str(&b).unwrap()
}

fn post_code(s: &Srv, path: &str, body: Value) -> u16 {
    raw(s.port, "POST", path, &body.to_string(), Some("t"), None).0
}

fn wait(s: &Srv, what: &str, done: impl Fn(&Value) -> bool) -> Value {
    let t0 = Instant::now();
    loop {
        let st = get(s, "/api/status");
        if done(&st) {
            return st;
        }
        assert!(t0.elapsed() < Duration::from_secs(120), "Zeitüberschreitung: {what}: {st}");
        std::thread::sleep(Duration::from_millis(40));
    }
}

fn scan(s: &Srv, root: &str) {
    post(s, "/api/scan", json!({"path": root}));
    wait(s, "Scan", |st| st["scan"].is_null());
    let st = get(s, "/api/status");
    assert!(st["scanError"].is_null(), "{st}");
}

fn enc(q: &str) -> String {
    q.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn find(s: &Srv, name: &str) -> Value {
    let v = get(s, &format!("/api/files?kind=all&limit=50&q={}", enc(name)));
    v["rows"].as_array().unwrap().iter().find(|r| r["name"] == name).unwrap_or_else(|| panic!("{name} nicht gefunden: {v}")).clone()
}

fn rule<'a>(junk: &'a Value, id: &str) -> Option<&'a Value> {
    junk["rules"].as_array().unwrap().iter().find(|r| r["id"] == id)
}

fn cleanup(s: &Srv, body: Value) -> Value {
    post(s, "/api/cleanup/start", body);
    let st = wait(s, "Aufräumen", |st| st["cleanup"]["running"] == false && !st["cleanup"]["last"].is_null());
    post(s, "/api/cleanup/ack", json!({}));
    st["cleanup"]["last"].clone()
}

fn long(p: &Path) -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(format!(r"\\?\{}", p.display()))
    } else {
        p.to_path_buf()
    }
}

/// Loescht den Testordner, wenn der Testfaden endet. Da sind die Server des Tests schon beendet.
struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(long(&self.0));
    }
}

thread_local! {
    static CLEANUP: std::cell::RefCell<Vec<Cleanup>> = const { std::cell::RefCell::new(vec![]) };
}

fn workspace(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    if d.exists() {
        std::fs::remove_dir_all(long(&d)).unwrap();
    }
    std::fs::create_dir_all(&d).unwrap();
    CLEANUP.with(|c| c.borrow_mut().push(Cleanup(d.clone())));
    d
}

fn fixture(name: &str) -> (PathBuf, PathBuf, String) {
    let base = workspace(name);
    let fx = base.join("fixture");
    let ok = Command::new(EXE).arg("--make-fixture").arg(&fx).status().unwrap();
    assert!(ok.success());
    let data = base.join("data");
    let root = fx.to_string_lossy().to_string();
    (fx, data, root)
}

fn sandbox(data: &Path, root: &str) -> Srv {
    start(&["--sandbox", root, "--data-dir", data.to_str().unwrap()])
}

#[test]
fn quit_ends_the_program() {
    let base = workspace("quit");
    let data = base.join("data");
    let mut s = start(&["--demo", "--data-dir", data.to_str().unwrap()]);
    let r = post(&s, "/api/quit", json!({}));
    assert_eq!(r["closesWindow"], false, "Ohne eigenes Fenster gibt es nichts zu schliessen");
    let t0 = Instant::now();
    loop {
        if let Some(st) = s.child.try_wait().unwrap() {
            assert_eq!(st.code(), Some(0));
            break;
        }
        assert!(t0.elapsed() < Duration::from_secs(10), "Programm laeuft nach dem Beenden weiter");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn output_to_a_closed_pipe_does_not_crash() {
    for arg in ["--help", "--version"] {
        let mut child = Command::new(EXE).arg(arg).stdout(std::process::Stdio::piped()).spawn().unwrap();
        // Der Leser ist weg, bevor das Programm schreibt: wie bei `storage-map --help | head -1`.
        drop(child.stdout.take());
        assert_eq!(child.wait().unwrap().code(), Some(0), "{arg}");
    }
}

#[test]
fn test_folder_accepts_forward_slashes_and_relative_paths() {
    let base = workspace("slashes");
    let with_slashes = base.join("fx").to_string_lossy().replace('\\', "/");
    let st = Command::new(EXE).arg("--make-fixture").arg(&with_slashes).status().unwrap();
    assert!(st.success());
    assert!(base.join("fx").join("Filme").join("urlaub-2019.mp4").exists());

    let st = Command::new(EXE).current_dir(&base).args(["--make-fixture", "relativ"]).status().unwrap();
    assert!(st.success());
    assert!(base.join("relativ").join("Filme").join("urlaub-2019.mp4").exists());
}

#[test]
fn security_rejects_foreign_requests() {
    let (_fx, data, root) = fixture("sec");
    let s = sandbox(&data, &root);
    assert_eq!(raw(s.port, "GET", "/api/status", "", None, None).0, 401, "ohne Token");
    assert_eq!(raw(s.port, "GET", "/api/status", "", Some("falsch"), None).0, 401, "falsches Token");
    assert_eq!(raw(s.port, "GET", "/api/status", "", Some("t"), Some("evil.example")).0, 403, "DNS-Rebinding");
    assert_eq!(raw(s.port, "POST", "/api/cart/clear", "{}", None, None).0, 401);
    assert_eq!(raw(s.port, "GET", "/../Cargo.toml", "", None, None).0, 403, "kein Pfad-Ausbruch");

    // Einmal-Code fuers Fenster: taugt genau einmal, und nur ein gueltiger bringt das Token.
    assert_eq!(raw(s.port, "POST", "/api/session/new", "{}", None, None).0, 401, "Code gibt es nur mit Token");
    assert_eq!(raw(s.port, "POST", "/session", r#"{"code":"falsch"}"#, None, None).0, 401);
    let code = post(&s, "/api/session/new", json!({}))["code"].as_str().unwrap().to_string();
    let body = json!({ "code": code }).to_string();
    let (c, reply) = raw(s.port, "POST", "/session", &body, None, None);
    assert_eq!(c, 200, "{reply}");
    assert_eq!(serde_json::from_str::<Value>(&reply).unwrap()["token"], "t");
    assert_eq!(raw(s.port, "POST", "/session", &body, None, None).0, 401, "zweites Mal nicht mehr");
}

#[test]
fn test_modes_never_touch_real_drives() {
    let (fx, data, root) = fixture("guard");
    let s = sandbox(&data, &root);
    assert_eq!(get(&s, "/api/status")["mode"], "sandbox");

    // Die Laufwerksliste enthaelt nur den Testordner.
    let drives = get(&s, "/api/drives");
    let rows = drives["drives"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{drives}");
    assert_eq!(rows[0]["root"], root.as_str());

    // Alles ausserhalb wird abgelehnt, bevor etwas gelesen wird.
    for p in FOREIGN {
        assert_eq!(post_code(&s, "/api/scan", json!({"path": p})), 403, "Scan von {p}");
        assert_eq!(post_code(&s, "/api/open-cached", json!({"path": p})), 403, "Laden von {p}");
        let (c, _) = raw(s.port, "GET", &format!("/api/browse?path={}", enc(p)), "", Some("t"), None);
        assert_eq!(c, 403, "Auflisten von {p}");
    }
    let escape = native(&format!(r"{root}\..\.."));
    assert_eq!(post_code(&s, "/api/scan", json!({"path": escape})), 403, "Ausbruch mit ..");
    assert!(get(&s, "/api/status")["index"].is_null(), "Es darf nichts geladen worden sein");

    // Auch der Kommandozeilen-Scan verweigert.
    let out = Command::new(EXE).args(["--sandbox", &root, "--data-dir", data.to_str().unwrap(), "--scan", FOREIGN[0]]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(fx.exists());

    // Shell-Aktionen werden nur simuliert.
    scan(&s, &root);
    let id = find(&s, "neu.zip")["id"].clone();
    assert_eq!(post(&s, "/api/reveal", json!({"id": id}))["simulated"], true);
    assert_eq!(post(&s, "/api/open", json!({"id": id}))["simulated"], true);
    assert_eq!(post_code(&s, "/api/elevate", json!({})), 400);
}

#[test]
fn scan_reports_exactly_what_is_there() {
    let (_fx, data, root) = fixture("scan");
    let s = sandbox(&data, &root);
    scan(&s, &root);

    // Unabhaengige Gegenprobe: selbst nachzaehlen und mit dem Scan vergleichen.
    fn walk(p: &Path, files: &mut u64, dirs: &mut u64, size: &mut u64) {
        for e in std::fs::read_dir(p).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                *dirs += 1;
                walk(&e.path(), files, dirs, size);
            } else {
                *files += 1;
                *size += e.metadata().unwrap().len();
            }
        }
    }
    let (mut files, mut dirs, mut size) = (0, 0, 0);
    walk(&long(Path::new(&root)), &mut files, &mut dirs, &mut size);

    let sum = get(&s, "/api/summary");
    assert_eq!(sum["files"], files);
    assert_eq!(sum["dirs"], dirs);
    assert_eq!(sum["logical"], size);
    assert_eq!(sum["isDrive"], false);
    assert_eq!(sum["denied"], 0);

    let cats = sum["cats"].as_array().unwrap();
    let video = cats.iter().find(|c| c["name"] == "Videos").unwrap();
    assert_eq!(video["files"], 3);
    assert!(video["alloc"].as_u64().unwrap() >= 36 * MB);

    assert_eq!(find(&s, "Übung größe.txt")["size"], 12 * 1024, "Umlaute");
    assert_eq!(find(&s, "tief.bin")["size"], 10 * 1024, "Pfad über 260 Zeichen");
    assert!(find(&s, "tief.bin")["path"].as_str().unwrap().len() > 300);

    // Alter kommt aus dem echten Aenderungsdatum.
    let old = find(&s, "urlaub-2019.mp4");
    let age_days = (sum["scannedAt"].as_u64().unwrap() - old["mtime"].as_u64().unwrap()) / 86_400;
    assert!((2199..=2201).contains(&age_days), "{age_days}");

    // Baum und Karte bilden den Inhalt vollstaendig ab.
    let kids = get(&s, "/api/children?id=0&limit=100");
    let sum_kids: u64 = kids["rows"].as_array().unwrap().iter().map(|r| r["alloc"].as_u64().unwrap()).sum();
    assert_eq!(sum_kids, sum["scanned"].as_u64().unwrap());
    let map = get(&s, "/api/map?id=0&limit=5000&minfrac=0");
    let top: u64 = map["nodes"].as_array().unwrap().iter().filter(|n| n[1] == 0).map(|n| n[3].as_u64().unwrap()).sum();
    assert_eq!(top, sum["scanned"].as_u64().unwrap());

    // Suche und Filter
    assert_eq!(get(&s, "/api/files?q=*.mp4")["total"], 3);
    assert_eq!(get(&s, &format!("/api/files?min={}", 10 * MB))["total"], 3);
    assert_eq!(get(&s, "/api/files?cat=0")["total"], 3);

    // Detailansicht findet den Hauptverursacher.
    let node = get(&s, &format!("/api/node?id={}", find(&s, "Filme")["id"]));
    assert_eq!(node["top"].as_array().unwrap().len(), 3);
    let _ = get(&s, "/api/types");
    let _ = get(&s, "/api/health?root=x");
}

#[test]
fn junk_rules_find_junk_and_spare_real_data() {
    let (_fx, data, root) = fixture("junk");
    let s = sandbox(&data, &root);
    scan(&s, &root);
    let j = get(&s, "/api/junk");
    assert_eq!(rule(&j, "build-rust").unwrap()["items"], 1);
    assert!(rule(&j, "build-rust").unwrap()["alloc"].as_u64().unwrap() >= 8 * MB);
    assert_eq!(rule(&j, "node-modules").unwrap()["files"], 400);
    assert_eq!(rule(&j, "pycache").unwrap()["items"], 1);
    assert_eq!(rule(&j, "venv").unwrap()["items"], 1);
    assert_eq!(rule(&j, "dumps").unwrap()["items"], 1);
    assert_eq!(rule(&j, "tmp-files").unwrap()["items"], 1);
    assert_eq!(rule(&j, "vm-disks").unwrap()["items"], 1);
    assert_eq!(rule(&j, "old-installers").unwrap()["items"], 1, "neu.zip ist zu frisch");
    assert_eq!(rule(&j, "logs").unwrap()["items"], 1, "frisch.log bleibt verschont");
    assert_eq!(j["emptyDirs"], 1);

    // Der target-Ordner ohne Cargo.toml daneben ist kein Build-Ordner.
    let items = get(&s, "/api/junk/items?rule=build-rust");
    let p = items["rows"][0]["path"].as_str().unwrap();
    assert!(p.ends_with(&native(r"Code\rust\target")), "{p}");
}

#[test]
fn duplicates_are_found_by_real_content() {
    let (_fx, data, root) = fixture("dups");
    let s = sandbox(&data, &root);
    scan(&s, &root);
    post(&s, "/api/dups/start", json!({"minSize": 1024, "includeSystem": true}));
    let st = wait(&s, "Duplikate", |st| st["dups"]["done"] == true);
    assert_eq!(st["dups"]["skipped"], 0, "{st}");
    let d = get(&s, "/api/dups");
    let groups = d["rows"].as_array().unwrap();
    let film = groups.iter().find(|g| g["size"] == 12 * MB).expect("Film-Duplikate");
    let names: Vec<&str> = film["files"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(names.contains(&"urlaub-2019.mp4") && names.contains(&"urlaub-kopie.mp4"), "{names:?}");
    assert!(!names.contains(&"fast-gleich.mp4"), "gleiche Größe und Ränder, aber anderer Inhalt");
    assert_eq!(film["verified"], true);
    assert!(groups.iter().any(|g| g["size"] == 2 * MB), "Backup-Kopien");

    // Doppelte Ordner
    let df = get(&s, &format!("/api/dupfolders?min={}&system=1", MB));
    let paths: Vec<String> = df["rows"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|r| r["folders"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap().to_string()))
        .collect();
    assert!(paths.contains(&"Backup-A".to_string()) && paths.contains(&"Backup-B".to_string()), "{paths:?}");

    // Duplikate bleiben ueber einen Neustart erhalten.
    drop(s);
    let s2 = sandbox(&data, &root);
    post(&s2, "/api/open-cached", json!({"path": root}));
    assert!(get(&s2, "/api/dups")["total"].as_u64().unwrap() >= 2);
}

#[test]
fn cleanup_removes_only_what_it_should() {
    let (fx, data, root) = fixture("clean");
    let s = sandbox(&data, &root);
    scan(&s, &root);
    let before = get(&s, "/api/summary")["scanned"].as_u64().unwrap();

    // Aufraeumliste: Ordner ersetzt enthaltene Datei, nichts wird doppelt gezaehlt.
    let log = find(&s, "alt.log");
    let daten = find(&s, "Daten");
    post(&s, "/api/cart/add", json!({"ids": [log["id"]]}));
    assert_eq!(get(&s, "/api/cart")["count"], 1);
    post(&s, "/api/cart/add", json!({"ids": [daten["id"]]}));
    let c = get(&s, "/api/cart");
    assert_eq!(c["count"], 1);
    assert_eq!(c["bytes"], daten["alloc"]);
    post(&s, "/api/cart/clear", json!({}));

    // Papierkorb-Modus legt im Test in einen eigenen Ordner, nie in den echten Papierkorb.
    let dmp = find(&s, "absturz.dmp");
    post(&s, "/api/cart/add", json!({"ids": [dmp["id"]]}));
    let last = cleanup(&s, json!({"mode": "recycle"}));
    assert_eq!(last["ok"], 1, "{last}");
    assert!(!rel(&fx, r"Daten\absturz.dmp").exists());
    assert!(data.join("sandbox-papierkorb").join("000000-absturz.dmp").exists(), "wiederherstellbar");
    let after = get(&s, "/api/summary");
    assert_eq!(after["scanned"].as_u64().unwrap(), before - dmp["alloc"].as_u64().unwrap());
    assert_eq!(get(&s, "/api/cart")["count"], 0);
    let gone = get(&s, &format!("/api/files?q={}", enc("absturz.dmp")));
    assert_eq!(gone["total"], 0, "Entfernte Datei taucht nicht mehr auf");

    // Geaenderte Dateien bleiben liegen.
    let xl = find(&s, "wichtig.xlsx");
    std::fs::write(rel(&fx, r"Daten\wichtig.xlsx"), b"inzwischen anders").unwrap();
    post(&s, "/api/cart/add", json!({"ids": [xl["id"]]}));
    let last = cleanup(&s, json!({"mode": "permanent", "confirm": "LÖSCHEN"}));
    assert_eq!(last["ok"], 0, "{last}");
    assert!(last["failures"][0]["reason"].as_str().unwrap().contains("geändert"), "{last}");
    assert!(rel(&fx, r"Daten\wichtig.xlsx").exists());
    post(&s, "/api/cart/clear", json!({}));

    // Endgueltiges Loeschen nur mit Bestaetigungswort.
    let tmp = find(&s, "reste.tmp");
    post(&s, "/api/cart/add", json!({"ids": [tmp["id"]]}));
    assert_eq!(post_code(&s, "/api/cleanup/start", json!({"mode": "permanent"})), 400);
    assert_eq!(post_code(&s, "/api/cleanup/start", json!({"mode": "permanent", "confirm": "ja"})), 400);
    assert!(rel(&fx, r"Daten\reste.tmp").exists());
    let last = cleanup(&s, json!({"mode": "permanent", "confirm": "LÖSCHEN"}));
    assert_eq!(last["ok"], 1, "{last}");
    assert!(!rel(&fx, r"Daten\reste.tmp").exists());

    // Verschieben, und zwar nur innerhalb des Testordners.
    let zip = find(&s, "neu.zip");
    post(&s, "/api/cart/add", json!({"ids": [zip["id"]]}));
    assert_eq!(post_code(&s, "/api/cleanup/start", json!({"mode": "move", "dest": FOREIGN[0]})), 403);
    let dest = fx.join("Leer").to_string_lossy().to_string();
    let last = cleanup(&s, json!({"mode": "move", "dest": dest}));
    assert_eq!(last["ok"], 1, "{last}");
    assert!(rel(&fx, r"Leer\neu.zip").exists());
    assert!(!rel(&fx, r"Downloads\neu.zip").exists());

    // Die Aufraeumliste uebersteht einen Neustart, Entferntes verschwindet aus ihr.
    let keep = find(&s, "wichtig.xlsx");
    post(&s, "/api/cart/add", json!({"ids": [keep["id"]]}));

    // Die Wurzel selbst laesst sich nicht auf die Liste setzen.
    let r = post(&s, "/api/cart/add", json!({"ids": [0]}));
    assert_eq!(r["added"], 0);

    // Der gespeicherte Stand kennt die Loeschungen auch nach einem Neustart.
    drop(s);
    let s2 = sandbox(&data, &root);
    post(&s2, "/api/open-cached", json!({"path": root}));
    assert_eq!(get(&s2, &format!("/api/files?q={}", enc("absturz.dmp")))["total"], 0);
    let c = get(&s2, "/api/cart");
    assert_eq!(c["count"], 1, "{c}");
    assert_eq!(c["rows"][0]["name"], "wichtig.xlsx");
}

#[test]
fn history_explains_what_grew() {
    let (fx, data, root) = fixture("hist");
    let s = sandbox(&data, &root);
    scan(&s, &root);
    let first = get(&s, "/api/summary")["scannedAt"].as_u64().unwrap();
    std::thread::sleep(Duration::from_millis(1100));

    std::fs::create_dir_all(rel(&fx, r"Neu\Render")).unwrap();
    std::fs::write(rel(&fx, r"Neu\Render\ausgabe.bin"), vec![9u8; 30 * MB as usize]).unwrap();
    scan(&s, &root);
    let h = get(&s, "/api/history");
    assert_eq!(h["entries"].as_array().unwrap().len(), 2);

    let d = get(&s, &format!("/api/diff?ts={first}"));
    let top = &d["rows"][0];
    assert!(top["path"].as_str().unwrap().ends_with(&native(r"Neu\Render")), "{d}");
    assert_eq!(top["status"], "new");
    assert_eq!(top["delta"], 30 * MB);
}

#[test]
fn csv_export_works_and_needs_token() {
    let (_fx, data, root) = fixture("csv");
    let s = sandbox(&data, &root);
    scan(&s, &root);
    let (c, _) = raw(s.port, "GET", "/api/export?what=files", "", None, None);
    assert_eq!(c, 401);
    let (c, body) = raw(s.port, "GET", "/api/export?what=files&t=t&q=*.mp4", "", None, None);
    assert_eq!(c, 200);
    assert!(body.starts_with('\u{feff}'), "BOM für Excel");
    assert_eq!(body.lines().count(), 4, "{body}");
    assert!(body.contains("urlaub-2019.mp4"));
}

#[test]
fn demo_mode_works_without_any_file_access() {
    let base = workspace("demo");
    let data = base.join("data");
    let s = start(&["--demo", "--data-dir", data.to_str().unwrap()]);
    assert_eq!(get(&s, "/api/status")["mode"], "demo");
    let (first, second, real_folder, other, system_file) =
        if cfg!(windows) { (r"C:\", r"E:\", r"C:\Users", r"D:\", "sys00001.dll") } else { ("/", "/mnt/archiv", "/home", "/etc", "sys00001.so") };
    let d = get(&s, "/api/drives");
    assert!(d["drives"].as_array().unwrap().iter().all(|x| x["label"].as_str().unwrap().starts_with("DEMO")), "{d}");

    // Nur erfundene Laufwerke lassen sich "scannen", kein echter Ordner.
    assert_eq!(post_code(&s, "/api/scan", json!({"path": real_folder})), 403);
    assert_eq!(post_code(&s, "/api/scan", json!({"path": other})), 403);

    post(&s, "/api/open-cached", json!({"path": first}));
    let sum = get(&s, "/api/summary");
    assert!(sum["files"].as_u64().unwrap() > 100_000);
    assert!(sum["recs"].as_array().unwrap().len() > 5);
    assert!(sum["unaccounted"].as_u64().unwrap() > 0);
    let h = get(&s, "/api/history");
    assert_eq!(h["entries"].as_array().unwrap().len(), 10);
    let ts = h["entries"][8]["ts"].as_u64().unwrap();
    assert!(!get(&s, &format!("/api/diff?ts={ts}"))["rows"].as_array().unwrap().is_empty());

    // Schutzregeln gelten auch in der Demo.
    let win = find(&s, system_file);
    let r = post(&s, "/api/cart/add", json!({"ids": [win["id"]]}));
    assert_eq!(r["added"], 0, "{r}");
    assert!(!r["blocked"].as_array().unwrap().is_empty());

    // Loeschen ist simuliert und aktualisiert nur die Zahlen.
    let before = sum["scanned"].as_u64().unwrap();
    let f = find(&s, "aktuell.pdf");
    post(&s, "/api/cart/add", json!({"ids": [f["id"]]}));
    let last = cleanup(&s, json!({"mode": "recycle"}));
    assert_eq!(last["ok"], 1);
    assert_eq!(get(&s, "/api/summary")["scanned"].as_u64().unwrap(), before - f["alloc"].as_u64().unwrap());

    // Simulierter Duplikat-Lauf
    post(&s, "/api/dups/start", json!({"minSize": MB}));
    wait(&s, "Demo-Duplikate", |st| st["dups"]["done"] == true);
    assert!(get(&s, "/api/dups")["total"].as_u64().unwrap() >= 2);

    // Simulierter Scan-Lauf
    post(&s, "/api/scan", json!({"path": second}));
    let st = get(&s, "/api/status");
    assert!(!st["scan"].is_null());
    wait(&s, "Demo-Scan", |st| st["scan"].is_null());
    assert_eq!(get(&s, "/api/summary")["root"], second);
}
