//! Linux-Seite der Plattform-Schicht.

use super::linux_pure::{self, Mount};
use super::{clamp_secs, DriveInfo, RawEntry};
use crate::i18n::Text;
use std::collections::HashSet;
use std::ffi::{CString, OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

pub const PLATFORM: &str = "linux";
pub const CAN_RESTART_ELEVATED: bool = false;
/// Unter Linux gibt es keinen Dialog, den man abbrechen koennte.
pub const CANCELLED_RC: i32 = i32::MIN;

pub fn long_pathbuf(p: &Path) -> PathBuf {
    p.to_path_buf()
}

fn all_mounts() -> Vec<Mount> {
    std::fs::read_to_string("/proc/self/mountinfo").map(|t| linux_pure::parse_mountinfo(&t)).unwrap_or_default()
}

fn statvfs(path: &str) -> Option<(u64, u64)> {
    let c = CString::new(path.as_bytes()).ok()?;
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let unit = s.f_frsize as u64;
    Some((s.f_blocks as u64 * unit, s.f_bavail as u64 * unit))
}

/// Festplatte oder SSD? `/sys` kennt die Antwort am Geraet oder an dessen Elternteil.
fn is_rotational(m: &Mount) -> Option<bool> {
    let name = Path::new(&m.source).file_name()?.to_string_lossy().to_string();
    let bases = [format!("/sys/class/block/{name}"), format!("/sys/dev/block/{}:{}", m.major, m.minor)];
    for base in bases {
        for rel in ["queue/rotational", "../queue/rotational"] {
            if let Ok(t) = std::fs::read_to_string(format!("{base}/{rel}")) {
                return Some(t.trim() == "1");
            }
        }
    }
    None
}

fn is_removable(m: &Mount) -> bool {
    if m.point.starts_with("/media/") || m.point.starts_with("/run/media/") {
        return true;
    }
    let Some(name) = Path::new(&m.source).file_name().map(|n| n.to_string_lossy().to_string()) else { return false };
    ["removable", "../removable"]
        .iter()
        .any(|rel| std::fs::read_to_string(format!("/sys/class/block/{name}/{rel}")).map(|t| t.trim() == "1").unwrap_or(false))
}

fn label_of(source: &str) -> String {
    let Ok(rd) = std::fs::read_dir("/dev/disk/by-label") else { return String::new() };
    let Ok(target) = std::fs::canonicalize(source) else { return String::new() };
    for e in rd.flatten() {
        if std::fs::canonicalize(e.path()).map(|p| p == target).unwrap_or(false) {
            return e.file_name().to_string_lossy().replace("\\x20", " ");
        }
    }
    String::new()
}

fn info_of(m: &Mount) -> DriveInfo {
    let network = linux_pure::is_network_fs(&m.fs);
    let mut d = DriveInfo {
        root: m.point.clone(),
        label: label_of(&m.source),
        fs: m.fs.clone(),
        kind: if network {
            "network"
        } else if m.fs == "iso9660" || m.fs == "udf" {
            "cdrom"
        } else if is_removable(m) {
            "removable"
        } else {
            "fixed"
        }
        .to_string(),
        media: if network {
            "net".to_string()
        } else {
            match is_rotational(m) {
                Some(true) => "hdd".to_string(),
                Some(false) => "ssd".to_string(),
                None => "unknown".to_string(),
            }
        },
        ..Default::default()
    };
    if let Some((total, free)) = statvfs(&m.point) {
        if total > 0 {
            d.total = total;
            d.free = free;
            d.ready = true;
        }
    }
    d
}

pub fn volume_info(root: &str) -> DriveInfo {
    let all = all_mounts();
    match linux_pure::mount_of(&all, root) {
        Some(m) => info_of(m),
        None => DriveInfo { root: root.to_string(), kind: "unknown".into(), media: "unknown".into(), ..Default::default() },
    }
}

/// Listet die Laufwerke. Ein haengendes Netzlaufwerk blockiert nicht: Wer nach `timeout` nicht
/// geantwortet hat, erscheint als "nicht bereit".
pub fn list_drives(timeout: Duration) -> Vec<DriveInfo> {
    let mounts = linux_pure::usable_mounts(all_mounts());
    let (tx, rx) = mpsc::channel();
    for m in mounts.clone() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(info_of(&m));
        });
    }
    drop(tx);
    let mut got: Vec<DriveInfo> = vec![];
    let deadline = std::time::Instant::now() + timeout;
    while got.len() < mounts.len() {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok(d) => got.push(d),
            Err(_) => break,
        }
    }
    for m in &mounts {
        if !got.iter().any(|d| d.root == m.point) {
            got.push(DriveInfo { root: m.point.clone(), fs: m.fs.clone(), kind: "unknown".into(), media: "unknown".into(), ..Default::default() });
        }
    }
    got.sort_by(|a, b| a.root.cmp(&b.root));
    got
}

