//! Snapshot tests for completion: labels, kinds, inserts, and ranges at
//! representative cursor positions (`cargo insta review` to approve
//! changes).

use xml_completion::complete;
use xml_schema::load_rng;

const SCHEMA: &str = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0"
    xmlns:a="http://relaxng.org/ns/compatibility/annotations/1.0">
  <start>
    <element name="form">
      <a:documentation>A simple form.</a:documentation>
      <attribute name="action"/>
      <optional>
        <attribute name="method">
          <choice>
            <value>get<a:documentation>Read-only request.</a:documentation></value>
            <value>post</value>
          </choice>
        </attribute>
      </optional>
      <zeroOrMore>
        <element name="input"><a:documentation>An input field.</a:documentation><empty/></element>
      </zeroOrMore>
    </element>
  </start>
</grammar>"#;

fn render(marked: &str) -> String {
    let offset = marked.find('|').expect("marker");
    let text = marked.replacen('|', "", 1);
    let schema = load_rng(SCHEMA).schema;
    let parse = xml_parser::parse(&text);
    let items = complete(&schema, &parse.document, &text, offset);
    if items.is_empty() {
        return "(no items)".to_string();
    }
    items
        .iter()
        .map(|i| {
            format!(
                "{:?}{} `{}` -> `{}` @ {}..{}{}",
                i.kind,
                if i.required { " (required)" } else { "" },
                i.label,
                i.snippet.as_deref().unwrap_or(&i.insert),
                i.replace.start,
                i.replace.end,
                i.documentation
                    .as_deref()
                    .map(|d| format!("  — {d}"))
                    .unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn snapshot_element_at_root() {
    insta::assert_snapshot!(render("<|"));
}

#[test]
fn snapshot_attributes() {
    insta::assert_snapshot!(render("<form |>"));
}

#[test]
fn snapshot_values() {
    insta::assert_snapshot!(render(r#"<form action="/x" method="|">"#));
}

#[test]
fn snapshot_child_and_close() {
    insta::assert_snapshot!(render(r#"<form action="/x"><inp|</form>"#));
    insta::assert_snapshot!(render(r#"<form action="/x"><input/></|"#));
}
