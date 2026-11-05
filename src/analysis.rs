//! Auswertungen fuer Dashboard, Listen und Karte.

use crate::category;
use crate::guard;
use crate::i18n::{Lang, Text};
use crate::index::*;
use crate::junk::{self, JunkReport, Risk};
use rayon::prelude::*;
use serde_json::{json, Value};
use std::collections::BinaryHeap;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

const DAY: u64 = 86_400;
const YEAR: u64 = 365 * DAY;

pub struct AgeBin {
    pub label: Text,
    /// Kurzform fuer die Achse eines Diagramms.
    pub short: Text,
    pub limit: u64,
}

const fn age_bin(en: &'static str, de: &'static str, short_en: &'static str, short_de: &'static str, limit: u64) -> AgeBin {
    AgeBin { label: Text::new(en, de), short: Text::new(short_en, short_de), limit }
}

pub static AGE_BINS: [AgeBin; 8] = [
    age_bin("< 1 week", "< 1 Woche", "<1w", "<1W", 7 * DAY),
    age_bin("< 1 month", "< 1 Monat", "<1mo", "<1M", 30 * DAY),
    age_bin("< 6 months", "< 6 Monate", "<6mo", "<6M", 182 * DAY),
    age_bin("< 1 year", "< 1 Jahr", "<1y", "<1J", YEAR),
    age_bin("1–2 years", "1–2 Jahre", "1–2y", "1–2J", 2 * YEAR),
    age_bin("2–3 years", "2–3 Jahre", "2–3y", "2–3J", 3 * YEAR),
    age_bin("3–5 years", "3–5 Jahre", "3–5y", "3–5J", 5 * YEAR),
    age_bin("> 5 years", "> 5 Jahre", ">5y", ">5J", u64::MAX),
];

pub const SIZE_BINS: [(&str, u64); 7] = [
    ("< 1 MB", 1 << 20),
    ("1–10 MB", 10 << 20),
    ("10–100 MB", 100 << 20),
    ("100 MB–1 GB", 1 << 30),
    ("1–10 GB", 10u64 << 30),
    ("10–50 GB", 50u64 << 30),
    ("> 50 GB", u64::MAX),
];

pub fn row(ix: &Index, id: u32, lang: Lang) -> Value {
    let n = &ix.nodes[id as usize];
    let path = ix.path(id);
    let guard = guard::check(&path).json(lang);
    json!({
        "id": id,
        "parent": n.parent,
        "name": ix.name(id),
        "path": path,
        "alloc": n.alloc,
        "size": n.size,
        "mtime": n.mtime,
        "atime": n.atime,
        "ctime": n.ctime,
        "newest": n.newest,
        "cat": n.cat,
        "ext": ix.meta.exts.get(n.ext as usize).map(|s| s.as_str()).unwrap_or(""),
        "dir": n.is_dir(),
        "files": n.files,
        "dirs": n.dirs,
        "flags": n.flags,
        "kids": n.child_count,
        "guard": guard,
    })
}

fn light_row(ix: &Index, id: u32) -> Value {
    let n = &ix.nodes[id as usize];
    json!({
        "id": id,
        "name": ix.name(id),
        "alloc": n.alloc,
        "size": n.size,
        "mtime": n.mtime,
        "atime": n.atime,
        "newest": n.newest,
        "cat": n.cat,
        "dir": n.is_dir(),
        "files": n.files,
        "dirs": n.dirs,
        "flags": n.flags,
        "kids": n.child_count,
    })
}

pub fn children(ix: &Index, id: u32, sort: &str, desc: bool, offset: usize, limit: usize) -> Value {
    let n = &ix.nodes[id as usize];
    let mut v: Vec<u32> = (n.first_child..n.first_child + n.child_count).filter(|c| !ix.nodes[*c as usize].is_deleted()).collect();
    sort_ids(ix, &mut v, sort, desc);
    let total = v.len();
    let rows: Vec<Value> = v.iter().skip(offset).take(limit).map(|c| light_row(ix, *c)).collect();
    json!({"total": total, "rows": rows, "parentAlloc": n.alloc})
}

