//! Schutzregeln: Was darf Storage Map niemals anfassen, und wo wird gewarnt?
//! Das Backend prueft das bei jedem Loeschen erneut, die Oberflaeche zeigt es nur an.

use crate::paths;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "level", content = "reason", rename_all = "lowercase")]
pub enum Guard {
    Ok,
    Warn(String),
    Block(String),
}

const ROOT_BLOCK: &[(&str, &str)] = &[
    ("system volume information", "Systemwiederherstellung: Wird von Windows verwaltet."),
    ("$recycle.bin", "Papierkorb: Bitte über \"Papierkorb leeren\" aufräumen."),
    ("recovery", "Windows-Wiederherstellung."),
    ("boot", "Startdateien von Windows."),
    ("efi", "Startdateien von Windows."),
    ("config.msi", "Wird vom Windows Installer verwaltet."),
    ("pagefile.sys", "Auslagerungsdatei: Wird in den Windows-Einstellungen verwaltet."),
    ("hiberfil.sys", "Ruhezustand: Mit \"powercfg /h off\" abschaltbar."),
    ("swapfile.sys", "Auslagerungsdatei: Wird von Windows verwaltet."),
    ("bootmgr", "Startdatei von Windows."),
    ("bootnxt", "Startdatei von Windows."),
    ("dumpstack.log", "Wird von Windows verwaltet."),
    ("dumpstack.log.tmp", "Wird von Windows verwaltet."),
    ("windows.old", "Bitte mit der Windows-Datenträgerbereinigung entfernen."),
    ("$windows.~bt", "Bitte mit der Windows-Datenträgerbereinigung entfernen."),
    ("$windows.~ws", "Bitte mit der Windows-Datenträgerbereinigung entfernen."),
    ("$getcurrent", "Bitte mit der Windows-Datenträgerbereinigung entfernen."),
    ("$sysreset", "Bitte mit der Windows-Datenträgerbereinigung entfernen."),
    ("$winreagent", "Bitte mit der Windows-Datenträgerbereinigung entfernen."),
];

/// Unterordner von Windows, deren Inhalt gefahrlos weg darf (der Ordner selbst bleibt).
const WINDOWS_CLEANABLE: &[&str] = &[
    "temp",
    "softwaredistribution\\download",
    "logs",
    "minidump",
    "livekernelreports",
    "serviceprofiles\\networkservice\\appdata\\local\\microsoft\\windows\\deliveryoptimization\\cache",
];

const PROFILE_FOLDERS: &[&str] = &[
    "appdata",
    "desktop",
    "documents",
    "downloads",
    "pictures",
    "videos",
    "music",
    "onedrive",
    "favorites",
    "contacts",
    "links",
    "saved games",
    "searches",
    "3d objects",
];

fn system_root() -> String {
    std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()).trim_end_matches('\\').to_lowercase()
}

fn system_drive() -> String {
    std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).to_lowercase()
}

pub fn check(path: &str) -> Guard {
    if paths::is_unix_style(path) {
        let home = std::env::var("HOME").unwrap_or_default();
        check_unix(path, &home)
    } else {
        check_windows(path, &system_root(), &system_drive())
    }
}

/// Ordner des Betriebssystems: Hier wird nichts angefasst.
const UNIX_SYSTEM: &[&str] = &["bin", "boot", "dev", "etc", "lib", "lib32", "lib64", "libx32", "proc", "sbin", "snap", "sys", "usr"];

/// Inhalte unter `/var`, die gefahrlos weg duerfen. Der Ordner selbst bleibt.
const VAR_CLEANABLE: &[&str] = &[
    "/var/cache/apt/archives/",
    "/var/cache/dnf/",
    "/var/cache/pacman/pkg/",
    "/var/cache/zypp/",
    "/var/tmp/",
    "/var/crash/",
    "/var/lib/systemd/coredump/",
    "/var/lib/snapd/cache/",
];

/// Standardordner eines Linux-Profils, auf Englisch und Deutsch.
const UNIX_PROFILE_FOLDERS: &[&str] = &[
    "desktop",
    "documents",
    "downloads",
    "pictures",
    "music",
    "videos",
    "public",
    "templates",
    "schreibtisch",
    "dokumente",
    "bilder",
    "musik",
    "öffentlich",
    "vorlagen",
    ".config",
    ".local",
    ".cache",
    ".var",
    "snap",
];

