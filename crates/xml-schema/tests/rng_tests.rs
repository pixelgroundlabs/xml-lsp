//! Integration tests for the RELAX NG loader against small, hand-written
//! schemas.

use xml_schema::{load_rng, Pattern, PatternId, Schema, SchemaDiagnosticCode};

fn load_clean(text: &str) -> Schema {
    let result = load_rng(text);
    assert!(
        result.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );
    result.schema
}

fn codes(text: &str) -> Vec<SchemaDiagnosticCode> {
    load_rng(text).diagnostics.iter().map(|d| d.code).collect()
}

/// Resolves refs and returns the element pattern id, panicking on anything
/// else.
fn expect_element(schema: &Schema, id: PatternId) -> PatternId {
    let resolved = schema.resolve(id);
    assert!(
        matches!(schema.pattern(resolved), Pattern::Element(_)),
        "expected element pattern, found {:?}",
        schema.pattern(resolved)
    );
    resolved
}

#[test]
fn grammar_with_start_define_and_docs() {
    let schema = load_clean(
        r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0"
                 xmlns:a="http://relaxng.org/ns/compatibility/annotations/1.0"
                 ns="urn:test">
  <start><ref name="root"/></start>
  <define name="root">
    <element name="root">
      <a:documentation>The root element.</a:documentation>
      <a:documentation>Second paragraph.</a:documentation>
      <optional>
        <attribute name="id">
          <a:documentation>An identifier.</a:documentation>
        </attribute>
      </optional>
      <empty/>
    </element>
  </define>
</grammar>"#,
    );

    let root = expect_element(&schema, schema.start());
    let element = schema.element_pattern(root).unwrap();
    assert!(element.name.matches(Some("urn:test"), "root"));
    assert_eq!(
        element.documentation.as_deref(),
        Some("The root element.\n\nSecond paragraph.")
    );

    let attrs = schema.attributes(element.content);
    assert_eq!(attrs.len(), 1);
    let attr = schema.attribute_pattern(attrs[0]).unwrap();
    // Unprefixed attribute names have no namespace, even with grammar ns.
    assert!(attr.name.matches(None, "id"));
    assert_eq!(attr.documentation.as_deref(), Some("An identifier."));
    // An attribute without a content pattern accepts text.
    assert!(matches!(
        schema.pattern(schema.resolve(attr.content)),
        Pattern::Text
    ));
}

#[test]
fn attribute_with_explicit_ns() {
    let schema = load_clean(
        r#"<element name="e" xmlns="http://relaxng.org/ns/structure/1.0" ns="urn:test">
  <attribute name="a" ns="urn:attr"/>
</element>"#,
    );
    let root = expect_element(&schema, schema.start());
    let content = schema.element_pattern(root).unwrap().content;
    let attr = schema
        .attribute_pattern(schema.attributes(content)[0])
        .unwrap();
    assert!(attr.name.matches(Some("urn:attr"), "a"));
}

#[test]
fn prefixed_names_resolve_via_schema_xmlns() {
    let schema = load_clean(
        r#"<element xmlns="http://relaxng.org/ns/structure/1.0"
             xmlns:x="urn:x" name="x:root">
  <attribute name="x:a"/>
</element>"#,
    );
    let root = expect_element(&schema, schema.start());
    let element = schema.element_pattern(root).unwrap();
    assert!(element.name.matches(Some("urn:x"), "root"));
    let attr = schema
        .attribute_pattern(schema.attributes(element.content)[0])
        .unwrap();
    assert!(attr.name.matches(Some("urn:x"), "a"));
}

#[test]
fn ns_inheritance_and_override() {
    let schema = load_clean(
        r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0" ns="urn:outer">
  <start>
    <element name="outer">
      <element name="inner" ns="urn:inner"><empty/></element>
    </element>
  </start>
</grammar>"#,
    );
    let outer = expect_element(&schema, schema.start());
    let outer_el = schema.element_pattern(outer).unwrap();
    assert!(outer_el.name.matches(Some("urn:outer"), "outer"));
    let inner = schema.child_elements(outer_el.content)[0];
    assert!(schema
        .element_pattern(inner)
        .unwrap()
        .name
        .matches(Some("urn:inner"), "inner"));
}

#[test]
fn implicit_group_and_collapsed_containers() {
    let schema = load_clean(
        r#"<element name="e" xmlns="http://relaxng.org/ns/structure/1.0">
  <optional>
    <attribute name="a"/>
    <attribute name="b"/>
  </optional>
</element>"#,
    );
    let root = expect_element(&schema, schema.start());
    let content = schema.element_pattern(root).unwrap().content;
    // optional with two children wraps them in an implicit group.
    let Pattern::Optional(inner) = schema.pattern(content) else {
        panic!("expected optional, found {:?}", schema.pattern(content));
    };
    assert!(matches!(schema.pattern(*inner), Pattern::Group(items) if items.len() == 2));
    assert_eq!(schema.attributes(content).len(), 2);
}

