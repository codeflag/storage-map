//! Duplikate finden in drei Stufen: gleiche Groesse, gleicher Anfang und gleiches Ende, gleicher Inhalt (BLAKE3).
//! Dazu doppelte Ordner: gleiche Struktur, gleiche Namen, gleiche Groessen (ohne Dateien zu lesen).

use crate::guard::{self, Guard};
use crate::index::*;
use crate::platform;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};

const PARTIAL: u64 = 64 * 1024;

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct DupGroup {
    pub size: u64,
    pub alloc: u64,
    pub ids: Vec<u32>,
    /// `true`: Inhalt komplett verglichen. `false`: nur Groesse, Anfang und Ende.
    pub verified: bool,
}

impl DupGroup {
    pub fn wasted(&self) -> u64 {
        self.alloc * (self.ids.len().saturating_sub(1) as u64)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DupParams {
    pub min_size: u64,
    pub quick: bool,
    pub include_system: bool,
    pub threads: usize,
}

#[derive(Default)]
pub struct DupEngine {
    pub running: AtomicBool,
    pub cancel: AtomicBool,
    pub done: AtomicBool,
    pub phase: Mutex<String>,
    pub total_files: AtomicU64,
    pub done_files: AtomicU64,
    pub total_bytes: AtomicU64,
    pub done_bytes: AtomicU64,
    pub skipped: AtomicU64,
    pub started: AtomicU64,
    pub finished: AtomicU64,
    pub groups: Mutex<Vec<DupGroup>>,
    pub quick: AtomicBool,
}

impl DupEngine {
    pub fn reset(&self) {
        self.running.store(false, Relaxed);
        self.cancel.store(false, Relaxed);
        self.done.store(false, Relaxed);
        self.total_files.store(0, Relaxed);
        self.done_files.store(0, Relaxed);
        self.total_bytes.store(0, Relaxed);
        self.done_bytes.store(0, Relaxed);
        self.skipped.store(0, Relaxed);
        self.started.store(0, Relaxed);
        self.finished.store(0, Relaxed);
        self.groups.lock().unwrap().clear();
        *self.phase.lock().unwrap() = String::new();
    }
}

struct Cand {
    id: u32,
    size: u64,
    alloc: u64,
    path: std::path::PathBuf,
}

fn partial_hash(path: &Path, size: u64, eng: &DupEngine) -> Option<[u8; 16]> {
    let mut f = platform::open_quiet(path).ok()?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; PARTIAL as usize];
    let first = PARTIAL.min(size) as usize;
    f.read_exact(&mut buf[..first]).ok()?;
    h.update(&buf[..first]);
    let mut read = first as u64;
    if size > PARTIAL {
        let tail = PARTIAL.min(size - PARTIAL);
        f.seek(SeekFrom::End(-(tail as i64))).ok()?;
        f.read_exact(&mut buf[..tail as usize]).ok()?;
        h.update(&buf[..tail as usize]);
        read += tail;
    }
    eng.done_bytes.fetch_add(read, Relaxed);
    let d = h.finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&d.as_bytes()[..16]);
    Some(out)
}

fn full_hash(path: &Path, eng: &DupEngine) -> Option<[u8; 32]> {
    let mut f = platform::open_quiet(path).ok()?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        if eng.cancel.load(Relaxed) {
            return None;
        }
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        eng.done_bytes.fetch_add(n as u64, Relaxed);
    }
    Some(*h.finalize().as_bytes())
}

/// Sammelt die Kandidaten. Braucht nur kurz die Lesesperre; das Hashen laeuft ohne.
fn collect(ix: &Index, p: &DupParams) -> Vec<Cand> {
    let mut by_size: HashMap<u64, Vec<u32>> = HashMap::new();
    for (i, n) in ix.nodes.iter().enumerate() {
        if n.is_dir() || n.is_deleted() || n.flags & (F_LINK | F_HARDLINK | F_CLOUD) != 0 || n.size < p.min_size.max(1) {
            continue;
        }
        by_size.entry(n.size).or_default().push(i as u32);
    }
    let mut out = vec![];
    for (size, ids) in by_size {
        if ids.len() < 2 {
            continue;
        }
        for id in ids {
            let display = ix.path(id);
            if !p.include_system && matches!(guard::check(&display), Guard::Block(_) | Guard::Warn(_)) {
                // System und Programme enthalten absichtlich viele gleiche Dateien.
                continue;
            }
            out.push(Cand { id, size, alloc: ix.nodes[id as usize].alloc, path: platform::long_pathbuf(Path::new(&ix.exact_path(id))) });
        }
    }
    out
}