/// Einhaengepunkt, zu dem der Pfad gehoert.
pub fn volume_root(path: &str) -> Option<String> {
    let all = all_mounts();
    linux_pure::mount_of(&all, path).map(|m| m.point.clone())
}

pub fn is_volume_root(path: &str) -> bool {
    path == "/" || all_mounts().iter().any(|m| m.point == path)
}

pub fn is_elevated() -> bool {
    unsafe { libc::geteuid() == 0 }
}

pub fn enable_backup_privilege() {}

/// Linux kennt keine Grenze wie die 260 Zeichen der Windows-Shell.
pub fn too_long_for_recycle(_path: &str) -> bool {
    false
}

pub fn attach_console() {}

pub fn restart_elevated(_args: &str) -> bool {
    false
}

fn quiet(cmd: &mut Command) -> &mut Command {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
}

/// Zeigt die Datei im Dateimanager. Die meisten Desktops bieten dafuer eine D-Bus-Schnittstelle,
/// sonst oeffnet sich wenigstens der Ordner.
pub fn reveal(path: &str) -> io::Result<()> {
    let uri = format!("file://{}", linux_pure::percent_encode_path(path.as_bytes()));
    let dbus = quiet(
        Command::new("dbus-send")
            .args(["--session", "--print-reply", "--dest=org.freedesktop.FileManager1", "/org/freedesktop/FileManager1"])
            .arg("org.freedesktop.FileManager1.ShowItems")
            .arg(format!("array:string:{uri}"))
            .arg("string:"),
    )
    .status();
    if matches!(dbus, Ok(s) if s.success()) {
        return Ok(());
    }
    let parent = Path::new(path).parent().unwrap_or(Path::new("/"));
    quiet(Command::new("xdg-open").arg(parent)).spawn().map(|_| ())
}

pub fn shell_open(path: &str) -> bool {
    quiet(Command::new("xdg-open").arg(path)).spawn().is_ok()
}

fn local_timestamp() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut t: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut t) };
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", t.tm_year + 1900, t.tm_mon + 1, t.tm_mday, t.tm_hour, t.tm_min, t.tm_sec)
}

fn home_trash() -> PathBuf {
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".local/share"));
    data.join("Trash")
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn device_of(p: &Path) -> Option<u64> {
    std::fs::symlink_metadata(p).ok().map(|m| m.dev())
}

fn uid() -> u32 {
    unsafe { libc::getuid() }
}

/// Der Papierkorb fuer eine Datei nach der freedesktop-Spezifikation:
/// auf dem Heim-Dateisystem der persoenliche, sonst einer in der Wurzel des jeweiligen Laufwerks.
/// Rueckgabe: Papierkorb-Ordner und die Wurzel, zu der `Path=` relativ ist (`None`: absolut).
fn trash_for(path: &Path) -> io::Result<(PathBuf, Option<PathBuf>)> {
    let home_tr = home_trash();
    let anchor = home_tr.ancestors().find(|a| a.exists()).unwrap_or(Path::new("/")).to_path_buf();
    if device_of(path) == device_of(&anchor) {
        return Ok((home_tr, None));
    }
    let all = all_mounts();
    let text = path.to_string_lossy();
    let m = linux_pure::mount_of(&all, &text).ok_or_else(|| io::Error::other("No mount point found"))?;
    let top = PathBuf::from(&m.point);
    // Gemeinsamer `.Trash` nur, wenn er die Spezifikation erfuellt: echter Ordner mit Sticky-Bit.
    let shared = top.join(".Trash");
    if let Ok(meta) = std::fs::symlink_metadata(&shared) {
        if meta.is_dir() && meta.mode() & 0o1000 != 0 {
            return Ok((shared.join(uid().to_string()), Some(top)));
        }
    }
    Ok((top.join(format!(".Trash-{}", uid())), Some(top)))
}

