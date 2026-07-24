//! Symbol reference rules, derived from `lsp:defines`/`lsp:references`
//! annotations in the schema (see docs/schema-annotations.md).
//!
//! The LSP carries no language-specific knowledge: a grammar that wants
//! navigation, symbol completion, and rename annotates its attribute
//! patterns, and the rules fall out of the loaded schema. A
//! [`SymbolIndex`] built from the rules and a parsed document backs
//! go-to-definition, find-references, and rename.

use xml_ast::{Document, NodeKind, TextRange};
use xml_schema::{NameClass, Pattern, Schema};

pub use xml_schema::{SymbolRole, ValueForm};

/// One annotated symbol site: this element/attribute pair defines or
/// references symbols of `kind`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolSite {
    /// Element namespace and local name.
    pub namespace: Option<String>,
    pub element: String,
    /// Attribute namespace and local name.
    pub attr_namespace: Option<String>,
    pub attribute: String,
    /// Symbol kind, the grouping key (e.g. "color").
    pub kind: String,
    pub role: SymbolRole,
    pub form: ValueForm,
}

/// All symbol sites of a schema.
#[derive(Debug, Clone, Default)]
pub struct ReferenceRules {
    sites: Vec<SymbolSite>,
}

impl ReferenceRules {
    /// Collects the symbol sites from a schema: every concrete element
    /// name × every reachable annotated attribute in its content.
    pub fn from_schema(schema: &Schema) -> Self {
        let mut sites = Vec::new();
        for id in schema.pattern_ids() {
            let Pattern::Element(element) = schema.pattern(id) else {
                continue;
            };
            let element_names = concrete_names(&element.name);
            if element_names.is_empty() {
                continue;
            }
            for attr_id in schema.attributes(element.content) {
                let Some(attr) = schema.attribute_pattern(attr_id) else {
                    continue;
                };
                let Some(symbol) = &attr.symbol else {
                    continue;
                };
                for (ns, local) in &element_names {
                    for (attr_ns, attr_local) in concrete_names(&attr.name) {
                        let site = SymbolSite {
                            namespace: ns.clone(),
                            element: local.clone(),
                            attr_namespace: attr_ns,
                            attribute: attr_local,
                            kind: symbol.kind.clone(),
                            role: symbol.role,
                            form: symbol.form,
                        };
                        if !sites.contains(&site) {
                            sites.push(site);
                        }
                    }
                }
            }
        }
        Self { sites }
    }

    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }

    pub fn sites(&self) -> &[SymbolSite] {
        &self.sites
    }

    /// The sites matching one element/attribute pair in a document.
    pub fn sites_for<'a>(
        &'a self,
        namespace: Option<&'a str>,
        element: &'a str,
        attr_namespace: Option<&'a str>,
        attribute: &'a str,
    ) -> impl Iterator<Item = &'a SymbolSite> {
        self.sites.iter().filter(move |site| {
            site.namespace.as_deref() == namespace
                && site.element == element
                && site.attr_namespace.as_deref() == attr_namespace
                && site.attribute == attribute
        })
    }
}

/// Expands a name class into its concrete names; wildcards yield
/// nothing (they cannot anchor a symbol site).
fn concrete_names(name: &NameClass) -> Vec<(Option<String>, String)> {
    match name {
        NameClass::Name { ns, local } => {
            vec![(ns.as_deref().map(str::to_string), local.clone())]
        }
        NameClass::Choice(choices) => choices.iter().flat_map(concrete_names).collect(),
        NameClass::NsName { .. } | NameClass::AnyName { .. } => Vec::new(),
    }
}

