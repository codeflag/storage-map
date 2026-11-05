//! Windows-Seite der Plattform-Schicht: duenne Huelle um die APIs, die Storage Map braucht.

use super::{clamp_secs, DriveInfo, RawEntry};
use std::ffi::{c_void, OsStr, OsString};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr::{null, null_mut};
use std::sync::mpsc;
use std::time::Duration;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, FILETIME, HANDLE, INVALID_HANDLE_VALUE, LUID};
use windows_sys::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
use windows_sys::Win32::Security::{AdjustTokenPrivileges, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW, SetFileTime, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, WaitForSingleObject};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::UI::Shell::{
    IsUserAnAdmin, SHEmptyRecycleBinW, SHFileOperationW, SHQueryRecycleBinW, ShellExecuteW, SHFILEOPSTRUCTW, SHQUERYRBINFO,
};

pub const PLATFORM: &str = "windows";
pub const CAN_RESTART_ELEVATED: bool = true;
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Die Shell meldet damit, dass der Nutzer ihren Dialog abgebrochen hat.
pub const CANCELLED_RC: i32 = 0x4C7;
const SW_SHOWNORMAL: i32 = 1;

pub fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

pub fn wide_os(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

/// Wandelt einen normalen Pfad in die lange Form um, damit tiefe Ordner
/// (ueber 260 Zeichen) gelesen werden koennen. Ohne abschliessendes NUL.
pub fn long_path(p: &OsStr) -> Vec<u16> {
    let raw: Vec<u16> = p.encode_wide().collect();
    let as_str = String::from_utf16_lossy(&raw);
    if as_str.starts_with(r"\\?\") {
        return raw;
    }
    if as_str.starts_with(r"\\") {
        let mut out: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
        out.extend_from_slice(&raw[2..]);
        return out;
    }
    let mut out: Vec<u16> = r"\\?\".encode_utf16().collect();
    out.extend_from_slice(&raw);
    out
}

pub fn long_pathbuf(p: &Path) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_wide(&long_path(p.as_os_str())))
}

fn drive_kind(t: u32) -> &'static str {
    match t {
        2 => "removable",
        3 => "fixed",
        4 => "network",
        5 => "cdrom",
        6 => "ram",
        _ => "unknown",
    }
}

/// Fragt Volume-Daten ab. Kann bei getrennten Netzlaufwerken lange haengen,
/// deshalb laeuft das immer in einem eigenen Thread (siehe `list_drives`).
pub fn volume_info(root: &str) -> DriveInfo {
    let w = wide(root);
    let kind = unsafe { GetDriveTypeW(w.as_ptr()) };
    let mut d = DriveInfo { root: root.to_string(), kind: drive_kind(kind).to_string(), ..Default::default() };
    let mut label = [0u16; 261];
    let mut fs = [0u16; 64];
    let mut serial = 0u32;
    let mut comp = 0u32;
    let mut flags = 0u32;
    let ok = unsafe {
        GetVolumeInformationW(
            w.as_ptr(),
            label.as_mut_ptr(),
            label.len() as u32,
            &mut serial,
            &mut comp,
            &mut flags,
            fs.as_mut_ptr(),
            fs.len() as u32,
        )
    };
    if ok != 0 {
        d.label = utf16_z(&label);
        d.fs = utf16_z(&fs);
        d.serial = serial;
    }
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    let ok2 = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) };
    if ok2 != 0 && total > 0 {
        d.total = total;
        d.free = free;
        d.ready = true;
    }
    d.media = match kind {
        4 => "net".into(),
        _ => match seek_penalty(root) {
            Some(true) => "hdd".into(),
            Some(false) => "ssd".into(),
            None => "unknown".into(),
        },
    };
    d
}

fn utf16_z(b: &[u16]) -> String {
    let n = b.iter().position(|c| *c == 0).unwrap_or(b.len());
    String::from_utf16_lossy(&b[..n])
}