fn sort_ids(ix: &Index, v: &mut [u32], sort: &str, desc: bool) {
    let nodes = &ix.nodes;
    let cmp = |a: &u32, b: &u32| -> std::cmp::Ordering {
        let (x, y) = (&nodes[*a as usize], &nodes[*b as usize]);
        let o = match sort {
            "size" => x.size.cmp(&y.size),
            "name" => ix.name(*a).to_lowercase().cmp(&ix.name(*b).to_lowercase()),
            "mtime" => x.mtime.cmp(&y.mtime),
            "atime" => x.atime.cmp(&y.atime),
            "ctime" => x.ctime.cmp(&y.ctime),
            "newest" => x.newest.cmp(&y.newest),
            "files" => x.files.cmp(&y.files),
            _ => x.alloc.cmp(&y.alloc),
        };
        let o = if desc { o.reverse() } else { o };
        o.then(a.cmp(b))
    };
    v.par_sort_unstable_by(cmp);
}

/// Daten fuer Karte und Ringe: Die groessten Ordner werden zuerst aufgeklappt, bis das Budget verbraucht ist.
/// Kleinteile eines Ordners werden zu einem Sammelblock zusammengefasst.
pub fn map(ix: &Index, id: u32, limit: usize, minfrac: f64, maxdepth: u32) -> Value {
    let root = &ix.nodes[id as usize];
    let total = root.alloc.max(1);
    let min_alloc = (total as f64 * minfrac) as u64;
    let mut out: Vec<Value> = Vec::with_capacity(limit.min(20_000));
    out.push(json!([id, -1, 1, root.alloc, root.files, root.cat, root.avg_mtime, ix.name(id)]));
    let mut heap: BinaryHeap<(u64, u32, u32, u32)> = BinaryHeap::new();
    heap.push((root.alloc, id, 0, 0));
    while let Some((_, nid, out_idx, depth)) = heap.pop() {
        if out.len() >= limit {
            break;
        }
        let (mut rest_alloc, mut rest_count) = (0u64, 0u32);
        let mut rest_cat = [0u64; category::COUNT];
        let kids = ix.children_by_size(nid);
        for c in kids {
            let ch = &ix.nodes[c as usize];
            if ch.alloc > 0 && ch.alloc >= min_alloc && out.len() < limit {
                let dir = ch.is_dir();
                let idx = out.len() as u32;
                out.push(json!([c, out_idx, if dir { 1 } else { 0 }, ch.alloc, if dir { ch.files } else { 1 }, ch.cat, ch.avg_mtime, ix.name(c)]));
                if dir && ch.child_count > 0 && depth + 1 < maxdepth {
                    heap.push((ch.alloc, c, idx, depth + 1));
                }
            } else {
                rest_alloc += ch.alloc;
                rest_count += if ch.is_dir() { ch.files.max(1) } else { 1 };
                rest_cat[ch.cat as usize] += ch.alloc;
            }
        }
        if rest_alloc > 0 {
            let cat = rest_cat.iter().enumerate().max_by_key(|(_, v)| **v).map(|(i, _)| i).unwrap_or(8);
            out.push(json!([nid, out_idx, 2, rest_alloc, rest_count, cat, 0, ""]));
        }
    }
    let unaccounted = if id == 0 && ix.meta.is_drive { ix.meta.unaccounted } else { 0 };
    json!({
        "id": id,
        "ancestors": ix.ancestors(id).iter().map(|a| json!({"id": a, "name": ix.name(*a)})).collect::<Vec<_>>(),
        "alloc": root.alloc,
        "unaccounted": unaccounted,
        "free": if id == 0 && ix.meta.is_drive { ix.meta.volume.free } else { 0 },
        "nodes": out,
    })
}

