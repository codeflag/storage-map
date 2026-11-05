//! Regelwerk fuer "unnoetiges Zeug": feste, nachvollziehbare Regeln mit Begruendung und Risikostufe.

use crate::guard::{self, Guard};
use crate::i18n::Text;
use crate::index::*;
use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    /// Wird automatisch neu erzeugt.
    Safe,
    /// Harmlos, kann aber Nebenwirkungen haben (Neu-Login, Neu-Download).
    Caution,
    /// Kann echte Daten sein: Selbst ansehen.
    Review,
    /// Nur zur Information, nicht ueber die Aufraeumliste loeschbar.
    Info,
}

#[derive(Clone, Copy, Serialize, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    None,
    EmptyRecycleBin,
    DiskCleanup,
}

enum M {
    /// Pfad endet genau so (alles klein, mit fuehrendem `/`). Pfade werden zum Vergleich auf `/` vereinheitlicht.
    Suffix(&'static [&'static str]),
    /// Pfad ist genau dieser, vom Dateisystem aus gesehen (Linux-Systemordner).
    Abs(&'static [&'static str]),
    /// Ordnername ist einer von diesen und der Pfad enthaelt eins dieser Stuecke.
    NameUnder(&'static [&'static str], &'static [&'static str]),
    /// Ordnername, irgendwo.
    Name(&'static [&'static str]),
    /// Ordner direkt in der Wurzel des Laufwerks.
    RootName(&'static [&'static str]),
    /// Ordner direkt in der Wurzel, dessen Name so beginnt.
    RootPrefix(&'static [&'static str]),
    /// Ordner, dessen Eltern-Pfad so endet (jedes Kind ist ein Treffer).
    ChildOf(&'static [&'static str]),
    /// Ordnername mit einer Datei daneben (z. B. `target` neben `Cargo.toml`).
    NameWithSibling(&'static [&'static str], &'static [&'static str]),
    /// Ordner, der eine bestimmte Datei enthaelt (z. B. `pyvenv.cfg`).
    Contains(&'static str),
    /// Dateiendung, optional aelter als n Tage.
    Ext(&'static [&'static str], u32),
    /// Dateiendung in einem Ordner mit diesem Namensteil, aelter als n Tage.
    ExtIn(&'static [&'static str], &'static str, u32),
    /// Dateiname exakt.
    File(&'static [&'static str]),
    /// Dateiname beginnt mit, und Pfad enthaelt.
    FilePrefixIn(&'static [&'static str], &'static str),
}

pub struct Rule {
    pub id: &'static str,
    pub group: Text,
    pub title: Text,
    pub why: Text,
    pub risk: Risk,
    /// Inhalt statt Ordner entfernen: Der Ordner selbst bleibt fuer die Programme bestehen.
    contents: bool,
    pub action: Action,
    m: M,
}

const G_SYS: Group = ("System", "System");
const G_APP: Group = ("Browser & programs", "Browser & Programme");
const G_DEV: Group = ("Development", "Entwicklung");
const G_REST: Group = ("Leftovers", "Reste & Altlasten");
const G_BIG: Group = ("Large items to check", "Große Posten prüfen");

type Group = (&'static str, &'static str);

/// Reihenfolge der Texte: Titel Englisch, Titel Deutsch, Begruendung Englisch, Begruendung Deutsch.
#[allow(clippy::too_many_arguments)]
const fn r(
    id: &'static str,
    group: Group,
    title_en: &'static str,
    title_de: &'static str,
    why_en: &'static str,
    why_de: &'static str,
    risk: Risk,
    contents: bool,
    action: Action,
    m: M,
) -> Rule {
    Rule { id, group: Text::new(group.0, group.1), title: Text::new(title_en, title_de), why: Text::new(why_en, why_de), risk, contents, action, m }
}

use Action as A;
use Risk::*;