/// Listet alle Laufwerke. Haengende Laufwerke blockieren nicht: Wer nach
/// `timeout` nicht geantwortet hat, erscheint als "nicht bereit".
pub fn list_drives(timeout: Duration) -> Vec<DriveInfo> {
    let mask = unsafe { GetLogicalDrives() };
    let (tx, rx) = mpsc::channel();
    let mut roots = vec![];
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        roots.push(root.clone());
        let tx = tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(volume_info(&root));
        });
    }
    drop(tx);
    let mut got: Vec<DriveInfo> = vec![];
    let deadline = std::time::Instant::now() + timeout;
    while got.len() < roots.len() {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok(d) => got.push(d),
            Err(_) => break,
        }
    }
    for r in roots {
        if !got.iter().any(|d| d.root == r) {
            let w = wide(&r);
            let kind = unsafe { GetDriveTypeW(w.as_ptr()) };
            got.push(DriveInfo { root: r, kind: drive_kind(kind).to_string(), media: "unknown".into(), ..Default::default() });
        }
    }
    got.sort_by(|a, b| a.root.cmp(&b.root));
    got
}

#[repr(C)]
struct StoragePropertyQuery {
    property_id: u32,
    query_type: u32,
    additional: [u8; 1],
}

#[repr(C)]
#[derive(Default)]
struct SeekPenaltyDescriptor {
    version: u32,
    size: u32,
    incurs_seek_penalty: u8,
}

/// `Some(true)` = HDD, `Some(false)` = SSD. Funktioniert auch ohne Adminrechte.
pub fn seek_penalty(root: &str) -> Option<bool> {
    const IOCTL_STORAGE_QUERY_PROPERTY: u32 = 0x002D_1400;
    const STORAGE_DEVICE_SEEK_PENALTY_PROPERTY: u32 = 7;
    let letter = root.chars().next()?;
    if !letter.is_ascii_alphabetic() || root.as_bytes().get(1) != Some(&b':') {
        return None;
    }
    let dev = wide(&format!(r"\\.\{letter}:"));
    let h = unsafe { CreateFileW(dev.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, null(), OPEN_EXISTING, 0, null_mut()) };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    let q = StoragePropertyQuery { property_id: STORAGE_DEVICE_SEEK_PENALTY_PROPERTY, query_type: 0, additional: [0] };
    let mut out = SeekPenaltyDescriptor::default();
    let mut ret = 0u32;
    let ok = unsafe {
        DeviceIoControl(
            h,
            IOCTL_STORAGE_QUERY_PROPERTY,
            &q as *const _ as *const c_void,
            std::mem::size_of::<StoragePropertyQuery>() as u32,
            &mut out as *mut _ as *mut c_void,
            std::mem::size_of::<SeekPenaltyDescriptor>() as u32,
            &mut ret,
            null_mut(),
        )
    };
    unsafe { CloseHandle(h) };
    if ok != 0 && ret as usize >= std::mem::size_of::<SeekPenaltyDescriptor>() - 3 {
        Some(out.incurs_seek_penalty != 0)
    } else {
        None
    }
}

pub fn is_elevated() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

/// Mit Adminrechten darf der Scan dank Backup-Privileg auch gesperrte Ordner lesen.
pub fn enable_backup_privilege() {
    unsafe {
        let mut token: HANDLE = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES, &mut token) == 0 {
            return;
        }
        let name = wide("SeBackupPrivilege");
        let mut luid = LUID { LowPart: 0, HighPart: 0 };
        if LookupPrivilegeValueW(null(), name.as_ptr(), &mut luid) != 0 {
            let tp = TOKEN_PRIVILEGES {
                PrivilegeCount: 1,
                Privileges: [windows_sys::Win32::Security::LUID_AND_ATTRIBUTES { Luid: luid, Attributes: SE_PRIVILEGE_ENABLED }],
            };
            AdjustTokenPrivileges(token, 0, &tp, 0, null_mut(), null_mut());
        }
        CloseHandle(token);
    }
}

