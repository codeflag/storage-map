//! Pfadlogik fuer Windows- und Linux-Pfade. Welche Welt gemeint ist, verraet der Pfad selbst:
//! Linux-Pfade beginnen mit `/`. So laufen die Tests beider Welten auf jedem System.

use crate::i18n::Text;

pub fn is_unix_style(path: &str) -> bool {
    path.starts_with('/')
}

pub fn sep_of(path: &str) -> char {
    if is_unix_style(path) {
        '/'
    } else {
        '\\'
    }
}

pub fn join(parent: &str, name: &str) -> String {
    let sep = sep_of(parent);
    if parent.ends_with(sep) {
        format!("{parent}{name}")
    } else {
        format!("{parent}{sep}{name}")
    }
}

/// Uebergeordneter Ordner. `None` an der Wurzel eines Laufwerks oder einer Netzfreigabe.
pub fn parent_of(path: &str) -> Option<String> {
    if is_unix_style(path) {
        let t = path.trim_end_matches('/');
        if t.is_empty() {
            return None;
        }
        let (p, _) = t.rsplit_once('/')?;
        return Some(if p.is_empty() { "/".to_string() } else { p.to_string() });
    }
    let t = path.trim_end_matches('\\');
    if let Some(unc) = t.strip_prefix(r"\\") {
        return if unc.matches('\\').count() < 2 { None } else { t.rsplit_once('\\').map(|(p, _)| p.to_string()) };
    }
    if t.len() == 2 {
        return None;
    }
    let (p, _) = t.rsplit_once('\\')?;
    Some(if p.len() == 2 { format!("{p}\\") } else { p.to_string() })
}

/// Enthaelt der Pfad `.` oder `..` als Bestandteil? Solche Pfade koennen aus jedem Ordner herausfuehren.
pub fn has_dot_parts(path: &str) -> bool {
    path.split(sep_of(path)).any(|c| c == ".." || c == ".")
}

/// Liegt `path` in `root` oder ist es `root`? Rein textuell und ohne `..`, damit nichts herausfuehren kann.
/// Windows-Pfade unterscheiden nicht zwischen Gross- und Kleinschreibung, Linux-Pfade schon.
pub fn inside(path: &str, root: &str) -> bool {
    if is_unix_style(path) != is_unix_style(root) || has_dot_parts(path) {
        return false;
    }
    let sep = sep_of(root);
    let fold = |s: &str| {
        let t = s.trim_end_matches(sep);
        if sep == '\\' {
            t.to_lowercase()
        } else {
            t.to_string()
        }
    };
    let (p, r) = (fold(path), fold(root));
    p == r || p.starts_with(&format!("{r}{sep}"))
}

const WINDOWS_HINT: Text = Text::new("Please enter a full path, for example D:\\Data", "Bitte einen vollständigen Pfad angeben, z. B. D:\\Daten");
const UNIX_HINT: Text = Text::new("Please enter a full path, for example /mnt/data", "Bitte einen vollständigen Pfad angeben, z. B. /mnt/daten");
const NO_PATH: Text = Text::new("No path given.", "Kein Pfad angegeben.");

/// `c:` wird `C:\`, abschliessende Trenner fallen weg.
pub fn normalize_windows(raw: &str) -> Result<String, Text> {
    let t = raw.trim().trim_matches('"').replace('/', "\\");
    if t.is_empty() {
        return Err(NO_PATH);
    }
    let b = t.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        let drive = (b[0] as char).to_ascii_uppercase();
        let rest = t[2..].trim_end_matches('\\');
        if rest.is_empty() {
            return Ok(format!("{drive}:\\"));
        }
        if !rest.starts_with('\\') {
            return Err(WINDOWS_HINT);
        }
        return Ok(format!("{drive}:{rest}"));
    }
    if t.starts_with(r"\\") {
        return Ok(t.trim_end_matches('\\').to_string());
    }
    Err(WINDOWS_HINT)
}

/// Doppelte und abschliessende `/` fallen weg. Nur absolute Pfade sind erlaubt.
pub fn normalize_unix(raw: &str) -> Result<String, Text> {
    let t = raw.trim().trim_matches('"');
    if t.is_empty() {
        return Err(NO_PATH);
    }
    if !t.starts_with('/') {
        return Err(UNIX_HINT);
    }
    let parts: Vec<&str> = t.split('/').filter(|c| !c.is_empty()).collect();
    Ok(format!("/{}", parts.join("/")))
}

