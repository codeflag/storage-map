//! Das Datenmodell: alle Dateien und Ordner eines Scans in einem kompakten, flachen Array.
//! Kinder eines Ordners liegen zusammenhaengend, Eltern immer vor ihren Kindern.

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::ffi::{OsStr, OsString};
use std::io::{Read, Write};
use std::path::Path;

pub const NONE: u32 = u32::MAX;

pub const F_DIR: u16 = 1;
pub const F_HIDDEN: u16 = 2;
pub const F_SYSTEM: u16 = 4;
/// Symlink oder Junction: Ziel wird nicht mitgezaehlt.
pub const F_LINK: u16 = 8;
/// Weiterer Name (Hardlink) einer Datei, die schon gezaehlt wurde.
pub const F_HARDLINK: u16 = 16;
/// Cloud-Platzhalter, liegt nicht wirklich auf der Platte.
pub const F_CLOUD: u16 = 32;
/// Ordner konnte nicht gelesen werden.
pub const F_DENIED: u16 = 64;
/// Wurde ueber die Aufraeumliste entfernt.
pub const F_DELETED: u16 = 128;
pub const F_COMPRESSED: u16 = 256;
/// Hier ist ein anderes Dateisystem eingehaengt: wird weder betreten noch geloescht.
pub const F_MOUNT: u16 = 512;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Node {
    /// Belegter Platz auf dem Datentraeger (bei Ordnern: inklusive Inhalt).
    pub alloc: u64,
    /// Logische Groesse (bei Ordnern: inklusive Inhalt).
    pub size: u64,
    pub file_id: u64,
    pub parent: u32,
    pub first_child: u32,
    pub child_count: u32,
    pub files: u32,
    pub dirs: u32,
    pub mtime: u32,
    pub atime: u32,
    pub ctime: u32,
    /// Ordner: juengste Dateiaenderung im Inhalt. Datei: gleich `mtime`.
    pub newest: u32,
    /// Ordner: nach Groesse gewichtetes mittleres Aenderungsdatum. Datei: gleich `mtime`.
    pub avg_mtime: u32,
    pub name_off: u32,
    pub name_len: u16,
    pub flags: u16,
    pub ext: u16,
    /// Datei: Kategorie. Ordner: Kategorie, die den meisten Platz belegt.
    pub cat: u8,
    pub _pad: u8,
    pub _pad2: u32,
}

impl Node {
    #[inline]
    pub fn is_dir(&self) -> bool {
        self.flags & F_DIR != 0
    }
    #[inline]
    pub fn is_deleted(&self) -> bool {
        self.flags & F_DELETED != 0
    }
}

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct VolumeMeta {
    pub total: u64,
    pub free: u64,
    pub label: String,
    pub fs: String,
    pub kind: String,
    pub media: String,
}

#[derive(Clone, Serialize, Deserialize, Default, Debug)]
pub struct Denied {
    pub path: String,
    pub code: u32,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Meta {
    pub root: String,
    pub is_drive: bool,
    pub scanned_at: u64,
    pub duration_ms: u64,
    pub volume: VolumeMeta,
    pub denied: Vec<Denied>,
    pub exts: Vec<String>,
    pub elevated: bool,
    /// Belegt laut Betriebssystem minus gefunden, gemessen beim Scan.
    #[serde(default)]
    pub unaccounted: u64,
}

#[derive(Default)]
pub struct Index {
    pub nodes: Vec<Node>,
    pub names: Vec<u8>,
    pub meta: Meta,
}

const MAGIC: &[u8; 8] = b"SMAPIDX3";

impl Index {
    #[inline]
    pub fn name_bytes(&self, id: u32) -> &[u8] {
        let n = &self.nodes[id as usize];
        &self.names[n.name_off as usize..n.name_off as usize + n.name_len as usize]
    }

    pub fn name(&self, id: u32) -> Cow<'_, str> {
        String::from_utf8_lossy(self.name_bytes(id))
    }

    /// Pfad fuer die Anzeige.
    pub fn path(&self, id: u32) -> String {
        self.exact_path(id).to_string_lossy().into_owned()
    }

