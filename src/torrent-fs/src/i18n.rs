pub const LOCALES: &[(&str, &str)] = &[
    ("en", include_str!("../locales/en.json")),
    ("ru", include_str!("../locales/ru.json")),
    ("pl", include_str!("../locales/pl.json")),
    ("cs", include_str!("../locales/cs.json")),
    ("sk", include_str!("../locales/sk.json")),
    ("de", include_str!("../locales/de.json")),
    ("es", include_str!("../locales/es.json")),
    ("uk", include_str!("../locales/uk.json")),
    ("it", include_str!("../locales/it.json")),
    ("fr", include_str!("../locales/fr.json")),
    ("ro", include_str!("../locales/ro.json")),
    ("hu", include_str!("../locales/hu.json")),
    ("be", include_str!("../locales/be.json")),
    ("bg", include_str!("../locales/bg.json")),
    ("sr", include_str!("../locales/sr.json")),
];

type Catalogue = std::collections::HashMap<String, std::collections::HashMap<String, String>>;

fn catalogues() -> &'static Catalogue {
    static PARSED: std::sync::OnceLock<Catalogue> = std::sync::OnceLock::new();
    PARSED.get_or_init(|| {
        LOCALES
            .iter()
            .filter_map(|(language, raw)| {
                serde_json::from_str(raw)
                    .ok()
                    .map(|table| ((*language).to_string(), table))
            })
            .collect()
    })
}

static SPOKEN: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Set once at init: the host does not change language without a restart.
pub fn speaks(language: &str) {
    let _ = SPOKEN.set(language.to_string());
}

pub fn language() -> &'static str {
    SPOKEN.get().map(String::as_str).unwrap_or("en")
}

pub fn phrase(language: &str, key: &str) -> String {
    let tables = catalogues();
    tables
        .get(language)
        .and_then(|table| table.get(key))
        .or_else(|| tables.get("en").and_then(|table| table.get(key)))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

pub fn tr(key: &str) -> String {
    phrase(language(), key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_fifteen_languages_are_bundled() {
        assert_eq!(LOCALES.len(), 15);
        assert_eq!(catalogues().len(), 15);
    }

    #[test]
    fn each_catalogue_answers_for_the_same_keys() {
        let english: std::collections::BTreeSet<&String> =
            catalogues().get("en").expect("english").keys().collect();
        assert!(!english.is_empty());
        for (language, table) in catalogues() {
            let theirs: std::collections::BTreeSet<&String> = table.keys().collect();
            assert_eq!(theirs, english, "`{language}` does not match english");
        }
    }

    #[test]
    fn a_phrase_is_taken_from_the_language_being_spoken() {
        assert_eq!(phrase("en", "torrent.start"), "Fetch the selected files");
        assert_eq!(phrase("ru", "torrent.start"), "Качать выбранные файлы");
    }

    #[test]
    fn a_language_nobody_shipped_falls_back_to_english() {
        assert_eq!(phrase("xx", "torrent.start"), "Fetch the selected files");
    }

    #[test]
    fn a_key_with_nothing_behind_it_is_its_own_text() {
        assert_eq!(phrase("en", "torrent.nothing"), "torrent.nothing");
    }

    #[test]
    fn nothing_was_said_yet_means_english() {
        assert!(!language().is_empty());
    }
}