/// Detailansicht eines Eintrags, inklusive "Hauptverursacher": dem Pfad, dem man folgen muss,
/// um den groessten Teil des Platzes zu finden.
pub fn node_detail(ix: &Index, id: u32, lang: Lang) -> Value {
    let mut base = row(ix, id, lang);
    let n = &ix.nodes[id as usize];
    let ancestors: Vec<Value> =
        ix.ancestors(id).iter().map(|a| json!({"id": a, "name": ix.name(*a), "alloc": ix.nodes[*a as usize].alloc})).collect();
    let mut heavy = vec![];
    if n.is_dir() {
        let mut cur = id;
        for _ in 0..12 {
            let kids = ix.children_by_size(cur);
            let Some(top) = kids.first().copied() else { break };
            let parent_alloc = ix.nodes[cur as usize].alloc.max(1);
            let share = ix.nodes[top as usize].alloc as f64 / parent_alloc as f64;
            if share < 0.5 || ix.nodes[top as usize].alloc == 0 {
                break;
            }
            heavy.push(json!({"id": top, "name": ix.name(top), "alloc": ix.nodes[top as usize].alloc, "share": share}));
            if !ix.nodes[top as usize].is_dir() {
                break;
            }
            cur = top;
        }
    }
    let top: Vec<Value> = if n.is_dir() { ix.children_by_size(id).iter().take(8).map(|c| light_row(ix, *c)).collect() } else { vec![] };
    let mut cats = vec![0u64; category::COUNT];
    if n.is_dir() && n.files < 2_000_000 {
        for c in ix.subtree(id) {
            let x = &ix.nodes[c as usize];
            if !x.is_dir() && !x.is_deleted() {
                cats[x.cat as usize] += x.alloc;
            }
        }
    }
    base["ancestors"] = json!(ancestors);
    base["heavy"] = json!(heavy);
    base["top"] = json!(top);
    base["cats"] = json!(cats);
    base
}

/// Ordner, die zusammen das meiste erklaeren. Geht in einen Ordner hinein, solange
/// ein einzelner Unterordner einen grossen Teil ausmacht; sonst ist der Ordner selbst der Treffer.
pub fn hotspots(ix: &Index, count: usize) -> Vec<u32> {
    let mut out = vec![];
    let mut heap: BinaryHeap<(u64, u32)> = BinaryHeap::new();
    heap.push((ix.nodes[0].alloc, 0));
    let total = ix.nodes[0].alloc.max(1);
    while let Some((alloc, id)) = heap.pop() {
        if out.len() >= count || (alloc as f64) < total as f64 * 0.001 {
            break;
        }
        let kids = ix.children_by_size(id);
        let dir_sum: u64 = kids.iter().filter(|k| ix.nodes[**k as usize].is_dir()).map(|k| ix.nodes[*k as usize].alloc).sum();
        let biggest = kids.iter().filter(|k| ix.nodes[**k as usize].is_dir()).map(|k| ix.nodes[*k as usize].alloc).max().unwrap_or(0);
        let expand = id == 0 || (biggest as f64 >= alloc as f64 * 0.3 && dir_sum as f64 >= alloc as f64 * 0.5);
        if expand {
            for k in kids {
                if ix.nodes[k as usize].is_dir() {
                    heap.push((ix.nodes[k as usize].alloc, k));
                }
            }
        } else {
            out.push(id);
        }
    }
    out
}