    /// Exakter Pfad; bleibt auch bei ungewoehnlichen Dateinamen korrekt, die keine gueltige Unicode-Folge sind.
    pub fn exact_path(&self, id: u32) -> OsString {
        let mut chain = vec![];
        let mut cur = id;
        while cur != NONE && cur != 0 {
            chain.push(cur);
            cur = self.nodes[cur as usize].parent;
        }
        let sep = crate::paths::sep_of(&self.meta.root);
        let mut out = OsString::from(&self.meta.root);
        for c in chain.iter().rev() {
            if !out.to_string_lossy().ends_with(sep) {
                out.push(sep.to_string());
            }
            // Sicher: Die Bytes stammen aus OsStr::as_encoded_bytes beim Scan.
            let s = unsafe { OsStr::from_encoded_bytes_unchecked(self.name_bytes(*c)) };
            out.push(s);
        }
        out
    }

    /// Sucht einen Eintrag ueber seinen Pfad. Windows-Pfade passen auch bei anderer Gross- und Kleinschreibung.
    pub fn find_path(&self, path: &str) -> Option<u32> {
        let root = &self.meta.root;
        if !crate::paths::inside(path, root) {
            return None;
        }
        let sep = crate::paths::sep_of(root);
        let rest = path.get(root.trim_end_matches(sep).len()..)?;
        let mut cur = 0u32;
        for part in rest.split(sep).filter(|c| !c.is_empty()) {
            let n = &self.nodes[cur as usize];
            if !n.is_dir() {
                return None;
            }
            let alive = |c: &u32| !self.nodes[*c as usize].is_deleted();
            let kids = n.first_child..n.first_child + n.child_count;
            cur = kids.clone().filter(alive).find(|c| self.name(*c) == part).or_else(|| {
                if sep == '/' {
                    return None;
                }
                let low = part.to_lowercase();
                kids.clone().filter(alive).find(|c| self.name(*c).to_lowercase() == low)
            })?;
        }
        (!self.nodes[cur as usize].is_deleted()).then_some(cur)
    }

    /// Kinder ohne geloeschte Eintraege, groesste zuerst.
    pub fn children_by_size(&self, id: u32) -> Vec<u32> {
        let n = &self.nodes[id as usize];
        let mut v: Vec<u32> = (n.first_child..n.first_child + n.child_count).filter(|c| !self.nodes[*c as usize].is_deleted()).collect();
        v.sort_unstable_by(|a, b| self.nodes[*b as usize].alloc.cmp(&self.nodes[*a as usize].alloc).then(a.cmp(b)));
        v
    }

    pub fn ancestors(&self, id: u32) -> Vec<u32> {
        let mut v = vec![];
        let mut cur = id;
        while cur != NONE {
            v.push(cur);
            cur = self.nodes[cur as usize].parent;
        }
        v.reverse();
        v
    }

    pub fn is_under(&self, id: u32, ancestor: u32) -> bool {
        let mut cur = id;
        while cur != NONE {
            if cur == ancestor {
                return true;
            }
            cur = self.nodes[cur as usize].parent;
        }
        false
    }

    /// Alle Knoten unterhalb von `id` (inklusive).
    pub fn subtree(&self, id: u32) -> Vec<u32> {
        let mut out = vec![];
        let mut stack = vec![id];
        while let Some(i) = stack.pop() {
            out.push(i);
            let n = &self.nodes[i as usize];
            if n.is_dir() {
                for c in n.first_child..n.first_child + n.child_count {
                    if self.nodes[c as usize].is_dir() {
                        stack.push(c);
                    } else {
                        out.push(c);
                    }
                }
            }
        }
        out
    }

