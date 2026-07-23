//! Integration tests for the validation engine against hand-written
//! schemas.

use xml_schema::{load_rng, Schema};
use xml_validation::{validate, ValidationDiagnostic, ValidationDiagnosticCode};

fn schema(text: &str) -> Schema {
    let result = load_rng(text);
    assert!(
        result.diagnostics.is_empty(),
        "schema must load cleanly: {:?}",
        result.diagnostics
    );
    result.schema
}

fn check(schema_text: &str, doc_text: &str) -> Vec<ValidationDiagnostic> {
    let schema = schema(schema_text);
    let parse = xml_parser::parse(doc_text);
    assert!(
        parse.diagnostics.is_empty(),
        "document must be well-formed: {:?}",
        parse.diagnostics
    );
    validate(&schema, &parse.document, doc_text)
}

fn codes(diags: &[ValidationDiagnostic]) -> Vec<ValidationDiagnosticCode> {
    diags.iter().map(|d| d.code).collect()
}

/// A small library-like schema used by several tests.
const LIBRARY: &str = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0">
  <start><ref name="library"/></start>
  <define name="library">
    <element name="library">
      <oneOrMore><ref name="book"/></oneOrMore>
    </element>
  </define>
  <define name="book">
    <element name="book">
      <attribute name="id"/>
      <optional>
        <attribute name="lang">
          <choice><value>de</value><value>en</value></choice>
        </attribute>
      </optional>
      <element name="title"><text/></element>
      <optional><element name="author"><text/></element></optional>
    </element>
  </define>
</grammar>"#;