const RULES: &[Rule] = &[
    r("recycle", G_SYS, "Recycle bin", "Papierkorb", "Files that were already deleted. They keep taking up space until the recycle bin is emptied.", "Bereits gelöschte Dateien. Sie belegen weiter Platz, bis der Papierkorb geleert wird.", Safe, false, A::EmptyRecycleBin,
        M::RootName(&["$recycle.bin"])),
    r("trash", G_SYS, "Recycle bin", "Papierkorb", "Files that were already deleted. They keep taking up space until the recycle bin is emptied.", "Bereits gelöschte Dateien. Sie belegen weiter Platz, bis der Papierkorb geleert wird.", Safe, false, A::EmptyRecycleBin,
        M::Suffix(&["/.local/share/trash"])),
    r("trash-volume", G_SYS, "Recycle bin of this drive", "Papierkorb dieses Laufwerks", "Files that were already deleted. They keep taking up space until the recycle bin is emptied.", "Bereits gelöschte Dateien. Sie belegen weiter Platz, bis der Papierkorb geleert wird.", Safe, false, A::EmptyRecycleBin,
        M::RootPrefix(&[".trash-", ".trash"])),
    r("pkg-downloads", G_SYS, "Package downloads", "Paket-Downloads", "Downloaded packages from apt, dnf or pacman. The package manager downloads them again when needed. Needs more rights.", "Heruntergeladene Pakete von apt, dnf oder pacman. Die Paketverwaltung lädt sie bei Bedarf neu. Mehr Rechte nötig.", Safe, true, A::None,
        M::Abs(&["/var/cache/apt/archives", "/var/cache/dnf", "/var/cache/pacman/pkg", "/var/cache/zypp/packages"])),
    r("coredumps", G_SYS, "System crash dumps", "Absturzabbilder des Systems", "Records of past crashes. Only useful for troubleshooting. Needs more rights.", "Protokolle vergangener Abstürze. Nur für die Fehlersuche nützlich. Mehr Rechte nötig.", Safe, true, A::None,
        M::Abs(&["/var/lib/systemd/coredump", "/var/crash"])),
    r("var-tmp", G_SYS, "Temporary files (system)", "Temporäre Dateien (System)", "Interim files that survive restarts. Running programs may still need them. Needs more rights.", "Zwischendateien, die Neustarts überleben. Laufende Programme können sie noch brauchen. Mehr Rechte nötig.", Caution, true, A::None,
        M::Abs(&["/var/tmp"])),
    r("rotated-logs", G_SYS, "Compressed system logs", "Gepackte Systemprotokolle", "Older, already compressed log files in /var/log. Needs more rights.", "Ältere, bereits gepackte Logdateien in /var/log. Mehr Rechte nötig.", Safe, false, A::None,
        M::ExtIn(&["gz", "xz", "zst", "bz2", "old"], "/var/log", 7)),
    r("thumbs-linux", G_SYS, "Thumbnail cache", "Vorschaubilder-Cache", "The file manager rebuilds thumbnails when needed.", "Der Dateimanager baut die Vorschaubilder bei Bedarf neu auf.", Safe, true, A::None,
        M::Suffix(&["/.cache/thumbnails"])),
    r("user-temp", G_SYS, "Temporary files (user)", "Temporäre Dateien (Benutzer)", "Interim files from programs. Files currently in use simply stay in place during cleanup.", "Zwischendateien von Programmen. Dateien, die gerade benutzt werden, bleiben beim Aufräumen einfach liegen.", Safe, true, A::None,
        M::Suffix(&["/appdata/local/temp"])),
    r("win-temp", G_SYS, "Temporary files (Windows)", "Temporäre Dateien (Windows)", "Interim files of the system. Removing them usually needs administrator rights.", "Zwischendateien des Systems. Zum Entfernen sind meist Adminrechte nötig.", Safe, true, A::None,
        M::Suffix(&["/windows/temp"])),
    r("wu-cache", G_SYS, "Windows Update downloads", "Windows-Update-Downloads", "Update packages that are already installed. Windows downloads them again when needed. Needs administrator rights.", "Bereits installierte Update-Pakete. Windows lädt bei Bedarf neu. Adminrechte nötig.", Safe, true, A::None,
        M::Suffix(&["/windows/softwaredistribution/download"])),
    r("delivery", G_SYS, "Delivery Optimization", "Übermittlungsoptimierung", "Cache that Windows uses to share updates with other PCs. Needs administrator rights.", "Zwischenspeicher, mit dem Windows Updates an andere PCs verteilt. Adminrechte nötig.", Safe, true, A::None,
        M::Suffix(&["/microsoft/windows/deliveryoptimization/cache"])),
    r("crash", G_SYS, "Crash reports & memory dumps", "Absturzberichte & Speicherabbilder", "Records of past crashes. Only useful for troubleshooting.", "Protokolle vergangener Abstürze. Nur für die Fehlersuche nützlich.", Safe, true, A::None,
        M::Suffix(&["/appdata/local/crashdumps", "/windows/minidump", "/windows/livekernelreports", "/microsoft/windows/wer"])),
    r("memdump", G_SYS, "Memory dump of the last crash", "Speicherabbild des letzten Absturzes", "Can be as large as the RAM. Only useful for troubleshooting.", "Kann so groß wie der Arbeitsspeicher sein. Nur für die Fehlersuche nützlich.", Safe, false, A::None,
        M::File(&["memory.dmp"])),
    r("win-logs", G_SYS, "Windows logs", "Windows-Protokolle", "Installation and diagnostic logs. Needs administrator rights.", "Installations- und Diagnoseprotokolle. Adminrechte nötig.", Safe, true, A::None,
        M::Suffix(&["/windows/logs"])),
    r("thumbs", G_SYS, "Thumbnail cache", "Vorschaubilder-Cache", "Explorer rebuilds thumbnails when needed.", "Explorer baut die Vorschaubilder bei Bedarf neu auf.", Safe, false, A::None,
        M::FilePrefixIn(&["thumbcache_", "iconcache_"], "/microsoft/windows/explorer")),
    r("win-old", G_SYS, "Old Windows installation", "Alte Windows-Installation", "Remains of the previous Windows. Remove them with Disk Cleanup, not by hand.", "Reste des vorherigen Windows. Entfernen Sie sie mit der Datenträgerbereinigung, nicht von Hand.", Caution, false, A::DiskCleanup,
        M::RootName(&["windows.old", "$windows.~bt", "$windows.~ws", "$getcurrent", "$sysreset"])),

    r("browser", G_APP, "Browser cache", "Browser-Cache", "Cached web pages. Pages load a little slower once on the next visit. Logins are kept.", "Zwischengespeicherte Webseiten. Seiten laden beim nächsten Besuch einmal etwas langsamer. Logins bleiben erhalten.", Safe, true, A::None,
        M::NameUnder(&["cache", "cache2", "code cache", "gpucache", "dawncache", "dawngraphitecache", "dawnwebgpucache", "grshadercache", "shadercache", "cachestorage"],
            &["/google/chrome/", "/microsoft/edge/", "/bravesoftware/", "/mozilla/firefox/", "/opera software/", "/vivaldi/", "/chromium/"])),
    r("browser-linux", G_APP, "Browser cache", "Browser-Cache", "Cached web pages. Pages load a little slower once on the next visit. Logins are kept.", "Zwischengespeicherte Webseiten. Seiten laden beim nächsten Besuch einmal etwas langsamer. Logins bleiben erhalten.", Safe, true, A::None,
        M::Suffix(&["/.cache/google-chrome", "/.cache/chromium", "/.cache/mozilla", "/.cache/bravesoftware", "/.cache/microsoft-edge", "/.cache/vivaldi", "/.cache/opera"])),
    r("flatpak-cache", G_APP, "Caches of Flatpak programs", "Caches von Flatpak-Programmen", "Caches of individual programs. They are recreated.", "Zwischenspeicher einzelner Programme. Werden neu erzeugt.", Safe, true, A::None,
        M::NameUnder(&["cache"], &["/.var/app/"])),
    r("shader", G_APP, "Graphics card shader cache", "Shader-Cache der Grafikkarte", "Recreated by the graphics driver. Games may stutter briefly on first start.", "Wird vom Grafiktreiber neu erzeugt. Spiele ruckeln beim ersten Start evtl. kurz.", Safe, true, A::None,
        M::Suffix(&["/nvidia/dxcache", "/nvidia/glcache", "/nvidia corporation/nv_cache", "/amd/dxcache", "/amd/glcache", "/amd/dx9cache", "/appdata/local/d3dscache", "/intel/shadercache", "/locallow/nvidia/perdrivercache"])),
    r("media-cache", G_APP, "Media caches (Adobe, Steam, Spotify)", "Medien-Caches (Adobe, Steam, Spotify)", "Recreated or downloaded again by the programs.", "Werden von den Programmen neu erzeugt oder neu geladen.", Safe, true, A::None,
        M::Suffix(&["/adobe/common/media cache files", "/adobe/common/media cache", "/adobe/common/peak files", "/steam/appcache/httpcache", "/steam/depotcache", "/appdata/local/spotify/data", "/appdata/local/spotify/storage"])),
    r("app-cache", G_APP, "Caches of other programs", "Caches anderer Programme", "Program caches in AppData. Usually harmless, but some programs have to load data again afterwards.", "Zwischenspeicher von Programmen in AppData. Meist gefahrlos, manche Programme müssen danach aber Daten neu laden.", Caution, true, A::None,
        M::NameUnder(&["cache", "caches", "code cache", "gpucache", "cacheddata", "cachedextensionvsixs", "cacheddata2"], &["/appdata/"])),

    r("pkg-cache", G_DEV, "Package caches (npm, pip, cargo …)", "Paket-Caches (npm, pip, cargo …)", "Downloaded packages. They are downloaded again automatically on the next build.", "Heruntergeladene Pakete. Werden beim nächsten Build automatisch neu geladen.", Safe, true, A::None,
        M::Suffix(&["/appdata/local/npm-cache", "/appdata/roaming/npm-cache", "/appdata/local/pip/cache", "/appdata/local/uv/cache", "/appdata/local/yarn/cache", "/appdata/local/pnpm/store", "/.cargo/registry", "/.gradle/caches", "/.m2/repository", "/.nuget/packages", "/go/pkg/mod", "/.conda/pkgs", "/anaconda3/pkgs", "/miniconda3/pkgs", "/.npm/_cacache", "/.cache/pip", "/.cache/uv", "/.cache/yarn", "/.cache/go-build", "/.cache/pnpm", "/.local/share/pnpm/store"])),
    r("pycache", G_DEV, "Python interim files", "Python-Zwischendateien", "Recreated automatically by Python.", "Werden von Python automatisch neu erzeugt.", Safe, false, A::None,
        M::Name(&["__pycache__", ".pytest_cache", ".mypy_cache", ".ruff_cache", ".ipynb_checkpoints", ".tox"])),
    r("build-rust", G_DEV, "Rust build folders", "Rust-Buildordner", "Build results. \"cargo build\" recreates them.", "Build-Ergebnisse. \"cargo build\" erzeugt sie neu.", Safe, false, A::None,
        M::NameWithSibling(&["target"], &["cargo.toml"])),
    r("build-dotnet", G_DEV, ".NET build folders", ".NET-Buildordner", "Build results. They are recreated on the next build.", "Build-Ergebnisse. Werden beim nächsten Build neu erzeugt.", Safe, false, A::None,
        M::NameWithSibling(&["bin", "obj"], &[".csproj", ".vbproj", ".fsproj", ".sln"])),
    r("build-js", G_DEV, "Web build folders", "Web-Buildordner", "Build caches of Next.js, Nuxt, Vite and others. They are recreated.", "Build-Caches von Next.js, Nuxt, Vite & Co. Werden neu erzeugt.", Safe, false, A::None,
        M::Name(&[".next", ".nuxt", ".turbo", ".parcel-cache", ".svelte-kit", ".angular", ".vite"])),
    r("node-modules", G_DEV, "node_modules", "node_modules", "Dependencies of JavaScript projects. Can be restored at any time with \"npm install\". Most worthwhile for dormant projects.", "Abhängigkeiten von JavaScript-Projekten. Mit \"npm install\" jederzeit wiederherstellbar. Lohnt sich vor allem bei ruhenden Projekten.", Caution, false, A::None,
        M::Name(&["node_modules"])),
    r("venv", G_DEV, "Python environments", "Python-Umgebungen", "Virtual environments. They can be rebuilt from the requirements file, if there is one.", "Virtuelle Umgebungen. Lassen sich aus der requirements-Datei neu aufbauen, falls eine existiert.", Caution, false, A::None,
        M::Contains("pyvenv.cfg")),

    r("incomplete", G_REST, "Aborted downloads", "Abgebrochene Downloads", "Incomplete files older than one week. Running downloads are not affected.", "Unvollständige Dateien, älter als eine Woche. Laufende Downloads sind damit nicht gemeint.", Safe, false, A::None,
        M::Ext(&["crdownload", "part", "partial", "download", "!ut", "!qb"], 7)),
    r("tmp-files", G_REST, "Temporary files everywhere", "Temporäre Dateien überall", "Files with .tmp or .temp, older than one week.", "Dateien mit .tmp oder .temp, älter als eine Woche.", Safe, false, A::None,
        M::Ext(&["tmp", "temp"], 7)),
    r("dumps", G_REST, "Memory dumps", "Speicherabbilder", "Dumps of program crashes (.dmp, .hprof). Only useful for troubleshooting.", "Abbilder von Programmabstürzen (.dmp, .hprof). Nur für die Fehlersuche nützlich.", Safe, false, A::None,
        M::Ext(&["dmp", "mdmp", "hprof"], 0)),
    r("logs", G_REST, "Old log files", "Alte Protokolldateien", "Log files that have not been written to for over 30 days.", "Logdateien, die seit über 30 Tagen nicht mehr geschrieben wurden.", Caution, false, A::None,
        M::Ext(&["log", "etl", "trace"], 30)),
    r("old-installers", G_REST, "Old installers & archives in Downloads", "Alte Installer & Archive in Downloads", "Setup files and archives in the download folder, older than 90 days. Often installed or unpacked long ago.", "Setup-Dateien und Archive im Download-Ordner, älter als 90 Tage. Oft längst installiert oder entpackt.", Review, false, A::None,
        M::ExtIn(&["exe", "msi", "msix", "iso", "zip", "rar", "7z", "dmg", "cab", "tar", "gz", "tgz", "xz", "bz2", "zst", "deb", "rpm", "appimage", "run"], "/downloads", 90)),
    r("backups", G_REST, "Backup copies", "Sicherungskopien", "Files with .bak, .old or .orig. Check whether the original still exists.", "Dateien mit .bak, .old oder .orig. Prüfen Sie, ob das Original noch da ist.", Review, false, A::None,
        M::Ext(&["bak", "old", "orig", "backup"], 30)),

    r("games", G_BIG, "Installed games", "Installierte Spiele", "Can be downloaded again at any time. Please uninstall them through the launcher so that it knows.", "Lassen sich jederzeit neu herunterladen. Bitte über den Launcher deinstallieren, damit er Bescheid weiß.", Review, false, A::None,
        M::ChildOf(&["/steamapps/common", "/epic games", "/gog games", "/riot games", "/xboxgames", "/ea games", "/ubisoft game launcher/games", "/battle.net/games"])),
    r("ml-models", G_BIG, "AI model caches", "KI-Modell-Caches", "Downloaded models (Hugging Face, Ollama, LM Studio). They can be downloaded again, but that takes time.", "Heruntergeladene Modelle (Hugging Face, Ollama, LM Studio). Lassen sich erneut laden, das dauert aber.", Review, false, A::None,
        M::Suffix(&["/.cache/huggingface", "/.ollama/models", "/.cache/torch", "/.cache/lm-studio/models", "/.lmstudio/models"])),
    r("vm-disks", G_BIG, "Virtual hard disks", "Virtuelle Festplatten", "Disks of VMs, WSL and Docker. They often contain important data, but quietly keep growing.", "Festplatten von VMs, WSL und Docker. Enthalten oft wichtige Daten, wachsen aber still vor sich hin.", Review, false, A::None,
        M::Ext(&["vhd", "vhdx", "vmdk", "vdi", "qcow2", "avhdx"], 0)),
    r("phone-backups", G_BIG, "iPhone backups", "iPhone-Backups", "Local backups from iTunes or the Devices app. Those of old devices can usually go.", "Lokale Sicherungen aus iTunes oder der Geräte-App. Alte Geräte können meist weg.", Review, false, A::None,
        M::Suffix(&["/apple/mobilesync/backup", "/apple computer/mobilesync/backup"])),

    r("hibernate", G_BIG, "Hibernation file", "Ruhezustand-Datei", "Holds the RAM contents for hibernation. Do not delete it. Turn it off with \"powercfg /h off\" if you do not use hibernation.", "Hält den Arbeitsspeicher für den Ruhezustand bereit. Nicht löschen, sondern mit \"powercfg /h off\" abschalten, wenn Sie keinen Ruhezustand nutzen.", Info, false, A::None,
        M::File(&["hiberfil.sys"])),
    r("pagefile", G_BIG, "Page file", "Auslagerungsdatei", "Extends the RAM. Do not delete it. The size can be set under System Properties > Advanced > Performance.", "Erweitert den Arbeitsspeicher. Nicht löschen. Die Größe lässt sich unter Systemeigenschaften > Erweitert > Leistung einstellen.", Info, false, A::None,
        M::File(&["pagefile.sys", "swapfile.sys"])),
    r("winsxs", G_BIG, "Windows component store", "Windows-Komponentenspeicher", "Do not delete it. The only way to clean it up: \"Dism /Online /Cleanup-Image /StartComponentCleanup\" as administrator.", "Nicht löschen. Aufräumen geht nur so: \"Dism /Online /Cleanup-Image /StartComponentCleanup\" als Administrator.", Info, false, A::None,
        M::Suffix(&["/windows/winsxs"])),
    r("installer-cache", G_BIG, "Windows Installer cache", "Windows-Installer-Cache", "Do not delete it. It is needed to repair and uninstall programs.", "Nicht löschen. Wird zum Reparieren und Deinstallieren von Programmen gebraucht.", Info, false, A::None,
        M::Suffix(&["/windows/installer"])),
    r("swap-linux", G_BIG, "Swap file", "Auslagerungsdatei", "Extends the RAM. Do not delete it. It is managed with swapon, swapoff and /etc/fstab.", "Erweitert den Arbeitsspeicher. Nicht löschen. Verwaltet wird sie mit swapon, swapoff und /etc/fstab.", Info, false, A::None,
        M::File(&["swapfile", "swap.img"])),
    r("journal", G_BIG, "System log (journald)", "Systemprotokoll (journald)", "Do not delete it by hand. Shrink it with \"journalctl --vacuum-size=200M\" as administrator.", "Nicht von Hand löschen. Verkleinern lässt es sich mit \"journalctl --vacuum-size=200M\" als Administrator.", Info, false, A::None,
        M::Abs(&["/var/log/journal"])),
    r("snap-store", G_BIG, "Snap packages", "Snap-Pakete", "Do not delete them by hand. The system keeps fewer old versions with \"snap set system refresh.retain=2\".", "Nicht von Hand löschen. Weniger alte Versionen behält das System mit \"snap set system refresh.retain=2\".", Info, false, A::None,
        M::Abs(&["/var/lib/snapd/snaps"])),
    r("flatpak-store", G_BIG, "Flatpak storage", "Flatpak-Speicher", "Do not delete it by hand. \"flatpak uninstall --unused\" removes what is unused.", "Nicht von Hand löschen. Unbenutztes entfernt \"flatpak uninstall --unused\".", Info, false, A::None,
        M::Abs(&["/var/lib/flatpak"])),
    r("container-store", G_BIG, "Container data", "Container-Daten", "Do not delete it by hand. Clean it up with \"docker system prune\" or \"podman system prune\".", "Nicht von Hand löschen. Aufräumen geht mit \"docker system prune\" oder \"podman system prune\".", Info, false, A::None,
        M::Abs(&["/var/lib/docker", "/var/lib/containers"])),
    // Steht zuletzt, damit die genaueren Regeln fuer ~/.cache zuerst greifen.
    r("user-cache", G_APP, "Caches of other programs", "Caches anderer Programme", "Contents of ~/.cache. Usually harmless, but some programs have to load data again afterwards.", "Inhalt von ~/.cache. Meist gefahrlos, manche Programme müssen danach aber Daten neu laden.", Caution, false, A::None,
        M::ChildOf(&["/.cache"])),
];

