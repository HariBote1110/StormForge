//! Parses a mod's `Metadata.xml`, mirroring the `xml2js`-based parsing in
//! `src/main/ipcHandlers.js` (`add-mod` handler): a flat `<Metadata><Author/><Version/></Metadata>`
//! document, with `"Unknown"` used for either field when missing.

use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// Parsed mod metadata. Defaults to `"Unknown"` for either field, matching the Electron
/// behaviour when `Metadata.xml` is absent or a field is missing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModMetadata {
    pub author: String,
    pub version: String,
}

impl Default for ModMetadata {
    fn default() -> Self {
        Self { author: "Unknown".to_string(), version: "Unknown".to_string() }
    }
}

/// Parse `Metadata.xml` contents. Malformed XML or a missing document both fall back to
/// `ModMetadata::default()` rather than erroring, matching the JS handler's behaviour of
/// only special-casing "the file doesn't exist" and otherwise trusting the parse.
pub fn parse_metadata_xml(xml: &str) -> ModMetadata {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut metadata = ModMetadata::default();
    let mut current_tag: Option<String> = None;
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                current_tag = Some(String::from_utf8_lossy(e.name().as_ref()).to_string());
            }
            Ok(Event::Text(e)) => {
                if let Some(tag) = &current_tag {
                    let text = e.unescape().unwrap_or_default().to_string();
                    match tag.as_str() {
                        "Author" => metadata.author = text,
                        "Version" => metadata.version = text,
                        _ => {}
                    }
                }
            }
            Ok(Event::End(_)) => current_tag = None,
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    metadata
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_author_and_version() {
        let xml = r#"<Metadata><Author>Someone</Author><Version>1.2.3</Version></Metadata>"#;
        let metadata = parse_metadata_xml(xml);
        assert_eq!(metadata.author, "Someone");
        assert_eq!(metadata.version, "1.2.3");
    }

    #[test]
    fn defaults_to_unknown_for_missing_fields() {
        let xml = r#"<Metadata><Author>Someone</Author></Metadata>"#;
        let metadata = parse_metadata_xml(xml);
        assert_eq!(metadata.author, "Someone");
        assert_eq!(metadata.version, "Unknown");
    }

    #[test]
    fn defaults_fully_on_garbage_input() {
        let metadata = parse_metadata_xml("not xml at all");
        assert_eq!(metadata, ModMetadata::default());
    }
}
