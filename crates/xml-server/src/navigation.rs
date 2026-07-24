//! Go-to-definition and find-references over the per-language symbol
//! index (named colors, text formats, marks, …).

use xml_ast::{Document, TextRange};
use xml_languages::{reference_rules_for, SymbolIndex};

/// Builds the symbol index for a document, if its root namespace has
/// reference rules.
pub fn symbol_index(doc: &Document, text: &str) -> Option<SymbolIndex> {
    let root = doc.root_element().and_then(|id| doc.element(id))?;
    let rules = reference_rules_for(root.name.namespace.as_deref())?;
    Some(SymbolIndex::build(&rules, doc, text))
}

/// Definition ranges for the symbol under the cursor.
pub fn definitions(doc: &Document, text: &str, offset: usize) -> Vec<TextRange> {
    let Some(index) = symbol_index(doc, text) else {
        return Vec::new();
    };
    let Some(occurrence) = index.occurrence_at(offset) else {
        return Vec::new();
    };
    index
        .definitions(&occurrence.kind, &occurrence.name)
        .into_iter()
        .map(|occ| occ.range)
        .collect()
}

/// All occurrence ranges of the symbol under the cursor.
pub fn references(
    doc: &Document,
    text: &str,
    offset: usize,
    include_definitions: bool,
) -> Vec<TextRange> {
    let Some(index) = symbol_index(doc, text) else {
        return Vec::new();
    };
    let Some(occurrence) = index.occurrence_at(offset) else {
        return Vec::new();
    };
    index
        .occurrences(&occurrence.kind, &occurrence.name, include_definitions)
        .into_iter()
        .map(|occ| occ.range)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAYOUT: &str = r#"<Layout xmlns="urn:speedata.de:2009/publisher/en">
  <DefineColor name="accent"/>
  <Paragraph color="accent"/>
  <Frame framecolor="accent"/>
</Layout>"#;

    #[test]
    fn definition_from_reference() {
        let parse = xml_parser::parse(LAYOUT);
        let offset = LAYOUT.find("color=\"accent\"").unwrap() + 8;
        let defs = definitions(&parse.document, LAYOUT, offset);
        assert_eq!(defs.len(), 1);
        assert_eq!(&LAYOUT[defs[0].start..defs[0].end], "accent");
        assert!(defs[0].start < LAYOUT.find("Paragraph").unwrap());
    }

    #[test]
    fn references_from_definition() {
        let parse = xml_parser::parse(LAYOUT);
        let offset = LAYOUT.find("name=\"accent\"").unwrap() + 7;
        let refs = references(&parse.document, LAYOUT, offset, true);
        assert_eq!(refs.len(), 3);
        let refs = references(&parse.document, LAYOUT, offset, false);
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn no_rules_no_results() {
        let text = r#"<html xmlns="http://www.w3.org/1999/xhtml"><body id="x"/></html>"#;
        let parse = xml_parser::parse(text);
        assert!(definitions(&parse.document, text, 50).is_empty());
    }
}
