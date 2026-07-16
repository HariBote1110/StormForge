// Minimal i18n: the Electron app's locale files are embedded at compile time and
// looked up by key, with English as the fallback and the key itself as a last resort.
// A handful of labels that only exist in the Rust app are provided as extras.

use std::collections::HashMap;

// Reuses the Electron app's locale files so wording stays identical between versions.
const EN_JSON: &str = include_str!("../../../../locales/en.json");
const JA_JSON: &str = include_str!("../../../../locales/ja.json");

pub struct I18n {
    lang: String,
    map: HashMap<String, String>,
    fallback: HashMap<String, String>,
}

fn parse(json: &str) -> HashMap<String, String> {
    serde_json::from_str(json).unwrap_or_default()
}

/// Labels the Rust app needs that have no key in the Electron locale files.
fn extras(lang: &str) -> HashMap<String, String> {
    let pairs: &[(&str, &str)] = if lang == "ja" {
        &[
            ("LOADED_BADGE", "適用中"),
            ("EXPORT", "エクスポート"),
            ("COPY", "コピー"),
            ("REPAIR_FULL_REBUILD", "修復（フル再構築）"),
        ]
    } else {
        &[
            ("LOADED_BADGE", "Loaded"),
            ("EXPORT", "Export"),
            ("COPY", "Copy"),
            ("REPAIR_FULL_REBUILD", "Repair (full rebuild)"),
        ]
    };
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

impl I18n {
    pub fn new(lang: &str) -> Self {
        let lang = if lang == "ja" { "ja" } else { "en" };
        let source = if lang == "ja" { JA_JSON } else { EN_JSON };
        let mut map = parse(source);
        map.extend(extras(lang));
        let mut fallback = parse(EN_JSON);
        fallback.extend(extras("en"));
        Self { lang: lang.to_string(), map, fallback }
    }

    pub fn lang(&self) -> &str {
        &self.lang
    }

    pub fn t(&self, key: &str) -> String {
        self.map
            .get(key)
            .or_else(|| self.fallback.get(key))
            .cloned()
            .unwrap_or_else(|| key.to_string())
    }
}
