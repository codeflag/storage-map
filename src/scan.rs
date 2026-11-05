//! Paralleler Laufwerks-Scan. Das Lesen der Verzeichnisse uebernimmt die Plattform-Schicht:
//! Sie liefert in einem Rutsch Groesse, belegten Platz, Zeiten und Datei-ID.

use crate::category;
use crate::i18n::Text;
use crate::index::*;
use crate::platform::{self, DirPath, RawEntry};
use rayon::prelude::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Condvar, Mutex};
use std::time::Instant;

pub struct Top {
    pub name: String,
    pub bytes: u64,
    pub files: u64,
}

pub struct ScanProgress {
    pub root: String,
    pub started: Instant,
    pub cancel: AtomicBool,
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    pub expected: u64,
    pub threads: usize,
    pub media: String,
    pub current: Mutex<String>,
    pub phase: Mutex<String>,
    pub tops: Mutex<Vec<Top>>,
}

impl ScanProgress {
    pub fn new(root: &str, expected: u64, threads: usize, media: &str) -> Self {
        ScanProgress {
            root: root.to_string(),
            started: Instant::now(),
            cancel: AtomicBool::new(false),
            files: AtomicU64::new(0),
            dirs: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            expected,
            threads,
            media: media.to_string(),
            current: Mutex::new(String::new()),
            phase: Mutex::new("scan".into()),
            tops: Mutex::new(vec![]),
        }
    }
}

struct Task {
    node: u32,
    top: u32,
    path: DirPath,
}

struct Queue {
    tasks: Vec<Task>,
    active: usize,
}

struct Arena {
    nodes: Vec<Node>,
    names: Vec<u8>,
    denied: Vec<Denied>,
}

const MAX_DENIED: usize = 20_000;
const MAX_NAME_BYTES: usize = u32::MAX as usize - (1 << 20);

/// Sinnvolle Thread-Zahl je Datentraeger. Eine HDD wird durch zu viele parallele Zugriffe langsamer.
pub fn threads_for(media: &str) -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    match media {
        "hdd" => 4,
        "net" => 8.min(cores),
        "ssd" => cores.min(16),
        _ => cores.min(8),
    }
}

/// Fuehrt den Scan aus. `None`, wenn er abgebrochen wurde oder die Wurzel nicht lesbar ist.
pub fn scan(root: &str, progress: &ScanProgress) -> Result<Index, Text> {
    let root_path = platform::root_dir_path(root);

    let arena = Mutex::new(Arena { nodes: Vec::with_capacity(1 << 20), names: Vec::with_capacity(1 << 24), denied: vec![] });
    {
        let mut a = arena.lock().unwrap();
        a.names.extend_from_slice(root.as_bytes());
        a.nodes.push(Node { parent: NONE, name_len: root.len() as u16, flags: F_DIR, ..Default::default() });
    }
    let queue = Mutex::new(Queue { tasks: vec![Task { node: 0, top: NONE, path: root_path }], active: 0 });
    let cv = Condvar::new();
    let fatal: Mutex<Option<Text>> = Mutex::new(None);

    std::thread::scope(|s| {
        for _ in 0..progress.threads.max(1) {
            s.spawn(|| worker(&queue, &cv, &arena, progress, &fatal));
        }
    });

    if let Some(e) = fatal.into_inner().unwrap() {
        return Err(e);
    }
    if progress.cancel.load(Relaxed) {
        return Err(Text::new("Cancelled", "Abgebrochen"));
    }
    let a = arena.into_inner().unwrap();
    let mut ix = Index { nodes: a.nodes, names: a.names, meta: Meta::default() };
    ix.meta.root = root.to_string();
    ix.meta.denied = a.denied;
    ix.meta.is_drive = platform::is_volume_root(root);
    Ok(ix)
}