fn check_profile(rel: &[&str]) -> Guard {
    let block = |m: &str| Guard::Block(m.to_string());
    if rel.is_empty() {
        return block("Ganzes Benutzerprofil. Wird vom System benötigt.");
    }
    let first = rel[0].to_lowercase();
    if matches!(first.as_str(), ".ssh" | ".gnupg" | ".pki") {
        return block("Schlüssel und Zugangsdaten.");
    }
    if rel.len() == 1 && UNIX_PROFILE_FOLDERS.contains(&first.as_str()) {
        return block("Standardordner deines Profils. Einzelne Inhalte können gelöscht werden.");
    }
    if first == ".local" {
        if rel.len() >= 3 && rel[1] == "share" && rel[2].eq_ignore_ascii_case("trash") {
            return block("Papierkorb: Bitte über \"Papierkorb leeren\" aufräumen.");
        }
        if rel.len() == 2 {
            return block("Programmdaten deines Profils. Einzelne Inhalte können gelöscht werden.");
        }
        return Guard::Warn("Programmdaten: Einstellungen oder Spielstände können verloren gehen.".into());
    }
    if first == ".config" {
        return Guard::Warn("Einstellungen von Programmen. Sie gehen dabei verloren.".into());
    }
    Guard::Ok
}

/// Schutzregeln fuer Linux. Gross- und Kleinschreibung zaehlen hier.
pub fn check_unix(path: &str, home: &str) -> Guard {
    let block = |m: &str| Guard::Block(m.to_string());
    let p = path.trim_end_matches('/');
    if p.is_empty() {
        return block("Das ganze Dateisystem kann nicht gelöscht werden.");
    }
    let parts: Vec<&str> = p.split('/').skip(1).collect();
    let first = parts[0];
    let name = parts[parts.len() - 1];

    if parts.contains(&"lost+found") {
        return block("Wird vom Dateisystem verwaltet.");
    }
    if name == ".Trash" || name.starts_with(".Trash-") || parts.iter().any(|c| *c == ".Trash" || c.starts_with(".Trash-")) {
        return block("Papierkorb: Bitte über \"Papierkorb leeren\" aufräumen.");
    }
    if parts.len() == 1 && matches!(first, "swapfile" | "swap.img" | "swap") {
        return block("Auslagerungsdatei: Wird vom System verwaltet.");
    }
    // Profil zuerst: Es kann auch ausserhalb von /home liegen, etwa unter /var/home.
    let home = home.trim_end_matches('/');
    if !home.is_empty() && home != "/" && (p == home || p.starts_with(&format!("{home}/"))) {
        let rel: Vec<&str> = p[home.len()..].split('/').filter(|c| !c.is_empty()).collect();
        return check_profile(&rel);
    }
    match first {
        "usr" if parts.len() >= 4 && parts[1] == "local" => {
            Guard::Warn("Von Hand installierte Software. Andere Programme können davon abhängen.".into())
        }
        f if UNIX_SYSTEM.contains(&f) => block("Teil des Betriebssystems. Löschen könnte das System beschädigen."),
        "run" => {
            // Wechseldatentraeger haengen unter /run/media/<nutzer>/<laufwerk>.
            if parts.len() >= 5 && parts[1] == "media" {
                Guard::Ok
            } else {
                block("Teil des Betriebssystems. Löschen könnte das System beschädigen.")
            }
        }
        "var" => {
            let inner = format!("{p}/");
            if VAR_CLEANABLE.iter().any(|c| inner.starts_with(c) && inner.len() > c.len()) {
                return Guard::Ok;
            }
            if parts.len() >= 3 && parts[1] == "log" {
                return Guard::Warn("Protokolle des Systems. Laufende Dienste schreiben evtl. noch hinein.".into());
            }
            block("Wird vom System und von Diensten benötigt.")
        }
        "tmp" if parts.len() == 1 => block("Ordner des Systems. Einzelne Inhalte können gelöscht werden."),
        "opt" | "srv" => {
            if parts.len() == 1 {
                block("Ordner des Systems. Bitte einzelne Programme deinstallieren.")
            } else {
                Guard::Warn("Installierte Software. Besser über die Paketverwaltung entfernen.".into())
            }
        }
        "root" => check_profile(&parts[1..]),
        "home" => {
            if parts.len() == 1 {
                block("Ordner aller Benutzerprofile.")
            } else {
                check_profile(&parts[2..])
            }
        }
        "mnt" if parts.len() <= 2 => block("Hier werden Laufwerke eingehängt. Einzelne Inhalte können gelöscht werden."),
        "media" if parts.len() <= 3 => block("Hier werden Laufwerke eingehängt. Einzelne Inhalte können gelöscht werden."),
        _ => Guard::Ok,
    }
}

