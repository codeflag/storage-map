//! Speichert Scans dauerhaft: den letzten kompletten Index (Sofortstart), einen Verlauf und
//! pro Scan eine kompakte Ordnerliste, damit sich spaeter sagen laesst, was gewachsen ist.

use crate::category;
use crate::index::*;
use crate::paths;
use crate::platform;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Ordner kleiner als das werden im Verlauf nicht gespeichert. Sie erklaeren kein Wachstum.
const SNAPSHOT_MIN: u64 = 1024 * 1024;
const KEEP_SNAPSHOTS: usize = 40;

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Eigener Datenordner fuer Demo, Sandbox und Tests, damit nichts in den echten Verlauf gelangt.
pub fn set_data_dir(p: PathBuf) {
    let _ = DATA_DIR.set(p);
}

pub fn data_dir() -> PathBuf {
    if let Some(d) = DATA_DIR.get() {
        return d.clone();
    }
    if cfg!(test) {
        return std::env::temp_dir().join(format!("smap-unit-data-{}", std::process::id()));
    }
    platform::default_data_dir()
}

/// Laeuft das Programm mit eigenem Datenordner? Dann gehoert auch die Konfiguration dorthin.
pub fn has_custom_data_dir() -> bool {
    DATA_DIR.get().is_some()
}

/// Ordnername eines Scans. Laufwerke heissen einfach `C` oder `root`. Bei allem anderen macht
/// ein Pruefwert den Namen eindeutig, denn `/mnt/a_b` und `/mnt/a/b` saehen sonst gleich aus.
pub fn root_key(root: &str) -> String {
    // Windows unterscheidet die Schreibweise nicht, also darf sie auch keinen neuen Ordner ergeben.
    let folded = if paths::is_unix_style(root) { root.to_string() } else { root.to_lowercase() };
    let s: String = folded.chars().map(|c| if c.is_alphanumeric() { c } else { '_' }).collect();
    let s = s.trim_matches('_').to_string();
    if s.is_empty() {
        return "root".into();
    }
    if s.chars().count() == 1 {
        return s.to_uppercase();
    }
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for b in folded.bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let short: String = s.chars().take(60).collect();
    format!("{short}-{:08x}", hash as u32)
}

pub fn scan_dir(root: &str) -> PathBuf {
    data_dir().join("scans").join(root_key(root))
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    pub ts: u64,
    pub root: String,
    pub total: u64,
    pub free: u64,
    pub scanned: u64,
    pub files: u64,
    pub dirs: u64,
    pub denied: u64,
    pub duration_ms: u64,
    pub cats: Vec<u64>,
}

pub fn entry_of(ix: &Index) -> Entry {
    let mut cats = vec![0u64; category::COUNT];
    for n in &ix.nodes {
        if !n.is_dir() && !n.is_deleted() {
            cats[n.cat as usize] += n.alloc;
        }
    }
    Entry {
        ts: ix.meta.scanned_at,
        root: ix.meta.root.clone(),
        total: ix.meta.volume.total,
        free: ix.meta.volume.free,
        scanned: ix.nodes[0].alloc,
        files: ix.nodes[0].files as u64,
        dirs: ix.nodes[0].dirs as u64,
        denied: ix.meta.denied.len() as u64,
        duration_ms: ix.meta.duration_ms,
        cats,
    }
}

pub fn read_entries(root: &str) -> Vec<Entry> {
    let p = scan_dir(root).join("history.jsonl");
    let Ok(text) = std::fs::read_to_string(p) else { return vec![] };
    text.lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

fn snapshot_path(root: &str, ts: u64) -> PathBuf {
    scan_dir(root).join(format!("dirs-{ts}.lz4"))
}

pub fn dir_rows(ix: &Index) -> Vec<(String, u64, u32)> {
    let mut out = vec![];
    for (i, n) in ix.nodes.iter().enumerate() {
        if n.is_dir() && !n.is_deleted() && n.alloc >= SNAPSHOT_MIN {
            out.push((ix.path(i as u32), n.alloc, n.files));
        }
    }
    out
}

pub fn append_entry(e: &Entry) -> std::io::Result<()> {
    let dir = scan_dir(&e.root);
    std::fs::create_dir_all(&dir)?;
    let line = serde_json::to_string(e)?;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("history.jsonl"))?;
    writeln!(f, "{line}")
}