pub fn run(ix_cands: impl FnOnce(&DupParams) -> Vec<(u32, u64, u64, std::path::PathBuf)>, p: DupParams, eng: &Arc<DupEngine>) {
    let cands: Vec<Cand> = ix_cands(&p).into_iter().map(|(id, size, alloc, path)| Cand { id, size, alloc, path }).collect();
    execute(cands, p, eng);
}

pub fn candidates(ix: &Index, p: &DupParams) -> Vec<(u32, u64, u64, std::path::PathBuf)> {
    collect(ix, p).into_iter().map(|c| (c.id, c.size, c.alloc, c.path)).collect()
}

fn execute(cands: Vec<Cand>, p: DupParams, eng: &Arc<DupEngine>) {
    let pool = rayon::ThreadPoolBuilder::new().num_threads(p.threads.max(1)).build().unwrap();
    pool.install(|| {
        // Stufe 1: Anfang und Ende vergleichen.
        *eng.phase.lock().unwrap() = "partial".into();
        eng.total_files.store(cands.len() as u64, Relaxed);
        eng.total_bytes.store(cands.iter().map(|c| c.size.min(PARTIAL * 2)).sum(), Relaxed);
        let hashed: Vec<(usize, [u8; 16])> = cands
            .par_iter()
            .enumerate()
            .filter_map(|(i, c)| {
                if eng.cancel.load(Relaxed) {
                    return None;
                }
                let r = partial_hash(&c.path, c.size, eng);
                eng.done_files.fetch_add(1, Relaxed);
                if r.is_none() {
                    eng.skipped.fetch_add(1, Relaxed);
                }
                r.map(|h| (i, h))
            })
            .collect();
        let mut groups: HashMap<(u64, [u8; 16]), Vec<usize>> = HashMap::new();
        for (i, h) in hashed {
            groups.entry((cands[i].size, h)).or_default().push(i);
        }
        let mut partial: Vec<Vec<usize>> = groups.into_values().filter(|g| g.len() >= 2).collect();
        partial.sort_by_key(|g| std::cmp::Reverse(cands[g[0]].size * (g.len() as u64 - 1)));

        let publish = |g: &[usize], verified: bool| {
            let first = &cands[g[0]];
            eng.groups.lock().unwrap().push(DupGroup {
                size: first.size,
                alloc: first.alloc,
                ids: g.iter().map(|i| cands[*i].id).collect(),
                verified,
            });
        };

        // Kleine Dateien wurden in Stufe 1 schon komplett gelesen.
        let (small, large): (Vec<_>, Vec<_>) = partial.into_iter().partition(|g| cands[g[0]].size <= PARTIAL * 2);
        for g in &small {
            publish(g, true);
        }

        if p.quick {
            for g in &large {
                publish(g, false);
            }
        } else if !eng.cancel.load(Relaxed) {
            // Stufe 2: Inhalt komplett vergleichen. Groesste Verschwendung zuerst.
            *eng.phase.lock().unwrap() = "full".into();
            eng.done_bytes.store(0, Relaxed);
            eng.done_files.store(0, Relaxed);
            let total: u64 = large.iter().map(|g| cands[g[0]].size * g.len() as u64).sum();
            eng.total_bytes.store(total, Relaxed);
            eng.total_files.store(large.iter().map(|g| g.len() as u64).sum(), Relaxed);
            large.par_iter().for_each(|g| {
                let mut by_hash: HashMap<[u8; 32], Vec<usize>> = HashMap::new();
                for &i in g {
                    if eng.cancel.load(Relaxed) {
                        return;
                    }
                    match full_hash(&cands[i].path, eng) {
                        Some(h) => by_hash.entry(h).or_default().push(i),
                        None => {
                            eng.skipped.fetch_add(1, Relaxed);
                        }
                    }
                    eng.done_files.fetch_add(1, Relaxed);
                }
                for sub in by_hash.values() {
                    if sub.len() >= 2 {
                        publish(sub, true);
                    }
                }
            });
        }
    });
    eng.groups.lock().unwrap().sort_by_key(|g| std::cmp::Reverse(g.wasted()));
}