    /// Nimmt einen Eintrag aus der Bilanz, nachdem er entfernt wurde.
    pub fn mark_deleted(&mut self, id: u32) {
        if id == 0 || self.nodes[id as usize].is_deleted() {
            return;
        }
        let n = self.nodes[id as usize];
        let (files, dirs) = if n.is_dir() { (n.files, n.dirs + 1) } else { (1, 0) };
        let mut cur = n.parent;
        while cur != NONE {
            let p = &mut self.nodes[cur as usize];
            p.alloc = p.alloc.saturating_sub(n.alloc);
            p.size = p.size.saturating_sub(n.size);
            p.files = p.files.saturating_sub(files);
            p.dirs = p.dirs.saturating_sub(dirs);
            cur = p.parent;
        }
        for c in self.subtree(id) {
            self.nodes[c as usize].flags |= F_DELETED;
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let tmp = path.with_extension("tmp");
        {
            let f = std::fs::File::create(&tmp)?;
            let mut enc = lz4_flex::frame::FrameEncoder::new(std::io::BufWriter::new(f));
            enc.write_all(MAGIC)?;
            enc.write_all(&(std::mem::size_of::<Node>() as u32).to_le_bytes())?;
            let meta = serde_json::to_vec(&self.meta)?;
            enc.write_all(&(meta.len() as u64).to_le_bytes())?;
            enc.write_all(&meta)?;
            enc.write_all(&(self.nodes.len() as u64).to_le_bytes())?;
            enc.write_all(&(self.names.len() as u64).to_le_bytes())?;
            let raw = unsafe { std::slice::from_raw_parts(self.nodes.as_ptr() as *const u8, self.nodes.len() * std::mem::size_of::<Node>()) };
            enc.write_all(raw)?;
            enc.write_all(&self.names)?;
            enc.finish().map_err(std::io::Error::other)?.flush()?;
        }
        std::fs::rename(tmp, path)
    }

    pub fn load(path: &Path) -> std::io::Result<Index> {
        let bad = |m: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, m.to_string());
        let f = std::fs::File::open(path)?;
        let mut dec = lz4_flex::frame::FrameDecoder::new(std::io::BufReader::new(f));
        let mut magic = [0u8; 8];
        dec.read_exact(&mut magic)?;
        if &magic != MAGIC {
            return Err(bad("Old or unknown file format"));
        }
        let mut b4 = [0u8; 4];
        dec.read_exact(&mut b4)?;
        if u32::from_le_bytes(b4) as usize != std::mem::size_of::<Node>() {
            return Err(bad("Node size does not match"));
        }
        let mut b8 = [0u8; 8];
        dec.read_exact(&mut b8)?;
        let mut meta = vec![0u8; u64::from_le_bytes(b8) as usize];
        dec.read_exact(&mut meta)?;
        let meta: Meta = serde_json::from_slice(&meta)?;
        dec.read_exact(&mut b8)?;
        let n_nodes = u64::from_le_bytes(b8) as usize;
        dec.read_exact(&mut b8)?;
        let n_names = u64::from_le_bytes(b8) as usize;
        let mut nodes = vec![Node::default(); n_nodes];
        let raw = unsafe { std::slice::from_raw_parts_mut(nodes.as_mut_ptr() as *mut u8, n_nodes * std::mem::size_of::<Node>()) };
        dec.read_exact(raw)?;
        let mut names = vec![0u8; n_names];
        dec.read_exact(&mut names)?;
        // Plausibilitaetspruefung, damit eine beschaedigte Datei nicht zu falschen Zugriffen fuehrt.
        for n in &nodes {
            if n.name_off as usize + n.name_len as usize > names.len()
                || (n.is_dir() && (n.first_child as u64 + n.child_count as u64) > n_nodes as u64)
            {
                return Err(bad("Damaged index file"));
            }
        }
        Ok(Index { nodes, names, meta })
    }
}

#[cfg(test)]
pub mod testutil {
    use super::*;
    use crate::category;

