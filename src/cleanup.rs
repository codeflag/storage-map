//! Aufraeumliste und das eigentliche Entfernen.
//! Jeder Eintrag wird unmittelbar vor dem Entfernen erneut geprueft:
//! Schutzregeln, existiert er noch, hat er sich seit dem Scan veraendert?

use crate::guard::{self, Guard};
use crate::i18n::{Lang, Text};
use crate::index::*;
use crate::{paths, platform, tr};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Default)]
pub struct Cart {
    pub ids: BTreeSet<u32>,
}

pub struct AddResult {
    pub added: usize,
    pub blocked: Vec<(String, Text)>,
}

impl Cart {
    /// Fuegt Eintraege hinzu. Liegt ein Elternordner schon drin, ist das Kind bereits abgedeckt;
    /// kommt ein Elternordner neu dazu, ersetzt er seine Kinder. So wird nichts doppelt gezaehlt.
    pub fn add(&mut self, ix: &Index, ids: &[u32]) -> AddResult {
        let mut res = AddResult { added: 0, blocked: vec![] };
        for &id in ids {
            let Some(n) = ix.nodes.get(id as usize) else { continue };
            if n.is_deleted() || id == 0 {
                continue;
            }
            let path = ix.path(id);
            if n.flags & F_MOUNT != 0 {
                if res.blocked.len() < 20 {
                    res.blocked.push((path, Text::new("Another drive is mounted here.", "Hier ist ein anderes Laufwerk eingehängt.")));
                }
                continue;
            }
            if let Guard::Block(why) = guard::check(&path) {
                if res.blocked.len() < 20 {
                    res.blocked.push((path, why));
                }
                continue;
            }
            let covered = ix.ancestors(id).iter().any(|a| self.ids.contains(a));
            if covered {
                continue;
            }
            if n.is_dir() && !self.ids.is_empty() {
                let inner: Vec<u32> = self.ids.iter().copied().filter(|x| ix.is_under(*x, id)).collect();
                for x in inner {
                    self.ids.remove(&x);
                }
            }
            if self.ids.insert(id) {
                res.added += 1;
            }
        }
        res
    }

    pub fn bytes(&self, ix: &Index) -> u64 {
        self.ids.iter().map(|i| ix.nodes[*i as usize].alloc).sum()
    }
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Recycle,
    Permanent,
    Move,
}

#[derive(Clone, Debug)]
pub struct ItemResult {
    pub path: String,
    pub ok: bool,
    pub bytes: u64,
    pub reason: Text,
}

const NO_REASON: Text = Text::new("", "");

/// Ergebnis eines Laufs. Es bleibt bestehen, bis die Oberflaeche es abgeholt hat, und wird erst dann in deren Sprache ausgegeben.
pub struct Report {
    pub mode: Option<Mode>,
    pub ok: usize,
    pub failed: usize,
    pub freed: u64,
    pub failures: Vec<ItemResult>,
}

impl Report {
    pub fn new(mode: Option<Mode>, results: &[ItemResult]) -> Report {
        Report {
            mode,
            ok: results.iter().filter(|r| r.ok).count(),
            failed: results.iter().filter(|r| !r.ok).count(),
            freed: results.iter().filter(|r| r.ok).map(|r| r.bytes).sum(),
            failures: results.iter().filter(|r| !r.ok).take(200).cloned().collect(),
        }
    }

    pub fn json(&self, lang: Lang) -> Value {
        let failures: Vec<Value> =
            self.failures.iter().map(|f| json!({"path": f.path, "ok": f.ok, "bytes": f.bytes, "reason": f.reason.get(lang)})).collect();
        json!({"mode": self.mode, "ok": self.ok, "failed": self.failed, "freed": self.freed, "failures": failures})
    }
}

/// Snapshot eines Eintrags aus dem Index, damit waehrend der Dateioperation keine Sperre gehalten wird.
#[derive(Clone)]
pub struct Item {
    pub id: u32,
    pub display: String,
    pub exact: PathBuf,
    pub alloc: u64,
    pub size: u64,
    pub mtime: u32,
    pub is_dir: bool,
}

