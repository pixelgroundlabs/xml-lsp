//! Go-to-definition, find-references, and symbol-name completion over
//! the symbol index. The rules come from `lsp:defines`/`lsp:references`
//! annotations in the document's schema — the server itself knows no
//! language specifics.

use xml_ast::{Document, TextRange};
use xml_completion::{cursor_context, CompletionItem, CompletionItemKind, CursorContext};
use xml_languages::{ReferenceRules, SymbolIndex, SymbolRole, ValueForm};

/// Definition ranges for the symbol under the cursor.
pub fn definitions(
    rules: &ReferenceRules,
    doc: &Document,
    text: &str,
    offset: usize,
) -> Vec<TextRange> {
    let index = SymbolIndex::build(rules, doc, text);
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
    rules: &ReferenceRules,
    doc: &Document,
    text: &str,
    offset: usize,
    include_definitions: bool,
) -> Vec<TextRange> {
    let index = SymbolIndex::build(rules, doc, text);
    let Some(occurrence) = index.occurrence_at(offset) else {
        return Vec::new();
    };
    index
        .occurrences(&occurrence.kind, &occurrence.name, include_definitions)
        .into_iter()
        .map(|occ| occ.range)
        .collect()
}

/// Completion items for an attribute value that references named
/// symbols: every name the document defines for the matching kind.
pub fn symbol_value_completions(
    rules: &ReferenceRules,
    doc: &Document,
    text: &str,
    offset: usize,
) -> Vec<CompletionItem> {
    let CursorContext::AttributeValue {
        element,
        attr_index,
        replace,
    } = cursor_context(doc, text, offset)
    else {
        return Vec::new();
    };
    let Some(el) = doc.element(element) else {
        return Vec::new();
    };
    let Some(attr) = el.attributes.get(attr_index) else {
        return Vec::new();
    };

    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let index = SymbolIndex::build(rules, doc, text);
    for site in rules.sites_for(
        el.name.namespace.as_deref(),
        el.name.local.as_str(),
        attr.name.namespace.as_deref(),
        attr.name.local.as_str(),
    ) {
        if site.role != SymbolRole::Reference {
            continue;
        }
        for occ in index.all() {
            if occ.role != SymbolRole::Definition || occ.kind != site.kind {
                continue;
            }
            let label = match site.form {
                ValueForm::XPathVariable => format!("${}", occ.name),
                _ => occ.name.clone(),
            };
            if !seen.insert(label.clone()) {
                continue;
            }
            items.push(CompletionItem {
                insert: label.clone(),
                snippet: None,
                kind: CompletionItemKind::Value,
                documentation: Some(format!("{} defined in this document", site.kind)),
                replace,
                required: false,
                label,
            });
        }
    }
    items
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Annotated test grammar shared by the navigation-related tests.
    pub(crate) const GRAMMAR: &str = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0"
    xmlns:lsp="urn:xml-lsp:annotations" ns="urn:test">
  <start>
    <element name="root">
      <zeroOrMore>
        <choice>
          <element name="color">
            <attribute name="name"><lsp:defines symbol="color"/></attribute>
          </element>
          <element name="box">
            <optional><attribute name="fill"><lsp:references symbol="color"/></attribute></optional>
          </element>
        </choice>
      </zeroOrMore>
    </element>
  </start>
</grammar>"#;

    pub(crate) fn rules() -> ReferenceRules {
        let result = xml_schema::load_rng(GRAMMAR);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        ReferenceRules::from_schema(&result.schema)
    }

    const DOC: &str = r#"<root xmlns="urn:test">
  <color name="accent"/>
  <box fill="accent"/>
  <box fill="accent"/>
</root>"#;

    #[test]
    fn definition_from_reference() {
        let parse = xml_parser::parse(DOC);
        let offset = DOC.find("fill=\"accent\"").unwrap() + 7;
        let defs = definitions(&rules(), &parse.document, DOC, offset);
        assert_eq!(defs.len(), 1);
        assert_eq!(&DOC[defs[0].start..defs[0].end], "accent");
        assert!(defs[0].start < DOC.find("box").unwrap());
    }

    #[test]
    fn references_from_definition() {
        let parse = xml_parser::parse(DOC);
        let offset = DOC.find("name=\"accent\"").unwrap() + 7;
        let refs = references(&rules(), &parse.document, DOC, offset, true);
        assert_eq!(refs.len(), 3);
        let refs = references(&rules(), &parse.document, DOC, offset, false);
        assert_eq!(refs.len(), 2);
    }

    #[test]
    fn value_completion_offers_defined_symbols() {
        let text = r#"<root xmlns="urn:test">
  <color name="accent"/>
  <color name="muted"/>
  <box fill=""/>
</root>"#;
        let parse = xml_parser::parse(text);
        let offset = text.find("fill=\"\"").unwrap() + 6;
        let items = symbol_value_completions(&rules(), &parse.document, text, offset);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, vec!["accent", "muted"]);
    }

    #[test]
    fn no_symbol_completion_on_definition_sites() {
        let text = r#"<root xmlns="urn:test">
  <color name="accent"/>
  <color name=""/>
</root>"#;
        let parse = xml_parser::parse(text);
        let offset = text.find("name=\"\"").unwrap() + 6;
        assert!(symbol_value_completions(&rules(), &parse.document, text, offset).is_empty());
    }
}