pub fn check_windows(path: &str, sysroot: &str, sysdrive: &str) -> Guard {
    let p = path.trim_end_matches('\\').to_lowercase();
    let block = |m: &str| Guard::Block(m.to_string());

    // Laufwerkswurzel oder UNC-Freigabe selbst
    let (drive, rest) = match p.split_once('\\') {
        Some((d, r)) if !d.is_empty() => (d, r),
        None => return block("Das ganze Laufwerk kann nicht gelöscht werden."),
        _ => return Guard::Ok, // UNC-Pfad: keine Windows-Systemordner
    };
    if rest.is_empty() {
        return block("Das ganze Laufwerk kann nicht gelöscht werden.");
    }
    let parts: Vec<&str> = rest.split('\\').collect();
    let first = parts[0];

    if let Some((_, why)) = ROOT_BLOCK.iter().find(|(n, _)| *n == first) {
        return block(why);
    }

    // Windows-Ordner
    if p == sysroot || p.starts_with(&format!("{sysroot}\\")) {
        let inner = p.strip_prefix(sysroot).unwrap_or("").trim_start_matches('\\');
        if inner == "memory.dmp" {
            return Guard::Ok;
        }
        for c in WINDOWS_CLEANABLE {
            if inner.starts_with(&format!("{c}\\")) {
                return Guard::Ok;
            }
        }
        return block("Teil von Windows. Löschen könnte das System beschädigen.");
    }

    let on_system = drive == sysdrive;
    if first == "program files" || first == "program files (x86)" {
        if parts.len() == 1 {
            return block("Programmordner: Bitte einzelne Programme deinstallieren.");
        }
        return Guard::Warn("Installierte Software. Besser über \"Apps & Features\" deinstallieren.".into());
    }
    if on_system && first == "programdata" {
        if parts.len() == 1 || parts[1] == "microsoft" {
            return block("Wird von Windows und Programmen benötigt.");
        }
        return Guard::Warn("Programmdaten. Manche Programme funktionieren danach nicht mehr.".into());
    }
    if first == "users" && (on_system || parts.len() > 1) {
        if parts.len() == 1 {
            return block("Benutzerordner von Windows.");
        }
        if matches!(parts[1], "default" | "default user" | "all users" | "public") && parts.len() == 2 {
            return block("Benutzerordner von Windows.");
        }
        if parts.len() == 2 {
            return block("Ganzes Benutzerprofil. Wird von Windows benötigt.");
        }
        if parts.len() == 3 && PROFILE_FOLDERS.contains(&parts[2]) {
            return block("Standardordner deines Profils. Einzelne Inhalte können gelöscht werden.");
        }
        if parts.get(2) == Some(&"appdata") {
            if parts.len() == 4 {
                return block("Programmdaten deines Profils. Einzelne Inhalte können gelöscht werden.");
            }
            return Guard::Warn("Programmdaten: Einstellungen oder Spielstände können verloren gehen.".into());
        }
    }
    Guard::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(p: &str) -> Guard {
        check_windows(p, r"c:\windows", "c:")
    }
    fn blocked(p: &str) -> bool {
        matches!(c(p), Guard::Block(_))
    }
    fn warned(p: &str) -> bool {
        matches!(c(p), Guard::Warn(_))
    }

    #[test]
    fn system_paths_are_blocked() {
        for p in [
            r"C:\",
            r"E:\",
            r"C:\Windows",
            r"C:\Windows\System32\kernel32.dll",
            r"C:\Windows\WinSxS\foo",
            r"C:\Windows\Installer\abc.msi",
            r"C:\Windows\Temp",
            r"C:\pagefile.sys",
            r"C:\hiberfil.sys",
            r"C:\System Volume Information\x",
            r"C:\$Recycle.Bin\S-1-5",
            r"C:\Windows.old\Users",
            r"C:\Program Files",
            r"C:\Program Files (x86)",
            r"C:\ProgramData",
            r"C:\ProgramData\Microsoft\Windows",
            r"C:\Users",
            r"C:\Users\Demo",
            r"C:\Users\Demo\Documents",
            r"C:\Users\Demo\Downloads",
            r"C:\Users\Demo\AppData",
            r"C:\Users\Demo\AppData\Local",
            r"C:\Users\Public",
        ] {
            assert!(blocked(p), "muss gesperrt sein: {p}");
        }
    }

    #[test]
    fn cleanable_contents_are_allowed() {
        for p in [
            r"C:\Windows\Temp\foo.tmp",
            r"C:\Windows\SoftwareDistribution\Download\abc",
            r"C:\Windows\Logs\CBS\x.log",
            r"C:\Windows\MEMORY.DMP",
            r"C:\Users\Demo\Downloads\film.mp4",
            r"C:\Users\Demo\Documents\alt.zip",
            r"D:\Backup\alt",
            r"E:\Windows\meinzeug.txt",
            r"\\nas\share\film.mkv",
        ] {
            assert_eq!(c(p), Guard::Ok, "muss erlaubt sein: {p}");
        }
    }

    #[test]
    fn risky_locations_warn() {
        for p in [r"C:\Program Files\Foo", r"D:\Program Files\Foo\bar.exe", r"C:\ProgramData\Foo", r"C:\Users\Demo\AppData\Local\Foo"] {
            assert!(warned(p), "muss warnen: {p}");
        }
    }

    #[test]
    fn case_and_trailing_slash_do_not_bypass() {
        assert!(blocked(r"c:\WINDOWS\"));
        assert!(blocked(r"C:\WiNdOwS\system32\"));
        assert!(blocked(r"C:\USERS\DEMO\"));
    }

    fn u(p: &str) -> Guard {
        check_unix(p, "/home/demo")
    }

    #[test]
    fn linux_system_paths_are_blocked() {
        for p in [
            "/",
            "/usr",
            "/usr/lib/libc.so.6",
            "/etc/passwd",
            "/boot/vmlinuz",
            "/proc/1",
            "/sys/block",
            "/run/lock",
            "/var",
            "/var/lib/dpkg/status",
            "/var/cache/apt/archives",
            "/var/log",
            "/home",
            "/home/demo",
            "/home/andere",
            "/home/demo/Dokumente",
            "/home/demo/Downloads",
            "/home/demo/.cache",
            "/home/demo/.config",
            "/home/demo/.local",
            "/home/demo/.local/share",
            "/home/demo/.ssh/id_ed25519",
            "/home/demo/.gnupg/pubring.kbx",
            "/home/demo/.local/share/Trash/files/x",
            "/mnt/daten/.Trash-1000/files/x",
            "/mnt/daten",
            "/media/demo/USB",
            "/swapfile",
            "/mnt/daten/lost+found",
            "/opt",
            "/tmp",
        ] {
            assert!(matches!(u(p), Guard::Block(_)), "muss gesperrt sein: {p}");
        }
    }

    #[test]
    fn linux_cleanable_contents_are_allowed() {
        for p in [
            "/home/demo/Downloads/film.mp4",
            "/home/demo/.cache/thumbnails/x.png",
            "/home/demo/Code/app/node_modules",
            "/var/cache/apt/archives/a.deb",
            "/var/tmp/alt",
            "/var/lib/systemd/coredump/core.x",
            "/tmp/alt.bin",
            "/mnt/daten/film.mkv",
            "/media/demo/USB/film.mkv",
            "/run/media/demo/USB/film.mkv",
            "/daten/film.mkv",
        ] {
            assert_eq!(u(p), Guard::Ok, "muss erlaubt sein: {p}");
        }
    }

    #[test]
    fn linux_risky_locations_warn() {
        for p in ["/opt/app", "/usr/local/share/app", "/var/log/syslog.1.gz", "/home/demo/.config/app", "/home/demo/.local/share/app"] {
            assert!(matches!(u(p), Guard::Warn(_)), "muss warnen: {p}");
        }
    }

    #[test]
    fn linux_profile_outside_home_is_protected() {
        assert!(matches!(check_unix("/var/home/demo", "/var/home/demo"), Guard::Block(_)));
        assert_eq!(check_unix("/var/home/demo/Downloads/x", "/var/home/demo"), Guard::Ok);
    }

    #[test]
    fn linux_is_case_sensitive_and_ignores_trailing_slash() {
        assert!(matches!(u("/usr/"), Guard::Block(_)));
        assert_eq!(u("/USR/alt"), Guard::Ok, "/USR ist unter Linux ein anderer Ordner");
    }

    #[test]
    fn path_style_selects_the_rules() {
        assert!(matches!(check("/usr/lib/x"), Guard::Block(_)));
        assert!(matches!(check(r"C:\pagefile.sys"), Guard::Block(_)));
    }
}