pub fn snapshot(ix: &Index, ids: &[u32]) -> Vec<Item> {
    ids.iter()
        .filter(|i| !ix.nodes[**i as usize].is_deleted())
        .map(|i| {
            let n = &ix.nodes[*i as usize];
            Item {
                id: *i,
                display: ix.path(*i),
                exact: PathBuf::from(ix.exact_path(*i)),
                alloc: n.alloc,
                size: n.size,
                mtime: n.mtime,
                is_dir: n.is_dir(),
            }
        })
        .collect()
}

/// Prueft, ob der Eintrag noch dem entspricht, was der Nutzer gesehen hat.
/// Hat sich eine Datei seit dem Scan veraendert, bleibt sie sicherheitshalber liegen.
pub fn precheck(item: &Item) -> Result<(), Text> {
    if let Guard::Block(why) = guard::check(&item.display) {
        return Err(why);
    }
    let long = platform::long_pathbuf(&item.exact);
    let meta = std::fs::symlink_metadata(&long).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Text::new("No longer there.", "Nicht mehr vorhanden.")
        } else {
            tr!("No access: {e}", "Kein Zugriff: {e}")
        }
    })?;
    if item.is_dir != meta.is_dir() {
        return Err(Text::new("Has changed since the scan. Please scan again.", "Hat sich seit dem Scan verändert. Bitte neu scannen."));
    }
    if item.is_dir && platform::has_mount_inside(&item.exact) {
        return Err(Text::new(
            "Another drive is mounted inside this folder, so it stays in place.",
            "In diesem Ordner ist ein anderes Laufwerk eingehängt. Er bleibt deshalb liegen.",
        ));
    }
    if !item.is_dir {
        let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs() as u32).unwrap_or(0);
        // Die Laenge eines Symlinks ist die seines Ziel-Pfads. Der Scan zaehlt ihn mit 0.
        let size_changed = !meta.file_type().is_symlink() && meta.len() != item.size;
        if size_changed || mtime != item.mtime {
            return Err(Text::new(
                "File has changed since the scan. It stays in place to be safe.",
                "Datei wurde seit dem Scan geändert. Sie bleibt sicherheitshalber liegen.",
            ));
        }
    }
    Ok(())
}

fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(platform::long_pathbuf(p)).is_ok()
}

const BATCH: usize = 64;
const BATCH_CHARS: usize = 24_000;

/// Entfernt in den Papierkorb. Dateien gebuendelt (schneller), Ordner einzeln.
pub fn recycle_all(items: &[Item], mut report: impl FnMut(ItemResult, Option<u32>)) {
    let mut batch: Vec<&Item> = vec![];
    let mut chars = 0usize;
    let flush = |batch: &mut Vec<&Item>, report: &mut dyn FnMut(ItemResult, Option<u32>)| {
        if batch.is_empty() {
            return;
        }
        let paths: Vec<PathBuf> = batch.iter().map(|i| i.exact.clone()).collect();
        let rc = platform::recycle(&paths);
        for it in batch.iter() {
            let gone = !exists(&it.exact);
            let reason = if gone {
                NO_REASON
            } else if rc == platform::CANCELLED_RC {
                Text::new("Cancelled.", "Abgebrochen.")
            } else {
                platform::describe_remaining(&it.exact, it.is_dir)
            };
            report(ItemResult { path: it.display.clone(), ok: gone, bytes: it.alloc, reason }, if gone { Some(it.id) } else { None });
        }
        batch.clear();
    };
    for it in items {
        if let Err(why) = precheck(it) {
            report(ItemResult { path: it.display.clone(), ok: false, bytes: it.alloc, reason: why }, None);
            continue;
        }
        if platform::too_long_for_recycle(&it.display) {
            report(
                ItemResult {
                    path: it.display.clone(),
                    ok: false,
                    bytes: it.alloc,
                    reason: Text::new(
                        "Path too long for the recycle bin. Please use \"Delete permanently\".",
                        "Pfad zu lang für den Papierkorb. Bitte \"Endgültig löschen\" verwenden.",
                    ),
                },
                None,
            );
            continue;
        }
        if it.is_dir {
            flush(&mut batch, &mut report);
            chars = 0;
            batch.push(it);
            flush(&mut batch, &mut report);
            continue;
        }
        chars += it.display.len() + 1;
        batch.push(it);
        if batch.len() >= BATCH || chars >= BATCH_CHARS {
            flush(&mut batch, &mut report);
            chars = 0;
        }
    }
    flush(&mut batch, &mut report);
}

