//! Schutzregeln: Was darf Storage Map niemals anfassen, und wo wird gewarnt?
//! Das Backend prueft das bei jedem Loeschen erneut, die Oberflaeche zeigt es nur an.

use crate::i18n::{Lang, Text};
use crate::paths;
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq)]
pub enum Guard {
    Ok,
    Warn(Text),
    Block(Text),
}

impl Guard {
    /// Die Form, in der die Schnittstelle den Schutz meldet: `level` und, wenn es einen gibt, `reason`.
    pub fn json(&self, lang: Lang) -> Value {
        match self {
            Guard::Ok => json!({"level": "ok"}),
            Guard::Warn(t) => json!({"level": "warn", "reason": t.get(lang)}),
            Guard::Block(t) => json!({"level": "block", "reason": t.get(lang)}),
        }
    }
}

const BIN: Text = Text::new("Recycle bin: please clean it up with \"Empty recycle bin\".", "Papierkorb: Bitte über \"Papierkorb leeren\" aufräumen.");
const WIN_BOOT: Text = Text::new("Windows boot files.", "Startdateien von Windows.");
const WIN_BOOT_FILE: Text = Text::new("Windows boot file.", "Startdatei von Windows.");
const WIN_MANAGED: Text = Text::new("Managed by Windows.", "Wird von Windows verwaltet.");
const WIN_CLEANUP: Text = Text::new("Please remove this with Windows Disk Cleanup.", "Bitte mit der Windows-Datenträgerbereinigung entfernen.");
const USER_FOLDERS: Text = Text::new("Windows user folder.", "Benutzerordner von Windows.");
const PROFILE_FOLDER: Text = Text::new(
    "Standard folder of your profile. Individual contents can be deleted.",
    "Standardordner deines Profils. Einzelne Inhalte können gelöscht werden.",
);
const PROFILE_APP_DATA: Text = Text::new(
    "Program data of your profile. Individual contents can be deleted.",
    "Programmdaten deines Profils. Einzelne Inhalte können gelöscht werden.",
);
const APP_DATA_WARN: Text =
    Text::new("Program data: settings or saved games may be lost.", "Programmdaten: Einstellungen oder Spielstände können verloren gehen.");
const WHOLE_DRIVE: Text = Text::new("The whole drive cannot be deleted.", "Das ganze Laufwerk kann nicht gelöscht werden.");
const OS_PART: Text = Text::new(
    "Part of the operating system. Deleting it could damage the system.",
    "Teil des Betriebssystems. Löschen könnte das System beschädigen.",
);
const MOUNT_POINTS: Text = Text::new(
    "Drives are mounted here. Individual contents can be deleted.",
    "Hier werden Laufwerke eingehängt. Einzelne Inhalte können gelöscht werden.",
);

const ROOT_BLOCK: &[(&str, Text)] = &[
    ("system volume information", Text::new("System restore: managed by Windows.", "Systemwiederherstellung: Wird von Windows verwaltet.")),
    ("$recycle.bin", BIN),
    ("recovery", Text::new("Windows recovery.", "Windows-Wiederherstellung.")),
    ("boot", WIN_BOOT),
    ("efi", WIN_BOOT),
    ("config.msi", Text::new("Managed by Windows Installer.", "Wird vom Windows Installer verwaltet.")),
    ("pagefile.sys", Text::new("Page file: managed in the Windows settings.", "Auslagerungsdatei: Wird in den Windows-Einstellungen verwaltet.")),
    ("hiberfil.sys", Text::new("Hibernation: can be turned off with \"powercfg /h off\".", "Ruhezustand: Mit \"powercfg /h off\" abschaltbar.")),
    ("swapfile.sys", Text::new("Page file: managed by Windows.", "Auslagerungsdatei: Wird von Windows verwaltet.")),
    ("bootmgr", WIN_BOOT_FILE),
    ("bootnxt", WIN_BOOT_FILE),
    ("dumpstack.log", WIN_MANAGED),
    ("dumpstack.log.tmp", WIN_MANAGED),
    ("windows.old", WIN_CLEANUP),
    ("$windows.~bt", WIN_CLEANUP),
    ("$windows.~ws", WIN_CLEANUP),
    ("$getcurrent", WIN_CLEANUP),
    ("$sysreset", WIN_CLEANUP),
    ("$winreagent", WIN_CLEANUP),
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
    if rel.is_empty() {
        return Guard::Block(Text::new("The whole user profile. The system needs it.", "Ganzes Benutzerprofil. Wird vom System benötigt."));
    }
    let first = rel[0].to_lowercase();
    if matches!(first.as_str(), ".ssh" | ".gnupg" | ".pki") {
        return Guard::Block(Text::new("Keys and credentials.", "Schlüssel und Zugangsdaten."));
    }
    if rel.len() == 1 && UNIX_PROFILE_FOLDERS.contains(&first.as_str()) {
        return Guard::Block(PROFILE_FOLDER);
    }
    if first == ".local" {
        if rel.len() >= 3 && rel[1] == "share" && rel[2].eq_ignore_ascii_case("trash") {
            return Guard::Block(BIN);
        }
        if rel.len() == 2 {
            return Guard::Block(PROFILE_APP_DATA);
        }
        return Guard::Warn(APP_DATA_WARN);
    }
    if first == ".config" {
        return Guard::Warn(Text::new("Program settings. They will be lost.", "Einstellungen von Programmen. Sie gehen dabei verloren."));
    }
    Guard::Ok
}

