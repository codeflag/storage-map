//! Texts in two languages. The backend always carries both versions. The language is picked at the interface:
//! The window names its language with `?lang=`, everything else (MCP, command line) gets English.

use std::borrow::Cow;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    En,
    De,
}

impl Lang {
    /// The language a request asked for. Without a known value it is English.
    pub fn of_query(m: &HashMap<String, String>) -> Lang {
        match m.get("lang").map(|s| s.as_str()) {
            Some("de") => Lang::De,
            _ => Lang::En,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    en: Cow<'static, str>,
    de: Cow<'static, str>,
}

impl Text {
    pub const fn new(en: &'static str, de: &'static str) -> Text {
        Text { en: Cow::Borrowed(en), de: Cow::Borrowed(de) }
    }

    pub fn owned(en: String, de: String) -> Text {
        Text { en: Cow::Owned(en), de: Cow::Owned(de) }
    }

    /// Text without a translation, for example a message from the operating system.
    pub fn same(s: impl Into<String>) -> Text {
        let s = s.into();
        Text { en: Cow::Owned(s.clone()), de: Cow::Owned(s) }
    }

    pub fn get(&self, lang: Lang) -> &str {
        match lang {
            Lang::En => &self.en,
            Lang::De => &self.de,
        }
    }

    pub fn en(&self) -> &str {
        &self.en
    }
}

/// Builds a text with values in both languages: `tr!("Not readable: {e}", "Nicht lesbar: {e}")`.
#[macro_export]
macro_rules! tr {
    ($en:literal, $de:literal) => {
        $crate::i18n::Text::owned(format!($en), format!($de))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_is_the_default() {
        let q = |v: &str| HashMap::from([("lang".to_string(), v.to_string())]);
        assert_eq!(Lang::of_query(&HashMap::new()), Lang::En);
        assert_eq!(Lang::of_query(&q("de")), Lang::De);
        assert_eq!(Lang::of_query(&q("en")), Lang::En);
        assert_eq!(Lang::of_query(&q("fr")), Lang::En);
    }

    #[test]
    fn text_picks_the_language() {
        const T: Text = Text::new("Yes", "Ja");
        assert_eq!(T.get(Lang::En), "Yes");
        assert_eq!(T.get(Lang::De), "Ja");
        let n = 3;
        let t = tr!("{n} files", "{n} Dateien");
        assert_eq!(t.en(), "3 files");
        assert_eq!(t.get(Lang::De), "3 Dateien");
    }
}
