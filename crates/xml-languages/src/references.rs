//! Per-language reference rules: which attributes define named symbols
//! (colors, text formats, …) and which attributes refer to them.
//!
//! The rules are pure data — new languages or additional symbol kinds are
//! configuration, not code. A [`SymbolIndex`] built from the rules and a
//! parsed document backs go-to-definition, find-references, and rename.

use xml_ast::{Document, NodeKind, TextRange};

/// How an attribute value carries a symbol name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueForm {
    /// The attribute value is the name itself.
    Literal,
    /// The attribute value is an XPath expression; only a quoted string
    /// literal (`'name'` or `"name"`) is indexable.
    XPathString,
}

/// A place where a symbol is defined: an element with a naming attribute,
/// e.g. `DefineColor`/`name`.
#[derive(Debug, Clone)]
pub struct DefinitionSite {
    pub element: String,
    pub attribute: String,
    pub form: ValueForm,
}

/// A place where a symbol is referenced: an attribute, optionally
/// restricted to one element (e.g. `existing` only on `DefineFontalias`).
#[derive(Debug, Clone)]
pub struct ReferenceSite {
    pub element: Option<String>,
    pub attribute: String,
    pub form: ValueForm,
}

/// Definitions and references of one symbol kind (e.g. "color").
#[derive(Debug, Clone)]
pub struct SymbolRule {
    /// Human-readable kind, also the grouping key ("color", "text format").
    pub kind: String,
    pub definitions: Vec<DefinitionSite>,
    pub references: Vec<ReferenceSite>,
}

/// All symbol rules of one language, keyed by its element namespace.
#[derive(Debug, Clone)]
pub struct ReferenceRules {
    /// Namespace the rule elements live in.
    pub namespace: String,
    pub rules: Vec<SymbolRule>,
}

/// Built-in rules for a namespace. Currently ships the speedata Publisher
/// layout rules; other languages can be added here or, later, through
/// configuration.
pub fn reference_rules_for(namespace: Option<&str>) -> Option<ReferenceRules> {
    match namespace {
        Some(SPEEDATA_NS) => Some(speedata_rules()),
        _ => None,
    }
}

const SPEEDATA_NS: &str = "urn:speedata.de:2009/publisher/en";

fn speedata_rules() -> ReferenceRules {
    let def = |element: &str, attribute: &str| DefinitionSite {
        element: element.to_string(),
        attribute: attribute.to_string(),
        form: ValueForm::Literal,
    };
    let any = |attribute: &str| ReferenceSite {
        element: None,
        attribute: attribute.to_string(),
        form: ValueForm::Literal,
    };
    let on = |element: &str, attribute: &str| ReferenceSite {
        element: Some(element.to_string()),
        attribute: attribute.to_string(),
        form: ValueForm::Literal,
    };
    let rule =
        |kind: &str, definitions: Vec<DefinitionSite>, references: Vec<ReferenceSite>| SymbolRule {
            kind: kind.to_string(),
            definitions,
            references,
        };

    ReferenceRules {
        namespace: SPEEDATA_NS.to_string(),
        rules: vec![
            rule(
                "color",
                vec![def("DefineColor", "name")],
                [
                    "color",
                    "background-color",
                    "background-textcolor",
                    "bordercolor",
                    "border-color",
                    "border-top-color",
                    "border-right-color",
                    "border-bottom-color",
                    "border-left-color",
                    "defaultcolor",
                    "framecolor",
                    "hyperlinkbordercolor",
                ]
                .into_iter()
                .map(any)
                .collect(),
            ),
            rule(
                "color profile",
                vec![def("DefineColorprofile", "name")],
                vec![any("colorprofile")],
            ),
            rule(
                "font family",
                vec![
                    def("DefineFontfamily", "name"),
                    def("DefineFontalias", "alias"),
                ],
                vec![
                    any("fontfamily"),
                    any("fontface"),
                    on("DefineFontalias", "existing"),
                ],
            ),
            rule(
                "text format",
                vec![def("DefineTextformat", "name")],
                vec![any("textformat")],
            ),
            rule(
                "graphic",
                vec![def("DefineGraphic", "name")],
                vec![any("graphic")],
            ),
            rule(
                "matter",
                vec![def("DefineMatter", "name")],
                vec![any("matter")],
            ),
            rule(
                "area",
                vec![def("PositioningArea", "name")],
                vec![any("area")],
            ),
            rule("group", vec![def("Group", "name")], vec![any("groupname")]),
            rule(
                "page type",
                vec![def("Pagetype", "name")],
                vec![any("pagetype")],
            ),
            rule(
                "mark",
                vec![DefinitionSite {
                    element: "Mark".to_string(),
                    attribute: "select".to_string(),
                    form: ValueForm::XPathString,
                }],
                vec![on("A", "link")],
            ),
        ],
    }
}