/// Startet die Exe mit Adminrechten neu. `true`, wenn der Nutzer die UAC-Abfrage bestaetigt hat.
pub fn restart_elevated(args: &str) -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe_w = wide_os(exe.as_os_str());
    let verb = wide("runas");
    let params = wide(args);
    let r = unsafe { ShellExecuteW(null_mut(), verb.as_ptr(), exe_w.as_ptr(), params.as_ptr(), null(), SW_SHOWNORMAL) };
    r as isize > 32
}

pub fn reveal(path: &str) -> std::io::Result<()> {
    Command::new("explorer.exe").raw_arg(format!("/select,\"{path}\"")).spawn().map(|_| ())
}

pub fn shell_open(path: &str) -> bool {
    let p = wide(path);
    let verb = wide("open");
    let r = unsafe { ShellExecuteW(null_mut(), verb.as_ptr(), p.as_ptr(), null(), null(), SW_SHOWNORMAL) };
    r as isize > 32
}

const FO_MOVE: u32 = 1;
const FO_DELETE: u32 = 3;
const FOF_SILENT: u16 = 0x0004;
const FOF_NOCONFIRMATION: u16 = 0x0010;
const FOF_ALLOWUNDO: u16 = 0x0040;
const FOF_NOCONFIRMMKDIR: u16 = 0x0200;
const FOF_NOERRORUI: u16 = 0x0400;
const FOF_WANTNUKEWARNING: u16 = 0x4000;

fn multi_z(paths: &[PathBuf]) -> Vec<u16> {
    let mut out = vec![];
    for p in paths {
        out.extend(p.as_os_str().encode_wide());
        out.push(0);
    }
    out.push(0);
    out
}

/// Maximale Pfadlaenge, mit der die Shell-Funktionen zuverlaessig arbeiten.
pub fn too_long_for_recycle(path: &str) -> bool {
    path.encode_utf16().count() > 259
}

/// Verschiebt Pfade in den Papierkorb. FOF_WANTNUKEWARNING sorgt dafuer, dass Windows
/// nachfragt, falls etwas nicht in den Papierkorb passt, statt es stillschweigend zu loeschen.
pub fn recycle(paths: &[PathBuf]) -> i32 {
    let from = multi_z(paths);
    let mut op = SHFILEOPSTRUCTW {
        hwnd: null_mut(),
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        pTo: null(),
        fFlags: FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING,
        fAnyOperationsAborted: 0,
        hNameMappings: null_mut(),
        lpszProgressTitle: null(),
    };
    unsafe { SHFileOperationW(&mut op) }
}

/// Verschiebt mit dem Windows-eigenen Dialog (Fortschritt, Konfliktabfragen).
/// Ohne Dialog (`interactive == false`) meldet Windows Konflikte nur als Fehler.
pub fn move_to(paths: &[PathBuf], dest: &Path, interactive: bool) -> i32 {
    let from = multi_z(paths);
    let to = multi_z(&[dest.to_path_buf()]);
    let mut op = SHFILEOPSTRUCTW {
        hwnd: null_mut(),
        wFunc: FO_MOVE,
        pFrom: from.as_ptr(),
        pTo: to.as_ptr(),
        fFlags: if interactive { FOF_NOCONFIRMMKDIR } else { FOF_NOCONFIRMMKDIR | FOF_SILENT | FOF_NOERRORUI },
        fAnyOperationsAborted: 0,
        hNameMappings: null_mut(),
        lpszProgressTitle: null(),
    };
    unsafe { SHFileOperationW(&mut op) }
}

/// Belegter Platz im Papierkorb dieses Laufwerks; `None`, wenn es keinen gibt.
pub fn recycle_bin_info(root: &str) -> Option<(u64, u64)> {
    let w = wide(root);
    let mut q = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, i64Size: 0, i64NumItems: 0 };
    let hr = unsafe { SHQueryRecycleBinW(w.as_ptr(), &mut q) };
    if hr == 0 {
        Some((q.i64Size as u64, q.i64NumItems as u64))
    } else {
        None
    }
}

/// Leert den Papierkorb eines Laufwerks. Windows zeigt dabei seine eigene Sicherheitsabfrage.
pub fn empty_recycle_bin(root: &str) -> i32 {
    let w = wide(root);
    unsafe { SHEmptyRecycleBinW(null_mut(), w.as_ptr(), 0) }
}