pub fn summary(ix: &Index, junk: &JunkReport, dup_wasted: Option<u64>, dup_verified: bool, lang: Lang) -> Value {
    let now = now_secs();
    let mut cats = [(0u64, 0u64); category::COUNT];
    let mut ages = vec![(0u64, 0u64); AGE_BINS.len()];
    let mut acc_ages = vec![(0u64, 0u64); AGE_BINS.len()];
    let mut sizes = vec![(0u64, 0u64); SIZE_BINS.len()];
    let mut years: std::collections::BTreeMap<i32, (u64, u64)> = Default::default();
    let mut exts = vec![(0u64, 0u64); ix.meta.exts.len()];
    let mut stale_2y = 0u64;
    let mut stale_5y = 0u64;
    let mut top_files: BinaryHeap<std::cmp::Reverse<(u64, u32)>> = BinaryHeap::new();
    let mut hidden = 0u64;
    let mut hardlinks = 0u64;
    let mut cloud = (0u64, 0u64);

    for (i, n) in ix.nodes.iter().enumerate() {
        if n.is_dir() || n.is_deleted() {
            continue;
        }
        if n.flags & F_HARDLINK != 0 {
            hardlinks += 1;
        }
        if n.flags & F_CLOUD != 0 {
            cloud.0 += 1;
            cloud.1 += n.size;
        }
        if n.flags & F_HIDDEN != 0 {
            hidden += n.alloc;
        }
        let c = &mut cats[n.cat as usize];
        c.0 += n.alloc;
        c.1 += 1;
        let e = &mut exts[n.ext as usize];
        e.0 += n.alloc;
        e.1 += 1;

        let age = now.saturating_sub(n.mtime as u64);
        let b = AGE_BINS.iter().position(|bin| age < bin.limit).unwrap_or(AGE_BINS.len() - 1);
        ages[b].0 += n.alloc;
        ages[b].1 += 1;
        let aage = now.saturating_sub(n.atime as u64);
        let b = AGE_BINS.iter().position(|bin| aage < bin.limit).unwrap_or(AGE_BINS.len() - 1);
        acc_ages[b].0 += n.alloc;
        acc_ages[b].1 += 1;
        if age > 2 * YEAR {
            stale_2y += n.alloc;
        }
        if age > 5 * YEAR {
            stale_5y += n.alloc;
        }
        let b = SIZE_BINS.iter().position(|(_, lim)| n.alloc < *lim).unwrap_or(SIZE_BINS.len() - 1);
        sizes[b].0 += n.alloc;
        sizes[b].1 += 1;
        if n.mtime > 0 {
            let y = 1970 + (n.mtime as u64 / 31_556_952) as i32;
            let e = years.entry(y).or_default();
            e.0 += n.alloc;
            e.1 += 1;
        }
        if n.alloc > 0 {
            if top_files.len() < 15 {
                top_files.push(std::cmp::Reverse((n.alloc, i as u32)));
            } else if top_files.peek().map(|x| x.0 .0 < n.alloc).unwrap_or(false) {
                top_files.pop();
                top_files.push(std::cmp::Reverse((n.alloc, i as u32)));
            }
        }
    }

    let mut files: Vec<(u64, u32)> = top_files.into_iter().map(|x| x.0).collect();
    files.sort_unstable_by(|a, b| b.cmp(a));
    let vol = &ix.meta.volume;
    let used = vol.total.saturating_sub(vol.free);
    let scanned = ix.nodes[0].alloc;
    let mut ext_rows: Vec<(usize, u64, u64)> = exts.iter().enumerate().map(|(i, e)| (i, e.0, e.1)).collect();
    ext_rows.sort_unstable_by_key(|e| std::cmp::Reverse(e.1));

    // Empfehlungen: rein regelbasiert, groesster Hebel zuerst.
    let mut recs: Vec<Value> = vec![];
    for (rule, res) in junk::rules().iter().zip(&junk.results) {
        if res.alloc < 50 * 1024 * 1024 {
            continue;
        }
        recs.push(json!({
            "kind": "rule",
            "rule": rule.id,
            "title": rule.title.get(lang),
            "why": rule.why.get(lang),
            "risk": rule.risk,
            "group": rule.group.get(lang),
            "bytes": res.alloc,
            "items": res.items.len(),
        }));
    }
    if let Some(w) = dup_wasted {
        if w > 0 {
            let why = if dup_verified {
                Text::new(
                    "Same content, stored more than once. One copy of each group is enough.",
                    "Gleicher Inhalt, mehrfach gespeichert. Von jeder Gruppe genügt eine Kopie.",
                )
            } else {
                Text::new("Very likely the same content (quick check).", "Sehr wahrscheinlich gleicher Inhalt (Schnellprüfung).")
            };
            recs.push(json!({
                "kind": "dups", "title": Text::new("Duplicate files", "Doppelte Dateien").get(lang),
                "why": why.get(lang),
                "risk": Risk::Review, "bytes": w, "items": 0,
            }));
        }
    }
    if stale_5y > 1 << 30 {
        recs.push(json!({
            "kind": "stale", "title": Text::new("Files unchanged for over 5 years", "Dateien, seit über 5 Jahren unverändert").get(lang),
            "why": Text::new("Not touched for a long time. Candidates for the archive or for deleting.", "Lange nicht angefasst. Kandidaten fürs Archiv oder zum Löschen.").get(lang),
            "risk": Risk::Review, "bytes": stale_5y, "items": 0,
        }));
    }
    recs.sort_by_key(|r| {
        let info = r["risk"] == "info";
        (info, std::cmp::Reverse(r["bytes"].as_u64().unwrap_or(0)))
    });

    let hot: Vec<Value> = hotspots(ix, 14).iter().map(|h| row(ix, *h, lang)).collect();
    let top_dirs: Vec<Value> = ix.children_by_size(0).iter().take(12).map(|c| light_row(ix, *c)).collect();

    json!({
        "root": ix.meta.root,
        "isDrive": ix.meta.is_drive,
        "scannedAt": ix.meta.scanned_at,
        "durationMs": ix.meta.duration_ms,
        "elevatedScan": ix.meta.elevated,
        "volume": vol,
        "used": used,
        "scanned": scanned,
        "logical": ix.nodes[0].size,
        "unaccounted": if ix.meta.is_drive { ix.meta.unaccounted } else { 0 },
        "files": ix.nodes[0].files,
        "dirs": ix.nodes[0].dirs,
        "denied": ix.meta.denied.len(),
        "hidden": hidden,
        "hardlinks": hardlinks,
        "cloud": {"count": cloud.0, "bytes": cloud.1},
        "stale2y": stale_2y,
        "stale5y": stale_5y,
        "safeBytes": junk.safe_bytes(),
        "dupWasted": dup_wasted,
        "emptyDirs": junk.empty_dirs.len(),
        "cats": category::names(lang).iter().enumerate().map(|(i, n)| json!({"id": i, "name": n, "alloc": cats[i].0, "files": cats[i].1})).collect::<Vec<_>>(),
        "ages": AGE_BINS.iter().enumerate().map(|(i, b)| json!({"label": b.label.get(lang), "short": b.short.get(lang), "alloc": ages[i].0, "files": ages[i].1})).collect::<Vec<_>>(),
        "accessAges": AGE_BINS.iter().enumerate().map(|(i, b)| json!({"label": b.label.get(lang), "short": b.short.get(lang), "alloc": acc_ages[i].0, "files": acc_ages[i].1})).collect::<Vec<_>>(),
        "sizes": SIZE_BINS.iter().enumerate().map(|(i, (l, _))| json!({"label": l, "alloc": sizes[i].0, "files": sizes[i].1})).collect::<Vec<_>>(),
        "years": years.iter().map(|(y, v)| json!({"year": y, "alloc": v.0, "files": v.1})).collect::<Vec<_>>(),
        "exts": ext_rows.iter().filter(|e| e.1 > 0).take(12).map(|(i, a, c)| json!({"ext": ix.meta.exts[*i], "alloc": a, "files": c})).collect::<Vec<_>>(),
        "hotspots": hot,
        "topDirs": top_dirs,
        "topFiles": files.iter().map(|(_, i)| row(ix, *i, lang)).collect::<Vec<_>>(),
        "recs": recs,
    })
}

