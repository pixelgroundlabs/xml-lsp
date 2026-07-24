//! Integration and snapshot tests for hover. The `|` marker in the test
//! input marks the cursor.

use xml_hover::hover;
use xml_schema::load_rng;

const SCHEMA: &str = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0"
    xmlns:a="http://relaxng.org/ns/compatibility/annotations/1.0">
  <start>
    <element name="form">
      <a:documentation>A simple form.</a:documentation>
      <attribute name="action">
        <a:documentation>Target URL of the submission.</a:documentation>
      </attribute>
      <optional>
        <attribute name="method">
          <a:documentation>The HTTP method.</a:documentation>
          <choice>
            <value>get<a:documentation>Read-only request.</a:documentation></value>
            <value>post</value>
          </choice>
        </attribute>
      </optional>
      <optional>
        <attribute name="undocumented"/>
      </optional>
      <zeroOrMore>
        <element name="input"><empty/></element>
      </zeroOrMore>
    </element>
  </start>
</grammar>"#;

fn hover_at(marked: &str) -> Option<String> {
    let offset = marked.find('|').expect("marker");
    let text = marked.replacen('|', "", 1);
    let schema = load_rng(SCHEMA).schema;
    let parse = xml_parser::parse(&text);
    hover(&schema, &parse.document, &text, offset).map(|h| h.markdown)
}

#[test]
fn element_documentation_in_start_and_end_tag() {
    let markdown = hover_at(r#"<fo|rm action="/x"></form>"#).expect("hover on start tag");
    assert!(markdown.contains("**`<form>`**"), "{markdown}");
    assert!(markdown.contains("A simple form."), "{markdown}");

    let markdown = hover_at(r#"<form action="/x"></fo|rm>"#).expect("hover on end tag");
    assert!(markdown.contains("A simple form."), "{markdown}");
}

#[test]
fn attribute_documentation_with_allowed_values() {
    let markdown =
        hover_at(r#"<form action="/x" met|hod="get"></form>"#).expect("hover on attribute");
    assert!(markdown.contains("**`method`**"), "{markdown}");
    assert!(markdown.contains("The HTTP method."), "{markdown}");
    assert!(
        markdown.contains("Allowed values: `get`, `post`"),
        "{markdown}"
    );
}

#[test]
fn value_documentation_when_value_matches() {
    let markdown = hover_at(r#"<form action="/x" method="g|et"></form>"#).expect("hover on value");
    assert!(markdown.contains("**`get`**"), "{markdown}");
    assert!(markdown.contains("Read-only request."), "{markdown}");

    // A value without its own docs falls back to the attribute docs.
    let markdown = hover_at(r#"<form action="/x" method="p|ost"></form>"#).expect("fallback hover");
    assert!(markdown.contains("The HTTP method."), "{markdown}");
}

#[test]
fn no_hover_without_documentation_or_target() {
    // Attribute without docs and without value enum.
    assert!(hover_at(r#"<form action="/x" undoc|umented="1"></form>"#).is_none());
    // Text content is not a hover target.
    assert!(hover_at(r#"<form action="/x">te|xt</form>"#).is_none());
    // Unknown element.
    assert!(hover_at(r#"<unkn|own/>"#).is_none());
}

#[test]
fn snapshot_hover_variants() {
    let render = |marked: &str| hover_at(marked).unwrap_or_else(|| "(no hover)".to_string());
    insta::assert_snapshot!(render(r#"<fo|rm action="/x"></form>"#));
    insta::assert_snapshot!(render(r#"<form action="/x" met|hod="get"></form>"#));
    insta::assert_snapshot!(render(r#"<form action="/x" method="g|et"></form>"#));
}
