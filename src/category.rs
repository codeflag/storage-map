//! Ordnet Dateiendungen genau einer Kategorie zu. Acht Kategorien plus "Sonstige":
//! mehr Farben kann man in einer Karte nicht mehr auseinanderhalten.

pub const COUNT: usize = 9;
pub const OTHER: u8 = 8;

pub const NAMES: [&str; COUNT] =
    ["Videos", "Bilder", "Audio", "Dokumente", "Archive & Abbilder", "Programme & Spiele", "Entwicklung & Daten", "System & Temp", "Sonstige"];

/// Feste Kennungen fuer Schnittstellen. Die Namen oben sind fuer Menschen und koennen sich aendern.
pub const SLUGS: [&str; COUNT] = ["video", "image", "audio", "document", "archive", "program", "development", "system", "other"];

const VIDEO: &[&str] = &[
    "mp4", "mkv", "avi", "mov", "wmv", "flv", "webm", "m4v", "mpg", "mpeg", "m2ts", "mts", "vob", "3gp", "ogv", "divx", "rmvb", "asf", "braw", "r3d",
    "prores",
];
const IMAGE: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "bmp", "tif", "tiff", "webp", "heic", "heif", "raw", "cr2", "cr3", "nef", "arw", "dng", "orf", "rw2", "psd", "psb",
    "ai", "svg", "ico", "exr", "hdr", "tga", "kra", "xcf", "indd", "eps", "avif", "blend", "fig",
];
const AUDIO: &[&str] = &["mp3", "flac", "wav", "aac", "ogg", "m4a", "wma", "opus", "aiff", "aif", "ape", "alac", "mid", "midi", "m4b"];
const DOCS: &[&str] = &[
    "pdf", "doc", "docx", "docm", "odt", "rtf", "txt", "md", "xls", "xlsx", "xlsm", "ods", "csv", "ppt", "pptx", "odp", "epub", "mobi", "azw3",
    "pages", "numbers", "key", "tex", "one", "msg", "eml", "pst", "ost", "vsd", "vsdx", "djvu", "cbz", "cbr",
];
const ARCHIVE: &[&str] = &[
    "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "iso", "img", "dmg", "cab", "wim", "esd", "vhd", "vhdx", "vmdk", "vdi", "qcow2",
    "ova", "ovf", "tib", "bkf", "gho", "lz4", "arj", "avhdx", "vbox",
];
const PROGRAM: &[&str] = &[
    "exe",
    "dll",
    "msi",
    "msix",
    "msixbundle",
    "appx",
    "appxbundle",
    "msp",
    "ocx",
    "drv",
    "cpl",
    "scr",
    "bat",
    "cmd",
    "ps1",
    "com",
    "jar",
    "pak",
    "pck",
    "vpk",
    "unity3d",
    "uasset",
    "umap",
    "uexp",
    "ubulk",
    "forge",
    "wad",
    "gcf",
    "ncm",
    "bundle",
    "assets",
    "resource",
    "res",
    "rpf",
    "psarc",
    "nsp",
    "xci",
    "nsz",
    "sav",
];
const DEV: &[&str] = &[
    "py",
    "pyc",
    "pyd",
    "js",
    "mjs",
    "cjs",
    "jsx",
    "ts",
    "tsx",
    "java",
    "class",
    "c",
    "cc",
    "cpp",
    "h",
    "hpp",
    "cs",
    "rs",
    "rlib",
    "rmeta",
    "go",
    "rb",
    "php",
    "html",
    "htm",
    "css",
    "scss",
    "less",
    "json",
    "jsonl",
    "xml",
    "yaml",
    "yml",
    "toml",
    "sql",
    "db",
    "sqlite",
    "sqlite3",
    "mdb",
    "accdb",
    "parquet",
    "feather",
    "arrow",
    "h5",
    "hdf5",
    "npy",
    "npz",
    "pkl",
    "pickle",
    "pt",
    "pth",
    "ckpt",
    "safetensors",
    "gguf",
    "onnx",
    "pb",
    "tflite",
    "lock",
    "map",
    "wasm",
    "o",
    "obj",
    "a",
    "lib",
    "pdb",
    "ipynb",
    "sln",
    "csproj",
    "vcxproj",
    "ilk",
    "idb",
    "ipdb",
    "iobj",
    "ldf",
    "mdf",
    "bson",
    "rda",
    "rds",
    "dta",
    "sav2",
    "tsv",
];
const SYSTEM: &[&str] = &[
    "tmp",
    "temp",
    "log",
    "etl",
    "dmp",
    "mdmp",
    "sys",
    "cat",
    "mui",
    "manifest",
    "nls",
    "ttf",
    "otf",
    "ttc",
    "fon",
    "evtx",
    "regtrans-ms",
    "blf",
    "cache",
    "bak",
    "old",
    "swp",
    "lnk",
    "mum",
    "edb",
    "jfm",
    "chk",
    "pf",
    "hve",
    "ini",
    "inf",
    "ax",
    "tlb",
    "winmd",
    "ni",
    "cdf-ms",
    "evt",
    "wer",
    "trace",
    "crdownload",
    "part",
];

pub fn classify(ext: &str) -> u8 {
    const TABLE: [&[&str]; 8] = [VIDEO, IMAGE, AUDIO, DOCS, ARCHIVE, PROGRAM, DEV, SYSTEM];
    for (i, list) in TABLE.iter().enumerate() {
        if list.contains(&ext) {
            return i as u8;
        }
    }
    OTHER
}

/// `.ts` ist beides: TypeScript-Quelltext und MPEG-Videostream. Quelltext wird nie so gross.
pub fn classify_sized(ext: &str, size: u64) -> u8 {
    if ext == "ts" && size >= 16 * 1024 * 1024 {
        return 0;
    }
    classify(ext)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_is_code_unless_it_is_huge() {
        assert_eq!(NAMES[classify_sized("ts", 3_000) as usize], "Entwicklung & Daten");
        assert_eq!(NAMES[classify_sized("ts", 2_000_000_000) as usize], "Videos");
        assert_eq!(NAMES[classify_sized("mkv", 10) as usize], "Videos");
    }

    #[test]
    fn classifies_common_types() {
        assert_eq!(NAMES[classify("mkv") as usize], "Videos");
        assert_eq!(NAMES[classify("heic") as usize], "Bilder");
        assert_eq!(NAMES[classify("flac") as usize], "Audio");
        assert_eq!(NAMES[classify("pdf") as usize], "Dokumente");
        assert_eq!(NAMES[classify("vhdx") as usize], "Archive & Abbilder");
        assert_eq!(NAMES[classify("exe") as usize], "Programme & Spiele");
        assert_eq!(NAMES[classify("safetensors") as usize], "Entwicklung & Daten");
        assert_eq!(NAMES[classify("log") as usize], "System & Temp");
        assert_eq!(classify("voellig-unbekannt"), OTHER);
    }

    #[test]
    fn no_extension_is_in_two_categories() {
        let all = [VIDEO, IMAGE, AUDIO, DOCS, ARCHIVE, PROGRAM, DEV, SYSTEM];
        let mut seen = std::collections::HashSet::new();
        for list in all {
            for e in list {
                assert!(seen.insert(*e), "Endung doppelt vergeben: {e}");
            }
        }
    }
}
