// Im Release ohne Konsolenfenster: Storage Map soll sich wie ein normales Programm verhalten.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod analysis;
mod assets;
mod category;
mod cleanup;
mod config;
mod demo;
mod dupes;
mod guard;
mod history;
mod index;
mod junk;
mod mcp;
mod paths;
mod platform;
mod scan;
mod server;

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

const DEFAULT_PORT: u16 = 47811;
const HELP: &str = "Storage Map shows what fills a drive and helps you clean it up.

Usage:
  storage-map                Open the user interface in its own window
  storage-map --scan PATH    Scan without a user interface and add the result to the history
  storage-map --mcp          Run as an MCP server over stdio (for AI clients)

Options:
  --no-open          Do not open a window, print the address instead
  --port N           Use a fixed port (default: 47811)
  --token T          Use a fixed access token instead of a random one
  --ui-dir PATH      Load the user interface from disk (for development)
  --data-dir PATH    Keep scans, history and settings in this folder
  -V, --version      Print the version
  -h, --help         Print this help

Testing without real drives:
  --demo             Invented drives, nothing is read or deleted
  --demo-scale N     Demo with N times as many files
  --sandbox PATH     Real scan and cleanup, limited to this test folder
  --make-fixture DIR Create a test folder with known contents
";

#[derive(Default)]
struct Args {
    no_open: bool,
    port: Option<u16>,
    token: Option<String>,
    ui_dir: Option<PathBuf>,
    scan: Option<String>,
    help: bool,
    version: bool,
    mcp: bool,
    demo: bool,
    demo_scale: u64,
    sandbox: Option<String>,
    data_dir: Option<PathBuf>,
    make_fixture: Option<PathBuf>,
    unknown: Option<String>,
}

fn parse_args() -> Args {
    let mut a = Args { demo_scale: 1, ..Default::default() };
    // args() wuerde bei Pfaden abstuerzen, die unter Linux kein gueltiges UTF-8 sind.
    let mut it = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned());
    while let Some(x) = it.next() {
        match x.as_str() {
            "--no-open" => a.no_open = true,
            "--port" => a.port = it.next().and_then(|v| v.parse().ok()),
            "--token" => a.token = it.next(),
            "--ui-dir" => a.ui_dir = it.next().map(PathBuf::from),
            "--scan" => a.scan = it.next(),
            "--mcp" => a.mcp = true,
            "--demo" => a.demo = true,
            "--demo-scale" => a.demo_scale = it.next().and_then(|v| v.parse().ok()).unwrap_or(1),
            "--sandbox" => a.sandbox = it.next(),
            "--data-dir" => a.data_dir = it.next().map(PathBuf::from),
            "--make-fixture" => a.make_fixture = it.next().map(PathBuf::from),
            "--help" | "-h" | "/?" => a.help = true,
            "--version" | "-V" => a.version = true,
            other => a.unknown = Some(other.to_string()),
        }
    }
    a
}

/// Oeffnet die Oberflaeche als eigenes App-Fenster und beendet das Programm, wenn es geschlossen wird.
fn open_window(url: &str, port: u16) {
    if let Some(browser) = platform::find_app_browser() {
        let profile = history::data_dir().join(if port == DEFAULT_PORT { "window".to_string() } else { format!("window-{port}") });
        let started = Instant::now();
        let child = Command::new(browser)
            .arg(format!("--app={url}"))
            .arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-extensions",
                "--disable-sync",
                "--disable-features=Translate",
                "--window-size=1560,980",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut c) = child {
            let fallback = url.to_string();
            std::thread::spawn(move || {
                let ok = c.wait().map(|s| s.success()).unwrap_or(false);
                if started.elapsed() > Duration::from_secs(5) {
                    std::process::exit(0);
                }
                // Beendet sich der Browser sofort, hat ein schon laufendes Fenster uebernommen.
                // Ist er dabei gescheitert, bleibt der Standardbrowser.
                if !ok {
                    platform::open_url(&fallback);
                }
            });
            return;
        }
    }
    if !platform::open_url(url) {
        println!("Storage Map is running. Open this address in a browser: {url}");
    }
}

fn instance_file() -> PathBuf {
    history::data_dir().join("instance.json")
}

fn http_body(reply: &str) -> Option<&str> {
    reply.split_once("\r\n\r\n").map(|x| x.1)
}

/// Laeuft schon eine Instanz? Dann nur ihr Fenster oeffnen statt einer zweiten.
fn existing_instance() -> Option<(u16, String)> {
    use std::io::{Read, Write};
    let t = std::fs::read_to_string(instance_file()).ok()?;
    let v: serde_json::Value = serde_json::from_str(&t).ok()?;
    let port = v["port"].as_u64()? as u16;
    let token = v["token"].as_str()?.to_string();
    let addr = format!("127.0.0.1:{port}").parse().ok()?;
    let mut s = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(300)).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(800))).ok()?;
    write!(
        s,
        "POST /api/session/new HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-SM-Token: {token}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
    )
    .ok()?;
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    let v: serde_json::Value = serde_json::from_str(http_body(&buf)?).ok()?;
    Some((port, v["code"].as_str()?.to_string()))
}

