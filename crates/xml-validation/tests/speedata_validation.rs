//! Validation tests against the real speedata Publisher layout schema.
//! Skipped when the schema is not available locally (see
//! `SPEEDATA_SCHEMA_DIR`).

use std::path::PathBuf;

use xml_schema::{load_rng, Schema};
use xml_validation::{validate, ValidationDiagnosticCode, Validator};

fn schema_dir() -> Option<PathBuf> {
    let dir = std::env::var("SPEEDATA_SCHEMA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join("work/software/publisher/schema")
        });
    dir.join("layoutschema-en.rng").exists().then_some(dir)
}

fn load_schema() -> Option<Schema> {
    let dir = schema_dir()?;
    let text = std::fs::read_to_string(dir.join("layoutschema-en.rng")).ok()?;
    Some(load_rng(&text).schema)
}

macro_rules! require_schema {
    () => {
        match load_schema() {
            Some(schema) => schema,
            None => {
                eprintln!("skipped: speedata schema not found (set SPEEDATA_SCHEMA_DIR)");
                return;
            }
        }
    };
}

const VALID_LAYOUT: &str = r#"<Layout xmlns="urn:speedata.de:2009/publisher/en">
  <SetGrid height="12pt" width="10mm"/>
  <Pageformat width="210mm" height="297mm"/>
  <Record element="data">
    <PlaceObject>
      <Textblock>
        <Paragraph>
          <Value select="'Hello world'"/>
        </Paragraph>
      </Textblock>
    </PlaceObject>
  </Record>
</Layout>"#;

#[test]
fn valid_layout_has_no_diagnostics() {
    let schema = require_schema!();
    let parse = xml_parser::parse(VALID_LAYOUT);
    assert!(parse.diagnostics.is_empty());
    let diags = validate(&schema, &parse.document, VALID_LAYOUT);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn typo_and_unknown_attribute_are_found() {
    let schema = require_schema!();
    let text = r#"<Layout xmlns="urn:speedata.de:2009/publisher/en">
  <Recod element="data"/>
  <Record element="data" nonsense="1">
    <PlaceObject/>
  </Record>
</Layout>"#;
    let parse = xml_parser::parse(text);
    assert!(parse.diagnostics.is_empty());
    let diags = validate(&schema, &parse.document, text);
    let codes: Vec<_> = diags.iter().map(|d| d.code).collect();
    assert!(
        codes.contains(&ValidationDiagnosticCode::ElementNotAllowed),
        "typo Recod should be reported: {diags:?}"
    );
    assert!(
        codes.contains(&ValidationDiagnosticCode::AttributeNotAllowed),
        "unknown unqualified attribute should be reported: {diags:?}"
    );
    // The typo message should suggest sibling elements from the schema.
    let typo = diags
        .iter()
        .find(|d| d.code == ValidationDiagnosticCode::ElementNotAllowed)
        .unwrap();
    assert!(typo.message.contains("`Record`"), "{}", typo.message);
}

#[test]
fn foreign_namespace_content_is_allowed() {
    let schema = require_schema!();
    // The schema explicitly allows foreign-namespace elements and
    // attributes everywhere (foreign-nodes).
    let text = r#"<Layout xmlns="urn:speedata.de:2009/publisher/en"
        xmlns:doc="urn:my-docs" doc:note="draft">
  <doc:comment>anything at all</doc:comment>
  <Record element="data"/>
</Layout>"#;
    let parse = xml_parser::parse(text);
    assert!(parse.diagnostics.is_empty());
    let diags = validate(&schema, &parse.document, text);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn validator_is_reusable_and_fast_enough() {
    let schema = require_schema!();
    // A larger synthetic layout: many records with text content.
    let mut text = String::from(r#"<Layout xmlns="urn:speedata.de:2009/publisher/en">"#);
    for i in 0..2_000 {
        text.push_str(&format!(
            r#"<Record element="r{i}"><PlaceObject><Textblock><Paragraph><Value select="{i}"/></Paragraph></Textblock></PlaceObject></Record>"#
        ));
    }
    text.push_str("</Layout>");
    let parse = xml_parser::parse(&text);
    assert!(parse.diagnostics.is_empty());

    let mut validator = Validator::new(&schema);
    let start = std::time::Instant::now();
    let diags = validator.validate(&parse.document, &text);
    let first = start.elapsed();
    assert!(diags.is_empty(), "{:?}", &diags[..diags.len().min(3)]);

    // Second run reuses the memo tables and must be clearly faster.
    let start = std::time::Instant::now();
    let diags = validator.validate(&parse.document, &text);
    let second = start.elapsed();
    assert!(diags.is_empty());
    eprintln!("validation: first {first:?}, second {second:?}");
    // Generous bound: catches accidental exponential behavior, not noise.
    assert!(
        second.as_millis() < 2_000,
        "revalidation took {second:?} — memoization broken?"
    );
}