pub fn types(ix: &Index) -> Value {
    let mut exts = vec![(0u64, 0u64, 0u64); ix.meta.exts.len()];
    for n in &ix.nodes {
        if !n.is_dir() && !n.is_deleted() {
            let e = &mut exts[n.ext as usize];
            e.0 += n.alloc;
            e.1 += 1;
            e.2 = e.2.max(n.alloc);
        }
    }
    let mut rows: Vec<(usize, (u64, u64, u64))> = exts.into_iter().enumerate().collect();
    rows.sort_unstable_by_key(|r| std::cmp::Reverse(r.1 .0));
    let rows: Vec<Value> = rows
        .iter()
        .filter(|(_, v)| v.1 > 0)
        .take(500)
        .map(|(i, v)| {
            let name = &ix.meta.exts[*i];
            json!({
                "ext": name,
                "cat": if *i == 0 { category::OTHER } else { category::classify(name) },
                "alloc": v.0,
                "files": v.1,
                "largest": v.2,
            })
        })
        .collect();
    json!({"exts": rows, "total": ix.nodes[0].alloc})
}

#[derive(Default, Debug)]
pub struct FileQuery {
    pub under: u32,
    pub q: String,
    pub cats: Vec<u8>,
    pub ext: String,
    pub min: u64,
    pub max: u64,
    pub mod_before: u32,
    pub mod_after: u32,
    pub acc_before: u32,
    pub kind: String,
    pub sort: String,
    pub desc: bool,
    pub offset: usize,
    pub limit: usize,
}