/// Liest, ob NTFS "Zuletzt geoeffnet" pflegt. Wert 0 oder 2 (ohne hohes Bit: 0) = aktiv.
pub fn last_access_tracking(_root: Option<&str>) -> Option<bool> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD};
    let sub = wide(r"SYSTEM\CurrentControlSet\Control\FileSystem");
    let val = wide("NtfsDisableLastAccessUpdate");
    let mut data = 0u32;
    let mut len = 4u32;
    let r = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, sub.as_ptr(), val.as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut data as *mut _ as *mut c_void, &mut len)
    };
    if r != 0 {
        return None;
    }
    // Unterste Bits: 0 = Nutzer aktiviert, 1 = Nutzer deaktiviert, 2 = System aktiviert, 3 = System deaktiviert.
    Some(data & 1 == 0)
}

/// Modell und Zustand des physischen Datentraegers. Dauert ein paar Sekunden, also im Hintergrund aufrufen.
pub fn disk_health(root: &str) -> Option<serde_json::Value> {
    let letter = root.chars().next()?;
    if !letter.is_ascii_alphabetic() {
        return None;
    }
    let script = format!(
        "$ErrorActionPreference='SilentlyContinue';\
         $p=Get-Partition -DriveLetter {letter};\
         $d=Get-PhysicalDisk | Where-Object {{ $_.DeviceId -eq [string]$p.DiskNumber }};\
         $r=$d | Get-StorageReliabilityCounter;\
         [pscustomobject]@{{model=$d.FriendlyName;media=[string]$d.MediaType;bus=[string]$d.BusType;\
         health=[string]$d.HealthStatus;size=$d.Size;temp=$r.Temperature;wear=$r.Wear;\
         hours=$r.PowerOnHours;readErrors=$r.ReadErrorsTotal;writeErrors=$r.WriteErrorsTotal}} | ConvertTo-Json -Compress"
    );
    let out =
        Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", &script]).creation_flags(CREATE_NO_WINDOW).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str(text.trim()).ok()
}

/// `C:\Daten\x` wird `C:\`. Fuer UNC-Pfade gibt es kein Laufwerk.
pub fn volume_root(path: &str) -> Option<String> {
    let b = path.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        Some(format!("{}:\\", (b[0] as char).to_ascii_uppercase()))
    } else {
        None
    }
}

pub fn is_volume_root(path: &str) -> bool {
    path.len() <= 3 && path.as_bytes().get(1) == Some(&b':')
}

/// Windows hat keine Einhaengepunkte, die beim Loeschen mitgerissen wuerden.
pub fn has_mount_inside(_path: &Path) -> bool {
    false
}

pub fn friendly_error(e: &std::io::Error) -> Option<String> {
    match e.raw_os_error() {
        Some(32) => Some("Wird gerade von einem Programm benutzt.".to_string()),
        Some(5) => Some("Keine Berechtigung. Evtl. als Administrator starten.".to_string()),
        _ => None,
    }
}

pub fn describe_remaining(p: &Path, is_dir: bool) -> String {
    if is_dir {
        return "Konnte nicht (vollständig) entfernt werden. Wird evtl. gerade benutzt oder braucht Adminrechte.".into();
    }
    match std::fs::OpenOptions::new().write(true).open(long_pathbuf(p)) {
        Err(e) => friendly_error(&e).unwrap_or_else(|| "Konnte nicht entfernt werden.".into()),
        _ => "Konnte nicht entfernt werden.".into(),
    }
}

/// Oeffnet zum Lesen, ohne "Zuletzt geoeffnet" zu veraendern. Sonst wuerde der
/// Duplikat-Scan die Auswertung "ungenutzte Dateien" verfaelschen.
pub fn open_quiet(path: &Path) -> std::io::Result<std::fs::File> {
    const GENERIC_READ: u32 = 0x8000_0000;
    const FILE_WRITE_ATTRIBUTES: u32 = 0x100;
    let f = std::fs::OpenOptions::new()
        .access_mode(GENERIC_READ | FILE_WRITE_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path);
    match f {
        Ok(f) => {
            let keep = FILETIME { dwLowDateTime: u32::MAX, dwHighDateTime: u32::MAX };
            unsafe {
                SetFileTime(f.as_raw_handle(), null(), &keep, null());
            }
            Ok(f)
        }
        Err(_) => std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).open(path),
    }
}