pub fn rules() -> &'static [Rule] {
    RULES
}

pub fn rule_by_id(id: &str) -> Option<usize> {
    RULES.iter().position(|r| r.id == id)
}

#[derive(Default, Clone)]
pub struct RuleResult {
    pub items: Vec<u32>,
    pub alloc: u64,
    pub files: u64,
}

pub struct JunkReport {
    pub results: Vec<RuleResult>,
    /// Leere Ordner ausserhalb von System- und Programmbereichen.
    pub empty_dirs: Vec<u32>,
}

impl JunkReport {
    /// Platz, der sich ohne Bedenken freigeben laesst.
    pub fn safe_bytes(&self) -> u64 {
        RULES.iter().zip(&self.results).filter(|(r, _)| r.risk == Safe).map(|(_, x)| x.alloc).sum()
    }
}

fn now_secs() -> u32 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as u32).unwrap_or(0)
}

/// Vorab aufbereitete Dateiregeln, damit nicht fuer jede Datei alle Regeln durchlaufen werden.
struct FileRules {
    by_ext: Vec<Vec<(usize, u32, Option<&'static str>)>>,
    names: Vec<(&'static str, usize)>,
    prefixes: Vec<(&'static str, &'static str, usize)>,
}

fn prepare(ix: &Index) -> FileRules {
    let mut by_ext = vec![vec![]; ix.meta.exts.len()];
    let mut names = vec![];
    let mut prefixes = vec![];
    for (ri, rule) in RULES.iter().enumerate() {
        match &rule.m {
            M::Ext(list, days) => {
                for (id, e) in ix.meta.exts.iter().enumerate() {
                    if list.contains(&e.as_str()) {
                        by_ext[id].push((ri, *days, None));
                    }
                }
            }
            M::ExtIn(list, part, days) => {
                for (id, e) in ix.meta.exts.iter().enumerate() {
                    if list.contains(&e.as_str()) {
                        by_ext[id].push((ri, *days, Some(*part)));
                    }
                }
            }
            M::File(list) => names.extend(list.iter().map(|n| (*n, ri))),
            M::FilePrefixIn(list, part) => prefixes.extend(list.iter().map(|n| (*n, *part, ri))),
            _ => {}
        }
    }
    FileRules { by_ext, names, prefixes }
}

fn child_named(ix: &Index, id: u32, names: &[&str]) -> bool {
    let n = &ix.nodes[id as usize];
    (n.first_child..n.first_child + n.child_count).any(|c| {
        let name = ix.name(c).to_lowercase();
        names.iter().any(|t| if t.starts_with('.') { name.ends_with(t) } else { name == *t })
    })
}

fn match_dir(ix: &Index, id: u32, lower_path: &str, parent_len: usize, name: &str, depth: u32) -> Option<usize> {
    for (ri, rule) in RULES.iter().enumerate() {
        let hit = match &rule.m {
            M::Suffix(list) => list.iter().any(|s| lower_path.ends_with(s)),
            M::Abs(list) => list.contains(&lower_path),
            M::RootPrefix(list) => depth == 1 && list.iter().any(|p| name.starts_with(p)),
            M::NameUnder(names, parts) => names.contains(&name) && parts.iter().any(|p| lower_path.contains(p)),
            M::Name(names) => names.contains(&name),
            M::RootName(names) => depth == 1 && names.contains(&name),
            M::ChildOf(parents) => parents.iter().any(|s| lower_path[..parent_len].ends_with(s)),
            M::NameWithSibling(names, siblings) => names.contains(&name) && child_named(ix, ix.nodes[id as usize].parent, siblings),
            M::Contains(file) => child_named(ix, id, &[file]),
            _ => false,
        };
        if hit {
            return Some(ri);
        }
    }
    None
}

/// Ordner, die beim Abstieg gar nicht erst durchsucht werden.
fn skip_descent(lower_path: &str) -> bool {
    lower_path.ends_with("/windows/winsxs")
}

pub fn analyze(ix: &Index) -> JunkReport {
    let fr = prepare(ix);
    let now = now_secs();
    let mut results = vec![RuleResult::default(); RULES.len()];
    let mut empty_dirs = vec![];

    let mut path = ix.meta.root.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let root_len = path.len();
    // Stapel: (Knoten, Pfadlaenge des Elternordners, Tiefe)
    let mut stack: Vec<(u32, usize, u32)> = vec![(0, root_len, 0)];
    let mut lname = String::new();

    while let Some((id, parent_len, depth)) = stack.pop() {
        path.truncate(parent_len);
        if id != 0 {
            path.push('/');
            lname.clear();
            lname.push_str(&ix.name(id));
            lname.make_ascii_lowercase();
            if !lname.is_ascii() {
                lname = lname.to_lowercase();
            }
            path.push_str(&lname);
        }
        let n = &ix.nodes[id as usize];
        if n.is_deleted() || n.flags & F_LINK != 0 {
            continue;
        }

        if id != 0 {
            if let Some(ri) = match_dir(ix, id, &path, parent_len, &lname, depth) {
                let out = &mut results[ri];
                out.alloc += n.alloc;
                out.files += n.files as u64;
                if RULES[ri].contents {
                    for c in n.first_child..n.first_child + n.child_count {
                        if !ix.nodes[c as usize].is_deleted() {
                            out.items.push(c);
                        }
                    }
                } else {
                    out.items.push(id);
                }
                continue;
            }
            if n.child_count == 0 && n.flags & F_DENIED == 0 && !matches!(guard::check(&ix.path(id)), Guard::Block(_) | Guard::Warn(_)) {
                empty_dirs.push(id);
            }
        }
        if skip_descent(&path) {
            continue;
        }

        let dir_len = path.len();
        for c in n.first_child..n.first_child + n.child_count {
            let child = &ix.nodes[c as usize];
            if child.is_dir() {
                stack.push((c, dir_len, depth + 1));
                continue;
            }
            if child.is_deleted() || child.flags & (F_LINK | F_HARDLINK | F_CLOUD) != 0 {
                continue;
            }
            let mut matched: Option<usize> = None;
            if let Some(list) = fr.by_ext.get(child.ext as usize) {
                for (ri, days, part) in list {
                    let old_enough = *days == 0 || child.mtime < now.saturating_sub(days * 86_400);
                    let in_place = part.map(|p| path.contains(p)).unwrap_or(true);
                    if old_enough && in_place {
                        matched = Some(*ri);
                        break;
                    }
                }
            }
            if matched.is_none() && (!fr.names.is_empty() || !fr.prefixes.is_empty()) && child.name_len < 64 {
                let nm = ix.name(c).to_lowercase();
                matched = fr
                    .names
                    .iter()
                    .find(|(t, _)| *t == nm)
                    .map(|(_, ri)| *ri)
                    .or_else(|| fr.prefixes.iter().find(|(t, part, _)| nm.starts_with(t) && path.contains(part)).map(|(_, _, ri)| *ri));
            }
            if let Some(ri) = matched {
                let out = &mut results[ri];
                out.alloc += child.alloc;
                out.files += 1;
                out.items.push(c);
            }
        }
    }

    for x in results.iter_mut() {
        x.items.sort_unstable_by(|a, b| ix.nodes[*b as usize].alloc.cmp(&ix.nodes[*a as usize].alloc).then(a.cmp(b)));
    }
    JunkReport { results, empty_dirs }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::testutil::build;

    fn hit<'a>(rep: &'a JunkReport, id: &str) -> &'a RuleResult {
        &rep.results[rule_by_id(id).unwrap()]
    }

