//! Die Oberflaeche steckt komplett im Programm. Mit `--ui-dir` wird sie beim Entwickeln von der Platte gelesen.

macro_rules! asset {
    ($p:literal) => {
        ($p, include_bytes!(concat!("../ui/", $p)) as &[u8])
    };
}

const ASSETS: &[(&str, &[u8])] = &[
    asset!("index.html"),
    asset!("style.css"),
    asset!("js/app.js"),
    asset!("js/i18n.js"),
    asset!("js/util.js"),
    asset!("js/charts.js"),
    asset!("js/treemap.js"),
    asset!("js/sunburst.js"),
    asset!("js/views.js"),
    asset!("js/views2.js"),
    asset!("js/views3.js"),
    asset!("favicon.svg"),
];

pub fn get(path: &str) -> Option<&'static [u8]> {
    ASSETS.iter().find(|(p, _)| *p == path).map(|(_, b)| *b)
}

pub fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::category;
    use crate::i18n::Lang;
    use std::collections::HashSet;

    fn text(path: &str) -> &'static str {
        std::str::from_utf8(get(path).unwrap_or_else(|| panic!("{path} is not registered"))).unwrap()
    }

    fn scripts() -> Vec<&'static str> {
        ASSETS.iter().filter(|(p, _)| p.ends_with(".js") && *p != "js/i18n.js").map(|(p, _)| text(p)).collect()
    }

    /// Alle Schluessel des Woerterbuchs: Zeilen der Form `'schluessel': [`.
    fn defined() -> Vec<&'static str> {
        text("js/i18n.js").lines().filter_map(|l| l.trim_start().strip_prefix('\'')?.split_once("': [").map(|(k, _)| k)).collect()
    }

    /// Schluessel aus Aufrufen wie `t('a.b')`. Bei `t(`a.${x}`)` kommt der feste Anfang heraus.
    fn used(src: &str) -> (Vec<&str>, Vec<&str>) {
        let (mut fixed, mut prefixes) = (vec![], vec![]);
        for call in ["t(", "tn(", "tp("] {
            for (i, _) in src.match_indices(call) {
                let before = src[..i].chars().next_back().unwrap_or(' ');
                if before.is_alphanumeric() || matches!(before, '_' | '.' | '$') {
                    continue;
                }
                let rest = &src[i + call.len()..];
                if let Some(r) = rest.strip_prefix('\'') {
                    fixed.push(r.split('\'').next().unwrap());
                } else if let Some(r) = rest.strip_prefix('`') {
                    prefixes.push(r.split("${").next().unwrap());
                }
            }
        }
        (fixed, prefixes)
    }

    #[test]
    fn every_text_key_exists_and_is_used() {
        let dict = defined();
        assert!(dict.len() > 300, "{}", dict.len());
        let known: HashSet<&str> = dict.iter().copied().collect();
        assert_eq!(known.len(), dict.len(), "a key is defined twice");
        let (mut fixed, mut prefixes) = (HashSet::new(), HashSet::new());
        for src in scripts() {
            let (f, p) = used(src);
            fixed.extend(f);
            prefixes.extend(p);
        }
        for k in &fixed {
            assert!(known.contains(k), "missing in the dictionary: {k}");
        }
        for p in &prefixes {
            assert!(dict.iter().any(|k| k.starts_with(p)), "no key starts with: {p}");
        }
        // Ein Schluessel kann auch als Daten im Code stehen, wie die Eintraege der Navigation.
        for k in &dict {
            let in_code = scripts().iter().any(|src| src.contains(&format!("'{k}'")));
            assert!(in_code || prefixes.iter().any(|p| k.starts_with(p)), "unused key: {k}");
        }
    }

    #[test]
    fn no_german_text_outside_the_dictionary() {
        for src in scripts() {
            for line in src.lines() {
                let code = line.trim_start();
                if code.starts_with("//") {
                    continue;
                }
                let code = code.split(" // ").next().unwrap();
                assert!(!code.contains(['ä', 'ö', 'ü', 'Ä', 'Ö', 'Ü', 'ß', '„', '“']), "{code}");
            }
        }
    }

    #[test]
    fn category_names_match_the_dictionary() {
        let dict = text("js/i18n.js");
        for (i, name) in category::NAMES.iter().enumerate() {
            let line = format!("'cat.{i}': ['{}', '{}'],", name.get(Lang::En), name.get(Lang::De));
            assert!(dict.contains(&line), "{line}");
        }
    }
}
