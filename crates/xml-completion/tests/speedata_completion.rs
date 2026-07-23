//! Completion against the real speedata layout schema (skipped when not
//! available; see `SPEEDATA_SCHEMA_DIR`).

use std::path::PathBuf;

use xml_completion::{complete, CompletionItem};
use xml_schema::{load_rng, Schema};

fn load_schema(file: &str) -> Option<Schema> {
    let dir = std::env::var("SPEEDATA_SCHEMA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join("work/software/publisher/schema")
        });
    let path = dir.join(file);
    let text = std::fs::read_to_string(path).ok()?;
    Some(load_rng(&text).schema)
}

macro_rules! require_schema {
    ($file:expr) => {
        match load_schema($file) {
            Some(schema) => schema,
            None => {
                eprintln!("skipped: speedata schema not found (set SPEEDATA_SCHEMA_DIR)");
                return;
            }
        }
    };
}

fn complete_at(schema: &Schema, marked: &str) -> Vec<CompletionItem> {
    let offset = marked.find('|').expect("marker");
    let text = marked.replacen('|', "", 1);
    let parse = xml_parser::parse(&text);
    complete(schema, &parse.document, &text, offset)
}

#[test]
fn layout_children_with_documentation() {
    let schema = require_schema!("layoutschema-en.rng");
    let items = complete_at(
        &schema,
        r#"<Layout xmlns="urn:speedata.de:2009/publisher/en"><|</Layout>"#,
    );
    let record = items
        .iter()
        .find(|i| i.label == "Record")
        .expect("Record offered");
    assert!(
        record
            .documentation
            .as_deref()
            .is_some_and(|d| d.contains("instructions")),
        "{:?}",
        record.documentation
    );
    assert!(
        items.len() > 20,
        "Layout allows many children: {}",
        items.len()
    );
}

#[test]
fn record_attributes_with_documentation() {
    let schema = require_schema!("layoutschema-en.rng");
    let items = complete_at(
        &schema,
        r#"<Layout xmlns="urn:speedata.de:2009/publisher/en"><Record |</Layout>"#,
    );
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"element"), "{labels:?}");
    assert!(labels.contains(&"mode"), "{labels:?}");
    let element = items.iter().find(|i| i.label == "element").unwrap();
    assert_eq!(element.snippet.as_deref(), Some("element=\"$1\""));
    assert!(element
        .documentation
        .as_deref()
        .is_some_and(|d| d.contains("record matches")));
}

#[test]
fn german_documentation_variant() {
    let schema = require_schema!("layoutschema-de.rng");
    let items = complete_at(
        &schema,
        r#"<Layout xmlns="urn:speedata.de:2009/publisher/en"><|</Layout>"#,
    );
    let record = items
        .iter()
        .find(|i| i.label == "Record")
        .expect("Record offered");
    assert!(
        record
            .documentation
            .as_deref()
            .is_some_and(|d| d.contains("Datensatz") || d.contains("Element")),
        "German docs expected: {:?}",
        record.documentation
    );
}

#[test]
fn value_enum_completion_in_real_schema() {
    let schema = require_schema!("layoutschema-en.rng");
    // Options/@cutmarks is a yes/no enum, Options/@html a three-value one.
    let items = complete_at(
        &schema,
        r#"<Layout xmlns="urn:speedata.de:2009/publisher/en"><Options cutmarks="|"/></Layout>"#,
    );
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(
        labels.contains(&"yes") && labels.contains(&"no"),
        "yes/no enum expected: {labels:?}"
    );

    let items = complete_at(
        &schema,
        r#"<Layout xmlns="urn:speedata.de:2009/publisher/en"><Options html="|"/></Layout>"#,
    );
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, vec!["all", "inner", "off"], "{labels:?}");
}