#[test]
fn value_enum_and_data_params() {
    let schema = load_clean(
        r#"<element name="e" xmlns="http://relaxng.org/ns/structure/1.0"
             xmlns:a="http://relaxng.org/ns/compatibility/annotations/1.0"
             datatypeLibrary="http://www.w3.org/2001/XMLSchema-datatypes">
  <attribute name="mode">
    <choice>
      <value>fast</value>
      <value>
        <a:documentation>The careful mode.</a:documentation>
        careful
      </value>
      <data type="string">
        <param name="pattern">\{.+\}</param>
      </data>
    </choice>
  </attribute>
</element>"#,
    );
    let root = expect_element(&schema, schema.start());
    let content = schema.element_pattern(root).unwrap().content;
    let attr = schema
        .attribute_pattern(schema.attributes(content)[0])
        .unwrap();

    let values = schema.value_enum(attr.content);
    assert_eq!(values.len(), 2);
    let rendered: Vec<_> = values
        .iter()
        .map(|&id| schema.value_pattern(id).unwrap().value.trim().to_string())
        .collect();
    assert_eq!(rendered, vec!["fast", "careful"]);
    assert_eq!(schema.documentation(values[1]), Some("The careful mode."));

    // The data pattern keeps its type and params.
    let choice = schema.resolve(attr.content);
    let Pattern::Choice(items) = schema.pattern(choice) else {
        panic!("expected choice");
    };
    let data = items
        .iter()
        .find_map(|&id| match schema.pattern(id) {
            Pattern::Data(data) => Some(data),
            _ => None,
        })
        .expect("data pattern present");
    assert_eq!(data.datatype.name, "string");
    assert_eq!(
        data.datatype.library,
        "http://www.w3.org/2001/XMLSchema-datatypes"
    );
    assert_eq!(
        data.params,
        vec![("pattern".to_string(), "\\{.+\\}".to_string())]
    );
}

#[test]
fn name_classes_with_except() {
    let schema = load_clean(
        r#"<element name="e" xmlns="http://relaxng.org/ns/structure/1.0">
  <zeroOrMore>
    <element>
      <anyName>
        <except>
          <nsName ns="urn:forbidden"/>
        </except>
      </anyName>
      <text/>
    </element>
  </zeroOrMore>
</element>"#,
    );
    let root = expect_element(&schema, schema.start());
    let content = schema.element_pattern(root).unwrap().content;
    let wildcard = schema.child_elements(content)[0];
    let name = &schema.element_pattern(wildcard).unwrap().name;
    assert!(name.matches(Some("urn:anything"), "x"));
    assert!(name.matches(None, "x"));
    assert!(!name.matches(Some("urn:forbidden"), "x"));
}

#[test]
fn mixed_becomes_interleave_with_text() {
    let schema = load_clean(
        r#"<element name="e" xmlns="http://relaxng.org/ns/structure/1.0">
  <mixed><element name="b"><empty/></element></mixed>
</element>"#,
    );
    let root = expect_element(&schema, schema.start());
    let content = schema.element_pattern(root).unwrap().content;
    let Pattern::Interleave(items) = schema.pattern(content) else {
        panic!("expected interleave, found {:?}", schema.pattern(content));
    };
    assert!(items
        .iter()
        .any(|&id| matches!(schema.pattern(id), Pattern::Text)));
    assert_eq!(schema.child_elements(content).len(), 1);
}

#[test]
fn combine_choice_merges_defines() {
    let schema = load_clean(
        r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0">
  <start><ref name="content"/></start>
  <define name="content" combine="choice"><element name="a"><empty/></element></define>
  <define name="content" combine="choice"><element name="b"><empty/></element></define>
</grammar>"#,
    );
    let elements = schema.child_elements(schema.start());
    assert_eq!(elements.len(), 2);
}

#[test]
fn error_cases_are_reported() {
    // Unknown ref.
    assert_eq!(
        codes(
            r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0">
  <start><ref name="nope"/></start>
</grammar>"#
        ),
        vec![SchemaDiagnosticCode::UnknownRef]
    );

    // Duplicate define without combine.
    assert_eq!(
        codes(
            r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0">
  <start><ref name="d"/></start>
  <define name="d"><empty/></define>
  <define name="d"><empty/></define>
</grammar>"#
        ),
        vec![SchemaDiagnosticCode::DuplicateDefine]
    );

    // Missing start.
    assert_eq!(
        codes(
            r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0">
  <define name="d"><empty/></define>
</grammar>"#
        ),
        vec![SchemaDiagnosticCode::MissingStart]
    );

    // Unsupported include.
    assert!(codes(
        r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0">
  <include href="other.rng"/>
  <start><empty/></start>
</grammar>"#
    )
    .contains(&SchemaDiagnosticCode::UnsupportedFeature));

    // data without type.
    assert_eq!(
        codes(
            r#"<element name="e" xmlns="http://relaxng.org/ns/structure/1.0">
  <attribute name="a"><data/></attribute>
</element>"#
        ),
        vec![SchemaDiagnosticCode::MissingType]
    );

    // Not a RELAX NG document at all.
    assert_eq!(
        codes("<html><body/></html>"),
        vec![SchemaDiagnosticCode::UnexpectedElement]
    );
}

