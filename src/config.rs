//! Einstellungen, die ueber den Programmstart hinaus gelten. Bisher nur der MCP-Zugriff.
//!
//! Die Datei ist die einzige Stelle, an der sich Rechte aendern lassen: Der MCP-Server liest sie
//! vor jedem Aufruf und schreibt sie nie. Ist sie kaputt, gilt "Aus". Zweifel gehen nie zugunsten von mehr Rechten aus.

use crate::{history, platform};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Der MCP-Server antwortet nur auf die Frage, welche Rechte er hat.
    Off,
    /// Auswertungen abrufen, nichts aendern.
    #[default]
    Read,
    /// Zusaetzlich: Aufraeumliste bearbeiten und Dateien verschieben.
    Write,
    /// Zusaetzlich: Dateien in den Papierkorb legen.
    Delete,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Off => "off",
            Level::Read => "read",
            Level::Write => "write",
            Level::Delete => "delete",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    pub level: Level,
    /// Darf der MCP-Client selbst Scans und Duplikat-Suchen starten?
    pub allow_scan: bool,
    /// Darf er endgueltig loeschen, statt nur in den Papierkorb zu legen? Gilt nur auf Stufe `delete`.
    pub allow_permanent_delete: bool,
    /// Sichtbare Laufwerke und Ordner. Leer heisst: alle.
    pub roots: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub mcp: McpConfig,
}

pub fn path() -> PathBuf {
    let dir = if history::has_custom_data_dir() || cfg!(test) { history::data_dir() } else { platform::default_config_dir() };
    dir.join("config.json")
}

/// Liest die Einstellungen. Bei einer kaputten Datei kommt zusaetzlich die Fehlermeldung zurueck, und der Zugriff bleibt aus.
pub fn load() -> (Config, Option<String>) {
    match std::fs::read_to_string(path()) {
        Ok(text) => parse(&text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), None),
        Err(e) => (closed(), Some(format!("{e}"))),
    }
}

fn closed() -> Config {
    let mut c = Config::default();
    c.mcp.level = Level::Off;
    c
}

pub fn parse(text: &str) -> (Config, Option<String>) {
    match serde_json::from_str(text) {
        Ok(c) => (c, None),
        Err(e) => (closed(), Some(format!("{e}"))),
    }
}

pub fn save(c: &Config) -> std::io::Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        platform::create_private_dir(dir)?;
    }
    platform::write_private(&p, serde_json::to_string_pretty(c)?.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_read_only() {
        let c = Config::default();
        assert_eq!(c.mcp.level, Level::Read);
        assert!(!c.mcp.allow_scan);
        assert!(!c.mcp.allow_permanent_delete);
    }

    #[test]
    fn broken_config_closes_access() {
        for bad in ["{kaputt", r#"{"mcp":{"level":"alles"}}"#, r#"{"mcp":{"level":3}}"#, r#"{"mcp":{"allow_scan":"ja"}}"#] {
            let (c, err) = parse(bad);
            assert_eq!(c.mcp.level, Level::Off, "{bad}");
            assert!(err.is_some(), "{bad}");
        }
    }

    #[test]
    fn missing_fields_fall_back_to_safe_defaults() {
        let (c, err) = parse(r#"{"mcp":{"level":"delete"}}"#);
        assert!(err.is_none());
        assert_eq!(c.mcp.level, Level::Delete);
        assert!(!c.mcp.allow_permanent_delete);
        assert!(!c.mcp.allow_scan);
        assert_eq!(parse("{}").0.mcp.level, Level::Read);
    }

    #[test]
    fn levels_are_ordered() {
        assert!(Level::Off < Level::Read && Level::Read < Level::Write && Level::Write < Level::Delete);
    }
}
