//! Inlay hints: after the end tag of a long element, show the attribute
//! that identifies it (`</Pagetype> ⟸ name="left pages"`), so deeply
//! nested layout files stay navigable without scrolling up.

use tower_lsp::lsp_types::{InlayHint, InlayHintLabel, Position};
use xml_ast::{Document, LineIndex, NodeKind, TextRange};

/// Same attribute preference as the document outline.
const IDENTIFYING_ATTRIBUTES: [&str; 5] = ["name", "id", "element", "match", "mode"];

/// An element must span at least this many lines before its end tag gets
/// a hint; anything shorter is visible at a glance.
const MIN_LINES: u32 = 8;

/// Longer attribute values are cut off in the hint label.
const MAX_VALUE_LEN: usize = 40;

/// Hints for all element end tags inside `range`.
pub fn inlay_hints(
    doc: &Document,
    text: &str,
    line_index: &LineIndex,
    range: TextRange,
) -> Vec<InlayHint> {
    let mut hints = Vec::new();
    for node in doc.descendants(doc.root()) {
        let NodeKind::Element(element) = doc.kind(node) else {
            continue;
        };
        let Some(close) = element.close_tag else {
            continue;
        };
        if !range.contains_inclusive(close.end) {
            continue;
        }
        let open_line = line_index.line_col(text, element.open_tag.start).line;
        let close_pos = line_index.line_col(text, close.end);
        if close_pos.line - open_line < MIN_LINES {
            continue;
        }
        let Some(attr) = IDENTIFYING_ATTRIBUTES
            .iter()
            .find_map(|name| element.attribute(None, name))
        else {
            continue;
        };
        let mut value = attr.value.clone();
        if value.chars().count() > MAX_VALUE_LEN {
            value = value.chars().take(MAX_VALUE_LEN).collect::<String>() + "…";
        }
        hints.push(InlayHint {
            position: Position {
                line: close_pos.line,
                character: close_pos.col,
            },
            label: InlayHintLabel::String(format!("{}=\"{value}\"", attr.name.as_written())),
            kind: None,
            text_edits: None,
            tooltip: None,
            padding_left: Some(true),
            padding_right: None,
            data: None,
        });
    }
    hints
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hints(text: &str) -> Vec<InlayHint> {
        let parse = xml_parser::parse(text);
        let line_index = LineIndex::new(text);
        let all = TextRange::new(0, text.len());
        inlay_hints(&parse.document, text, &line_index, all)
    }

    fn long_element(name_attr: &str, lines: usize) -> String {
        format!("<Record {name_attr}>{}</Record>", "\n<Br/>".repeat(lines))
    }

    #[test]
    fn hint_after_long_element() {
        let text = long_element("element=\"data\"", 10);
        let hints = hints(&text);
        assert_eq!(hints.len(), 1);
        let InlayHintLabel::String(label) = &hints[0].label else {
            panic!("string label expected");
        };
        assert_eq!(label, "element=\"data\"");
        assert_eq!(hints[0].position.line, 10);
    }

    #[test]
    fn short_elements_get_no_hint() {
        let text = long_element("element=\"data\"", 3);
        assert!(hints(&text).is_empty());
    }

    #[test]
    fn elements_without_identifying_attribute_get_no_hint() {
        let text = long_element("class=\"x\"", 10);
        assert!(hints(&text).is_empty());
    }

    #[test]
    fn long_values_are_truncated() {
        let value = "x".repeat(60);
        let text = long_element(&format!("name=\"{value}\""), 10);
        let hints = hints(&text);
        let InlayHintLabel::String(label) = &hints[0].label else {
            panic!("string label expected");
        };
        assert!(label.chars().count() < 60);
        assert!(label.ends_with("…\""));
    }

    #[test]
    fn range_filters_hints() {
        let text = long_element("name=\"a\"", 10);
        let parse = xml_parser::parse(&text);
        let line_index = LineIndex::new(&text);
        // A range covering only the first line excludes the end tag.
        let hints = inlay_hints(&parse.document, &text, &line_index, TextRange::new(0, 10));
        assert!(hints.is_empty());
    }
}