    /// Baut einen Index aus (Pfad relativ zur Wurzel, Groesse). Pfade mit Trenner am Ende sind Ordner.
    pub fn build(root: &str, entries: &[(&str, u64)]) -> Index {
        use std::collections::BTreeMap;
        #[derive(Default)]
        struct T {
            size: Option<u64>,
            kids: BTreeMap<String, T>,
        }
        let mut tree = T::default();
        for (p, size) in entries {
            let is_dir = p.ends_with(['\\', '/']);
            let parts: Vec<&str> = p.trim_end_matches(['\\', '/']).split(['\\', '/']).collect();
            let mut cur = &mut tree;
            for (i, part) in parts.iter().enumerate() {
                cur = cur.kids.entry(part.to_string()).or_default();
                if i + 1 == parts.len() && !is_dir {
                    cur.size = Some(*size);
                }
            }
        }
        let mut ix = Index::default();
        ix.meta.root = root.to_string();
        ix.meta.exts = vec![String::new()];
        ix.names.extend_from_slice(root.as_bytes());
        ix.nodes.push(Node { parent: NONE, name_len: root.len() as u16, flags: F_DIR, ..Default::default() });
        fn add(ix: &mut Index, id: u32, t: &T) {
            let base = ix.nodes.len() as u32;
            ix.nodes[id as usize].first_child = base;
            ix.nodes[id as usize].child_count = t.kids.len() as u32;
            let mut order = vec![];
            for (name, k) in &t.kids {
                let off = ix.names.len() as u32;
                ix.names.extend_from_slice(name.as_bytes());
                let mut n = Node { parent: id, name_off: off, name_len: name.len() as u16, ..Default::default() };
                if let Some(s) = k.size {
                    n.alloc = s;
                    n.size = s;
                    if let Some((_, e)) = name.rsplit_once('.') {
                        let e = e.to_lowercase();
                        n.cat = category::classify(&e);
                        let pos = ix.meta.exts.iter().position(|x| *x == e);
                        n.ext = match pos {
                            Some(p) => p as u16,
                            None => {
                                ix.meta.exts.push(e);
                                (ix.meta.exts.len() - 1) as u16
                            }
                        };
                    } else {
                        n.cat = category::OTHER;
                    }
                } else {
                    n.flags = F_DIR;
                }
                ix.nodes.push(n);
                order.push(k);
            }
            for (i, k) in order.into_iter().enumerate() {
                if k.size.is_none() {
                    add(ix, base + i as u32, k);
                }
            }
        }
        add(&mut ix, 0, &tree);
        crate::scan::aggregate(&mut ix);
        ix
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::build;
    use super::*;

    #[test]
    fn aggregates_paths_and_children() {
        let ix = build(r"C:\", &[(r"a\b\film.mp4", 1000), (r"a\c.txt", 10), (r"d\", 0)]);
        assert_eq!(ix.nodes[0].alloc, 1010);
        assert_eq!(ix.nodes[0].files, 2);
        let a = ix.children_by_size(0)[0];
        assert_eq!(ix.path(a), r"C:\a");
        let big = ix.children_by_size(a)[0];
        assert_eq!(ix.path(big), r"C:\a\b");
        let film = ix.children_by_size(big)[0];
        assert_eq!(ix.path(film), r"C:\a\b\film.mp4");
    }

    #[test]
    fn linux_paths_use_slashes() {
        let ix = build("/", &[("home/ich/film.mp4", 1000)]);
        let id = ix.find_path("/home/ich/film.mp4").unwrap();
        assert_eq!(ix.path(id), "/home/ich/film.mp4");
        let ix = build("/mnt/daten", &[("a/b.txt", 1)]);
        assert_eq!(ix.path(ix.find_path("/mnt/daten/a/b.txt").unwrap()), "/mnt/daten/a/b.txt");
        assert_eq!(ix.find_path("/mnt/daten").unwrap(), 0);
    }

    #[test]
    fn find_path_finds_only_what_exists() {
        let mut ix = build(r"C:\", &[(r"Daten\Film.mp4", 1000)]);
        let id = ix.find_path(r"C:\Daten\Film.mp4").unwrap();
        assert_eq!(ix.find_path(r"c:\daten\film.MP4"), Some(id), "Windows ignoriert die Schreibweise");
        assert_eq!(ix.find_path(r"C:\Daten\Fehlt.mp4"), None);
        assert_eq!(ix.find_path(r"D:\Daten\Film.mp4"), None);
        assert_eq!(ix.find_path(r"C:\Daten\..\Daten\Film.mp4"), None);
        ix.mark_deleted(id);
        assert_eq!(ix.find_path(r"C:\Daten\Film.mp4"), None, "Geloeschtes gibt es nicht mehr");
        let linux = build("/", &[("Daten/Film.mp4", 1)]);
        assert_eq!(linux.find_path("/daten/film.mp4"), None, "Linux unterscheidet die Schreibweise");
    }

    #[test]
    fn mark_deleted_updates_all_ancestors() {
        let mut ix = build(r"C:\", &[(r"a\b\film.mp4", 1000), (r"a\c.txt", 10)]);
        let a = ix.children_by_size(0)[0];
        let b = ix.children_by_size(a)[0];
        ix.mark_deleted(b);
        assert_eq!(ix.nodes[0].alloc, 10);
        assert_eq!(ix.nodes[a as usize].files, 1);
        assert_eq!(ix.children_by_size(a).len(), 1);
    }

    #[test]
    fn save_and_load_roundtrip() {
        let ix = build(r"C:\", &[(r"a\b.bin", 123), (r"a\ü.txt", 7)]);
        let dir = std::env::temp_dir().join(format!("smap-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("rt.smidx");
        ix.save(&f).unwrap();
        let back = Index::load(&f).unwrap();
        assert_eq!(back.nodes.len(), ix.nodes.len());
        assert_eq!(back.nodes[0].alloc, 130);
        assert_eq!(back.names, ix.names);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