pub fn permanent_all(items: &[Item], mut report: impl FnMut(ItemResult, Option<u32>)) {
    for it in items {
        if let Err(why) = precheck(it) {
            report(ItemResult { path: it.display.clone(), ok: false, bytes: it.alloc, reason: why }, None);
            continue;
        }
        let long = platform::long_pathbuf(&it.exact);
        let r = if it.is_dir { std::fs::remove_dir_all(&long) } else { std::fs::remove_file(&long) };
        match r {
            Ok(()) => report(ItemResult { path: it.display.clone(), ok: true, bytes: it.alloc, reason: NO_REASON }, Some(it.id)),
            Err(e) => {
                let reason = platform::friendly_error(&e).unwrap_or_else(|| tr!("Error: {e}", "Fehler: {e}"));
                report(ItemResult { path: it.display.clone(), ok: false, bytes: it.alloc, reason }, None);
            }
        }
    }
}

/// Verschiebt in einen Ordner. Mit `interactive` zeigt Windows seinen Dialog samt Konfliktabfragen.
/// Ohne Dialog bleibt alles liegen, was am Ziel schon existiert: Es wird nie etwas ueberschrieben.
pub fn move_all(items: &[Item], dest: &str, interactive: bool, mut report: impl FnMut(ItemResult, Option<u32>)) {
    for it in items {
        if let Err(why) = precheck(it) {
            report(ItemResult { path: it.display.clone(), ok: false, bytes: it.alloc, reason: why }, None);
            continue;
        }
        let refuse = |reason: Text| ItemResult { path: it.display.clone(), ok: false, bytes: it.alloc, reason };
        if paths::inside(dest, &it.display) {
            report(refuse(Text::new("The target is inside this folder.", "Das Ziel liegt in diesem Ordner selbst.")), None);
            continue;
        }
        let target = it.exact.file_name().map(|n| Path::new(dest).join(n));
        if !interactive && target.as_deref().map(exists).unwrap_or(false) {
            report(
                refuse(Text::new("An entry with this name already exists at the target.", "Am Ziel gibt es schon einen Eintrag mit diesem Namen.")),
                None,
            );
            continue;
        }
        platform::move_to(std::slice::from_ref(&it.exact), Path::new(dest), interactive);
        let gone = !exists(&it.exact);
        report(
            ItemResult {
                path: it.display.clone(),
                ok: gone,
                bytes: it.alloc,
                reason: if gone { NO_REASON } else { Text::new("Not moved (cancelled or error).", "Nicht verschoben (abgebrochen oder Fehler).") },
            },
            if gone { Some(it.id) } else { None },
        );
    }
}

/// Sandbox-Ersatz fuer den Papierkorb: verschiebt in einen Ordner des Tests. So landet nichts im echten Papierkorb.
pub fn sandbox_recycle_all(items: &[Item], trash: &Path, mut report: impl FnMut(ItemResult, Option<u32>)) {
    let _ = std::fs::create_dir_all(platform::long_pathbuf(trash));
    for (n, it) in items.iter().enumerate() {
        if let Err(why) = precheck(it) {
            report(ItemResult { path: it.display.clone(), ok: false, bytes: it.alloc, reason: why }, None);
            continue;
        }
        let name = it.exact.file_name().map(|x| x.to_string_lossy().to_string()).unwrap_or_default();
        let target = trash.join(format!("{n:06}-{name}"));
        match std::fs::rename(platform::long_pathbuf(&it.exact), platform::long_pathbuf(&target)) {
            Ok(()) => report(ItemResult { path: it.display.clone(), ok: true, bytes: it.alloc, reason: NO_REASON }, Some(it.id)),
            Err(e) => report(ItemResult { path: it.display.clone(), ok: false, bytes: it.alloc, reason: tr!("Error: {e}", "Fehler: {e}") }, None),
        }
    }
}