fn worker(queue: &Mutex<Queue>, cv: &Condvar, arena: &Mutex<Arena>, progress: &ScanProgress, fatal: &Mutex<Option<Text>>) {
    let mut lister = platform::Lister::new();
    let mut entries: Vec<RawEntry> = Vec::with_capacity(1024);
    let mut names: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut local_tick = 0u32;
    loop {
        let task = {
            let mut q = queue.lock().unwrap();
            loop {
                if progress.cancel.load(Relaxed) {
                    q.tasks.clear();
                }
                if let Some(t) = q.tasks.pop() {
                    q.active += 1;
                    break Some(t);
                }
                if q.active == 0 {
                    cv.notify_all();
                    break None;
                }
                q = cv.wait(q).unwrap();
            }
        };
        let Some(task) = task else { return };

        entries.clear();
        names.clear();
        let res = lister.list(&task.path, &mut entries, &mut names);

        local_tick = local_tick.wrapping_add(1);
        if local_tick % 32 == 1 {
            if let Ok(mut cur) = progress.current.try_lock() {
                *cur = platform::display_dir_path(&task.path);
            }
        }

        let mut new_tasks: Vec<Task> = vec![];
        let (mut sum_alloc, mut sum_files) = (0u64, 0u64);
        {
            let mut a = arena.lock().unwrap();
            if let Err(code) = res {
                progress.errors.fetch_add(1, Relaxed);
                a.nodes[task.node as usize].flags |= F_DENIED;
                if a.denied.len() < MAX_DENIED {
                    a.denied.push(Denied { path: platform::display_dir_path(&task.path), code });
                }
            }
            if a.names.len() + names.len() > MAX_NAME_BYTES {
                *fatal.lock().unwrap() = Some(Text::new("Too many files for one scan", "Zu viele Dateien für einen Scan"));
                progress.cancel.store(true, Relaxed);
            }
            let base = a.nodes.len() as u32;
            let names_base = a.names.len() as u32;
            a.names.extend_from_slice(&names);
            a.nodes[task.node as usize].first_child = base;
            a.nodes[task.node as usize].child_count = entries.len() as u32;
            for (i, e) in entries.iter().enumerate() {
                let mut flags = 0u16;
                for (on, flag) in [
                    (e.is_dir, F_DIR),
                    (e.hidden, F_HIDDEN),
                    (e.system, F_SYSTEM),
                    (e.compressed, F_COMPRESSED),
                    (e.link, F_LINK),
                    (e.mount, F_MOUNT),
                    (e.cloud, F_CLOUD),
                ] {
                    if on {
                        flags |= flag;
                    }
                }
                let (alloc, size) = if e.is_dir || e.link { (0, 0) } else { (e.alloc, e.size) };
                a.nodes.push(Node {
                    alloc,
                    size,
                    file_id: e.file_id,
                    parent: task.node,
                    mtime: e.mtime,
                    atime: e.atime,
                    ctime: e.ctime,
                    newest: if e.is_dir { 0 } else { e.mtime },
                    avg_mtime: e.mtime,
                    name_off: names_base + e.name_off,
                    name_len: e.name_len,
                    flags,
                    ..Default::default()
                });
                if !e.is_dir {
                    sum_alloc += alloc;
                    sum_files += 1;
                } else if !e.link && !e.mount {
                    let name = &names[e.name_off as usize..(e.name_off as usize + e.name_len as usize)];
                    new_tasks.push(Task { node: base + i as u32, top: task.top, path: platform::child_dir_path(&task.path, name) });
                }
            }
        }

        // Live-Balken je Ordner der obersten Ebene.
        {
            let mut tops = progress.tops.lock().unwrap();
            if task.top == NONE {
                for t in new_tasks.iter_mut() {
                    let name = platform::display_dir_path(&t.path);
                    let short = name.rsplit(['\\', '/']).next().unwrap_or("").to_string();
                    tops.push(Top { name: short, bytes: 0, files: 0 });
                    t.top = (tops.len() - 1) as u32;
                }
                tops.push(Top { name: "(Dateien direkt hier)".into(), bytes: sum_alloc, files: sum_files });
            } else if let Some(t) = tops.get_mut(task.top as usize) {
                t.bytes += sum_alloc;
                t.files += sum_files;
            }
        }

        progress.files.fetch_add(sum_files, Relaxed);
        progress.dirs.fetch_add(new_tasks.len() as u64, Relaxed);
        progress.bytes.fetch_add(sum_alloc, Relaxed);

        {
            let mut q = queue.lock().unwrap();
            q.active -= 1;
            let added = !new_tasks.is_empty();
            q.tasks.append(&mut new_tasks);
            if added || q.active == 0 {
                cv.notify_all();
            }
        }
    }
}

/// Fuellt Endungen und Kategorien, erkennt Hardlinks, summiert alles von unten nach oben.
pub fn finalize(ix: &mut Index) {
    classify_all(ix);
    mark_hardlinks(ix);
    aggregate(ix);
}