#[test]
fn foreign_annotations_are_skipped() {
    // Schematron rules and other foreign elements must not disturb
    // pattern parsing.
    let schema = load_clean(
        r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0"
             xmlns:sch="http://purl.oclc.org/dsdl/schematron">
  <sch:ns prefix="t" uri="urn:test"/>
  <start>
    <element name="e">
      <sch:pattern><sch:rule context="e"/></sch:pattern>
      <empty/>
    </element>
  </start>
</grammar>"#,
    );
    let root = expect_element(&schema, schema.start());
    assert!(matches!(
        schema.pattern(schema.resolve(schema.element_pattern(root).unwrap().content)),
        Pattern::Empty
    ));
}

#[test]
fn broken_schema_still_loads_partially() {
    // A schema with an XML error keeps its parsed part and reports the
    // syntax problem.
    let result = load_rng(
        r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0">
  <start><element name="e"><empty/></element></start>
  <define name="unfinished"#,
    );
    assert!(result
        .diagnostics
        .iter()
        .any(|d| d.code == SchemaDiagnosticCode::XmlSyntax));
    let root = expect_element(&result.schema, result.schema.start());
    assert!(result
        .schema
        .element_pattern(root)
        .unwrap()
        .name
        .matches(None, "e"));
}

#[test]
fn find_element_by_name() {
    let schema = load_clean(
        r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0" ns="urn:test">
  <start><ref name="a"/></start>
  <define name="a">
    <element name="a"><ref name="b"/></element>
  </define>
  <define name="b">
    <element name="b"><empty/></element>
  </define>
</grammar>"#,
    );
    assert!(schema.find_element(Some("urn:test"), "b").is_some());
    assert!(schema.find_element(None, "b").is_none());
    assert!(schema.find_element(Some("urn:test"), "c").is_none());
}

// --- lsp:* annotations (urn:xml-lsp:annotations) ------------------------

const ANNOTATED: &str = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0"
    xmlns:lsp="urn:xml-lsp:annotations" ns="urn:test">
  <start>
    <element name="palette">
      <lsp:format blank-lines="true"/>
      <zeroOrMore>
        <element name="color">
          <attribute name="name">
            <lsp:defines symbol="color"/>
          </attribute>
        </element>
      </zeroOrMore>
      <zeroOrMore>
        <element name="box">
          <optional>
            <attribute name="fill">
              <lsp:references symbol="color"/>
            </attribute>
          </optional>
          <optional>
            <attribute name="select">
              <lsp:references symbol="variable" form="xpath-variable"/>
            </attribute>
          </optional>
        </element>
      </zeroOrMore>
      <optional>
        <element name="raw">
          <lsp:format preserve="true" inline="true"/>
          <text/>
        </element>
      </optional>
    </element>
  </start>
</grammar>"#;

fn attribute_symbol(schema: &Schema, local: &str) -> Option<xml_schema::SymbolAnnotation> {
    schema
        .pattern_ids()
        .find_map(|id| match schema.pattern(id) {
            Pattern::Attribute(attr) if attr.name.matches(None, local) => attr.symbol.clone(),
            _ => None,
        })
}

#[test]
fn symbol_annotations_are_collected() {
    let schema = load_clean(ANNOTATED);
    let def = attribute_symbol(&schema, "name").expect("defines");
    assert_eq!(def.role, xml_schema::SymbolRole::Definition);
    assert_eq!(def.kind, "color");
    assert_eq!(def.form, xml_schema::ValueForm::Literal);

    let refer = attribute_symbol(&schema, "fill").expect("references");
    assert_eq!(refer.role, xml_schema::SymbolRole::Reference);
    assert_eq!(refer.kind, "color");

    let var = attribute_symbol(&schema, "select").expect("references");
    assert_eq!(var.form, xml_schema::ValueForm::XPathVariable);
}

#[test]
fn format_hints_are_collected() {
    let schema = load_clean(ANNOTATED);
    let palette = schema.find_element(Some("urn:test"), "palette").unwrap();
    let hints = schema.element_pattern(palette).unwrap().format.unwrap();
    assert!(hints.blank_lines && !hints.preserve && !hints.inline);

    let raw = schema.find_element(Some("urn:test"), "raw").unwrap();
    let hints = schema.element_pattern(raw).unwrap().format.unwrap();
    assert!(hints.preserve && hints.inline && !hints.blank_lines);
}

#[test]
fn malformed_annotations_warn_but_load() {
    let result = load_rng(
        r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0"
    xmlns:lsp="urn:xml-lsp:annotations">
  <start>
    <element name="e">
      <attribute name="a"><lsp:defines/></attribute>
      <attribute name="b"><lsp:references symbol="x" form="bogus"/></attribute>
    </element>
  </start>
</grammar>"#,
    );
    let annotation_warnings = result
        .diagnostics
        .iter()
        .filter(|d| d.code == SchemaDiagnosticCode::Annotation)
        .count();
    assert_eq!(annotation_warnings, 2);
    // The bogus form falls back to literal but the annotation is kept.
    let symbol = attribute_symbol(&result.schema, "b").expect("kept");
    assert_eq!(symbol.form, xml_schema::ValueForm::Literal);
}