/// Beendet einen Prozess. Ohne `force` bekommt er die normale Aufforderung, sich zu schliessen.
pub fn close_process(pid: u32, force: bool) {
    let id = pid.to_string();
    let mut cmd = Command::new("taskkill");
    cmd.args(["/PID", &id]);
    if force {
        cmd.args(["/T", "/F"]);
    }
    let _ = cmd.creation_flags(CREATE_NO_WINDOW).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
}

/// Wartet, bis ein anderer Prozess beendet ist, hoechstens `timeout` lang. `true`, wenn er weg ist.
pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    const SYNCHRONIZE: u32 = 0x0010_0000;
    let h = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if h.is_null() {
        return true;
    }
    unsafe {
        let result = WaitForSingleObject(h, timeout.as_millis().min(u32::MAX as u128) as u32);
        CloseHandle(h);
        result == 0
    }
}

/// Kommandozeile eines anderen Prozesses. Das geht ohne besondere Rechte, solange er demselben Nutzer gehoert.
fn command_line_of(pid: u32) -> Option<String> {
    #[repr(C)]
    struct UnicodeString {
        length: u16,
        max: u16,
        buffer: *const u16,
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const PROCESS_COMMAND_LINE_INFORMATION: u32 = 60;
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if h.is_null() {
        return None;
    }
    let mut buf = vec![0u64; 8 * 1024];
    let mut ret = 0u32;
    let st =
        unsafe { NtQueryInformationProcess(h, PROCESS_COMMAND_LINE_INFORMATION, buf.as_mut_ptr() as *mut c_void, (buf.len() * 8) as u32, &mut ret) };
    unsafe { CloseHandle(h) };
    if st < 0 {
        return None;
    }
    let s = unsafe { &*(buf.as_ptr() as *const UnicodeString) };
    if s.buffer.is_null() {
        return None;
    }
    let chars = unsafe { std::slice::from_raw_parts(s.buffer, s.length as usize / 2) };
    Some(String::from_utf16_lossy(chars))
}

/// Hauptprozesse von Edge oder Chrome, die mit diesem Profil laufen. Das Profil gehoert nur Storage Map,
/// der normale Browser des Nutzers ist also nie dabei.
pub fn browsers_using_profile(profile: &Path) -> Vec<u32> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    let wanted = profile.to_string_lossy().to_lowercase();
    let mut out = vec![];
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap == INVALID_HANDLE_VALUE {
        return out;
    }
    let mut e: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
    let mut more = unsafe { Process32FirstW(snap, &mut e) } != 0;
    while more {
        let name = utf16_z(&e.szExeFile).to_lowercase();
        if name == "msedge.exe" || name == "chrome.exe" {
            if let Some(cmd) = command_line_of(e.th32ProcessID) {
                if super::is_browser_on_profile(&cmd.to_lowercase(), &wanted) {
                    out.push(e.th32ProcessID);
                }
            }
        }
        more = unsafe { Process32NextW(snap, &mut e) } != 0;
    }
    unsafe { CloseHandle(snap) };
    out
}

/// Edge oder Chrome: Beide koennen eine Seite als eigenes App-Fenster zeigen.
pub fn find_app_browser() -> Option<PathBuf> {
    let mut bases = vec![];
    for var in ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"] {
        if let Some(b) = std::env::var_os(var) {
            bases.push(PathBuf::from(b));
        }
    }
    for rel in [r"Microsoft\Edge\Application\msedge.exe", r"Google\Chrome\Application\chrome.exe"] {
        for b in &bases {
            let p = b.join(rel);
            if p.exists() {
                return Some(p);
            }
        }
    }
    None
}

