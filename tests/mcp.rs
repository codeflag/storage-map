//! Prueft den MCP-Server von aussen, so wie ihn ein Client startet: ueber stdin und stdout.
//! Alles laeuft im Test-Betrieb auf einem eigenen Testordner unter target/tmp. Echte Laufwerke sind tabu.

use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

const EXE: &str = env!("CARGO_BIN_EXE_storage-map");

struct Client {
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<Result<Value, String>>,
    next_id: u64,
    notifications: Vec<Value>,
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(args: &[&str]) -> Client {
    let mut child = Command::new(EXE).arg("--mcp").args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            // Auf stdout darf nur JSON stehen, sonst bricht der Client die Verbindung ab.
            let parsed = serde_json::from_str::<Value>(&line).map_err(|_| line);
            if tx.send(parsed).is_err() {
                break;
            }
        }
    });
    Client { child, stdin, rx, next_id: 1, notifications: vec![] }
}

impl Client {
    fn send_raw(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
    }

    fn next_message(&mut self) -> Value {
        match self.rx.recv_timeout(Duration::from_secs(60)) {
            Ok(Ok(v)) => v,
            Ok(Err(line)) => panic!("Kein JSON auf stdout: {line}"),
            Err(e) => panic!("Keine Antwort: {e}"),
        }
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send_raw(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string());
        loop {
            let m = self.next_message();
            if m["id"] == id {
                return m;
            }
            self.notifications.push(m);
        }
    }

    fn initialize(&mut self) -> Value {
        let r = self.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test-client", "version": "1"}}),
        );
        self.send_raw(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}).to_string());
        r
    }

    fn tool_names(&mut self) -> Vec<String> {
        let r = self.request("tools/list", json!({}));
        r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
    }

    /// Ruft ein Werkzeug auf. `Ok` mit dem Ergebnis oder `Err` mit der Fehlermeldung des Werkzeugs.
    fn call(&mut self, tool: &str, args: Value) -> Result<Value, String> {
        let r = self.request("tools/call", json!({"name": tool, "arguments": args}));
        let text = r["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("{r}")).to_string();
        if r["result"]["isError"] == true {
            Err(text)
        } else {
            Ok(serde_json::from_str(&text).unwrap_or_else(|_| panic!("Kein JSON: {text}")))
        }
    }

    fn ok(&mut self, tool: &str, args: Value) -> Value {
        self.call(tool, args).unwrap_or_else(|e| panic!("{tool}: {e}"))
    }

    fn err(&mut self, tool: &str, args: Value) -> String {
        match self.call(tool, args) {
            Err(e) => e,
            Ok(v) => panic!("{tool} haette scheitern muessen: {v}"),
        }
    }
}

struct Space {
    base: PathBuf,
    fx: PathBuf,
    data: PathBuf,
    root: String,
}

impl Drop for Space {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

fn write(p: &Path, size: usize, seed: u8) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    let data: Vec<u8> = (0..size).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect();
    std::fs::write(p, data).unwrap();
}

/// Kleiner Testordner mit bekanntem Inhalt, per `--scan` erfasst wie vom Nutzer.
fn space(name: &str) -> Space {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("mcp-{name}"));
    let _ = std::fs::remove_dir_all(&base);
    let fx = base.join("fixture");
    let data = base.join("data");
    std::fs::create_dir_all(&fx).unwrap();
    write(&fx.join("Filme").join("urlaub.mp4"), 300_000, 1);
    write(&fx.join("Filme").join("urlaub-kopie.mp4"), 300_000, 1);
    write(&fx.join("Filme").join("anders.mp4"), 300_000, 2);
    write(&fx.join("Daten").join("absturz.dmp"), 50_000, 3);
    write(&fx.join("Daten").join("wichtig.xlsx"), 40_000, 4);
    write(&fx.join("Code").join("rust").join("Cargo.toml"), 100, 5);
    write(&fx.join("Code").join("rust").join("target").join("app.bin"), 90_000, 6);
    std::fs::create_dir_all(fx.join("Ziel")).unwrap();
    let root = fx.to_string_lossy().to_string();
    let st = Command::new(EXE).args(["--sandbox", &root, "--data-dir", data.to_str().unwrap(), "--scan", &root]).status().unwrap();
    assert!(st.success());
    Space { base, fx, data, root }
}

impl Space {
    fn client(&self) -> Client {
        let mut c = spawn(&["--sandbox", &self.root, "--data-dir", self.data.to_str().unwrap()]);
        c.initialize();
        c
    }

    fn set_config(&self, cfg: Value) {
        std::fs::write(self.data.join("config.json"), cfg.to_string()).unwrap();
    }