fn headless_scan(root: &str, mode: &server::RunMode) -> i32 {
    let root = match paths::normalize(root) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Error: {e}");
            return 2;
        }
    };
    if !mode.allows(&root) {
        eprintln!("Error: in a test mode only the test folder is allowed.");
        return 2;
    }
    let drive = if mode.is_normal() { platform::volume_root(&root).map(|r| platform::volume_info(&r)) } else { None };
    let media = drive.as_ref().map(|d| d.media.clone()).unwrap_or_else(|| "net".into());
    let expected = drive.as_ref().map(|d| d.total.saturating_sub(d.free)).unwrap_or(0);
    let p = scan::ScanProgress::new(&root, expected, scan::threads_for(&media), &media);
    match server::run_scan(&root, &p, drive, mode) {
        Ok(ix) => {
            let n = &ix.nodes[0];
            println!(
                "{}: {:.1} GB in {} files and {} folders, {:.1} s, {} threads ({}), {} unreadable",
                root,
                n.alloc as f64 / 1e9,
                n.files,
                n.dirs,
                ix.meta.duration_ms as f64 / 1000.0,
                p.threads,
                media,
                ix.meta.denied.len()
            );
            0
        }
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

fn main() {
    let args = parse_args();
    // Der MCP-Betrieb spricht ueber Pipes. Eine angehaengte Konsole wuerde die Ausgabe umlenken.
    if !args.mcp && (args.help || args.version || args.unknown.is_some() || args.scan.is_some() || args.no_open || args.make_fixture.is_some()) {
        platform::attach_console();
    }
    if let Some(u) = &args.unknown {
        eprintln!("Unknown option: {u}\n\n{HELP}");
        std::process::exit(2);
    }
    if args.help {
        println!("{HELP}");
        return;
    }
    if args.version {
        println!("storage-map {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if let Some(dir) = args.make_fixture.as_deref() {
        match demo::make_fixture(dir) {
            Ok(()) => println!("Test folder created: {}", dir.display()),
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let mode = if args.demo {
        server::RunMode::Demo { scale: args.demo_scale }
    } else if let Some(sb) = &args.sandbox {
        match paths::normalize(sb) {
            Ok(r) => server::RunMode::Sandbox(r),
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(2);
            }
        }
    } else {
        server::RunMode::Normal
    };
    // Test-Betriebe nie in den echten Verlauf schreiben.
    match (&args.data_dir, &mode) {
        (Some(d), _) => history::set_data_dir(d.clone()),
        (None, server::RunMode::Demo { .. }) => history::set_data_dir(std::env::temp_dir().join("StorageMap-Demo")),
        (None, server::RunMode::Sandbox(_)) => history::set_data_dir(std::env::temp_dir().join("StorageMap-Sandbox")),
        _ => {}
    }
    let _ = platform::create_private_dir(&history::data_dir());
    if mode.is_normal() {
        platform::enable_backup_privilege();
    }
    if args.mcp {
        if let server::RunMode::Demo { scale } = mode {
            if history::known_scans().is_empty() {
                demo::seed(scale);
            }
        }
        std::process::exit(mcp::run(mode));
    }
    if let server::RunMode::Demo { scale } = mode {
        demo::seed(scale);
    }
    if let Some(root) = args.scan.as_deref() {
        let code = headless_scan(root, &mode);
        std::process::exit(code);
    }

    if args.port.is_none() && !args.no_open && mode.is_normal() {
        if let Some((port, code)) = existing_instance() {
            open_window(&format!("http://127.0.0.1:{port}/?c={code}"), port);
            std::thread::sleep(Duration::from_secs(1));
            return;
        }
    }

    let wanted = args.port.unwrap_or(DEFAULT_PORT);
    let server =
        tiny_http::Server::http(format!("127.0.0.1:{wanted}")).or_else(|_| tiny_http::Server::http("127.0.0.1:0")).expect("No free port found");
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(wanted);
    let token = args.token.clone().unwrap_or_else(platform::random_hex);

    let info = serde_json::json!({"port": port, "token": token}).to_string();
    let _ = platform::write_private(&instance_file(), info.as_bytes());

    let no_open = args.no_open;
    let app = server::App::new(token.clone(), port, args.ui_dir.clone(), mode);
    if no_open {
        println!("Storage Map is running: http://127.0.0.1:{port}/?t={token}");
    } else {
        // Das Token gehoert nicht in die Kommandozeile des Browsers, die sehen unter Linux auch andere Nutzer.
        // Das Fenster bekommt deshalb nur einen Einmal-Code und tauscht ihn gegen das Token.
        open_window(&format!("http://127.0.0.1:{port}/?c={}", app.new_session_code()), port);
    }

    // Im Browser-Modus gibt es kein Fenster, das wir beobachten koennten: Nach langer Stille beenden.
    if !no_open {
        let a = app.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_secs(60));
            let silent = analysis::now_secs().saturating_sub(a.last_ping.load(Relaxed));
            if silent > 600 && !a.is_busy() && a.started.elapsed() > Duration::from_secs(600) {
                std::process::exit(0);
            }
        });
    }

    server::serve(app, server);
}