pub fn write_snapshot(root: &str, ts: u64, rows: &[(String, u64, u32)]) -> std::io::Result<()> {
    std::fs::create_dir_all(scan_dir(root))?;
    let data = serde_json::to_vec(rows)?;
    let raw = std::fs::File::create(snapshot_path(root, ts))?;
    let mut enc = lz4_flex::frame::FrameEncoder::new(raw);
    enc.write_all(&data)?;
    enc.finish().map_err(std::io::Error::other)?;
    Ok(())
}

/// Sichert den Scan: Verlaufszeile, Ordnerliste, kompletter Index.
pub fn save_scan(ix: &Index) -> std::io::Result<()> {
    let root = &ix.meta.root;
    let dir = scan_dir(root);
    append_entry(&entry_of(ix))?;
    write_snapshot(root, ix.meta.scanned_at, &dir_rows(ix))?;

    let mut snaps: Vec<PathBuf> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().map(|n| n.to_string_lossy().starts_with("dirs-")).unwrap_or(false))
        .collect();
    snaps.sort();
    if snaps.len() > KEEP_SNAPSHOTS {
        for old in &snaps[..snaps.len() - KEEP_SNAPSHOTS] {
            let _ = std::fs::remove_file(old);
        }
    }

    ix.save(&dir.join("latest.smidx"))?;
    let _ = std::fs::remove_file(dir.join("dups.json"));
    let meta = json!({"root": root, "ts": ix.meta.scanned_at, "isDrive": ix.meta.is_drive});
    std::fs::write(dir.join("meta.json"), meta.to_string())?;
    Ok(())
}

pub fn load_latest(root: &str) -> std::io::Result<Index> {
    Index::load(&scan_dir(root).join("latest.smidx"))
}

/// Alle Wurzeln, von denen ein gespeicherter Scan existiert.
pub fn known_scans() -> Vec<Value> {
    let mut out = vec![];
    let Ok(rd) = std::fs::read_dir(data_dir().join("scans")) else { return out };
    for e in rd.flatten() {
        let m = e.path().join("meta.json");
        if !e.path().join("latest.smidx").exists() {
            continue;
        }
        if let Some(v) = std::fs::read_to_string(m).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) {
            out.push(v);
        }
    }
    out
}

fn read_snapshot(path: &Path) -> Option<Vec<(String, u64, u32)>> {
    let f = std::fs::File::open(path).ok()?;
    let mut dec = lz4_flex::frame::FrameDecoder::new(f);
    let mut buf = vec![];
    dec.read_to_end(&mut buf).ok()?;
    serde_json::from_slice(&buf).ok()
}