/// One indexed symbol occurrence in a document.
#[derive(Debug, Clone)]
pub struct SymbolOccurrence {
    /// Kind from the matching [`SymbolSite`].
    pub kind: String,
    /// The symbol name (attribute value, literal unwrapped).
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
            for attr in &element.attributes {
                for site in rules.sites_for(
                    element.name.namespace.as_deref(),
                    element.name.local.as_str(),
                    attr.name.namespace.as_deref(),
                    attr.name.local.as_str(),
                ) {
                    let Some((name, range)) =
                        symbol_name(&attr.value, attr.value_range, site.form, text)
                    else {
                        continue;
                    };
                    occurrences.push(SymbolOccurrence {
                        kind: site.kind.clone(),
                        name,
                        range,
                        role: site.role,
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
        ValueForm::XPathVariable => {
            let trimmed = value.trim();
            let name = trimmed.strip_prefix('$')?;
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
            {
                return None;
            }
            let raw = text.get(value_range.start..value_range.end)?;
            let start = value_range.start + raw.find(name)?;
            if text.get(start..start + name.len()) != Some(name) {
                return None;
            }
            Some((name.to_string(), TextRange::new(start, start + name.len())))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small annotated grammar: colors are defined by
    /// `<color name="…">` and referenced by `fill`/`stroke`; variables
    /// are defined by `<set var="…">` and referenced by `$name` in
    /// `select`; marks are defined by XPath string literals in
    /// `<mark select="…">` and referenced by `<a link="…">`.
    const GRAMMAR: &str = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0"
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
            <optional><attribute name="stroke"><lsp:references symbol="color"/></attribute></optional>
            <optional><attribute name="select">
              <lsp:references symbol="variable" form="xpath-variable"/>
            </attribute></optional>
          </element>
          <element name="set">
            <attribute name="var"><lsp:defines symbol="variable"/></attribute>
          </element>
          <element name="mark">
            <attribute name="select">
              <lsp:defines symbol="mark" form="xpath-string"/>
            </attribute>
          </element>
          <element name="a">
            <attribute name="link"><lsp:references symbol="mark"/></attribute>
          </element>
        </choice>
      </zeroOrMore>
    </element>
  </start>
</grammar>"#;

    fn rules() -> ReferenceRules {
        let result = xml_schema::load_rng(GRAMMAR);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        ReferenceRules::from_schema(&result.schema)
    }

    fn index(text: &str) -> SymbolIndex {
        let parse = xml_parser::parse(text);
        SymbolIndex::build(&rules(), &parse.document, text)
    }

    const DOC: &str = r#"<root xmlns="urn:test">
  <color name="accent"/>
  <box fill="accent" stroke="accent"/>
  <set var="chapter"/>
  <box select="$chapter"/>
  <mark select="'article123'"/>
  <a link="article123"/>
</root>"#;

    #[test]
    fn rules_come_from_the_schema() {
        let rules = rules();
        assert!(!rules.is_empty());
        let fill: Vec<_> = rules
            .sites_for(Some("urn:test"), "box", None, "fill")
            .collect();
        assert_eq!(fill.len(), 1);
        assert_eq!(fill[0].kind, "color");
        assert_eq!(fill[0].role, SymbolRole::Reference);
        assert!(rules
            .sites_for(Some("urn:test"), "box", None, "width")
            .next()
            .is_none());
        // Wrong namespace: no match.
        assert!(rules
            .sites_for(Some("urn:other"), "box", None, "fill")
            .next()
            .is_none());
    }

    #[test]
    fn definitions_and_references() {
        let index = index(DOC);
        assert_eq!(index.definitions("color", "accent").len(), 1);
        assert_eq!(index.occurrences("color", "accent", true).len(), 3);
        assert_eq!(index.occurrences("color", "accent", false).len(), 2);
    }

    #[test]
    fn occurrence_at_cursor() {
        let index = index(DOC);
        let offset = DOC.find("fill=\"accent\"").unwrap() + 7;
        let occ = index.occurrence_at(offset).expect("occurrence");
        assert_eq!(occ.kind, "color");
        assert_eq!(occ.role, SymbolRole::Reference);
    }

    #[test]
    fn xpath_variable_form() {
        let index = index(DOC);
        let refs = index.occurrences("variable", "chapter", false);
        assert_eq!(refs.len(), 1);
        // The range excludes the `$`.
        assert_eq!(&DOC[refs[0].range.start..refs[0].range.end], "chapter");
        assert_eq!(index.definitions("variable", "chapter").len(), 1);
    }

    #[test]
    fn xpath_string_form() {
        let index = index(DOC);
        let defs = index.definitions("mark", "article123");
        assert_eq!(defs.len(), 1);
        assert_eq!(&DOC[defs[0].range.start..defs[0].range.end], "article123");
        assert_eq!(index.occurrences("mark", "article123", true).len(), 2);
    }

    #[test]
    fn xpath_expressions_are_not_indexed() {
        let text = r#"<root xmlns="urn:test">
  <mark select="concat('a', $b)"/>
  <box select="$a + 1"/>
</root>"#;
        assert!(index(text).all().is_empty());
    }

    #[test]
    fn unannotated_schema_has_no_rules() {
        let result = xml_schema::load_rng(
            r#"<element name="e" xmlns="http://relaxng.org/ns/structure/1.0">
  <attribute name="a"/>
</element>"#,
        );
        assert!(ReferenceRules::from_schema(&result.schema).is_empty());
    }
}