fn make_private_dirs(p: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(p)
}

/// Legt eine Datei in den Papierkorb. Zuerst wird der Name mit der Info-Datei reserviert,
/// dann verschoben. Klappt das Verschieben nicht, bleibt die Datei, wo sie war.
pub fn trash_one(path: &Path, trash: &Path, relative_to: Option<&Path>) -> io::Result<()> {
    let files = trash.join("files");
    let info = trash.join("info");
    make_private_dirs(&files)?;
    make_private_dirs(&info)?;
    let name = path.file_name().ok_or_else(|| io::Error::other("Pfad ohne Namen"))?.to_os_string();
    let shown = match relative_to {
        Some(top) => path.strip_prefix(top).unwrap_or(path),
        None => path,
    };
    let text = linux_pure::trash_info(shown.as_os_str().as_bytes(), &local_timestamp());
    for i in 0..10_000u32 {
        let mut candidate: OsString = name.clone();
        if i > 0 {
            candidate.push(format!(".{i}"));
        }
        let mut info_name = candidate.clone();
        info_name.push(".trashinfo");
        let info_path = info.join(&info_name);
        match std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&info_path) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(text.as_bytes())?;
                drop(f);
                return std::fs::rename(path, files.join(&candidate)).inspect_err(|_| {
                    let _ = std::fs::remove_file(&info_path);
                });
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::other("No free name in the trash"))
}

/// Verschiebt Pfade in den Papierkorb. Rueckgabe wie bei Windows: 0 oder der erste Fehlercode.
pub fn recycle(paths: &[PathBuf]) -> i32 {
    let mut rc = 0;
    for p in paths {
        let res = trash_for(p).and_then(|(trash, top)| trash_one(p, &trash, top.as_deref()));
        if let Err(e) = res {
            if rc == 0 {
                rc = e.raw_os_error().unwrap_or(-1);
            }
        }
    }
    rc
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let meta = std::fs::symlink_metadata(from)?;
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(from)?, to)?;
    } else if meta.is_dir() {
        std::fs::create_dir(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_tree(&e.path(), &to.join(e.file_name()))?;
        }
        std::fs::set_permissions(to, meta.permissions())?;
    } else if meta.is_file() {
        let mut src = std::fs::File::open(from)?;
        let mut dst = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
        io::copy(&mut src, &mut dst)?;
        dst.set_permissions(meta.permissions())?;
        dst.set_modified(meta.modified()?)?;
    }
    Ok(())
}

fn move_one(from: &Path, dest_dir: &Path) -> io::Result<()> {
    let name = from.file_name().ok_or_else(|| io::Error::other("Pfad ohne Namen"))?;
    let to = dest_dir.join(name);
    if std::fs::symlink_metadata(&to).is_ok() {
        return Err(io::Error::from_raw_os_error(libc::EEXIST));
    }
    match std::fs::rename(from, &to) {
        Ok(()) => Ok(()),
        Err(e) if e.raw_os_error() == Some(libc::EXDEV) => {
            // Anderes Laufwerk: erst vollstaendig kopieren, dann das Original loeschen.
            if let Err(e) = copy_tree(from, &to) {
                let _ = if to.is_dir() { std::fs::remove_dir_all(&to) } else { std::fs::remove_file(&to) };
                return Err(e);
            }
            if std::fs::symlink_metadata(from)?.is_dir() {
                std::fs::remove_dir_all(from)
            } else {
                std::fs::remove_file(from)
            }
        }
        Err(e) => Err(e),
    }
}

/// Verschiebt, ohne etwas zu ueberschreiben.
pub fn move_to(paths: &[PathBuf], dest: &Path, _interactive: bool) -> i32 {
    let mut rc = 0;
    for p in paths {
        if let Err(e) = move_one(p, dest) {
            if rc == 0 {
                rc = e.raw_os_error().unwrap_or(-1);
            }
        }
    }
    rc
}

fn trash_dirs_of(root: &str) -> Vec<PathBuf> {
    let mut out = vec![];
    let root_path = Path::new(root);
    let home_tr = home_trash();
    let anchor = home_tr.ancestors().find(|a| a.exists()).unwrap_or(Path::new("/")).to_path_buf();
    if device_of(root_path) == device_of(&anchor) {
        out.push(home_tr);
    }
    out.push(root_path.join(format!(".Trash-{}", uid())));
    out.push(root_path.join(".Trash").join(uid().to_string()));
    out
}