/// Schutzregeln fuer Linux. Gross- und Kleinschreibung zaehlen hier.
pub fn check_unix(path: &str, home: &str) -> Guard {
    let p = path.trim_end_matches('/');
    if p.is_empty() {
        return Guard::Block(Text::new("The whole file system cannot be deleted.", "Das ganze Dateisystem kann nicht gelöscht werden."));
    }
    let parts: Vec<&str> = p.split('/').skip(1).collect();
    let first = parts[0];
    let name = parts[parts.len() - 1];

    if parts.contains(&"lost+found") {
        return Guard::Block(Text::new("Managed by the file system.", "Wird vom Dateisystem verwaltet."));
    }
    if name == ".Trash" || name.starts_with(".Trash-") || parts.iter().any(|c| *c == ".Trash" || c.starts_with(".Trash-")) {
        return Guard::Block(BIN);
    }
    if parts.len() == 1 && matches!(first, "swapfile" | "swap.img" | "swap") {
        return Guard::Block(Text::new("Swap file: managed by the system.", "Auslagerungsdatei: Wird vom System verwaltet."));
    }
    // Profil zuerst: Es kann auch ausserhalb von /home liegen, etwa unter /var/home.
    let home = home.trim_end_matches('/');
    if !home.is_empty() && home != "/" && (p == home || p.starts_with(&format!("{home}/"))) {
        let rel: Vec<&str> = p[home.len()..].split('/').filter(|c| !c.is_empty()).collect();
        return check_profile(&rel);
    }
    match first {
        "usr" if parts.len() >= 4 && parts[1] == "local" => Guard::Warn(Text::new(
            "Software installed by hand. Other programs may depend on it.",
            "Von Hand installierte Software. Andere Programme können davon abhängen.",
        )),
        f if UNIX_SYSTEM.contains(&f) => Guard::Block(OS_PART),
        "run" => {
            // Wechseldatentraeger haengen unter /run/media/<nutzer>/<laufwerk>.
            if parts.len() >= 5 && parts[1] == "media" {
                Guard::Ok
            } else {
                Guard::Block(OS_PART)
            }
        }
        "var" => {
            let inner = format!("{p}/");
            if VAR_CLEANABLE.iter().any(|c| inner.starts_with(c) && inner.len() > c.len()) {
                return Guard::Ok;
            }
            if parts.len() >= 3 && parts[1] == "log" {
                return Guard::Warn(Text::new(
                    "System logs. Running services may still write to them.",
                    "Protokolle des Systems. Laufende Dienste schreiben evtl. noch hinein.",
                ));
            }
            Guard::Block(Text::new("Needed by the system and by services.", "Wird vom System und von Diensten benötigt."))
        }
        "tmp" if parts.len() == 1 => Guard::Block(Text::new(
            "System folder. Individual contents can be deleted.",
            "Ordner des Systems. Einzelne Inhalte können gelöscht werden.",
        )),
        "opt" | "srv" => {
            if parts.len() == 1 {
                Guard::Block(Text::new(
                    "System folder. Please uninstall individual programs.",
                    "Ordner des Systems. Bitte einzelne Programme deinstallieren.",
                ))
            } else {
                Guard::Warn(Text::new(
                    "Installed software. Better removed with the package manager.",
                    "Installierte Software. Besser über die Paketverwaltung entfernen.",
                ))
            }
        }
        "root" => check_profile(&parts[1..]),
        "home" => {
            if parts.len() == 1 {
                Guard::Block(Text::new("Folder of all user profiles.", "Ordner aller Benutzerprofile."))
            } else {
                check_profile(&parts[2..])
            }
        }
        "mnt" if parts.len() <= 2 => Guard::Block(MOUNT_POINTS),
        "media" if parts.len() <= 3 => Guard::Block(MOUNT_POINTS),
        _ => Guard::Ok,
    }
}