/// Whether an occurrence defines or references its symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolRole {
    Definition,
    Reference,
}

/// One indexed symbol occurrence in a document.
#[derive(Debug, Clone)]
pub struct SymbolOccurrence {
    /// Kind from the matching [`SymbolRule`].
    pub kind: String,
    /// The symbol name (attribute value, string literal unwrapped).
    pub name: String,
    /// Range of the name in the source text (inside the quotes).
    pub range: TextRange,
    pub role: SymbolRole,
}

/// All symbol occurrences of a document, in document order.
#[derive(Debug, Default)]
pub struct SymbolIndex {
    occurrences: Vec<SymbolOccurrence>,
}

impl SymbolIndex {
    /// Walks the document and indexes every attribute the rules match.
    pub fn build(rules: &ReferenceRules, doc: &Document, text: &str) -> Self {
        let mut occurrences = Vec::new();
        for node in doc.descendants(doc.root()) {
            let NodeKind::Element(element) = doc.kind(node) else {
                continue;
            };
            if element.name.namespace.as_deref() != Some(rules.namespace.as_str()) {
                continue;
            }
            let local = element.name.local.as_str();
            for attr in &element.attributes {
                // Rule attributes are unprefixed (no namespace).
                if attr.name.namespace.is_some() {
                    continue;
                }
                let attr_local = attr.name.local.as_str();
                for rule in &rules.rules {
                    let definition = rule
                        .definitions
                        .iter()
                        .find(|d| d.element == local && d.attribute == attr_local)
                        .map(|d| d.form);
                    let reference = rule
                        .references
                        .iter()
                        .find(|r| {
                            r.attribute == attr_local
                                && r.element.as_deref().is_none_or(|e| e == local)
                        })
                        .map(|r| r.form);
                    let (role, form) = match (definition, reference) {
                        (Some(form), _) => (SymbolRole::Definition, form),
                        (None, Some(form)) => (SymbolRole::Reference, form),
                        (None, None) => continue,
                    };
                    let Some((name, range)) =
                        symbol_name(&attr.value, attr.value_range, form, text)
                    else {
                        continue;
                    };
                    occurrences.push(SymbolOccurrence {
                        kind: rule.kind.clone(),
                        name,
                        range,
                        role,
                    });
                }
            }
        }
        Self { occurrences }
    }

    pub fn all(&self) -> &[SymbolOccurrence] {
        &self.occurrences
    }

    /// The occurrence whose name range contains the offset (cursor
    /// positions include the range end).
    pub fn occurrence_at(&self, offset: usize) -> Option<&SymbolOccurrence> {
        self.occurrences
            .iter()
            .find(|occ| occ.range.contains_inclusive(offset))
    }

    /// All definitions of a symbol, in document order.
    pub fn definitions(&self, kind: &str, name: &str) -> Vec<&SymbolOccurrence> {
        self.occurrences
            .iter()
            .filter(|occ| {
                occ.role == SymbolRole::Definition && occ.kind == kind && occ.name == name
            })
            .collect()
    }

    /// All occurrences of a symbol, optionally without the definitions.
    pub fn occurrences(
        &self,
        kind: &str,
        name: &str,
        include_definitions: bool,
    ) -> Vec<&SymbolOccurrence> {
        self.occurrences
            .iter()
            .filter(|occ| occ.kind == kind && occ.name == name)
            .filter(|occ| include_definitions || occ.role == SymbolRole::Reference)
            .collect()
    }
}