fn tree_alloc(p: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(p) else { return 0 };
    let mut sum = meta.blocks() * 512;
    if meta.is_dir() {
        if let Ok(rd) = std::fs::read_dir(p) {
            for e in rd.flatten() {
                sum += tree_alloc(&e.path());
            }
        }
    }
    sum
}

/// Belegter Platz im Papierkorb dieses Laufwerks. `None` bei Netzlaufwerken: Dort gilt er als nicht vorhanden.
pub fn recycle_bin_info(root: &str) -> Option<(u64, u64)> {
    let all = all_mounts();
    if linux_pure::mount_of(&all, root).map(|m| linux_pure::is_network_fs(&m.fs)).unwrap_or(false) {
        return None;
    }
    let (mut bytes, mut items) = (0u64, 0u64);
    for t in trash_dirs_of(root) {
        if let Ok(rd) = std::fs::read_dir(t.join("files")) {
            for e in rd.flatten() {
                bytes += tree_alloc(&e.path());
                items += 1;
            }
        }
    }
    Some((bytes, items))
}

/// Leert den Papierkorb eines Laufwerks. Die Sicherheitsabfrage hat die Oberflaeche schon gestellt.
pub fn empty_recycle_bin(root: &str) -> i32 {
    let mut rc = 0;
    for t in trash_dirs_of(root) {
        for sub in ["files", "info"] {
            let Ok(rd) = std::fs::read_dir(t.join(sub)) else { continue };
            for e in rd.flatten() {
                let is_dir = e.file_type().map(|x| x.is_dir()).unwrap_or(false);
                let r = if is_dir { std::fs::remove_dir_all(e.path()) } else { std::fs::remove_file(e.path()) };
                if let Err(err) = r {
                    rc = err.raw_os_error().unwrap_or(-1);
                }
            }
        }
        let _ = std::fs::remove_file(t.join("directorysizes"));
    }
    rc
}

/// Haengt innerhalb dieses Ordners ein anderes Dateisystem? Dann darf er nicht geloescht werden,
/// sonst wuerde das Loeschen auch dessen Inhalt erfassen.
pub fn has_mount_inside(path: &Path) -> bool {
    let text = path.to_string_lossy();
    let prefix = format!("{}/", text.trim_end_matches('/'));
    all_mounts().iter().any(|m| m.point == *text || m.point.starts_with(&prefix))
}

const NO_PERMISSION: Text = Text::new("No permission. Try starting with sudo.", "Keine Berechtigung. Evtl. mit sudo starten.");

pub fn friendly_error(e: &io::Error) -> Option<Text> {
    match e.raw_os_error() {
        Some(libc::EACCES) | Some(libc::EPERM) => Some(NO_PERMISSION),
        Some(libc::EBUSY) | Some(libc::ETXTBSY) => Some(Text::new("Currently in use by a program.", "Wird gerade von einem Programm benutzt.")),
        Some(libc::EEXIST) => {
            Some(Text::new("An entry with this name already exists at the target.", "Am Ziel gibt es schon einen Eintrag mit diesem Namen."))
        }
        Some(libc::EROFS) => Some(Text::new("The drive is read-only.", "Das Laufwerk ist schreibgeschützt.")),
        _ => None,
    }
}

pub fn describe_remaining(p: &Path, is_dir: bool) -> Text {
    let parent_ok =
        p.parent().and_then(|x| CString::new(x.as_os_str().as_bytes()).ok()).map(|c| unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 });
    if parent_ok == Some(false) {
        return NO_PERMISSION;
    }
    if is_dir {
        Text::new(
            "Could not be removed (completely). It may be in use or need more rights.",
            "Konnte nicht (vollständig) entfernt werden. Wird evtl. gerade benutzt oder braucht mehr Rechte.",
        )
    } else {
        Text::new("Could not be removed.", "Konnte nicht entfernt werden.")
    }
}

/// Oeffnet zum Lesen, ohne "Zuletzt geoeffnet" zu veraendern. O_NOATIME darf nur der Eigentuemer der Datei.
pub fn open_quiet(path: &Path) -> io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NOATIME).open(path).or_else(|_| std::fs::File::open(path))
}