    fn path(&self, rel: &[&str]) -> String {
        let mut p = self.fx.clone();
        for r in rel {
            p.push(r);
        }
        p.to_string_lossy().to_string()
    }

    fn tree(&self) -> HashSet<String> {
        fn walk(p: &Path, out: &mut HashSet<String>) {
            for e in std::fs::read_dir(p).unwrap().flatten() {
                out.insert(e.path().to_string_lossy().to_string());
                if e.file_type().unwrap().is_dir() {
                    walk(&e.path(), out);
                }
            }
        }
        let mut out = HashSet::new();
        walk(&self.fx, &mut out);
        out
    }
}

const WRITE_TOOLS: &[&str] = &["add_to_cleanup_list", "remove_from_cleanup_list", "preview_cleanup", "move_files"];
const DELETE_TOOLS: &[&str] = &["recycle_files", "delete_permanently"];
const SCAN_TOOLS: &[&str] = &["start_scan", "start_duplicate_search", "cancel_operation"];

#[test]
fn speaks_the_protocol() {
    let s = space("proto");
    let mut c = spawn(&["--sandbox", &s.root, "--data-dir", s.data.to_str().unwrap()]);

    // Handschlag: bekannte Version wird bestaetigt, unbekannte bekommt unsere neueste alte.
    let r = c.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}));
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(r["result"]["serverInfo"]["name"], "storage-map");
    assert_eq!(r["result"]["capabilities"]["tools"]["listChanged"], true);
    assert!(r["result"]["instructions"].as_str().unwrap().contains("untrusted"));
    let r = c.request("initialize", json!({"protocolVersion": "1999-01-01", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}));
    assert_eq!(r["result"]["protocolVersion"], "2025-11-25");

    // Fehlerfaelle von JSON-RPC
    c.send_raw("das ist kein json");
    assert_eq!(c.next_message()["error"]["code"], -32700);
    c.send_raw(r#"{"jsonrpc":"1.0","id":5,"method":"ping"}"#);
    assert_eq!(c.next_message()["error"]["code"], -32600);
    assert_eq!(c.request("gibt/es/nicht", json!({}))["error"]["code"], -32601);
    assert_eq!(c.request("tools/call", json!({"name": "gibt_es_nicht"}))["error"]["code"], -32602);
    assert_eq!(c.request("tools/call", json!({"arguments": {}}))["error"]["code"], -32602);
    assert!(c.request("ping", json!({}))["result"].is_object());

    // Benachrichtigungen bekommen keine Antwort: Die naechste Nachricht gehoert zur naechsten Anfrage.
    c.send_raw(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    c.send_raw(r#"{"jsonrpc":"2.0","id":"text-id","method":"ping"}"#);
    assert_eq!(c.next_message()["id"], "text-id");

    // Stapel aelterer Revisionen
    c.send_raw(
        r#"[{"jsonrpc":"2.0","id":71,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/x"},{"jsonrpc":"2.0","id":72,"method":"ping"}]"#,
    );
    let batch = c.next_message();
    assert_eq!(batch.as_array().unwrap().len(), 2, "{batch}");

    // Jedes Werkzeug beschreibt sich vollstaendig.
    let list = c.request("tools/list", json!({}));
    for t in list["result"]["tools"].as_array().unwrap() {
        assert!(t["description"].as_str().unwrap().len() > 10, "{t}");
        assert_eq!(t["inputSchema"]["type"], "object", "{t}");
        assert_eq!(t["annotations"]["openWorldHint"], false, "{t}");
        assert_eq!(t["annotations"]["readOnlyHint"], true, "Beim Lesen darf nur Lesendes sichtbar sein: {t}");
    }
}

#[test]
fn speaks_the_stateless_revision_too() {
    let s = space("modern");
    let mut c = spawn(&["--sandbox", &s.root, "--data-dir", s.data.to_str().unwrap()]);
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "neu", "version": "1"},
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    let d = c.request("server/discover", json!({ "_meta": meta }));
    assert_eq!(d["result"]["resultType"], "complete", "{d}");
    assert!(d["result"]["supportedVersions"].as_array().unwrap().iter().any(|v| v == "2026-07-28"));
    assert_eq!(d["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "storage-map");

    // Kein Handschlag noetig. Listen sagen, wie lange sie gelten.
    let l = c.request("tools/list", json!({ "_meta": meta }));
    assert_eq!(l["result"]["resultType"], "complete");
    assert_eq!(l["result"]["ttlMs"], 0);
    assert_eq!(l["result"]["cacheScope"], "private");
    let r = c.request("tools/call", json!({"_meta": meta, "name": "get_access_status", "arguments": {}}));
    assert_eq!(r["result"]["resultType"], "complete");
    assert_eq!(r["result"]["isError"], false);

    // Unbekannte Version: fester Fehlercode und Liste der moeglichen.
    let bad = json!({"_meta": {"io.modelcontextprotocol/protocolVersion": "1900-01-01"}});
    let r = c.request("tools/list", bad);
    assert_eq!(r["error"]["code"], -32022, "{r}");
    assert_eq!(r["error"]["data"]["requested"], "1900-01-01");
    assert!(r["error"]["data"]["supported"].as_array().unwrap().len() >= 2);

    // Benachrichtigungen an Clients ohne Handschlag gibt es nicht.
    s.set_config(json!({"mcp": {"level": "write"}}));
    std::thread::sleep(Duration::from_millis(2500));
    assert!(c.rx.try_recv().is_err(), "Ohne Handschlag keine Benachrichtigung");
}

#[test]
fn read_level_is_the_default_and_changes_nothing() {
    let s = space("read");
    let before = s.tree();
    let mut c = s.client();

    let tools = c.tool_names();
    for t in WRITE_TOOLS.iter().chain(DELETE_TOOLS).chain(SCAN_TOOLS) {
        assert!(!tools.iter().any(|x| x == t), "{t} darf nicht sichtbar sein");
    }

    let st = c.ok("get_access_status", json!({}));
    assert_eq!(st["level"], "read");
    assert_eq!(st["may_start_scans"], false);

    // Laufwerke: nur der Testordner.
    let d = c.ok("list_drives", json!({}));
    assert_eq!(d["drives"].as_array().unwrap().len(), 1, "{d}");
    assert_eq!(d["drives"][0]["root"], s.root.as_str());

    // Die Zahlen stimmen mit dem Testordner ueberein.
    let o = c.ok("get_overview", json!({"root": s.root}));
    assert_eq!(o["files"], 7);
    assert_eq!(o["folders"], 6);
    assert!(o["categories"].as_array().unwrap().iter().any(|x| x["category"] == "video"), "{o}");
    assert_eq!(o["largest_files"].as_array().unwrap().len(), 7);

    let f = c.ok("search_files", json!({"query": "*.mp4"}));
    assert_eq!(f["total"], 3, "{f}");
    assert_eq!(f["rows"][0]["type"], "file");
    assert!(f["rows"][0]["bytes"].as_u64().unwrap() > 0);
    assert!(f["rows"][0]["size"].as_str().unwrap().contains("KB"));
    let f = c.ok("search_files", json!({"category": "document"}));
    assert_eq!(f["total"], 1, "{f}");
    let f = c.ok("search_files", json!({"min_bytes": 100_000, "under": s.path(&["Filme"])}));
    assert_eq!(f["total"], 3, "{f}");
    let f = c.ok("search_files", json!({"limit": 2}));
    assert_eq!(f["rows"].as_array().unwrap().len(), 2);
    assert_eq!(f["more"], true);
    assert!(c.err("search_files", json!({"category": "gibtsnicht"})).contains("Unknown category"));

    let l = c.ok("list_folder", json!({"path": s.path(&["Daten"])}));
    assert_eq!(l["total"], 2);
    assert_eq!(l["rows"][0]["path"], s.path(&["Daten", "absturz.dmp"]));
    assert!(c.err("list_folder", json!({"path": s.path(&["GibtEsNicht"])})).contains("Not found"));

    let i = c.ok("get_item", json!({"path": s.path(&["Filme"])}));
    assert_eq!(i["type"], "folder");
    assert_eq!(i["files"], 3);
    assert_eq!(i["largest_contents"].as_array().unwrap().len(), 3);

    let j = c.ok("get_cleanup_suggestions", json!({}));
    let rules = j["rules"].as_array().unwrap();
    assert!(rules.iter().any(|r| r["rule"] == "build-rust"), "{j}");
    assert!(rules.iter().any(|r| r["rule"] == "dumps"), "{j}");
    let items = c.ok("list_cleanup_items", json!({"rule": "dumps"}));
    assert_eq!(items["rows"][0]["path"], s.path(&["Daten", "absturz.dmp"]));

    assert_eq!(c.ok("get_duplicates", json!({}))["searched"], false);
    assert!(c.ok("get_file_types", json!({}))["rows"].as_array().unwrap().iter().any(|r| r["extension"] == "mp4"));
    assert_eq!(c.ok("get_stale_folders", json!({"older_than_days": 36500}))["total"], 0);
    let h = c.ok("get_history", json!({}));
    assert_eq!(h["scans"].as_array().unwrap().len(), 1);
    assert_eq!(c.ok("get_cleanup_list", json!({}))["count"], 0);
    assert!(c.ok("get_operation_status", json!({}))["scan"].is_null());

    // Alles Veraendernde wird verweigert, auch wenn der Client es trotzdem versucht.
    let msg = c.err("add_to_cleanup_list", json!({"paths": [s.path(&["Daten", "absturz.dmp"])]}));
    assert!(msg.contains("write"), "{msg}");
    assert!(c.err("recycle_files", json!({"plan_id": "x"})).contains("delete"));
    assert!(c.err("delete_permanently", json!({"plan_id": "x"})).contains("not allowed"));
    assert!(c.err("start_scan", json!({"root": s.root})).contains("not allowed"));
    assert!(c.err("move_files", json!({"plan_id": "x", "target_dir": s.path(&["Ziel"])})).contains("write"));

    assert_eq!(s.tree(), before, "Beim Lesen darf sich nichts aendern");
}

#[test]
fn scans_need_their_own_permission() {
    let s = space("scan");
    s.set_config(json!({"mcp": {"level": "read", "allow_scan": true}}));
    let mut c = s.client();
    let tools = c.tool_names();
    assert!(tools.iter().any(|t| t == "start_scan"));
    assert!(!tools.iter().any(|t| t == "add_to_cleanup_list"), "Scannen heisst nicht schreiben");

    // Neue Datei, neuer Scan: Das Ergebnis enthaelt sie.
    write(&s.fx.join("neu.mp4"), 5000, 9);
    c.ok("start_scan", json!({"root": s.root}));
    let t0 = Instant::now();
    while !c.ok("get_operation_status", json!({}))["scan"].is_null() {
        assert!(t0.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(c.ok("search_files", json!({"query": "*.mp4"}))["total"], 4);

    // Duplikate nach echtem Inhalt: gleiche Groesse allein reicht nicht.
    c.ok("start_duplicate_search", json!({"min_bytes": 1000, "include_system": true}));
    let t0 = Instant::now();
    while c.ok("get_operation_status", json!({}))["duplicate_search"]["done"] != true {
        assert!(t0.elapsed() < Duration::from_secs(60));
        std::thread::sleep(Duration::from_millis(50));
    }
    let d = c.ok("get_duplicates", json!({}));
    assert_eq!(d["searched"], true);
    assert_eq!(d["total"], 1, "{d}");
    let names: Vec<String> = d["rows"][0]["files"].as_array().unwrap().iter().map(|f| f["path"].as_str().unwrap().to_string()).collect();
    assert_eq!(names.len(), 2);
    assert!(names.iter().all(|n| n.contains("urlaub")), "{names:?}");

    // Ausserhalb des Testordners wird nicht gescannt.
    let foreign = if cfg!(windows) { r"C:\" } else { "/" };
    let e = c.err("start_scan", json!({"root": foreign}));
    assert!(e.contains("test folder"), "{e}");
}

#[test]
fn write_level_edits_the_list_and_moves_but_never_deletes() {
    let s = space("write");
    let mut c = s.client();
    assert!(!c.tool_names().iter().any(|t| t == "preview_cleanup"));

    // Hoehere Stufe: Der Client erfaehrt es, ohne neu zu starten.
    s.set_config(json!({"mcp": {"level": "write"}}));
    let t0 = Instant::now();
    loop {
        if let Ok(Ok(m)) = c.rx.recv_timeout(Duration::from_millis(200)) {
            if m["method"] == "notifications/tools/list_changed" {
                break;
            }
        }
        assert!(t0.elapsed() < Duration::from_secs(10), "Keine Benachrichtigung");
    }
    let tools = c.tool_names();
    assert!(WRITE_TOOLS.iter().all(|t| tools.iter().any(|x| x == t)));
    assert!(!DELETE_TOOLS.iter().any(|t| tools.iter().any(|x| x == t)));

    // Liste bearbeiten aendert nichts auf der Platte.
    let before = s.tree();
    let dmp = s.path(&["Daten", "absturz.dmp"]);
    let r = c.ok("add_to_cleanup_list", json!({"paths": [dmp, s.path(&["GibtEsNicht"])]}));
    assert_eq!(r["added"], 1, "{r}");
    assert_eq!(r["not_found"].as_array().unwrap().len(), 1);
    assert_eq!(c.ok("get_cleanup_list", json!({}))["count"], 1);
    let r = c.ok("add_to_cleanup_list", json!({"rule": "build-rust"}));
    assert_eq!(r["count"], 2, "{r}");
    assert!(c.err("add_to_cleanup_list", json!({})).contains("paths or a rule"));
    assert!(c.err("add_to_cleanup_list", json!({"rule": "gibtsnicht"})).contains("Unknown rule"));
    assert_eq!(c.ok("remove_from_cleanup_list", json!({"paths": [dmp]}))["count"], 1);
    assert_eq!(c.ok("remove_from_cleanup_list", json!({"all": true}))["count"], 0);
    assert_eq!(s.tree(), before, "Die Liste allein aendert nichts");
    assert!(c.err("preview_cleanup", json!({})).contains("empty"));

    // Verschieben: nur mit Plan, nur in den Testordner, und es wird nichts ueberschrieben.
    c.ok("add_to_cleanup_list", json!({"paths": [dmp]}));
    assert!(c.err("move_files", json!({"plan_id": "erfunden", "target_dir": s.path(&["Ziel"])})).contains("plan_id"));
    let plan = c.ok("preview_cleanup", json!({}));
    assert_eq!(plan["count"], 1);
    let id = plan["plan_id"].as_str().unwrap().to_string();

    let foreign = if cfg!(windows) { r"C:\" } else { "/tmp" };
    assert!(c.err("move_files", json!({"plan_id": id, "target_dir": foreign})).contains("test folder"));
    let dots = format!("{}{}..", s.path(&["Ziel"]), std::path::MAIN_SEPARATOR);
    assert!(c.err("move_files", json!({"plan_id": id, "target_dir": dots})).contains(".."));

    let r = c.ok("move_files", json!({"plan_id": id, "target_dir": s.path(&["Ziel"])}));
    assert_eq!(r["done"], true, "{r}");
    assert_eq!(r["succeeded"], 1, "{r}");
    assert!(s.fx.join("Ziel").join("absturz.dmp").exists());
    assert!(!s.fx.join("Daten").join("absturz.dmp").exists());
    assert_eq!(c.ok("get_cleanup_list", json!({}))["count"], 0);
    assert!(c.err("move_files", json!({"plan_id": id, "target_dir": s.path(&["Ziel"])})).contains("already used"), "Plan gilt nur einmal");

    // Der neue Stand steht auch im Scan.
    assert_eq!(c.ok("search_files", json!({"query": "absturz"}))["total"], 0);
}

#[test]
fn delete_level_goes_through_plan_and_recycle_bin() {
    let s = space("delete");
    s.set_config(json!({"mcp": {"level": "delete"}}));
    let mut c = s.client();
    let tools = c.tool_names();
    assert!(tools.iter().any(|t| t == "recycle_files"));
    assert!(!tools.iter().any(|t| t == "delete_permanently"), "Endgueltig nur mit eigener Freigabe");
    assert!(!tools.iter().any(|t| t == "start_scan"));

    let dmp = s.path(&["Daten", "absturz.dmp"]);
    let xl = s.path(&["Daten", "wichtig.xlsx"]);
    c.ok("add_to_cleanup_list", json!({"paths": [dmp]}));
    let plan = c.ok("preview_cleanup", json!({}));
    let id = plan["plan_id"].as_str().unwrap().to_string();
    assert_eq!(plan["largest"][0]["path"], dmp.as_str());

    // Aendert sich die Liste nach der Vorschau, gilt der Plan nicht mehr.
    c.ok("add_to_cleanup_list", json!({"paths": [xl]}));
    assert!(c.err("recycle_files", json!({"plan_id": id})).contains("changed"));
    assert!(s.fx.join("Daten").join("absturz.dmp").exists());

    let plan = c.ok("preview_cleanup", json!({}));
    let id = plan["plan_id"].as_str().unwrap().to_string();

    // Eine seit dem Scan geaenderte Datei bleibt liegen.
    write(&s.fx.join("Daten").join("wichtig.xlsx"), 12_345, 8);
    let r = c.ok("recycle_files", json!({"plan_id": id}));
    assert_eq!(r["succeeded"], 1, "{r}");
    assert_eq!(r["failed"], 1, "{r}");
    assert!(r["failures"][0]["reason"].as_str().unwrap().contains("changed"), "{r}");
    assert!(s.fx.join("Daten").join("wichtig.xlsx").exists());
    assert!(!s.fx.join("Daten").join("absturz.dmp").exists());
    assert!(
        s.data.join("sandbox-papierkorb").join("000000-absturz.dmp").exists()
            || s.data.join("sandbox-papierkorb").join("000001-absturz.dmp").exists(),
        "wiederherstellbar"
    );

    // Endgueltig loeschen erst nach ausdruecklicher Freigabe.
    c.ok("remove_from_cleanup_list", json!({"all": true}));
    c.ok("add_to_cleanup_list", json!({"rule": "build-rust"}));
    let id = c.ok("preview_cleanup", json!({}))["plan_id"].as_str().unwrap().to_string();
    assert!(c.err("delete_permanently", json!({"plan_id": id})).contains("not allowed"));
    assert!(s.fx.join("Code").join("rust").join("target").exists());
    s.set_config(json!({"mcp": {"level": "delete", "allow_permanent_delete": true}}));
    let r = c.ok("delete_permanently", json!({"plan_id": id}));
    assert_eq!(r["succeeded"], 1, "{r}");
    assert!(!s.fx.join("Code").join("rust").join("target").exists());

    // Rechte zurueckdrehen wirkt sofort, ohne Neustart.
    s.set_config(json!({"mcp": {"level": "read"}}));
    assert!(c.err("add_to_cleanup_list", json!({"paths": [xl]})).contains("write"));
}

#[test]
fn broken_settings_close_access() {
    let s = space("broken");
    std::fs::create_dir_all(&s.data).unwrap();
    std::fs::write(s.data.join("config.json"), "{ kaputt").unwrap();
    let mut c = s.client();
    assert_eq!(c.tool_names(), vec!["get_access_status"]);
    let st = c.ok("get_access_status", json!({}));
    assert_eq!(st["level"], "off");
    assert!(!st["config_error"].is_null());
    assert!(c.err("get_overview", json!({})).contains("damaged"));
    s.set_config(json!({"mcp": {"level": "alles"}}));
    assert!(c.err("list_drives", json!({})).contains("damaged"));
    s.set_config(json!({"mcp": {"level": "off"}}));
    assert!(c.err("list_drives", json!({})).contains("switched off"));
}

#[test]
fn visibility_limits_what_a_client_sees() {
    let s = space("roots");
    s.set_config(json!({"mcp": {"level": "delete", "roots": [s.path(&["Ziel"])]}}));
    let mut c = s.client();
    assert!(c.ok("list_drives", json!({}))["drives"].as_array().unwrap().is_empty());
    assert!(c.err("get_overview", json!({"root": s.root})).contains("not made"));
    s.set_config(json!({"mcp": {"level": "delete", "roots": [s.root]}}));
    assert_eq!(c.ok("list_drives", json!({}))["drives"].as_array().unwrap().len(), 1);
    assert_eq!(c.ok("get_overview", json!({"root": s.root}))["files"], 7);
}

#[test]
fn demo_mode_protects_system_paths_and_warns() {
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mcp-demo");
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(base.join("config.json"), json!({"mcp": {"level": "delete"}}).to_string()).unwrap();
    let mut c = spawn(&["--demo", "--data-dir", base.to_str().unwrap()]);
    c.initialize();

    let (root, system, warned, target_bad, doc) = if cfg!(windows) {
        (r"C:\", r"C:\Windows\System32\sys00001.dll", r"C:\Program Files\ImageBench\bin", r"C:\Windows", r"C:\Users\Demo\Downloads\current.pdf")
    } else {
        ("/", "/usr/lib/sys00001.so", "/opt/cutbench", "/usr/lib", "/home/demo/Downloads/current.pdf")
    };
    let d = c.ok("list_drives", json!({}));
    assert!(d["drives"].as_array().unwrap().iter().all(|x| x["label"].as_str().unwrap().starts_with("DEMO")), "{d}");

    // Systemdateien kommen nie auf die Liste.
    let r = c.ok("add_to_cleanup_list", json!({"root": root, "paths": [system]}));
    assert_eq!(r["added"], 0, "{r}");
    assert_eq!(r["protected_and_refused"].as_array().unwrap().len(), 1, "{r}");
    let i = c.ok("get_item", json!({"root": root, "path": system}));
    assert!(i["protected"].is_string(), "{i}");
    // Alles, was der Client liest, ist Englisch.
    assert!(i["protected"].as_str().unwrap().contains("could damage"), "{i}");
    assert!(r["protected_and_refused"][0]["why"].as_str().unwrap().contains("could damage"), "{r}");
    let umlaut = |v: &Value| v.to_string().contains(['ä', 'ö', 'ü', 'Ä', 'Ö', 'Ü', 'ß']);
    let tips = c.ok("get_cleanup_suggestions", json!({"root": root}));
    assert!(tips["rules"].as_array().unwrap().len() > 5, "{tips}");
    assert!(!umlaut(&tips), "{tips}");
    let sum = c.ok("get_overview", json!({"root": root}));
    assert!(!umlaut(&sum["suggestions"]), "{sum}");
    let instructions = c.request("server/discover", json!({}));
    assert!(!instructions.to_string().contains("German"), "{instructions}");
    // Regeln, die nur informieren, lassen sich nicht abarbeiten.
    assert!(c.err("add_to_cleanup_list", json!({"root": root, "rule": "journal"})).contains("cannot be handled"));

    // Warnungen muessen bestaetigt werden.
    c.ok("add_to_cleanup_list", json!({"root": root, "paths": [warned]}));
    let plan = c.ok("preview_cleanup", json!({"root": root}));
    assert_eq!(plan["with_warnings"], 1, "{plan}");
    let id = plan["plan_id"].as_str().unwrap().to_string();
    assert!(c.err("recycle_files", json!({"plan_id": id})).contains("acknowledge_warnings"));
    let r = c.ok("recycle_files", json!({"plan_id": id, "acknowledge_warnings": true}));
    assert_eq!(r["succeeded"], 1, "{r}");

    // Ziel in einem Systemordner wird abgelehnt.
    c.ok("add_to_cleanup_list", json!({"root": root, "paths": [doc]}));
    let id = c.ok("preview_cleanup", json!({"root": root}))["plan_id"].as_str().unwrap().to_string();
    assert!(c.err("move_files", json!({"plan_id": id, "target_dir": target_bad})).contains("Cannot move"));

    // Echte Pfade ausserhalb der Demo gibt es nicht.
    let foreign = if cfg!(windows) { r"D:\" } else { "/etc" };
    assert!(c.err("get_overview", json!({"root": foreign})).contains("test folder"));
    drop(c);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn every_call_lands_in_the_audit_log() {
    let s = space("audit");
    let mut c = s.client();
    c.ok("list_drives", json!({}));
    c.err("recycle_files", json!({"plan_id": "x"}));
    let log = std::fs::read_to_string(s.data.join("mcp-audit.jsonl")).unwrap();
    let rows: Vec<Value> = log.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(rows.len(), 2, "{log}");
    assert_eq!(rows[0]["tool"], "list_drives");
    assert_eq!(rows[0]["outcome"], "ok");
    assert_eq!(rows[0]["client"], "test-client");
    assert_eq!(rows[1]["tool"], "recycle_files");
    assert_eq!(rows[1]["outcome"], "denied");
    assert!(rows[0]["ts"].as_u64().unwrap() > 1_600_000_000);
}

/* ---------- Zusammenspiel mit der Oberflaeche ---------- */

struct Gui {
    child: Child,
    port: u16,
}

impl Drop for Gui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn gui(s: &Space) -> Gui {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let child = Command::new(EXE)
        .args(["--sandbox", &s.root, "--data-dir", s.data.to_str().unwrap(), "--no-open", "--token", "t", "--port", &port.to_string()])
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let t0 = Instant::now();
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(t0.elapsed() < Duration::from_secs(30));
        std::thread::sleep(Duration::from_millis(50));
    }
    Gui { child, port }
}

fn http(g: &Gui, method: &str, path: &str, body: &str) -> (u16, Value) {
    let mut s = TcpStream::connect(("127.0.0.1", g.port)).unwrap();
    write!(
        s,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-SM-Token: t\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        g.port,
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
        let mut out = vec![];
        let mut b = &payload[..];
        while let Some(i) = b.windows(2).position(|w| w == b"\r\n") {
            let len = usize::from_str_radix(String::from_utf8_lossy(&b[..i]).trim(), 16).unwrap_or(0);
            if len == 0 {
                break;
            }
            out.extend_from_slice(&b[i + 2..i + 2 + len]);
            b = &b[i + 2 + len + 2..];
        }
        payload = out;
    }
    (code, serde_json::from_slice(&payload).unwrap_or(Value::Null))
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let t0 = Instant::now();
    while !f() {
        assert!(t0.elapsed() < Duration::from_secs(20), "Zeitueberschreitung: {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn window_and_mcp_client_see_each_others_changes() {
    let s = space("shared");
    s.set_config(json!({"mcp": {"level": "delete"}}));
    let g = gui(&s);
    let open = json!({ "path": s.root }).to_string();
    assert_eq!(http(&g, "POST", "/api/open-cached", &open).0, 200);
    let mut c = s.client();

    // Was der Client auf die Liste setzt, erscheint im Fenster.
    let dmp = s.path(&["Daten", "absturz.dmp"]);
    c.ok("add_to_cleanup_list", json!({"paths": [dmp]}));
    wait_until("Liste im Fenster", || http(&g, "GET", "/api/cart", "").1["count"] == 1);

    // Was das Fenster aendert, sieht der Client beim naechsten Aufruf.
    let found = http(&g, "GET", "/api/files?q=wichtig.xlsx", "").1;
    let id = found["rows"][0]["id"].clone();
    assert_eq!(http(&g, "POST", "/api/cart/add", &json!({ "ids": [id] }).to_string()).0, 200);
    assert_eq!(c.ok("get_cleanup_list", json!({}))["count"], 2);
    c.ok("remove_from_cleanup_list", json!({"paths": [s.path(&["Daten", "wichtig.xlsx"])]}));

    // Entfernt der Client etwas, verschwindet es auch im Fenster.
    let id = c.ok("preview_cleanup", json!({}))["plan_id"].as_str().unwrap().to_string();
    assert_eq!(c.ok("recycle_files", json!({"plan_id": id}))["succeeded"], 1);
    wait_until("Datei weg im Fenster", || http(&g, "GET", "/api/files?q=absturz", "").1["total"] == 0);
    assert_eq!(http(&g, "GET", "/api/cart", "").1["count"], 0);
}

#[test]
fn window_sets_the_rights_with_explicit_confirmation() {
    let s = space("rights");
    let g = gui(&s);
    let info = http(&g, "GET", "/api/mcp", "").1;
    assert_eq!(info["config"]["level"], "read");
    assert!(info["args"].as_array().unwrap().iter().any(|a| a == "--mcp"));
    assert!(info["tools"].as_array().unwrap().len() > 15);

    let set = |body: Value| http(&g, "POST", "/api/mcp/config", &body.to_string());

    // Mehr Rechte nur mit dem Bestaetigungswort.
    assert_eq!(set(json!({"level": "write"})).0, 400);
    assert_eq!(set(json!({"level": "delete"})).0, 400);
    assert_eq!(set(json!({"level": "delete", "confirm": "SCHREIBEN"})).0, 400);
    assert_eq!(set(json!({"level": "delete", "confirm": "WRITE"})).0, 400);
    assert_eq!(set(json!({"level": "alles"})).0, 400);
    assert_eq!(http(&g, "GET", "/api/mcp", "").1["config"]["level"], "read", "Abgelehntes aendert nichts");

    // Die Bestaetigungswoerter gelten in beiden Sprachen.
    assert_eq!(set(json!({"level": "write", "confirm": "SCHREIBEN"})).0, 200);
    assert_eq!(set(json!({"level": "read"})).0, 200);
    assert_eq!(set(json!({"level": "write", "confirm": "WRITE"})).0, 200);
    assert_eq!(set(json!({"level": "delete", "confirm": "LÖSCHEN"})).0, 200);
    assert_eq!(set(json!({"level": "read"})).0, 200);
    assert_eq!(set(json!({"level": "delete", "confirm": "DELETE"})).0, 200);
    assert_eq!(set(json!({"allowPermanent": true})).0, 400);
    assert_eq!(set(json!({"allowPermanent": true, "confirm": "LÖSCHEN"})).0, 200);
    assert_eq!(http(&g, "GET", "/api/mcp", "").1["config"]["allow_permanent_delete"], true);

    // Weniger Rechte gehen ohne Rueckfrage, und die Freigabe fuers endgueltige Loeschen faellt mit.
    assert_eq!(set(json!({"level": "read"})).0, 200);
    let cfg = http(&g, "GET", "/api/mcp", "").1["config"].clone();
    assert_eq!(cfg["level"], "read");
    assert_eq!(cfg["allow_permanent_delete"], false);
    assert_eq!(set(json!({"level": "off"})).0, 200);
    assert_eq!(set(json!({"allowScan": true, "roots": [s.root]})).0, 200);

    // Die Oberflaeche zeigt, was der Client getan hat.
    set(json!({"level": "read"}));
    let mut c = s.client();
    c.ok("list_drives", json!({}));
    let log = http(&g, "GET", "/api/mcp/audit", "").1;
    assert_eq!(log["rows"][0]["tool"], "list_drives", "{log}");

    // Das Protokoll zeigt die Sprache der Oberflaeche, der Client bekommt Englisch.
    let denied = c.err("add_to_cleanup_list", json!({"paths": []}));
    assert!(denied.contains("AI access"), "{denied}");
    let log = http(&g, "GET", "/api/mcp/audit", "").1;
    assert!(log["rows"][0]["detail"].as_str().unwrap().contains("AI access"), "{log}");
    let log = http(&g, "GET", "/api/mcp/audit?lang=de", "").1;
    assert!(log["rows"][0]["detail"].as_str().unwrap().contains("KI-Zugriff"), "{log}");
    assert_eq!(http(&g, "GET", "/api/status", "").1["mcpLevel"], "read");
}