/// Extracts the symbol name and its exact range from an attribute value.
/// For [`ValueForm::XPathString`] the value must be a single quoted string
/// literal; anything else (a real XPath expression) is not indexable.
fn symbol_name(
    value: &str,
    value_range: TextRange,
    form: ValueForm,
    text: &str,
) -> Option<(String, TextRange)> {
    match form {
        ValueForm::Literal => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return None;
            }
            // The materialized value can differ from the source slice when
            // it contains entity references; index only the plain case so
            // the range is exact.
            let raw = text.get(value_range.start..value_range.end)?;
            let start = value_range.start + raw.find(trimmed.chars().next()?)?;
            if text.get(start..start + trimmed.len()) != Some(trimmed) {
                return None;
            }
            Some((
                trimmed.to_string(),
                TextRange::new(start, start + trimmed.len()),
            ))
        }
        ValueForm::XPathString => {
            let trimmed = value.trim();
            let inner = trimmed
                .strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
                .or_else(|| {
                    trimmed
                        .strip_prefix('"')
                        .and_then(|rest| rest.strip_suffix('"'))
                })?;
            if inner.is_empty() || inner.contains('\'') || inner.contains('"') {
                return None;
            }
            let raw = text.get(value_range.start..value_range.end)?;
            let start = value_range.start + raw.find(inner)?;
            if text.get(start..start + inner.len()) != Some(inner) {
                return None;
            }
            Some((
                inner.to_string(),
                TextRange::new(start, start + inner.len()),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(text: &str) -> SymbolIndex {
        let parse = xml_parser::parse(text);
        let rules = reference_rules_for(Some(SPEEDATA_NS)).expect("built-in rules");
        SymbolIndex::build(&rules, &parse.document, text)
    }

    const LAYOUT: &str = r##"<Layout xmlns="urn:speedata.de:2009/publisher/en">
  <DefineColor name="highlight" value="#ff0000"/>
  <DefineTextformat name="indent"/>
  <Record element="data">
    <Paragraph color="highlight" textformat="indent"/>
    <Mark select="'article123'"/>
    <A link="article123"/>
  </Record>
</Layout>"##;

    #[test]
    fn definitions_and_references() {
        let index = index(LAYOUT);
        let defs = index.definitions("color", "highlight");
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].role, SymbolRole::Definition);

        let all = index.occurrences("color", "highlight", true);
        assert_eq!(all.len(), 2);
        let refs = index.occurrences("color", "highlight", false);
        assert_eq!(refs.len(), 1);
    }

    #[test]
    fn occurrence_at_cursor() {
        let index = index(LAYOUT);
        let offset = LAYOUT.find("color=\"highlight\"").unwrap() + 8;
        let occ = index.occurrence_at(offset).expect("occurrence");
        assert_eq!(occ.kind, "color");
        assert_eq!(occ.name, "highlight");
        assert_eq!(occ.role, SymbolRole::Reference);
    }

    #[test]
    fn mark_string_literal_links() {
        let index = index(LAYOUT);
        let defs = index.definitions("mark", "article123");
        assert_eq!(defs.len(), 1);
        // The range covers only the literal, not the quotes.
        let text = &LAYOUT[defs[0].range.start..defs[0].range.end];
        assert_eq!(text, "article123");
        assert_eq!(index.occurrences("mark", "article123", true).len(), 2);
    }

    #[test]
    fn xpath_expression_is_not_indexed() {
        let text = r#"<Layout xmlns="urn:speedata.de:2009/publisher/en">
  <Mark select="concat('a', $b)"/>
</Layout>"#;
        assert!(index(text).all().is_empty());
    }

    #[test]
    fn other_namespaces_have_no_rules() {
        assert!(reference_rules_for(Some("urn:other")).is_none());
        assert!(reference_rules_for(None).is_none());
    }

    #[test]
    fn scoped_reference_site() {
        let text = r#"<Layout xmlns="urn:speedata.de:2009/publisher/en">
  <DefineFontfamily name="body"/>
  <DefineFontalias alias="text" existing="body"/>
</Layout>"#;
        let index = index(text);
        // `existing` refers to the family, `alias` defines a new name.
        assert_eq!(index.occurrences("font family", "body", false).len(), 1);
        assert_eq!(index.definitions("font family", "text").len(), 1);
    }
}
