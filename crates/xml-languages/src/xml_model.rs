//! Extraction of `<?xml-model?>` processing instructions (W3C
//! "Associating Schemas with XML documents").
//!
//! Example: `<?xml-model href="schema.rng" type="application/xml"
//! schematypens="http://relaxng.org/ns/structure/1.0"?>`

use xml_ast::{Document, NodeKind};
use xml_schema::RNG_NAMESPACE;

/// Returns the `href` of the first `xml-model` PI that plausibly points
/// to a RELAX NG (XML syntax) schema.
pub(crate) fn relaxng_model_href(doc: &Document, text: &str) -> Option<String> {
    for &child in doc.children(doc.root()) {
        let NodeKind::ProcessingInstruction { target } = doc.kind(child) else {
            continue;
        };
        if target != "xml-model" {
            continue;
        }
        let range = doc.range(child);
        // Inner content between `<?xml-model` and `?>`.
        let start = (range.start + 2 + target.len()).min(range.end);
        let end = range.end.saturating_sub(2).max(start);
        let attrs = pseudo_attributes(&text[start..end]);

        let href = attrs
            .iter()
            .find(|(name, _)| name == "href")
            .map(|(_, value)| value.clone());
        let Some(href) = href else { continue };

        // Only RELAX NG XML syntax is supported: accept if schematypens
        // says so, or if nothing contradicts it and the file ends in .rng.
        let schematypens = attrs
            .iter()
            .find(|(name, _)| name == "schematypens")
            .map(|(_, value)| value.as_str());
        let is_relaxng = match schematypens {
            Some(ns) => ns == RNG_NAMESPACE,
            None => href.ends_with(".rng"),
        };
        if is_relaxng && !href.ends_with(".rnc") {
            return Some(href);
        }
    }
    None
}

/// Parses PI pseudo-attributes (`name="value"` pairs).
fn pseudo_attributes(src: &str) -> Vec<(String, String)> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let name_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        let name = &src[name_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || (bytes[i] != b'"' && bytes[i] != b'\'') {
            continue;
        }
        let quote = bytes[i];
        i += 1;
        let value_start = i;
        while i < bytes.len() && bytes[i] != quote {
            i += 1;
        }
        out.push((name.to_string(), src[value_start..i].to_string()));
        if i < bytes.len() {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn href(text: &str) -> Option<String> {
        let parse = xml_parser::parse(text);
        relaxng_model_href(&parse.document, text)
    }

    #[test]
    fn finds_relaxng_model() {
        assert_eq!(
            href(r#"<?xml-model href="layout.rng"?><root/>"#),
            Some("layout.rng".to_string())
        );
        assert_eq!(
            href(
                r#"<?xml-model href="s.xml" schematypens="http://relaxng.org/ns/structure/1.0"?><root/>"#
            ),
            Some("s.xml".to_string())
        );
    }

    #[test]
    fn ignores_other_schema_types() {
        // XSD model: not ours.
        assert_eq!(
            href(
                r#"<?xml-model href="s.xsd" schematypens="http://www.w3.org/2001/XMLSchema"?><root/>"#
            ),
            None
        );
        // Compact syntax: not supported yet.
        assert_eq!(href(r#"<?xml-model href="s.rnc"?><root/>"#), None);
        // No href at all.
        assert_eq!(href(r#"<?xml-model type="application/xml"?><root/>"#), None);
    }

    #[test]
    fn pseudo_attribute_parsing() {
        let attrs = pseudo_attributes(r#" href="a.rng" type='application/xml' "#);
        assert_eq!(
            attrs,
            vec![
                ("href".to_string(), "a.rng".to_string()),
                ("type".to_string(), "application/xml".to_string()),
            ]
        );
    }
}
