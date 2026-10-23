//! Translation target languages and the codes each translation service expects.

pub struct Language {
    /// Code stored in the config.
    pub code: &'static str,
    /// English name, also used in the instruction to AI models.
    pub name: &'static str,
    /// DeepL `target_lang`; `None` where DeepL has no such target.
    pub deepl: Option<&'static str>,
    /// Microsoft Translator `to`.
    pub microsoft: &'static str,
    /// Google Cloud Translation `target`.
    pub google: &'static str,
}

const fn lang(
    code: &'static str,
    name: &'static str,
    deepl: Option<&'static str>,
    microsoft: &'static str,
    google: &'static str,
) -> Language {
    Language { code, name, deepl, microsoft, google }
}

/// Sorted by English name, the order shown in Settings.
pub const LANGUAGES: &[Language] = &[
    lang("ar", "Arabic", Some("AR"), "ar", "ar"),
    lang("bg", "Bulgarian", Some("BG"), "bg", "bg"),
    lang("zh-Hans", "Chinese (Simplified)", Some("ZH-HANS"), "zh-Hans", "zh-CN"),
    lang("zh-Hant", "Chinese (Traditional)", Some("ZH-HANT"), "zh-Hant", "zh-TW"),
    lang("cs", "Czech", Some("CS"), "cs", "cs"),
    lang("da", "Danish", Some("DA"), "da", "da"),
    lang("nl", "Dutch", Some("NL"), "nl", "nl"),
    lang("en", "English", Some("EN-US"), "en", "en"),
    lang("et", "Estonian", Some("ET"), "et", "et"),
    lang("fi", "Finnish", Some("FI"), "fi", "fi"),
    lang("fr", "French", Some("FR"), "fr", "fr"),
    lang("de", "German", Some("DE"), "de", "de"),
    lang("el", "Greek", Some("EL"), "el", "el"),
    lang("he", "Hebrew", Some("HE"), "he", "he"),
    lang("hi", "Hindi", None, "hi", "hi"),
    lang("hu", "Hungarian", Some("HU"), "hu", "hu"),
    lang("id", "Indonesian", Some("ID"), "id", "id"),
    lang("it", "Italian", Some("IT"), "it", "it"),
    lang("ja", "Japanese", Some("JA"), "ja", "ja"),
    lang("kk", "Kazakh", None, "kk", "kk"),
    lang("ko", "Korean", Some("KO"), "ko", "ko"),
    lang("lv", "Latvian", Some("LV"), "lv", "lv"),
    lang("lt", "Lithuanian", Some("LT"), "lt", "lt"),
    lang("nb", "Norwegian", Some("NB"), "nb", "no"),
    lang("fa", "Persian", None, "fa", "fa"),
    lang("pl", "Polish", Some("PL"), "pl", "pl"),
    lang("pt", "Portuguese", Some("PT-BR"), "pt", "pt"),
    lang("ro", "Romanian", Some("RO"), "ro", "ro"),
    lang("ru", "Russian", Some("RU"), "ru", "ru"),
    lang("sk", "Slovak", Some("SK"), "sk", "sk"),
    lang("sl", "Slovenian", Some("SL"), "sl", "sl"),
    lang("es", "Spanish", Some("ES"), "es", "es"),
    lang("sv", "Swedish", Some("SV"), "sv", "sv"),
    lang("th", "Thai", Some("TH"), "th", "th"),
    lang("tr", "Turkish", Some("TR"), "tr", "tr"),
    lang("uk", "Ukrainian", Some("UK"), "uk", "uk"),
    lang("vi", "Vietnamese", Some("VI"), "vi", "vi"),
];

pub fn find(code: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| l.code.eq_ignore_ascii_case(code))
}

pub fn index_of(code: &str) -> Option<usize> {
    LANGUAGES.iter().position(|l| l.code.eq_ignore_ascii_case(code))
}

/// Picks the target from a Windows locale name such as "ru-RU" or "zh-Hant-TW";
/// falls back to English.
pub fn target_for_locale(locale: &str) -> &'static str {
    let lower = locale.to_ascii_lowercase();
    if lower.starts_with("zh") {
        let traditional = ["-hant", "-tw", "-hk", "-mo"].iter().any(|t| lower.contains(t));
        return if traditional { "zh-Hant" } else { "zh-Hans" };
    }
    let primary = lower.split('-').next().unwrap_or("");
    let primary = if primary == "no" || primary == "nn" { "nb" } else { primary };
    find(primary).map_or("en", |l| l.code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_is_sorted_and_unique() {
        for pair in LANGUAGES.windows(2) {
            assert!(pair[0].name < pair[1].name, "{} before {}", pair[0].name, pair[1].name);
        }
        let mut codes: Vec<_> = LANGUAGES.iter().map(|l| l.code).collect();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), LANGUAGES.len());
    }

    #[test]
    fn locale_maps_to_target() {
        assert_eq!(target_for_locale("ru-RU"), "ru");
        assert_eq!(target_for_locale("en-GB"), "en");
        assert_eq!(target_for_locale("zh-CN"), "zh-Hans");
        assert_eq!(target_for_locale("zh-Hant-TW"), "zh-Hant");
        assert_eq!(target_for_locale("nn-NO"), "nb");
        assert_eq!(target_for_locale("xx-YY"), "en");
        assert_eq!(target_for_locale(""), "en");
    }
}
