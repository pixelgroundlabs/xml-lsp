//! Hover against the real speedata layout schema (skipped when not
//! available; see `SPEEDATA_SCHEMA_DIR`).

use std::path::PathBuf;

use xml_hover::hover;
use xml_schema::{load_rng, Schema};

fn load_schema(file: &str) -> Option<Schema> {
    let dir = std::env::var("SPEEDATA_SCHEMA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join("work/software/publisher/schema")
        });
    let text = std::fs::read_to_string(dir.join(file)).ok()?;
    Some(load_rng(&text).schema)
}

fn hover_at(schema: &Schema, marked: &str) -> Option<String> {
    let offset = marked.find('|').expect("marker");
    let text = marked.replacen('|', "", 1);
    let parse = xml_parser::parse(&text);
    hover(schema, &parse.document, &text, offset).map(|h| h.markdown)
}

#[test]
fn record_and_attribute_docs() {
    let Some(schema) = load_schema("layoutschema-en.rng") else {
        eprintln!("skipped: speedata schema not found (set SPEEDATA_SCHEMA_DIR)");
        return;
    };
    let markdown = hover_at(
        &schema,
        r#"<Layout xmlns="urn:speedata.de:2009/publisher/en"><Rec|ord element="data"/></Layout>"#,
    )
    .expect("hover on Record");
    assert!(markdown.contains("instructions"), "{markdown}");

    let markdown = hover_at(
        &schema,
        r#"<Layout xmlns="urn:speedata.de:2009/publisher/en"><Record ele|ment="data"/></Layout>"#,
    )
    .expect("hover on element attribute");
    assert!(markdown.contains("record matches"), "{markdown}");
}

#[test]
fn german_docs_variant() {
    let Some(schema) = load_schema("layoutschema-de.rng") else {
        eprintln!("skipped: speedata schema not found (set SPEEDATA_SCHEMA_DIR)");
        return;
    };
    let markdown = hover_at(
        &schema,
        r#"<La|yout xmlns="urn:speedata.de:2009/publisher/en"/>"#,
    )
    .expect("hover on Layout");
    assert!(
        markdown.contains("Wurzelelement"),
        "German docs expected: {markdown}"
    );
}