/// Oeffnet die Adresse im Standardbrowser.
pub fn open_url(url: &str) -> bool {
    Command::new("cmd").args(["/C", "start", "", url]).creation_flags(CREATE_NO_WINDOW).spawn().is_ok()
}

pub fn random_bytes(out: &mut [u8]) {
    let st = unsafe { BCryptGenRandom(null_mut(), out.as_mut_ptr(), out.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    assert!(st == 0, "Das Betriebssystem liefert keine Zufallszahlen");
}

pub fn default_data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("StorageMap")
}

pub fn default_config_dir() -> PathBuf {
    default_data_dir()
}

/// `%LOCALAPPDATA%` gehoert ohnehin nur dem aktuellen Nutzer.
pub fn create_private_dir(p: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(p)
}

/// Schreibt eine Datei, die vollstaendig oder gar nicht erscheint.
pub fn write_private(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(tmp, path)
}

/// Ohne Konsole gaebe es keine Ausgabe. Beim Start aus einem Terminal haengen wir uns an dessen Fenster.
pub fn attach_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/* ---------- Verzeichnisse lesen ---------- */

#[repr(C)]
struct IoStatusBlock {
    status: usize,
    info: usize,
}

#[link(name = "ntdll", kind = "raw-dylib")]
extern "system" {
    fn NtQueryDirectoryFile(
        file: HANDLE,
        event: HANDLE,
        apc: *const c_void,
        apc_ctx: *const c_void,
        iosb: *mut IoStatusBlock,
        info: *mut c_void,
        len: u32,
        class: u32,
        single: u8,
        name: *const c_void,
        restart: u8,
    ) -> i32;
    fn NtQueryInformationProcess(process: HANDLE, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

const STATUS_NO_MORE_FILES: i32 = 0x8000_0006u32 as i32;
const STATUS_NO_SUCH_FILE: i32 = 0xC000_000Fu32 as i32;
const CLASS_ID_FULL: u32 = 38; // FileIdFullDirectoryInformation
const CLASS_FULL: u32 = 2; // FileFullDirectoryInformation (Fallback fuer manche Netzlaufwerke)

const ATTR_DIRECTORY: u32 = 0x10;
const ATTR_HIDDEN: u32 = 0x2;
const ATTR_SYSTEM: u32 = 0x4;
const ATTR_REPARSE: u32 = 0x400;
const ATTR_COMPRESSED: u32 = 0x800;
const ATTR_OFFLINE: u32 = 0x1000;
const ATTR_RECALL_OPEN: u32 = 0x40000;
const ATTR_RECALL_ACCESS: u32 = 0x400000;

/// Bit in Reparse-Tags, das "verweist auf etwas anderes" bedeutet (Symlink, Junction).
const REPARSE_NAME_SURROGATE: u32 = 0x2000_0000;

#[inline]
fn unix_secs(ft: i64) -> u32 {
    if ft <= 0 {
        return 0;
    }
    clamp_secs(ft / 10_000_000 - 11_644_473_600)
}

#[inline]
fn rd_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
#[inline]
fn rd_i64(b: &[u8], o: usize) -> i64 {
    i64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

pub type DirPath = Vec<u16>;

pub fn root_dir_path(root: &str) -> DirPath {
    let mut p = long_path(OsStr::new(root));
    // "\\?\C:" ohne Backslash waere das aktuelle Verzeichnis auf C:, nicht die Wurzel.
    if p.last() != Some(&(b'\\' as u16)) && root.len() == 2 {
        p.push(b'\\' as u16);
    }
    p
}

pub fn child_dir_path(parent: &DirPath, name: &[u8]) -> DirPath {
    let mut p = parent.clone();
    if p.last() != Some(&(b'\\' as u16)) {
        p.push(b'\\' as u16);
    }
    let os = unsafe { OsStr::from_encoded_bytes_unchecked(name) };
    p.extend(os.encode_wide());
    p
}

pub fn display_dir_path(p: &DirPath) -> String {
    let s = String::from_utf16_lossy(p);
    if let Some(unc) = s.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{unc}");
    }
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_string()
}

pub struct Lister {
    buf: Vec<u64>,
}

impl Lister {
    pub fn new() -> Self {
        Lister { buf: vec![0u64; 64 * 1024 / 8] }
    }

    /// Listet ein Verzeichnis. Fehler kommen als Win32-Code zurueck.
    pub fn list(&mut self, path: &DirPath, out: &mut Vec<RawEntry>, names: &mut Vec<u8>) -> Result<(), u32> {
        const FILE_LIST_DIRECTORY: u32 = 1;
        let mut path_z = path.clone();
        path_z.push(0);
        let h = unsafe {
            CreateFileW(
                path_z.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            return Err(unsafe { GetLastError() });
        }
        let buf = &mut self.buf;
        let byte_len = (buf.len() * 8) as u32;
        let mut class = CLASS_ID_FULL;
        let mut restart = 1u8;
        let result = loop {
            let mut iosb = IoStatusBlock { status: 0, info: 0 };
            let st = unsafe {
                NtQueryDirectoryFile(h, null_mut(), null(), null(), &mut iosb, buf.as_mut_ptr() as *mut c_void, byte_len, class, 0, null(), restart)
            };
            restart = 0;
            if st == STATUS_NO_MORE_FILES || st == STATUS_NO_SUCH_FILE {
                break Ok(());
            }
            if st < 0 {
                if class == CLASS_ID_FULL {
                    class = CLASS_FULL;
                    restart = 1;
                    continue;
                }
                break Err(st as u32);
            }
            let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, iosb.info.min(byte_len as usize)) };
            let name_at = if class == CLASS_ID_FULL { 80 } else { 68 };
            let mut pos = 0usize;
            loop {
                if pos + name_at > bytes.len() {
                    break;
                }
                let e = &bytes[pos..];
                let next = rd_u32(e, 0) as usize;
                let name_bytes = rd_u32(e, 60) as usize;
                if name_at + name_bytes > e.len() {
                    break;
                }
                let wname: &[u16] = unsafe { std::slice::from_raw_parts(e.as_ptr().add(name_at) as *const u16, name_bytes / 2) };
                let is_dot = wname == [46] || wname == [46, 46];
                if !is_dot {
                    let name_off = names.len() as u32;
                    if wname.iter().all(|c| *c < 128) {
                        names.extend(wname.iter().map(|c| *c as u8));
                    } else {
                        names.extend_from_slice(OsString::from_wide(wname).as_encoded_bytes());
                    }
                    let attrs = rd_u32(e, 56);
                    let tag = if attrs & ATTR_REPARSE != 0 { rd_u32(e, 64) } else { 0 };
                    let is_dir = attrs & ATTR_DIRECTORY != 0;
                    let size = rd_i64(e, 40).max(0) as u64;
                    let alloc = rd_i64(e, 48).max(0) as u64;
                    out.push(RawEntry {
                        name_off,
                        name_len: (names.len() as u32 - name_off).min(u16::MAX as u32) as u16,
                        is_dir,
                        hidden: attrs & ATTR_HIDDEN != 0,
                        system: attrs & ATTR_SYSTEM != 0,
                        compressed: attrs & ATTR_COMPRESSED != 0,
                        link: attrs & ATTR_REPARSE != 0 && tag & REPARSE_NAME_SURROGATE != 0,
                        mount: false,
                        cloud: !is_dir && attrs & (ATTR_OFFLINE | ATTR_RECALL_OPEN | ATTR_RECALL_ACCESS) != 0 && alloc < size,
                        size,
                        alloc,
                        ctime: unix_secs(rd_i64(e, 8)),
                        atime: unix_secs(rd_i64(e, 16)),
                        mtime: unix_secs(rd_i64(e, 24)),
                        file_id: if class == CLASS_ID_FULL { rd_i64(e, 72) as u64 } else { 0 },
                    });
                }
                if next == 0 {
                    break;
                }
                pos += next;
            }
        };
        unsafe { CloseHandle(h) };
        result
    }
}
