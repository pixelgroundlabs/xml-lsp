//! Rename: symbols (definition plus all references) and element tag
//! pairs (start and end tag stay in sync).

use xml_ast::{Document, TextRange};

use crate::navigation::symbol_index;

/// What a rename at a position would change: the exact range of the
/// current name and its text (the placeholder for the rename dialog).
pub struct RenameTarget {
    pub range: TextRange,
    pub text: String,
}

/// The renameable thing under the cursor. Symbol occurrences win over
/// tag names (they live in attribute values, so ranges never overlap).
pub fn prepare(doc: &Document, text: &str, offset: usize) -> Option<RenameTarget> {
    if let Some(index) = symbol_index(doc, text) {
        if let Some(occ) = index.occurrence_at(offset) {
            return Some(RenameTarget {
                range: occ.range,
                text: occ.name.clone(),
            });
        }
    }
    let (name_range, _) = tag_name_at(doc, text, offset)?;
    Some(RenameTarget {
        range: name_range,
        text: text.get(name_range.start..name_range.end)?.to_string(),
    })
}

/// Computes the text edits for a rename. Returns `None` when there is
/// nothing renameable at the offset or the new name is invalid.
pub fn rename(
    doc: &Document,
    text: &str,
    offset: usize,
    new_name: &str,
) -> Option<Vec<(TextRange, String)>> {
    if let Some(index) = symbol_index(doc, text) {
        if let Some(occ) = index.occurrence_at(offset) {
            if !is_valid_symbol_name(new_name) {
                return None;
            }
            return Some(
                index
                    .occurrences(&occ.kind, &occ.name, true)
                    .into_iter()
                    .map(|occ| (occ.range, new_name.to_string()))
                    .collect(),
            );
        }
    }

    let (start_name, end_name) = tag_name_at(doc, text, offset)?;
    if !is_valid_xml_name(new_name) {
        return None;
    }
    let mut edits = vec![(start_name, new_name.to_string())];
    if let Some(end_name) = end_name {
        edits.push((end_name, new_name.to_string()));
    }
    Some(edits)
}

/// The tag-name ranges of the element whose start- or end-tag name
/// contains the offset: (start-tag name, end-tag name if present).
fn tag_name_at(
    doc: &Document,
    text: &str,
    offset: usize,
) -> Option<(TextRange, Option<TextRange>)> {
    let node = doc.node_at_offset(offset);
    let element = doc.element(node)?;
    let end_name = element.close_tag.and_then(|close| {
        let written = element.name.as_written();
        let slice = text.get(close.start..close.end)?;
        let pos = slice.find(&written)?;
        Some(TextRange::new(
            close.start + pos,
            close.start + pos + written.len(),
        ))
    });
    let on_start = element.name_range.contains_inclusive(offset);
    let on_end = end_name.is_some_and(|range| range.contains_inclusive(offset));
    if !on_start && !on_end {
        return None;
    }
    Some((element.name_range, end_name))
}

/// Symbol names live inside quoted attribute values; everything that
/// cannot break out of the value is allowed.
fn is_valid_symbol_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['"', '\'', '<', '>', '&'])
}

/// Rough check for XML `Name` (element names, possibly prefixed).
fn is_valid_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_alphanumeric() || matches!(c, '-' | '.' | '_' | ':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUT: &str = r#"<Layout xmlns="urn:speedata.de:2009/publisher/en">
  <DefineColor name="accent"/>
  <Paragraph color="accent">text</Paragraph>
</Layout>"#;

    #[test]
    fn prepare_on_symbol() {
        let parse = xml_parser::parse(LAYOUT);
        let offset = LAYOUT.find("color=\"accent\"").unwrap() + 8;
        let target = prepare(&parse.document, LAYOUT, offset).expect("target");
        assert_eq!(target.text, "accent");
    }

    #[test]
    fn rename_symbol_changes_all_occurrences() {
        let parse = xml_parser::parse(LAYOUT);
        let offset = LAYOUT.find("name=\"accent\"").unwrap() + 7;
        let edits = rename(&parse.document, LAYOUT, offset, "brand").expect("edits");
        assert_eq!(edits.len(), 2);
        for (range, new) in &edits {
            assert_eq!(&LAYOUT[range.start..range.end], "accent");
            assert_eq!(new, "brand");
        }
    }

    #[test]
    fn rename_tag_pair() {
        let parse = xml_parser::parse(LAYOUT);
        let offset = LAYOUT.find("<Paragraph").unwrap() + 3;
        let edits = rename(&parse.document, LAYOUT, offset, "Textblock").expect("edits");
        assert_eq!(edits.len(), 2);
        assert_eq!(&LAYOUT[edits[0].0.start..edits[0].0.end], "Paragraph");
        assert_eq!(&LAYOUT[edits[1].0.start..edits[1].0.end], "Paragraph");
    }

    #[test]
    fn rename_from_end_tag() {
        let parse = xml_parser::parse(LAYOUT);
        let offset = LAYOUT.find("</Paragraph>").unwrap() + 4;
        let edits = rename(&parse.document, LAYOUT, offset, "Textblock").expect("edits");
        assert_eq!(edits.len(), 2);
    }

    #[test]
    fn invalid_names_are_rejected() {
        let parse = xml_parser::parse(LAYOUT);
        let tag_offset = LAYOUT.find("<Paragraph").unwrap() + 3;
        assert!(rename(&parse.document, LAYOUT, tag_offset, "1bad").is_none());
        assert!(rename(&parse.document, LAYOUT, tag_offset, "").is_none());
        let sym_offset = LAYOUT.find("name=\"accent\"").unwrap() + 7;
        assert!(rename(&parse.document, LAYOUT, sym_offset, "a\"b").is_none());
    }

    #[test]
    fn text_content_is_not_renameable() {
        let parse = xml_parser::parse(LAYOUT);
        let offset = LAYOUT.find(">text<").unwrap() + 2;
        assert!(prepare(&parse.document, LAYOUT, offset).is_none());
    }
}