pub fn normalize(raw: &str) -> Result<String, Text> {
    if cfg!(windows) {
        normalize_windows(raw)
    } else {
        normalize_unix(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_containment_cannot_be_tricked() {
        let root = r"D:\Test\Fixture";
        assert!(inside(r"D:\Test\Fixture", root));
        assert!(inside(r"d:\test\fixture\Filme\a.mp4", root));
        assert!(inside(r"D:\Test\Fixture\", root));
        assert!(!inside(r"D:\Test\Fixture-Andere", root), "Namensanfang reicht nicht");
        assert!(!inside(r"D:\Test", root));
        assert!(!inside(r"C:\", root));
        assert!(!inside(r"D:\Test\Fixture\..\..\Windows", root), "kein Ausbruch mit ..");
        assert!(!inside(r"D:\Test\Fixture\.", root));
    }

    #[test]
    fn unix_containment_cannot_be_tricked() {
        let root = "/mnt/test/fixture";
        assert!(inside("/mnt/test/fixture", root));
        assert!(inside("/mnt/test/fixture/filme/a.mp4", root));
        assert!(inside("/mnt/test/fixture/", root));
        assert!(!inside("/mnt/test/Fixture", root), "Linux unterscheidet Gross- und Kleinschreibung");
        assert!(!inside("/mnt/test/fixture-andere", root));
        assert!(!inside("/mnt/test", root));
        assert!(!inside("/", root));
        assert!(!inside("/mnt/test/fixture/../../../etc", root));
        assert!(!inside(r"C:\", root));
        assert!(inside("/etc/passwd", "/"));
    }

    #[test]
    fn normalizes_windows_input() {
        assert_eq!(normalize_windows("c:").unwrap(), r"C:\");
        assert_eq!(normalize_windows(r"c:\").unwrap(), r"C:\");
        assert_eq!(normalize_windows(r"d:\Daten\").unwrap(), r"D:\Daten");
        assert_eq!(normalize_windows("D:/Daten/Film").unwrap(), r"D:\Daten\Film");
        assert_eq!(normalize_windows(r#""E:\Mit Leerzeichen""#).unwrap(), r"E:\Mit Leerzeichen");
        assert_eq!(normalize_windows(r"\\nas\film\").unwrap(), r"\\nas\film");
        assert!(normalize_windows("relativ\\pfad").is_err());
        assert!(normalize_windows("C:relativ").is_err());
        assert!(normalize_windows("").is_err());
    }

    #[test]
    fn normalizes_unix_input() {
        assert_eq!(normalize_unix("/").unwrap(), "/");
        assert_eq!(normalize_unix("/mnt/daten/").unwrap(), "/mnt/daten");
        assert_eq!(normalize_unix("//mnt//daten").unwrap(), "/mnt/daten");
        assert_eq!(normalize_unix(r#""/mnt/mit leerzeichen""#).unwrap(), "/mnt/mit leerzeichen");
        assert!(normalize_unix("relativ/pfad").is_err());
        assert!(normalize_unix("").is_err());
    }

    #[test]
    fn finds_parents() {
        assert_eq!(parent_of(r"C:\"), None);
        assert_eq!(parent_of(r"C:\a").as_deref(), Some(r"C:\"));
        assert_eq!(parent_of(r"C:\a\b").as_deref(), Some(r"C:\a"));
        assert_eq!(parent_of(r"\\nas\film"), None);
        assert_eq!(parent_of(r"\\nas\film\a").as_deref(), Some(r"\\nas\film"));
        assert_eq!(parent_of("/"), None);
        assert_eq!(parent_of("/a").as_deref(), Some("/"));
        assert_eq!(parent_of("/a/b").as_deref(), Some("/a"));
    }

    #[test]
    fn joins_with_the_right_separator() {
        assert_eq!(join(r"C:\", "a"), r"C:\a");
        assert_eq!(join(r"C:\x", "a"), r"C:\x\a");
        assert_eq!(join("/", "a"), "/a");
        assert_eq!(join("/x", "a"), "/x/a");
    }
}
