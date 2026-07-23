//! Snapshot tests: the exact wording and positions of validation
//! diagnostics are part of the user experience; changes must be reviewed
//! deliberately (`cargo insta review`).

use xml_schema::load_rng;
use xml_validation::validate;

const SCHEMA: &str = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0">
  <start><ref name="recipe"/></start>
  <define name="recipe">
    <element name="recipe">
      <attribute name="serves">
        <choice><value>2</value><value>4</value></choice>
      </attribute>
      <element name="name"><text/></element>
      <oneOrMore><element name="step"><text/></element></oneOrMore>
    </element>
  </define>
</grammar>"#;

fn render(doc_text: &str) -> String {
    let schema = load_rng(SCHEMA).schema;
    let parse = xml_parser::parse(doc_text);
    let diags = validate(&schema, &parse.document, doc_text);
    if diags.is_empty() {
        return "(no diagnostics)".to_string();
    }
    diags
        .iter()
        .map(|d| {
            format!(
                "{:?} [{}] {}..{}: {}",
                d.severity,
                d.code.as_str(),
                d.range.start,
                d.range.end,
                d.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn snapshot_valid_document() {
    insta::assert_snapshot!(render(
        r#"<recipe serves="2"><name>Soup</name><step>Cook.</step></recipe>"#
    ));
}

#[test]
fn snapshot_multiple_problems() {
    insta::assert_snapshot!(render(
        r#"<recipe serves="3" vegan="yes"><step>Cook.</step><garnish/></recipe>"#
    ));
}

#[test]
fn snapshot_missing_everything() {
    insta::assert_snapshot!(render("<recipe/>"));
}