/// Vergleicht den aktuellen Stand mit einem frueheren Scan.
/// Nennt nur die Ordner, die eine Aenderung wirklich erklaeren, nicht auch noch jeden Oberordner.
pub fn diff(ix: &Index, base_ts: u64) -> Option<Value> {
    let prev = read_snapshot(&snapshot_path(&ix.meta.root, base_ts))?;
    let prev_map: HashMap<&str, (u64, u32)> = prev.iter().map(|(p, a, f)| (p.as_str(), (*a, *f))).collect();
    let now = dir_rows(ix);
    let now_map: HashMap<&str, (u64, u32)> = now.iter().map(|(p, a, f)| (p.as_str(), (*a, *f))).collect();

    struct D<'a> {
        path: &'a str,
        before: u64,
        after: u64,
        delta: i64,
        status: &'static str,
    }
    let mut all: Vec<D> = vec![];
    for (p, (a, _)) in &now_map {
        match prev_map.get(p) {
            Some((b, _)) => {
                if a != b {
                    all.push(D { path: p, before: *b, after: *a, delta: *a as i64 - *b as i64, status: "changed" });
                }
            }
            None => all.push(D { path: p, before: 0, after: *a, delta: *a as i64, status: "new" }),
        }
    }
    for (p, (b, _)) in &prev_map {
        if !now_map.contains_key(p) {
            all.push(D { path: p, before: *b, after: 0, delta: -(*b as i64), status: "gone" });
        }
    }

    let by_path: HashMap<&str, i64> = all.iter().map(|d| (d.path, d.delta)).collect();
    // Staerkste Aenderung eines direkten Unterordners je Ordner.
    let mut best_child: HashMap<&str, i64> = HashMap::new();
    for d in &all {
        if let Some(pos) = d.path.rfind(paths::sep_of(d.path)) {
            let mut parent = &d.path[..pos];
            if !by_path.contains_key(parent) {
                parent = &d.path[..pos + 1]; // Wurzel, also "C:\" oder "/"
            }
            let e = best_child.entry(parent).or_insert(0);
            if d.delta.abs() > e.abs() {
                *e = d.delta;
            }
        }
    }
    let threshold = 1024 * 1024 * 10;
    let mut causes: Vec<&D> = all
        .iter()
        .filter(|d| {
            if d.delta.abs() < threshold {
                return false;
            }
            match best_child.get(d.path) {
                // Ordner ist nur Durchreiche, wenn ein Unterordner den Grossteil erklaert.
                Some(c) => !(c.signum() == d.delta.signum() && c.abs() as f64 >= d.delta.abs() as f64 * 0.7),
                None => true,
            }
        })
        .collect();
    causes.sort_by_key(|d| std::cmp::Reverse(d.delta.abs()));
    causes.truncate(100);

    let rows: Vec<Value> =
        causes.iter().map(|d| json!({"path": d.path, "before": d.before, "after": d.after, "delta": d.delta, "status": d.status})).collect();
    let prev_total = prev_map.get(ix.meta.root.as_str()).map(|x| x.0).unwrap_or(0);
    Some(json!({
        "baseTs": base_ts,
        "beforeTotal": prev_total,
        "afterTotal": ix.nodes[0].alloc,
        "rows": rows,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::testutil::build;

    #[test]
    fn root_key_is_a_safe_folder_name() {
        assert_eq!(root_key(r"C:\"), "C");
        assert_eq!(root_key("/"), "root");
        assert!(root_key(r"D:\Daten\Ü 1").starts_with("d__daten_ü_1-"));
        assert!(root_key(r"\\nas\film").starts_with("nas_film-"));
        assert!(root_key("/mnt/daten").starts_with("mnt_daten-"));
        assert_ne!(root_key("/mnt/a_b"), root_key("/mnt/a/b"), "verschiedene Wurzeln, verschiedene Ordner");
        assert_eq!(root_key(r"D:\Daten"), root_key(r"d:\daten"), "Windows ignoriert die Schreibweise");
    }

    #[test]
    fn diff_names_the_cause_not_every_parent() {
        const MB: u64 = 1024 * 1024;
        let before = build(r"Q:\", &[(r"Daten\Projekt\klein.bin", 50 * MB), (r"Daten\Weg\alt.bin", 200 * MB), (r"Ruhig\gleich.bin", 80 * MB)]);
        let after =
            build(r"Q:\", &[(r"Daten\Projekt\klein.bin", 50 * MB), (r"Daten\Projekt\Render\riesig.bin", 900 * MB), (r"Ruhig\gleich.bin", 80 * MB)]);
        write_snapshot(r"Q:\", 1000, &dir_rows(&before)).unwrap();
        let d = diff(&after, 1000).unwrap();
        let _ = std::fs::remove_dir_all(data_dir());

        let rows = d["rows"].as_array().unwrap();
        let paths: Vec<&str> = rows.iter().map(|r| r["path"].as_str().unwrap()).collect();
        assert!(paths.contains(&r"Q:\Daten\Projekt\Render"), "{paths:?}");
        assert!(paths.contains(&r"Q:\Daten\Weg"), "{paths:?}");
        assert!(!paths.contains(&r"Q:\Daten\Projekt"), "Durchreiche-Ordner nicht nennen: {paths:?}");
        assert!(!paths.contains(&r"Q:\Ruhig"), "{paths:?}");
        let render = rows.iter().find(|r| r["path"] == r"Q:\Daten\Projekt\Render").unwrap();
        assert_eq!(render["status"], "new");
        let weg = rows.iter().find(|r| r["path"] == r"Q:\Daten\Weg").unwrap();
        assert_eq!(weg["status"], "gone");
    }
}