#[derive(Serialize, Clone)]
pub struct DupFolderGroup {
    pub alloc: u64,
    pub files: u32,
    pub ids: Vec<u32>,
}

#[inline]
fn fnv(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in bytes {
        h ^= b.to_ascii_lowercase() as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[inline]
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Doppelte Ordner: gleiche Namen, gleiche Dateigroessen, gleiche Struktur.
/// Der Inhalt wird hier nicht gelesen. Die Oberflaeche sagt das auch so.
pub fn duplicate_folders(ix: &Index, min_alloc: u64, include_system: bool) -> Vec<DupFolderGroup> {
    let n = ix.nodes.len();
    let mut sig = vec![0u64; n];
    for i in (0..n).rev() {
        let node = &ix.nodes[i];
        if !node.is_dir() || node.is_deleted() {
            continue;
        }
        let mut acc = 0u64;
        let mut count = 0u64;
        for c in node.first_child..node.first_child + node.child_count {
            let ch = &ix.nodes[c as usize];
            if ch.is_deleted() {
                continue;
            }
            count += 1;
            let name = fnv(ix.name_bytes(c));
            let val = if ch.is_dir() { sig[c as usize] ^ 0x5555 } else { ch.size };
            // Addition, damit die Reihenfolge der Kinder keine Rolle spielt.
            acc = acc.wrapping_add(mix(name ^ mix(val)));
        }
        sig[i] = mix(acc ^ count.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    }
    let mut groups: HashMap<(u64, u64), Vec<u32>> = HashMap::new();
    for (i, node) in ix.nodes.iter().enumerate().skip(1) {
        if node.is_dir() && !node.is_deleted() && node.alloc >= min_alloc && node.files >= 2 {
            groups.entry((sig[i], node.alloc)).or_default().push(i as u32);
        }
    }
    let mut member: HashMap<u32, (u64, u64)> = HashMap::new();
    for (key, ids) in &groups {
        if ids.len() >= 2 {
            for id in ids {
                member.insert(*id, *key);
            }
        }
    }
    let mut out = vec![];
    for (key, ids) in &groups {
        if ids.len() < 2 {
            continue;
        }
        // Ist auch der Elternordner jedes Mitglieds doppelt, zeigen wir nur die oberste Ebene.
        let nested = ids.iter().all(|id| {
            let p = ix.nodes[*id as usize].parent;
            member.contains_key(&p)
        }) && {
            let parents: std::collections::HashSet<_> = ids.iter().map(|id| member.get(&ix.nodes[*id as usize].parent)).collect();
            parents.len() == 1
        };
        if nested {
            continue;
        }
        let keep: Vec<u32> = if include_system {
            ids.clone()
        } else {
            ids.iter().copied().filter(|id| !matches!(guard::check(&ix.path(*id)), Guard::Block(_) | Guard::Warn(_))).collect()
        };
        if keep.len() < 2 {
            continue;
        }
        let first = &ix.nodes[keep[0] as usize];
        out.push(DupFolderGroup { alloc: key.1, files: first.files, ids: keep });
    }
    out.sort_by_key(|g| std::cmp::Reverse(g.alloc * (g.ids.len() as u64 - 1)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::testutil::build;
    use crate::scan;
    use std::fs;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("smap-dup-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn index_of(d: &Path) -> Index {
        let r = d.to_string_lossy().to_string();
        let prog = scan::ScanProgress::new(&r, 0, 2, "ssd");
        let mut ix = scan::scan(&r, &prog).unwrap();
        scan::finalize(&mut ix);
        ix
    }

    fn run_on(ix: &Index, quick: bool) -> Vec<DupGroup> {
        let eng = Arc::new(DupEngine::default());
        let params = DupParams { min_size: 1, quick, include_system: true, threads: 2 };
        run(|p| candidates(ix, p), params, &eng);
        let g = eng.groups.lock().unwrap().clone();
        g
    }

    fn names(ix: &Index, g: &DupGroup) -> Vec<String> {
        let mut v: Vec<String> = g.ids.iter().map(|i| ix.name(*i).to_string()).collect();
        v.sort();
        v
    }

    #[test]
    fn finds_real_duplicates_only() {
        let d = tmp("real");
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(d.join("a.bin"), &big).unwrap();
        fs::write(d.join("b-kopie.bin"), &big).unwrap();
        // Gleiche Groesse, gleicher Anfang, gleiches Ende, aber in der Mitte anders.
        let mut mid = big.clone();
        mid[150_000] ^= 0xFF;
        fs::write(d.join("c-anders.bin"), &mid).unwrap();
        fs::write(d.join("klein1.txt"), b"hallo welt").unwrap();
        fs::write(d.join("klein2.txt"), b"hallo welt").unwrap();
        fs::write(d.join("klein3.txt"), b"hallo mond").unwrap();
        let ix = index_of(&d);
        let g = run_on(&ix, false);
        assert_eq!(g.len(), 2, "{g:?}");
        let big_group = g.iter().find(|x| x.size == 300_000).unwrap();
        assert_eq!(names(&ix, big_group), vec!["a.bin", "b-kopie.bin"]);
        assert!(big_group.verified);
        let small = g.iter().find(|x| x.size == 10).unwrap();
        assert_eq!(names(&ix, small), vec!["klein1.txt", "klein2.txt"]);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn quick_mode_marks_groups_as_unverified() {
        let d = tmp("quick");
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let mut mid = big.clone();
        mid[150_000] ^= 0xFF;
        fs::write(d.join("a.bin"), &big).unwrap();
        fs::write(d.join("b.bin"), &mid).unwrap();
        let ix = index_of(&d);
        let g = run_on(&ix, true);
        assert_eq!(g.len(), 1);
        assert!(!g[0].verified, "Schnellmodus darf nichts als geprueft ausgeben");
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn hardlinks_are_not_duplicates() {
        let d = tmp("hard");
        fs::write(d.join("x.bin"), vec![3u8; 9000]).unwrap();
        fs::hard_link(d.join("x.bin"), d.join("y.bin")).unwrap();
        let ix = index_of(&d);
        assert!(run_on(&ix, false).is_empty());
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn duplicate_scan_does_not_change_last_access() {
        let d = tmp("atime");
        let f = d.join("a.bin");
        fs::write(&f, vec![1u8; 200_000]).unwrap();
        fs::write(d.join("b.bin"), vec![1u8; 200_000]).unwrap();
        let before = fs::metadata(&f).unwrap().accessed().unwrap();
        let ix = index_of(&d);
        run_on(&ix, false);
        assert_eq!(fs::metadata(&f).unwrap().accessed().unwrap(), before);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn finds_duplicate_folders_top_level_only() {
        let ix = build(
            r"D:\",
            &[
                (r"Backup1\Fotos\a.jpg", 5000),
                (r"Backup1\Fotos\b.jpg", 7000),
                (r"Backup1\info.txt", 10),
                (r"Backup2\Fotos\b.jpg", 7000),
                (r"Backup2\Fotos\a.jpg", 5000),
                (r"Backup2\info.txt", 10),
                (r"Anders\Fotos\a.jpg", 5000),
                (r"Anders\Fotos\b.jpg", 7001),
            ],
        );
        let g = duplicate_folders(&ix, 100, true);
        assert_eq!(g.len(), 1, "nur die oberste Ebene zeigen");
        let mut paths: Vec<String> = g[0].ids.iter().map(|i| ix.path(*i)).collect();
        paths.sort();
        assert_eq!(paths, vec![r"D:\Backup1", r"D:\Backup2"]);
    }
}
