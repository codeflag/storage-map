//! Alles, was vom Betriebssystem abhaengt. Der Rest des Programms kennt nur diese Schnittstelle.

use serde::Serialize;

#[cfg_attr(not(unix), allow(dead_code))]
pub mod linux_pure;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::*;
#[cfg(windows)]
pub use windows::*;

#[derive(Clone, Debug, Serialize, Default)]
pub struct DriveInfo {
    pub root: String,
    pub label: String,
    pub fs: String,
    pub kind: String,
    pub media: String,
    pub total: u64,
    pub free: u64,
    pub ready: bool,
    pub serial: u32,
}

/// Ein Verzeichniseintrag, so wie der Scan ihn braucht. Der Name liegt im gemeinsamen Namenspuffer.
pub struct RawEntry {
    pub name_off: u32,
    pub name_len: u16,
    pub is_dir: bool,
    pub hidden: bool,
    pub system: bool,
    pub compressed: bool,
    /// Verweis auf etwas anderes (Symlink, Junction): Ziel wird nicht mitgezaehlt.
    pub link: bool,
    /// Hier ist ein anderes Dateisystem eingehaengt: wird nicht betreten und nie geloescht.
    pub mount: bool,
    /// Platzhalter, dessen Inhalt nur online liegt.
    pub cloud: bool,
    pub size: u64,
    pub alloc: u64,
    pub mtime: u32,
    pub atime: u32,
    pub ctime: u32,
    pub file_id: u64,
}

/// Ist das der Hauptprozess eines Browsers auf diesem Profil? Seine Hilfsprozesse tragen `--type=`.
/// Nach dem Pfad muss die Angabe enden, sonst passte `window` auch auf `window-47892`.
pub fn is_browser_on_profile(cmdline: &str, profile: &str) -> bool {
    if cmdline.contains("--type=") {
        return false;
    }
    let wanted = format!("--user-data-dir={profile}");
    let mut rest = cmdline;
    while let Some(i) = rest.find(&wanted) {
        rest = &rest[i + wanted.len()..];
        if matches!(rest.chars().next(), None | Some('"') | Some(' ') | Some('\0')) {
            return true;
        }
    }
    false
}

/// Wandelt Zeitstempel in Sekunden seit 1970 um, ohne bei exotischen Werten ueberzulaufen.
#[cfg_attr(not(unix), allow(dead_code))]
pub(crate) fn clamp_secs(secs: i64) -> u32 {
    secs.clamp(0, u32::MAX as i64) as u32
}

/// Zufaelliges Geheimnis als Hex-Text, fuer Zugriffs-Token und Einmal-Codes.
pub fn random_hex() -> String {
    let mut b = [0u8; 16];
    random_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_the_main_process_of_a_profile() {
        let p = r"C:\Daten\StorageMap\window";
        assert!(is_browser_on_profile(r#"msedge.exe --app=x --user-data-dir=C:\Daten\StorageMap\window --no-first-run"#, p));
        assert!(is_browser_on_profile(r#"msedge.exe --user-data-dir=C:\Daten\StorageMap\window"#, p));
        assert!(is_browser_on_profile("chromium\0--app=x\0--user-data-dir=/home/a/window\0--no-first-run\0", "/home/a/window"));
        assert!(is_browser_on_profile(r#"msedge.exe "--user-data-dir=C:\Mit Leerzeichen\window" --x"#, r"C:\Mit Leerzeichen\window"));
    }

    #[test]
    fn ignores_helpers_other_profiles_and_the_normal_browser() {
        let p = r"C:\Daten\StorageMap\window";
        assert!(!is_browser_on_profile(r#"msedge.exe --type=renderer --user-data-dir=C:\Daten\StorageMap\window"#, p), "Hilfsprozess");
        assert!(!is_browser_on_profile(r#"msedge.exe --user-data-dir=C:\Daten\StorageMap\window-47892"#, p), "anderes Profil");
        assert!(!is_browser_on_profile(r#"msedge.exe --profile-directory=Default"#, p), "normaler Browser des Nutzers");
        assert!(!is_browser_on_profile(r#"msedge.exe https://example.com"#, p));
    }
}
