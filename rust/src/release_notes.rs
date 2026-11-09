//! Localized release notes, rendered as text-only Markdown without remote media.
use std::collections::BTreeMap;

#[derive(serde::Deserialize)]
struct HistoricalRelease {
    version: String,
    notes: BTreeMap<String, String>,
}

/// Reviewed history ships with the manager, so browsing old releases needs no network.
pub fn history(language: &str) -> Vec<(String, String)> {
    let entries: Vec<HistoricalRelease> =
        serde_json::from_str(include_str!("../assets/release-history.json"))
            .expect("bundled release history");
    entries
        .into_iter()
        .filter_map(|entry| {
            select(&entry.notes, language).map(|text| (entry.version, text.to_owned()))
        })
        .collect()
}

pub fn current(language: &str) -> String {
    let notes: BTreeMap<String, String> =
        serde_json::from_str(include_str!("../assets/release-notes.json"))
            .expect("bundled release notes");
    select(&notes, language).unwrap_or_default().to_owned()
}
pub fn select<'a>(notes: &'a BTreeMap<String, String>, language: &str) -> Option<&'a str> {
    [language, "en", "zh-CN"]
        .into_iter()
        .filter_map(|key| notes.get(key).map(String::as_str))
        .find(|s| !s.trim().is_empty())
}

/// Keep headings, lists and emphasis while disabling HTML and automatic images.
/// Link activation is separately restricted by the view to HTTPS browser URLs.
pub fn display_markdown(notes: &str) -> String {
    notes
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace("![", "[")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_keeps_structure_without_loading_remote_media() {
        let text = display_markdown(
            "## 更新\n- **修复** `DLL`\n![track](https://example.com/pixel)\n<img src=x>\n![ref][id]",
        );
        assert!(text.starts_with("## 更新\n- **修复** `DLL`"));
        assert!(!text.contains("!["));
        assert!(!text.contains('<'));
        assert!(text.contains("&lt;img src=x&gt;"));
    }
    #[test]
    fn notes_have_all_five_languages_and_legacy_fallback() {
        for language in ["zh-CN", "en", "ru", "ja", "ko"] {
            assert!(!current(language).is_empty());
        }
        assert!(select(&BTreeMap::new(), "zh-CN").is_none());
        let notes = BTreeMap::from([("en".into(), "English notes".into())]);
        assert_eq!(select(&notes, "ru"), Some("English notes"));
    }
    #[test]
    fn offline_history_is_unique_descending_and_localized() {
        let entries: Vec<HistoricalRelease> =
            serde_json::from_str(include_str!("../assets/release-history.json")).unwrap();
        assert!(!entries.is_empty());
        let mut previous = crate::updater::version(crate::VERSION).unwrap();
        for entry in entries {
            let version = crate::updater::version(&entry.version).unwrap();
            assert!(
                version < previous,
                "Historical versions must be older and unique"
            );
            previous = version;
            for language in ["zh-CN", "en", "ru", "ja", "ko"] {
                assert!(
                    entry
                        .notes
                        .get(language)
                        .is_some_and(|s| !s.trim().is_empty())
                );
            }
        }
    }
}