/// Sandbox-Ersatz fuers Verschieben: ohne Dialog, damit Tests nicht haengen.
pub fn sandbox_move_all(items: &[Item], dest: &Path, mut report: impl FnMut(ItemResult, Option<u32>)) {
    for it in items {
        if let Err(why) = precheck(it) {
            report(ItemResult { path: it.display.clone(), ok: false, bytes: it.alloc, reason: why }, None);
            continue;
        }
        let name = it.exact.file_name().map(|x| x.to_os_string()).unwrap_or_default();
        match std::fs::rename(platform::long_pathbuf(&it.exact), platform::long_pathbuf(&dest.join(name))) {
            Ok(()) => report(ItemResult { path: it.display.clone(), ok: true, bytes: it.alloc, reason: NO_REASON }, Some(it.id)),
            Err(e) => report(ItemResult { path: it.display.clone(), ok: false, bytes: it.alloc, reason: tr!("Error: {e}", "Fehler: {e}") }, None),
        }
    }
}

/// Demo: Nichts wird angefasst. Nur die Schutzregeln greifen wie im Echtbetrieb.
pub fn simulated_all(items: &[Item], mut report: impl FnMut(ItemResult, Option<u32>)) {
    for it in items {
        match guard::check(&it.display) {
            Guard::Block(why) => report(ItemResult { path: it.display.clone(), ok: false, bytes: it.alloc, reason: why }, None),
            _ => report(ItemResult { path: it.display.clone(), ok: true, bytes: it.alloc, reason: NO_REASON }, Some(it.id)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::testutil::build;
    use crate::scan;
    use std::fs;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("smap-clean-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(platform::long_pathbuf(&d));
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn index_of(d: &Path) -> Index {
        let r = d.to_string_lossy().to_string();
        let p = scan::ScanProgress::new(&r, 0, 2, "ssd");
        let mut ix = scan::scan(&r, &p).unwrap();
        scan::finalize(&mut ix);
        ix
    }

    fn id_of(ix: &Index, name: &str) -> u32 {
        (0..ix.nodes.len() as u32).find(|i| ix.name(*i) == name).expect(name)
    }

    #[test]
    fn cart_does_not_double_count_nested_items() {
        let ix = build(r"D:\", &[(r"Ordner\a.bin", 100), (r"Ordner\b.bin", 50), (r"c.bin", 7)]);
        let (o, a, c) = (id_of(&ix, "Ordner"), id_of(&ix, "a.bin"), id_of(&ix, "c.bin"));
        let mut cart = Cart::default();
        cart.add(&ix, &[a, c]);
        assert_eq!(cart.bytes(&ix), 107);
        // Der Ordner ersetzt die enthaltene Datei.
        cart.add(&ix, &[o]);
        assert_eq!(cart.bytes(&ix), 157);
        assert_eq!(cart.ids.len(), 2);
        // Bereits abgedeckt: aendert nichts.
        cart.add(&ix, &[a]);
        assert_eq!(cart.bytes(&ix), 157);
    }

    #[test]
    fn cart_rejects_protected_paths() {
        let ix = build(r"C:\", &[(r"Windows\System32\kernel32.dll", 100), (r"pagefile.sys", 10), (r"Daten\x.bin", 1)]);
        let mut cart = Cart::default();
        let r = cart.add(&ix, &[id_of(&ix, "kernel32.dll"), id_of(&ix, "pagefile.sys"), id_of(&ix, "x.bin")]);
        assert_eq!(r.added, 1);
        assert_eq!(r.blocked.len(), 2);
        assert_eq!(cart.ids.len(), 1);
    }

    #[test]
    fn precheck_refuses_changed_and_missing_files() {
        let d = tmp("pre");
        let f = d.join("a.bin");
        fs::write(&f, vec![1u8; 1000]).unwrap();
        fs::write(d.join("b.bin"), vec![1u8; 1000]).unwrap();
        let ix = index_of(&d);
        let items = snapshot(&ix, &[id_of(&ix, "a.bin"), id_of(&ix, "b.bin")]);
        assert!(precheck(&items[0]).is_ok());
        fs::write(&f, vec![2u8; 2000]).unwrap();
        let changed = precheck(&items[0]).unwrap_err();
        assert!(changed.en().contains("changed"));
        assert!(changed.get(Lang::De).contains("geändert"));
        fs::remove_file(d.join("b.bin")).unwrap();
        let missing = precheck(&items[1]).unwrap_err();
        assert!(missing.en().contains("No longer there"));
        assert!(missing.get(Lang::De).contains("Nicht mehr vorhanden"));
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn permanent_delete_removes_and_reports() {
        let d = tmp("perm");
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("sub").join("x.bin"), vec![1u8; 5000]).unwrap();
        fs::write(d.join("y.bin"), vec![1u8; 500]).unwrap();
        fs::write(d.join("bleibt.bin"), vec![1u8; 50]).unwrap();
        let mut ix = index_of(&d);
        let items = snapshot(&ix, &[id_of(&ix, "sub"), id_of(&ix, "y.bin")]);
        let mut done = vec![];
        permanent_all(&items, |r, id| {
            assert!(r.ok, "{r:?}");
            done.push(id.unwrap());
        });
        for id in done {
            ix.mark_deleted(id);
        }
        assert!(!d.join("sub").exists());
        assert!(!d.join("y.bin").exists());
        assert!(d.join("bleibt.bin").exists());
        assert_eq!(ix.nodes[0].size, 50);
        fs::remove_dir_all(d).unwrap();
    }

    // Legt eine Testdatei in den echten Papierkorb. Deshalb nur bewusst starten, die CI tut das auf ihren Wegwerf-Rechnern:
    // cargo test -- --ignored recycle_moves_file
    #[test]
    #[ignore]
    fn recycle_moves_file_away_and_leaves_others() {
        let d = tmp("bin");
        fs::write(d.join("weg.smaptest"), vec![1u8; 2000]).unwrap();
        fs::write(d.join("bleibt.smaptest"), vec![1u8; 2000]).unwrap();
        let ix = index_of(&d);
        let items = snapshot(&ix, &[id_of(&ix, "weg.smaptest")]);
        let mut results = vec![];
        recycle_all(&items, |r, _| results.push(r));
        assert!(results[0].ok, "{:?}", results[0]);
        assert!(!d.join("weg.smaptest").exists());
        assert!(d.join("bleibt.smaptest").exists());
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn silent_move_never_overwrites() {
        let d = tmp("noclobber");
        fs::create_dir_all(d.join("ziel")).unwrap();
        fs::write(d.join("a.bin"), vec![1u8; 10]).unwrap();
        fs::write(d.join("ziel").join("a.bin"), vec![2u8; 20]).unwrap();
        let ix = index_of(&d);
        let items = snapshot(&ix, &[ix.find_path(&d.join("a.bin").to_string_lossy()).unwrap()]);
        let dest = d.join("ziel").to_string_lossy().to_string();
        let mut results = vec![];
        move_all(&items, &dest, false, |r, _| results.push(r));
        assert!(!results[0].ok, "{:?}", results[0]);
        assert_eq!(fs::read(d.join("ziel").join("a.bin")).unwrap().len(), 20, "Ziel bleibt unberuehrt");
        assert!(d.join("a.bin").exists());
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn move_refuses_target_inside_source() {
        let d = tmp("mv");
        fs::create_dir_all(d.join("quelle").join("ziel")).unwrap();
        fs::write(d.join("quelle").join("a.bin"), vec![1u8; 10]).unwrap();
        let ix = index_of(&d);
        let items = snapshot(&ix, &[id_of(&ix, "quelle")]);
        let dest = d.join("quelle").join("ziel").to_string_lossy().to_string();
        let mut results = vec![];
        move_all(&items, &dest, true, |r, _| results.push(r));
        assert!(!results[0].ok);
        assert!(d.join("quelle").join("a.bin").exists());
        fs::remove_dir_all(d).unwrap();
    }
}