/// Mehrere Woerter muessen alle vorkommen. `*.mp4` filtert nach Endung, ein `\` oder `/` sucht im ganzen Pfad.
struct Terms {
    words: Vec<String>,
    path_words: Vec<String>,
    ext: Option<String>,
}

fn parse_terms(q: &str) -> Terms {
    let mut t = Terms { words: vec![], path_words: vec![], ext: None };
    for raw in q.split_whitespace() {
        let w = raw.to_lowercase();
        if let Some(e) = w.strip_prefix("*.") {
            t.ext = Some(e.to_string());
        } else if w.contains(['\\', '/']) {
            t.path_words.push(w.replace('\\', "/"));
        } else {
            t.words.push(w);
        }
    }
    t
}

pub fn run_query(ix: &Index, q: &FileQuery) -> (Vec<u32>, u64) {
    let terms = parse_terms(&q.q);
    let ext_filter: Option<String> = terms.ext.clone().or_else(|| if q.ext.is_empty() { None } else { Some(q.ext.to_lowercase()) });
    let ext_id: Option<Option<u16>> =
        ext_filter.as_ref().map(|e| if e == "(ohne)" { Some(0) } else { ix.meta.exts.iter().position(|x| x == e).map(|p| p as u16) });
    if matches!(ext_id, Some(None)) {
        return (vec![], 0);
    }
    let ext_id = ext_id.flatten();
    let want_dirs = q.kind == "dir" || q.kind == "all";
    let want_files = q.kind != "dir";

    let test = |i: u32| -> bool {
        let n = &ix.nodes[i as usize];
        if n.is_deleted() || i == 0 {
            return false;
        }
        let dir = n.is_dir();
        if (dir && !want_dirs) || (!dir && !want_files) {
            return false;
        }
        if n.alloc < q.min || (q.max > 0 && n.alloc > q.max) {
            return false;
        }
        if !q.cats.is_empty() && !q.cats.contains(&n.cat) {
            return false;
        }
        if let Some(e) = ext_id {
            if n.ext != e || dir {
                return false;
            }
        }
        let t = if dir { n.newest } else { n.mtime };
        if q.mod_before > 0 && t >= q.mod_before {
            return false;
        }
        if q.mod_after > 0 && t <= q.mod_after {
            return false;
        }
        if q.acc_before > 0 && n.atime >= q.acc_before {
            return false;
        }
        if !terms.words.is_empty() {
            let name = String::from_utf8_lossy(ix.name_bytes(i)).to_lowercase();
            if !terms.words.iter().all(|w| name.contains(w.as_str())) {
                return false;
            }
        }
        if !terms.path_words.is_empty() {
            let p = ix.path(i).to_lowercase().replace('\\', "/");
            if !terms.path_words.iter().all(|w| p.contains(w.as_str())) {
                return false;
            }
        }
        true
    };

    let mut ids: Vec<u32> = if q.under == 0 {
        (0..ix.nodes.len() as u32).into_par_iter().filter(|i| test(*i)).collect()
    } else {
        ix.subtree(q.under).into_par_iter().filter(|i| test(*i)).collect()
    };
    let sum: u64 = ids.par_iter().map(|i| ix.nodes[*i as usize].alloc).sum();
    sort_ids(ix, &mut ids, &q.sort, q.desc);
    (ids, sum)
}