#[test]
fn valid_document_has_no_diagnostics() {
    let diags = check(
        LIBRARY,
        r#"<library>
  <book id="1" lang="de"><title>T</title><author>A</author></book>
  <book id="2"><title>U</title></book>
</library>"#,
    );
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn wrong_root_element() {
    let diags = check(LIBRARY, "<bibliothek/>");
    assert_eq!(
        codes(&diags),
        vec![ValidationDiagnosticCode::ElementNotAllowed]
    );
    assert!(
        diags[0].message.contains("`library`"),
        "{}",
        diags[0].message
    );
}

#[test]
fn unknown_element_reports_expected_names() {
    let diags = check(
        LIBRARY,
        r#"<library><book id="1"><title>T</title><isbn>x</isbn></book></library>"#,
    );
    assert_eq!(
        codes(&diags),
        vec![ValidationDiagnosticCode::ElementNotAllowed]
    );
    // After <title>, only <author> (or the end tag) is possible.
    assert!(
        diags[0].message.contains("`author`"),
        "{}",
        diags[0].message
    );
}

#[test]
fn wrong_order_is_reported() {
    let diags = check(
        LIBRARY,
        r#"<library><book id="1"><author>A</author><title>T</title></book></library>"#,
    );
    // <author> before <title> is not allowed; recovery then accepts
    // <title> so no follow-up error appears.
    assert_eq!(
        codes(&diags),
        vec![ValidationDiagnosticCode::ElementNotAllowed]
    );
    assert!(diags[0].message.contains("`title`"), "{}", diags[0].message);
}

#[test]
fn missing_required_child() {
    let diags = check(LIBRARY, r#"<library><book id="1"/></library>"#);
    assert_eq!(
        codes(&diags),
        vec![ValidationDiagnosticCode::IncompleteContent]
    );
    assert!(diags[0].message.contains("`title`"), "{}", diags[0].message);
}

#[test]
fn missing_required_attribute() {
    let diags = check(
        LIBRARY,
        r#"<library><book><title>T</title></book></library>"#,
    );
    assert_eq!(
        codes(&diags),
        vec![ValidationDiagnosticCode::MissingAttributes]
    );
    assert!(diags[0].message.contains("`id`"), "{}", diags[0].message);
}

#[test]
fn unknown_attribute() {
    let diags = check(
        LIBRARY,
        r#"<library><book id="1" isbn="x"><title>T</title></book></library>"#,
    );
    assert_eq!(
        codes(&diags),
        vec![ValidationDiagnosticCode::AttributeNotAllowed]
    );
    assert!(diags[0].message.contains("`isbn`"), "{}", diags[0].message);
}

#[test]
fn invalid_enum_value_lists_alternatives() {
    let diags = check(
        LIBRARY,
        r#"<library><book id="1" lang="fr"><title>T</title></book></library>"#,
    );
    assert_eq!(
        codes(&diags),
        vec![ValidationDiagnosticCode::InvalidAttributeValue]
    );
    let message = &diags[0].message;
    assert!(message.contains("`fr`"), "{message}");
    assert!(
        message.contains("`de`") && message.contains("`en`"),
        "{message}"
    );
}

#[test]
fn text_in_element_only_content() {
    let diags = check(
        LIBRARY,
        r#"<library><book id="1"><title>T</title>loose text</book></library>"#,
    );
    assert_eq!(
        codes(&diags),
        vec![ValidationDiagnosticCode::TextNotAllowed]
    );
}

#[test]
fn one_error_does_not_cascade() {
    // Both books contain one error each; both are reported, nothing else.
    let diags = check(
        LIBRARY,
        r#"<library>
  <book id="1"><titel>T</titel><title>T</title></book>
  <book id="2" lang="xx"><title>U</title></book>
</library>"#,
    );
    assert_eq!(
        codes(&diags),
        vec![
            ValidationDiagnosticCode::ElementNotAllowed,
            ValidationDiagnosticCode::InvalidAttributeValue,
        ]
    );
}

#[test]
fn data_pattern_validates_attribute_value() {
    let schema_text = r#"<element name="e" xmlns="http://relaxng.org/ns/structure/1.0"
      datatypeLibrary="http://www.w3.org/2001/XMLSchema-datatypes">
  <attribute name="n">
    <choice>
      <value>auto</value>
      <data type="string"><param name="pattern">[0-9]+</param></data>
    </choice>
  </attribute>
</element>"#;
    assert!(check(schema_text, r#"<e n="42"/>"#).is_empty());
    assert!(check(schema_text, r#"<e n="auto"/>"#).is_empty());
    let diags = check(schema_text, r#"<e n="x1"/>"#);
    assert_eq!(
        codes(&diags),
        vec![ValidationDiagnosticCode::InvalidAttributeValue]
    );
}

#[test]
fn simple_content_with_value() {
    let schema_text = r#"<element name="flag" xmlns="http://relaxng.org/ns/structure/1.0">
  <choice><value>on</value><value>off</value></choice>
</element>"#;
    assert!(check(schema_text, "<flag>on</flag>").is_empty());
    // token semantics: surrounding whitespace is fine.
    assert!(check(schema_text, "<flag> off </flag>").is_empty());
    let diags = check(schema_text, "<flag>maybe</flag>");
    assert_eq!(
        codes(&diags),
        vec![ValidationDiagnosticCode::InvalidTextValue]
    );
}

#[test]
fn empty_string_satisfies_data_content() {
    let schema_text = r#"<element name="note" xmlns="http://relaxng.org/ns/structure/1.0"
      datatypeLibrary="http://www.w3.org/2001/XMLSchema-datatypes">
  <data type="string"/>
</element>"#;
    // No children at all still matches string content ("").
    assert!(check(schema_text, "<note/>").is_empty());
}

#[test]
fn interleave_allows_any_order() {
    let schema_text = r#"<element name="e" xmlns="http://relaxng.org/ns/structure/1.0">
  <interleave>
    <element name="a"><empty/></element>
    <element name="b"><empty/></element>
    <optional><element name="c"><empty/></element></optional>
  </interleave>
</element>"#;
    assert!(check(schema_text, "<e><a/><b/></e>").is_empty());
    assert!(check(schema_text, "<e><b/><c/><a/></e>").is_empty());
    let diags = check(schema_text, "<e><b/></e>");
    assert_eq!(
        codes(&diags),
        vec![ValidationDiagnosticCode::IncompleteContent]
    );
    assert!(diags[0].message.contains("`a`"), "{}", diags[0].message);
}

#[test]
fn wildcard_foreign_elements_and_attributes() {
    // Mirrors the speedata foreign-nodes idiom.
    let schema_text = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0" ns="urn:main">
  <start>
    <element name="root">
      <interleave>
        <group>
          <optional><attribute name="known"/></optional>
          <optional><element name="child"><empty/></element></optional>
        </group>
        <zeroOrMore>
          <choice>
            <attribute><anyName><except><nsName ns=""/><nsName ns="urn:main"/></except></anyName></attribute>
            <element><anyName><except><nsName ns=""/><nsName ns="urn:main"/></except></anyName><text/></element>
          </choice>
        </zeroOrMore>
      </interleave>
    </element>
  </start>
</grammar>"#;
    // Foreign namespace: allowed.
    assert!(check(
        schema_text,
        r#"<root xmlns="urn:main" xmlns:f="urn:foreign" f:extra="1"><f:note>x</f:note><child/></root>"#
    )
    .is_empty());
    // Unqualified unknown attribute: rejected by the except clause.
    let diags = check(schema_text, r#"<root xmlns="urn:main" unknown="1"/>"#);
    assert_eq!(
        codes(&diags),
        vec![ValidationDiagnosticCode::AttributeNotAllowed]
    );
}

#[test]
fn recursive_content_model() {
    let schema_text = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0">
  <start><ref name="item"/></start>
  <define name="item">
    <element name="item">
      <zeroOrMore><ref name="item"/></zeroOrMore>
    </element>
  </define>
</grammar>"#;
    assert!(check(schema_text, "<item><item><item/></item><item/></item>").is_empty());
}

#[test]
fn mixed_content() {
    let schema_text = r#"<element name="p" xmlns="http://relaxng.org/ns/structure/1.0">
  <mixed><zeroOrMore><element name="b"><text/></element></zeroOrMore></mixed>
</element>"#;
    assert!(check(schema_text, "<p>Hello <b>bold</b> world</p>").is_empty());
}

#[test]
fn nested_recovery_keeps_validating_subtree() {
    // The unknown wrapper is reported, but errors inside a known element
    // under it are still found via recovery.
    let diags = check(
        LIBRARY,
        r#"<library><shelf><book id="1" lang="xx"><title>T</title></book></shelf></library>"#,
    );
    assert!(codes(&diags).contains(&ValidationDiagnosticCode::ElementNotAllowed));
    assert!(
        codes(&diags).contains(&ValidationDiagnosticCode::InvalidAttributeValue),
        "nested error should be found: {diags:?}"
    );
}