/// Fuehrt das Dateisystem Buch darueber, wann Dateien geoeffnet wurden? `noatime` schaltet das ab.
pub fn last_access_tracking(root: Option<&str>) -> Option<bool> {
    let all = all_mounts();
    let m = linux_pure::mount_of(&all, root.unwrap_or("/"))?;
    Some(!m.opts.split(',').any(|o| o == "noatime"))
}

fn read_trim(p: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(p).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

/// Modell und Art des Datentraegers aus `/sys`. Den Zustand kennt nur SMART: Das fragt `smartctl`,
/// falls es installiert ist und die noetigen Rechte reichen.
pub fn disk_health(root: &str) -> Option<serde_json::Value> {
    let all = all_mounts();
    let m = linux_pure::mount_of(&all, root)?;
    let dev = Path::new(&m.source).file_name()?.to_string_lossy().to_string();
    let block = std::fs::canonicalize(format!("/sys/class/block/{dev}")).ok()?;
    let disk = if block.join("device").exists() { block } else { block.parent()?.to_path_buf() };
    let name = disk.file_name()?.to_string_lossy().to_string();
    let model = read_trim(disk.join("device/model"))?;
    let rotational = read_trim(disk.join("queue/rotational")).map(|t| t == "1").unwrap_or(false);
    let sectors: u64 = read_trim(disk.join("size")).and_then(|t| t.parse().ok()).unwrap_or(0);
    let bus = if name.starts_with("nvme") {
        "NVMe"
    } else if disk.to_string_lossy().contains("/usb") {
        "USB"
    } else {
        "SATA"
    };
    let mut v = serde_json::json!({
        "model": model,
        "media": if rotational { "HDD" } else { "SSD" },
        "bus": bus,
        "size": sectors * 512,
        "health": "Unknown",
    });
    if let Ok(out) = Command::new("smartctl").args(["-j", "-H", "-A", &format!("/dev/{name}")]).stderr(Stdio::null()).output() {
        if let Ok(s) = serde_json::from_slice::<serde_json::Value>(&out.stdout) {
            match s["smart_status"]["passed"].as_bool() {
                Some(true) => v["health"] = "Healthy".into(),
                Some(false) => v["health"] = "Warning".into(),
                None => {}
            }
            if let Some(t) = s["temperature"]["current"].as_u64() {
                v["temp"] = t.into();
            }
            if let Some(h) = s["power_on_time"]["hours"].as_u64() {
                v["hours"] = h.into();
            }
            if let Some(w) = s["nvme_smart_health_information_log"]["percentage_used"].as_u64() {
                v["wear"] = w.into();
            }
        }
    }
    Some(v)
}

/// Beendet einen Prozess. Ohne `force` bekommt er die normale Aufforderung, sich zu schliessen.
pub fn close_process(pid: u32, force: bool) {
    // 0 waere die eigene Prozessgruppe, 1 das Init-System.
    if pid > 1 {
        unsafe {
            libc::kill(pid as i32, if force { libc::SIGKILL } else { libc::SIGTERM });
        }
    }
}

/// Wartet, bis ein anderer Prozess beendet ist, hoechstens `timeout` lang. `true`, wenn er weg ist.
pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    let t0 = std::time::Instant::now();
    loop {
        if !Path::new(&format!("/proc/{pid}")).exists() {
            return true;
        }
        if t0.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Hauptprozesse eines Browsers, die mit diesem Profil laufen. Das Profil gehoert nur Storage Map,
/// der normale Browser des Nutzers ist also nie dabei.
pub fn browsers_using_profile(profile: &Path) -> Vec<u32> {
    let wanted = profile.to_string_lossy();
    let Ok(rd) = std::fs::read_dir("/proc") else { return vec![] };
    let mut out = vec![];
    for e in rd.flatten() {
        let Some(pid) = e.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
        let Ok(raw) = std::fs::read(e.path().join("cmdline")) else { continue };
        if super::is_browser_on_profile(&String::from_utf8_lossy(&raw), &wanted) {
            out.push(pid);
        }
    }
    out
}

fn in_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(name)).find(|p| p.is_file())
}

pub fn find_app_browser() -> Option<PathBuf> {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return None;
    }
    ["microsoft-edge", "microsoft-edge-stable", "google-chrome", "google-chrome-stable", "chromium", "chromium-browser", "brave-browser", "vivaldi"]
        .iter()
        .find_map(|n| in_path(n))
}