pub fn files_json(ix: &Index, q: &FileQuery, lang: Lang) -> Value {
    let (ids, sum) = run_query(ix, q);
    let rows: Vec<Value> = ids.iter().skip(q.offset).take(q.limit).map(|i| row(ix, *i, lang)).collect();
    json!({"total": ids.len(), "bytes": sum, "rows": rows})
}

/// Vergessene Ordner: In ihnen hat sich seit dem Stichtag nichts mehr geaendert.
/// Nur die oberste Ebene, damit nicht jeder Unterordner einzeln auftaucht.
pub fn stale_dirs(ix: &Index, cutoff: u32, min: u64, limit: usize, lang: Lang) -> Value {
    let mut out: Vec<u32> = (1..ix.nodes.len() as u32)
        .into_par_iter()
        .filter(|i| {
            let n = &ix.nodes[*i as usize];
            if !n.is_dir() || n.is_deleted() || n.alloc < min || n.newest >= cutoff || n.files == 0 {
                return false;
            }
            let p = n.parent;
            p == 0 || p == NONE || ix.nodes[p as usize].newest >= cutoff
        })
        .collect();
    out.par_sort_unstable_by(|a, b| ix.nodes[*b as usize].alloc.cmp(&ix.nodes[*a as usize].alloc).then(a.cmp(b)));
    let total: u64 = out.iter().map(|i| ix.nodes[*i as usize].alloc).sum();
    let count = out.len();
    let rows: Vec<Value> = out.iter().take(limit).map(|i| row(ix, *i, lang)).collect();
    json!({"total": count, "bytes": total, "rows": rows})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::testutil::build;

    fn q() -> FileQuery {
        FileQuery { kind: "file".into(), sort: "alloc".into(), desc: true, limit: 100, ..Default::default() }
    }

    fn sample() -> Index {
        build(
            r"D:\",
            &[
                (r"Filme\gross.mkv", 5000),
                (r"Filme\Urlaub Film.mp4", 3000),
                (r"Fotos\a.jpg", 200),
                (r"Fotos\b.JPG", 100),
                (r"Texte\brief.docx", 10),
                (r"ohneendung", 5),
            ],
        )
    }

    #[test]
    fn query_filters_and_sorts() {
        let ix = sample();
        let (ids, sum) = run_query(&ix, &q());
        assert_eq!(ids.len(), 6);
        assert_eq!(sum, 8315);
        assert_eq!(ix.name(ids[0]), "gross.mkv");

        let mut f = q();
        f.ext = "jpg".into();
        let (ids, sum) = run_query(&ix, &f);
        assert_eq!(ids.len(), 2, "Endungen ohne Rücksicht auf Groß-/Kleinschreibung");
        assert_eq!(sum, 300);

        let mut f = q();
        f.q = "*.mp4".into();
        assert_eq!(run_query(&ix, &f).0.len(), 1);

        let mut f = q();
        f.q = "urlaub film".into();
        assert_eq!(run_query(&ix, &f).0.len(), 1, "alle Wörter müssen vorkommen");

        let mut f = q();
        f.q = r"filme\".into();
        assert_eq!(run_query(&ix, &f).0.len(), 2);

        let mut f = q();
        f.min = 1000;
        f.max = 4000;
        assert_eq!(run_query(&ix, &f).0.len(), 1);

        let mut f = q();
        f.cats = vec![1];
        assert_eq!(run_query(&ix, &f).0.len(), 2);

        let mut f = q();
        f.ext = "(ohne)".into();
        let (ids, _) = run_query(&ix, &f);
        assert_eq!(ix.name(ids[0]), "ohneendung");

        let mut f = q();
        f.ext = "gibtsnicht".into();
        assert!(run_query(&ix, &f).0.is_empty());
    }

    #[test]
    fn query_under_folder_and_folder_kind() {
        let ix = sample();
        let filme = (0..ix.nodes.len() as u32).find(|i| ix.name(*i) == "Filme").unwrap();
        let mut f = q();
        f.under = filme;
        assert_eq!(run_query(&ix, &f).0.len(), 2);
        let mut f = q();
        f.kind = "dir".into();
        let (ids, _) = run_query(&ix, &f);
        assert_eq!(ids.len(), 3);
        assert_eq!(ix.name(ids[0]), "Filme");
    }

    #[test]
    fn map_respects_budget_and_lumps_small_files() {
        let mut entries: Vec<(String, u64)> = vec![(r"Gross\film.mkv".to_string(), 1_000_000)];
        for i in 0..50 {
            entries.push((format!(r"Klein\f{i}.txt"), 10));
        }
        let refs: Vec<(&str, u64)> = entries.iter().map(|(a, b)| (a.as_str(), *b)).collect();
        let ix = build(r"D:\", &refs);
        let m = map(&ix, 0, 100, 0.001, 8);
        let nodes = m["nodes"].as_array().unwrap();
        assert!(nodes.len() < 12, "Kleinteile müssen zusammengefasst werden: {}", nodes.len());
        let sum_top: u64 = nodes.iter().filter(|n| n[1] == 0).map(|n| n[3].as_u64().unwrap()).sum();
        assert_eq!(sum_top, ix.nodes[0].alloc, "Die Fläche muss die Gesamtgröße ergeben");

        let tight = map(&ix, 0, 3, 0.0, 8);
        assert!(tight["nodes"].as_array().unwrap().len() <= 5);
    }

    #[test]
    fn hotspots_descend_into_dominant_folders() {
        let ix = build(
            r"C:\",
            &[(r"Users\Max\Videos\a.mkv", 900), (r"Users\Max\Videos\b.mkv", 800), (r"Users\Max\Videos\c.mkv", 700), (r"Tools\t.exe", 100)],
        );
        let hot = hotspots(&ix, 5);
        let paths: Vec<String> = hot.iter().map(|h| ix.path(*h)).collect();
        assert_eq!(paths[0], r"C:\Users\Max\Videos");
        assert!(!paths.iter().any(|p| p == r"C:\Users"));
    }

    #[test]
    fn stale_dirs_report_only_top_level() {
        let mut ix = build(r"D:\", &[(r"Alt\Unter\x.bin", 100), (r"Alt\y.bin", 100), (r"Neu\z.bin", 100)]);
        // Alt = 2010, Neu = 2030
        for i in 0..ix.nodes.len() as u32 {
            let p = ix.path(i);
            let t = if p.contains("Neu") { 1_900_000_000 } else { 1_262_304_000 };
            let n = &mut ix.nodes[i as usize];
            n.mtime = t;
            n.newest = 0;
        }
        for i in 0..ix.nodes.len() {
            if ix.nodes[i].is_dir() {
                ix.nodes[i].alloc = 0;
                ix.nodes[i].size = 0;
                ix.nodes[i].files = 0;
                ix.nodes[i].dirs = 0;
            }
        }
        crate::scan::aggregate(&mut ix);
        let v = stale_dirs(&ix, 1_500_000_000, 0, 10, Lang::En);
        let rows = v["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{v}");
        assert_eq!(rows[0]["path"], r"D:\Alt");
    }
}