fn classify_all(ix: &mut Index) {
    let mut map: HashMap<Vec<u8>, u16> = HashMap::new();
    let mut exts = vec![String::new()];
    let mut cats = vec![category::OTHER];
    let names = std::mem::take(&mut ix.names);
    let mut lower = [0u8; 16];
    for n in ix.nodes.iter_mut() {
        if n.is_dir() {
            continue;
        }
        n.cat = category::OTHER;
        let name = &names[n.name_off as usize..n.name_off as usize + n.name_len as usize];
        let Some(dot) = name.iter().rposition(|b| *b == b'.') else { continue };
        let e = &name[dot + 1..];
        if e.is_empty() || e.len() > 12 || dot == 0 {
            continue;
        }
        if !e.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_' || *b == b'!') {
            continue;
        }
        for (i, b) in e.iter().enumerate() {
            lower[i] = b.to_ascii_lowercase();
        }
        let key = &lower[..e.len()];
        let id = match map.get(key) {
            Some(id) => *id,
            None => {
                if exts.len() >= 60_000 {
                    continue;
                }
                let s = String::from_utf8_lossy(key).to_string();
                let id = exts.len() as u16;
                cats.push(category::classify(&s));
                exts.push(s);
                map.insert(key.to_vec(), id);
                id
            }
        };
        n.ext = id;
        n.cat = cats[id as usize];
        if key == b"ts" {
            n.cat = category::classify_sized("ts", n.size);
        }
    }
    ix.names = names;
    ix.meta.exts = exts;
}

/// Mehrere Namen einer Datei (Hardlinks, z. B. in WinSxS) belegen den Platz nur einmal.
/// Die Datei-ID allein reicht nicht: Manche Netzlaufwerke liefern keine eindeutigen IDs,
/// darum muessen auch Groesse und Aenderungszeit passen.
fn mark_hardlinks(ix: &mut Index) {
    let mut v: Vec<(u64, u64, u32, u32)> = ix
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| !n.is_dir() && n.file_id != 0 && n.flags & F_LINK == 0)
        .map(|(i, n)| (n.file_id, n.size, n.mtime, i as u32))
        .collect();
    v.par_sort_unstable();
    let mut i = 0;
    while i < v.len() {
        let mut j = i + 1;
        while j < v.len() && v[j].0 == v[i].0 && v[j].1 == v[i].1 && v[j].2 == v[i].2 {
            j += 1;
        }
        if j - i > 1 {
            // Immer der alphabetisch erste Pfad behaelt die Groesse. So bleiben Ordnergroessen von Scan zu Scan vergleichbar.
            let primary = v[i..j].iter().map(|x| x.3).min_by_key(|id| ix.path(*id)).unwrap();
            for x in &v[i..j] {
                if x.3 != primary {
                    let n = &mut ix.nodes[x.3 as usize];
                    n.flags |= F_HARDLINK;
                    n.alloc = 0;
                }
            }
        }
        i = j;
    }
}