    fn sample() -> Index {
        build(
            r"C:\",
            &[
                (r"Users\Demo\AppData\Local\Temp\a.tmp", 100),
                (r"Users\Demo\AppData\Local\Temp\sub\b.dat", 50),
                (r"Users\Demo\AppData\Local\Google\Chrome\User Data\Default\Cache\Cache_Data\f_01", 700),
                (r"Users\Demo\AppData\Local\Foo\Cache\x", 33),
                (r"Users\Demo\Code\app\node_modules\left-pad\index.js", 500),
                (r"Users\Demo\Code\rusty\Cargo.toml", 1),
                (r"Users\Demo\Code\rusty\target\debug\x.exe", 900),
                (r"Users\Demo\Code\other\target\wichtig.txt", 5),
                (r"Users\Demo\Code\py\venv\pyvenv.cfg", 1),
                (r"Users\Demo\Code\py\venv\lib\x.py", 300),
                (r"Users\Demo\Code\py\__pycache__\m.pyc", 11),
                (r"Users\Demo\Downloads\setup.exe", 9000),
                (r"Users\Demo\Documents\wichtig.exe", 8000),
                (r"Users\Demo\Documents\foto.jpg", 77),
                (r"Users\Demo\Leer\", 0),
                (r"Windows\Temp\w.tmp", 40),
                (r"Windows\System32\kernel32.dll", 4000),
                (r"Windows\Logs\CBS\c.log", 60),
                (r"Games\Steam\steamapps\common\Spiel Eins\game.pak", 100000),
                (r"$Recycle.Bin\S-1-5-21\$R123.mp4", 1234),
                (r"hiberfil.sys", 6400),
                (r"pagefile.sys", 3200),
                (r"Daten\kaputt.dmp", 12),
                (r"Daten\vm.vhdx", 555),
            ],
        )
    }

    #[test]
    fn every_rule_speaks_both_languages() {
        use crate::i18n::Lang::{De, En};
        let names = ["node_modules", "System"];
        for rule in RULES {
            for text in [&rule.title, &rule.why, &rule.group] {
                assert!(!text.get(En).is_empty() && !text.get(De).is_empty(), "{}", rule.id);
                let umlaut = |s: &str| s.contains(['ä', 'ö', 'ü', 'Ä', 'Ö', 'Ü', 'ß']);
                assert!(!umlaut(text.get(En)), "{}: {}", rule.id, text.get(En));
            }
            assert!(rule.why.get(En) != rule.why.get(De), "{}", rule.id);
            assert!(names.contains(&rule.title.get(En)) || rule.title.get(En) != rule.title.get(De), "{}", rule.id);
        }
    }

    #[test]
    fn matches_the_expected_rules() {
        let ix = sample();
        let rep = analyze(&ix);
        assert_eq!(hit(&rep, "user-temp").alloc, 150);
        assert_eq!(hit(&rep, "win-temp").alloc, 40);
        assert_eq!(hit(&rep, "browser").alloc, 700);
        assert_eq!(hit(&rep, "app-cache").alloc, 33);
        assert_eq!(hit(&rep, "node-modules").alloc, 500);
        assert_eq!(hit(&rep, "build-rust").alloc, 900);
        assert_eq!(hit(&rep, "venv").alloc, 301);
        assert_eq!(hit(&rep, "pycache").alloc, 11);
        assert_eq!(hit(&rep, "win-logs").alloc, 60);
        assert_eq!(hit(&rep, "games").alloc, 100000);
        assert_eq!(hit(&rep, "recycle").alloc, 1234);
        assert_eq!(hit(&rep, "hibernate").alloc, 6400);
        assert_eq!(hit(&rep, "pagefile").alloc, 3200);
        assert_eq!(hit(&rep, "dumps").alloc, 12);
        assert_eq!(hit(&rep, "vm-disks").alloc, 555);
    }

    #[test]
    fn real_data_is_not_flagged() {
        let ix = sample();
        let rep = analyze(&ix);
        let flagged: std::collections::HashSet<u32> = rep.results.iter().flat_map(|r| r.items.iter().flat_map(|i| ix.subtree(*i))).collect();
        for keep in ["wichtig.txt", "foto.jpg", "wichtig.exe", "kernel32.dll"] {
            let id = (0..ix.nodes.len() as u32).find(|i| ix.name(*i) == keep).unwrap();
            assert!(!flagged.contains(&id), "{keep} darf nicht markiert sein");
        }
    }

    #[test]
    fn contents_rules_list_children_and_keep_folder() {
        let ix = sample();
        let rep = analyze(&ix);
        let names: Vec<String> = hit(&rep, "user-temp").items.iter().map(|i| ix.name(*i).to_string()).collect();
        assert!(names.contains(&"a.tmp".to_string()) && names.contains(&"sub".to_string()));
        assert!(!names.contains(&"Temp".to_string()));
    }

    #[test]
    fn age_conditions_apply() {
        let ix = sample();
        let rep = analyze(&ix);
        // Testdateien haben Datum 0 und gelten damit als alt.
        assert_eq!(hit(&rep, "old-installers").alloc, 9000);
        let mut fresh = sample();
        let now = now_secs();
        for n in fresh.nodes.iter_mut() {
            n.mtime = now;
        }
        let rep = analyze(&fresh);
        assert_eq!(hit(&rep, "old-installers").alloc, 0);
        assert_eq!(hit(&rep, "logs").alloc, 0);
    }

    #[test]
    fn finds_empty_folders_only_outside_system() {
        let ix = sample();
        let rep = analyze(&ix);
        let names: Vec<String> = rep.empty_dirs.iter().map(|i| ix.path(*i)).collect();
        assert_eq!(names, vec![r"C:\Users\Demo\Leer".to_string()]);
    }

    #[test]
    fn every_item_a_rule_offers_for_deletion_passes_the_guard_or_is_info() {
        // Regeln mit Aktion oder Info-Stufe werden nicht ueber die Aufraeumliste geloescht,
        // alle anderen muessen vom Schutz durchgelassen werden.
        let ix = sample();
        let rep = analyze(&ix);
        for (rule, res) in RULES.iter().zip(&rep.results) {
            if rule.risk == Info || rule.action != Action::None {
                continue;
            }
            for i in &res.items {
                let p = ix.path(*i);
                assert!(
                    !matches!(guard::check_windows(&p, r"c:\windows", "c:"), Guard::Block(_)),
                    "Regel {} bietet gesperrten Pfad an: {p}",
                    rule.id
                );
            }
        }
    }

    fn linux_sample() -> Index {
        build(
            "/",
            &[
                ("home/demo/.local/share/Trash/files/alt.mp4", 1234),
                ("home/demo/.cache/thumbnails/normal/a.png", 10),
                ("home/demo/.cache/google-chrome/Default/Cache/x", 700),
                ("home/demo/.cache/pip/http/y", 20),
                ("home/demo/.cache/huggingface/hub/z.bin", 9000),
                ("home/demo/.cache/irgendwas/cache.bin", 33),
                ("home/demo/.var/app/org.demo.App/cache/c", 44),
                ("home/demo/Downloads/setup.deb", 5000),
                ("home/demo/Dokumente/wichtig.deb", 8000),
                ("home/demo/Code/app/node_modules/p/i.js", 500),
                ("home/demo/Code/rust/Cargo.toml", 1),
                ("home/demo/Code/rust/target/debug/app", 900),
                ("home/demo/.local/share/Steam/steamapps/common/Spiel/game.pak", 100000),
                ("var/cache/apt/archives/a.deb", 300),
                ("var/lib/systemd/coredump/core.1", 77),
                ("var/log/journal/abc/system.journal", 888),
                ("var/log/syslog.1.gz", 66),
                ("var/lib/docker/overlay2/x", 4000),
                ("var/lib/snapd/snaps/core.snap", 3000),
                ("var/tmp/rest.bin", 55),
                ("usr/lib/libc.so", 4000),
                ("swapfile", 6400),
                (".Trash-1000/files/x.bin", 11),
            ],
        )
    }

    #[test]
    fn linux_rules_match() {
        let ix = linux_sample();
        let rep = analyze(&ix);
        assert_eq!(hit(&rep, "trash").alloc, 1234);
        assert_eq!(hit(&rep, "trash-volume").alloc, 11);
        assert_eq!(hit(&rep, "thumbs-linux").alloc, 10);
        assert_eq!(hit(&rep, "browser-linux").alloc, 700);
        assert_eq!(hit(&rep, "pkg-cache").alloc, 20);
        assert_eq!(hit(&rep, "ml-models").alloc, 9000);
        assert_eq!(hit(&rep, "user-cache").alloc, 33);
        assert_eq!(hit(&rep, "flatpak-cache").alloc, 44);
        assert_eq!(hit(&rep, "old-installers").alloc, 5000);
        assert_eq!(hit(&rep, "node-modules").alloc, 500);
        assert_eq!(hit(&rep, "build-rust").alloc, 900);
        assert_eq!(hit(&rep, "games").alloc, 100000);
        assert_eq!(hit(&rep, "pkg-downloads").alloc, 300);
        assert_eq!(hit(&rep, "coredumps").alloc, 77);
        assert_eq!(hit(&rep, "journal").alloc, 888);
        assert_eq!(hit(&rep, "rotated-logs").alloc, 66);
        assert_eq!(hit(&rep, "container-store").alloc, 4000);
        assert_eq!(hit(&rep, "snap-store").alloc, 3000);
        assert_eq!(hit(&rep, "var-tmp").alloc, 55);
        assert_eq!(hit(&rep, "swap-linux").alloc, 6400);
    }

    #[test]
    fn linux_rules_are_anchored_and_spare_real_data() {
        // Ein eigener Ordner namens var/tmp irgendwo im Profil ist kein Systemordner.
        let ix = build("/", &[("home/demo/Projekt/var/tmp/daten.bin", 500), ("home/demo/Dokumente/wichtig.deb", 8000)]);
        let rep = analyze(&ix);
        assert_eq!(hit(&rep, "var-tmp").alloc, 0);
        assert_eq!(hit(&rep, "old-installers").alloc, 0);
    }

    #[test]
    fn linux_offers_nothing_the_guard_would_refuse() {
        let ix = linux_sample();
        let rep = analyze(&ix);
        for (rule, res) in RULES.iter().zip(&rep.results) {
            if rule.risk == Info || rule.action != Action::None {
                continue;
            }
            for i in &res.items {
                let p = ix.path(*i);
                assert!(!matches!(guard::check_unix(&p, "/home/demo"), Guard::Block(_)), "Regel {} bietet gesperrten Pfad an: {p}", rule.id);
            }
        }
    }
}