pub fn check_windows(path: &str, sysroot: &str, sysdrive: &str) -> Guard {
    let p = path.trim_end_matches('\\').to_lowercase();

    // Laufwerkswurzel oder UNC-Freigabe selbst
    let (drive, rest) = match p.split_once('\\') {
        Some((d, r)) if !d.is_empty() => (d, r),
        None => return Guard::Block(WHOLE_DRIVE),
        _ => return Guard::Ok, // UNC-Pfad: keine Windows-Systemordner
    };
    if rest.is_empty() {
        return Guard::Block(WHOLE_DRIVE);
    }
    let parts: Vec<&str> = rest.split('\\').collect();
    let first = parts[0];

    if let Some((_, why)) = ROOT_BLOCK.iter().find(|(n, _)| *n == first) {
        return Guard::Block(why.clone());
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
        return Guard::Block(Text::new(
            "Part of Windows. Deleting it could damage the system.",
            "Teil von Windows. Löschen könnte das System beschädigen.",
        ));
    }

    let on_system = drive == sysdrive;
    if first == "program files" || first == "program files (x86)" {
        if parts.len() == 1 {
            return Guard::Block(Text::new(
                "Programs folder: please uninstall individual programs.",
                "Programmordner: Bitte einzelne Programme deinstallieren.",
            ));
        }
        return Guard::Warn(Text::new(
            "Installed software. Better uninstalled via \"Apps & features\".",
            "Installierte Software. Besser über \"Apps & Features\" deinstallieren.",
        ));
    }
    if on_system && first == "programdata" {
        if parts.len() == 1 || parts[1] == "microsoft" {
            return Guard::Block(Text::new("Needed by Windows and programs.", "Wird von Windows und Programmen benötigt."));
        }
        return Guard::Warn(Text::new(
            "Program data. Some programs may stop working.",
            "Programmdaten. Manche Programme funktionieren danach nicht mehr.",
        ));
    }
    if first == "users" && (on_system || parts.len() > 1) {
        if parts.len() == 1 {
            return Guard::Block(USER_FOLDERS);
        }
        if matches!(parts[1], "default" | "default user" | "all users" | "public") && parts.len() == 2 {
            return Guard::Block(USER_FOLDERS);
        }
        if parts.len() == 2 {
            return Guard::Block(Text::new("The whole user profile. Windows needs it.", "Ganzes Benutzerprofil. Wird von Windows benötigt."));
        }
        if parts.len() == 3 && PROFILE_FOLDERS.contains(&parts[2]) {
            return Guard::Block(PROFILE_FOLDER);
        }
        if parts.get(2) == Some(&"appdata") {
            if parts.len() == 4 {
                return Guard::Block(PROFILE_APP_DATA);
            }
            return Guard::Warn(APP_DATA_WARN);
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

    #[test]
    fn reasons_exist_in_both_languages() {
        let g = c(r"C:\Windows\System32\x.dll");
        assert_eq!(g.json(Lang::En)["level"], "block");
        assert!(g.json(Lang::En)["reason"].as_str().unwrap().starts_with("Part of Windows"));
        assert!(g.json(Lang::De)["reason"].as_str().unwrap().starts_with("Teil von Windows"));
        assert_eq!(Guard::Ok.json(Lang::De), json!({"level": "ok"}));
        assert_eq!(c(r"C:\Program Files\Foo").json(Lang::En)["level"], "warn");
    }
}