/// Oeffnet die Adresse im Standardbrowser. `false`, wenn es hier keinen Desktop gibt.
pub fn open_url(url: &str) -> bool {
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return false;
    }
    quiet(Command::new("xdg-open").arg(url)).spawn().is_ok()
}

pub fn random_bytes(out: &mut [u8]) {
    use std::io::Read;
    let ok = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(out));
    assert!(ok.is_ok(), "The operating system provides no random numbers");
}

pub fn default_data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".local/share"));
    base.join("storage-map")
}

pub fn default_config_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home().join(".config"));
    base.join("storage-map")
}

/// Ordner, den nur der aktuelle Nutzer lesen kann. Scans enthalten alle Dateinamen.
pub fn create_private_dir(p: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(p)?;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))
}

/// Schreibt eine Datei, die nur der aktuelle Nutzer lesen kann. Sie erscheint vollstaendig oder gar nicht.
pub fn write_private(path: &Path, data: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(data)?;
    drop(f);
    std::fs::rename(tmp, path)
}

/* ---------- Verzeichnisse lesen ---------- */

pub type DirPath = PathBuf;

pub fn root_dir_path(root: &str) -> DirPath {
    PathBuf::from(root)
}

pub fn child_dir_path(parent: &DirPath, name: &[u8]) -> DirPath {
    parent.join(OsStr::from_bytes(name))
}

pub fn display_dir_path(p: &DirPath) -> String {
    p.to_string_lossy().into_owned()
}

pub struct Lister {
    /// Einhaengepunkte. Der Scan bleibt auf dem Dateisystem, auf dem er begonnen hat.
    mounts: HashSet<OsString>,
}

impl Lister {
    pub fn new() -> Self {
        let mounts = all_mounts().into_iter().map(|m| OsString::from_vec(m.point.into_bytes())).collect();
        Lister { mounts }
    }

    /// Listet ein Verzeichnis. Fehler kommen als Fehlercode des Systems zurueck.
    pub fn list(&mut self, path: &DirPath, out: &mut Vec<RawEntry>, names: &mut Vec<u8>) -> Result<(), u32> {
        let rd = std::fs::read_dir(path).map_err(|e| e.raw_os_error().unwrap_or(5) as u32)?;
        for e in rd {
            let Ok(e) = e else { continue };
            // Fehlt die Datei schon wieder, ist sie zwischendurch verschwunden: einfach auslassen.
            let Ok(m) = e.metadata() else { continue };
            let name = e.file_name();
            let bytes = name.as_bytes();
            let name_off = names.len() as u32;
            names.extend_from_slice(bytes);
            let ft = m.file_type();
            let is_dir = ft.is_dir();
            let mount = is_dir && self.mounts.contains(path.join(&name).as_os_str());
            out.push(RawEntry {
                name_off,
                name_len: bytes.len().min(u16::MAX as usize) as u16,
                is_dir,
                hidden: bytes.first() == Some(&b'.'),
                system: false,
                compressed: false,
                link: ft.is_symlink(),
                mount,
                cloud: false,
                size: if ft.is_file() { m.len() } else { 0 },
                alloc: if ft.is_file() { m.blocks() * 512 } else { 0 },
                mtime: clamp_secs(m.mtime()),
                atime: clamp_secs(m.atime()),
                ctime: m
                    .created()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| clamp_secs(d.as_secs() as i64))
                    .unwrap_or(0),
                // Nur Dateien mit mehreren Namen brauchen die Hardlink-Pruefung.
                file_id: if m.nlink() > 1 { m.ino() } else { 0 },
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_process_by_its_profile_and_closes_it() {
        let dir = std::env::temp_dir().join(format!("smap-profile-{}", std::process::id()));
        let mut c = Command::new("sh").args(["-c", "sleep 20; true"]).arg(format!("--user-data-dir={}", dir.display())).spawn().unwrap();
        let t0 = std::time::Instant::now();
        while !browsers_using_profile(&dir).contains(&c.id()) {
            assert!(t0.elapsed() < Duration::from_secs(5), "Prozess wird nicht gefunden");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(browsers_using_profile(&dir.join("anders")).is_empty());
        close_process(c.id(), false);
        c.wait().unwrap();
        assert!(browsers_using_profile(&dir).is_empty());
        assert!(wait_for_exit(c.id(), Duration::from_secs(1)));
    }
}