/// Summiert von unten nach oben. Funktioniert, weil Eltern immer vor ihren Kindern im Array liegen.
pub fn aggregate(ix: &mut Index) {
    #[derive(Clone, Copy, Default)]
    struct Acc {
        cats: [u64; category::COUNT],
        weighted: f64,
    }
    let n = ix.nodes.len();
    let mut slot = vec![NONE; n];
    let mut count = 0u32;
    for (i, node) in ix.nodes.iter().enumerate() {
        if node.is_dir() {
            slot[i] = count;
            count += 1;
        }
    }
    let mut acc = vec![Acc::default(); count as usize];
    for i in (0..n).rev() {
        let node = ix.nodes[i];
        if node.is_dir() {
            let a = acc[slot[i] as usize];
            let dom = a.cats.iter().enumerate().max_by_key(|(_, v)| **v).map(|(c, _)| c as u8).unwrap_or(category::OTHER);
            let d = &mut ix.nodes[i];
            d.cat = if d.alloc == 0 { category::OTHER } else { dom };
            d.avg_mtime = if d.alloc > 0 { (a.weighted / d.alloc as f64) as u32 } else { d.mtime };
            if d.newest == 0 {
                d.newest = d.mtime;
            }
        }
        if node.parent == NONE {
            continue;
        }
        let p = node.parent as usize;
        let (alloc, size, files, dirs, newest) = {
            let c = &ix.nodes[i];
            if c.is_dir() {
                (c.alloc, c.size, c.files, c.dirs + 1, if c.files > 0 { c.newest } else { 0 })
            } else {
                (c.alloc, c.size, 1, 0, c.mtime)
            }
        };
        let parent = &mut ix.nodes[p];
        parent.alloc += alloc;
        parent.size += size;
        parent.files += files;
        parent.dirs += dirs;
        parent.newest = parent.newest.max(newest);
        let pslot = slot[p] as usize;
        if ix.nodes[i].is_dir() {
            let a = acc[slot[i] as usize];
            for c in 0..category::COUNT {
                acc[pslot].cats[c] += a.cats[c];
            }
            acc[pslot].weighted += a.weighted;
        } else {
            let c = &ix.nodes[i];
            acc[pslot].cats[c.cat as usize] += c.alloc;
            acc[pslot].weighted += c.alloc as f64 * c.mtime as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("smap-scan-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn run(root: &std::path::Path) -> Index {
        let r = root.to_string_lossy().to_string();
        let p = ScanProgress::new(&r, 0, 4, "ssd");
        let mut ix = scan(&r, &p).unwrap();
        finalize(&mut ix);
        ix
    }

    fn find(ix: &Index, name: &str) -> u32 {
        (0..ix.nodes.len() as u32).find(|i| ix.name(*i) == name).expect(name)
    }

    #[test]
    fn scans_real_tree_with_umlauts_and_sizes() {
        let d = tmp("tree");
        fs::create_dir_all(d.join("Übung").join("tief")).unwrap();
        fs::write(d.join("Übung").join("tief").join("größe.mp4"), vec![1u8; 5000]).unwrap();
        fs::write(d.join("a.txt"), vec![1u8; 100]).unwrap();
        let ix = run(&d);
        assert_eq!(ix.nodes[0].size, 5100);
        assert_eq!(ix.nodes[0].files, 2);
        assert_eq!(ix.nodes[0].dirs, 2);
        let v = find(&ix, "größe.mp4");
        assert_eq!(ix.nodes[v as usize].size, 5000);
        assert_eq!(ix.meta.exts[ix.nodes[v as usize].ext as usize], "mp4");
        assert_eq!(ix.nodes[v as usize].cat, 0);
        assert_eq!(ix.path(v), d.join("Übung").join("tief").join("größe.mp4").to_string_lossy());
        assert!(ix.nodes[v as usize].mtime > 1_500_000_000);
        // Dominante Kategorie des Ordners ist Video.
        let o = find(&ix, "Übung");
        assert_eq!(ix.nodes[o as usize].cat, 0);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn hardlinks_are_counted_once() {
        let d = tmp("hard");
        fs::write(d.join("original.bin"), vec![7u8; 8192]).unwrap();
        fs::hard_link(d.join("original.bin"), d.join("zweitname.bin")).unwrap();
        let ix = run(&d);
        // Beide Namen sind da, belegt wird der Platz aber nur einmal.
        assert_eq!(ix.nodes[0].files, 2);
        assert_eq!(ix.nodes[0].alloc, ix.nodes[find(&ix, "original.bin") as usize].alloc);
        assert_eq!(ix.nodes[find(&ix, "zweitname.bin") as usize].alloc, 0);
        assert!(ix.nodes[find(&ix, "zweitname.bin") as usize].flags & F_HARDLINK != 0);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn long_paths_are_scanned() {
        let d = tmp("long");
        let mut p = platform::long_pathbuf(&d);
        for _ in 0..14 {
            p.push("ein_sehr_langer_ordnername_zum_testen");
        }
        fs::create_dir_all(&p).unwrap();
        fs::write(p.join("x.bin"), vec![1u8; 10]).unwrap();
        let ix = run(&d);
        assert_eq!(ix.nodes[0].size, 10);
        assert_eq!(ix.nodes[0].dirs, 14);
        fs::remove_dir_all(platform::long_pathbuf(&d)).unwrap();
    }

    #[test]
    fn missing_root_is_reported_as_denied() {
        let d = tmp("missing").join("gibt-es-nicht");
        let r = d.to_string_lossy().to_string();
        let p = ScanProgress::new(&r, 0, 2, "ssd");
        let ix = scan(&r, &p).unwrap();
        assert_eq!(ix.meta.denied.len(), 1);
        assert!(ix.nodes[0].flags & F_DENIED != 0);
    }
}
