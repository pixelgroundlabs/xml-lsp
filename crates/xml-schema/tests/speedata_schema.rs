//! Integration tests against the real speedata Publisher layout schema.
//!
//! The schema is not part of this repository. The tests look for it in
//! `SPEEDATA_SCHEMA_DIR` (falling back to the conventional local checkout
//! path) and are skipped with a notice if it is not present.

use std::path::PathBuf;

use xml_schema::{load_rng, Schema};

const EN_NAMESPACE: &str = "urn:speedata.de:2009/publisher/en";

fn schema_dir() -> Option<PathBuf> {
    let dir = std::env::var("SPEEDATA_SCHEMA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_default();
            PathBuf::from(home).join("work/software/publisher/schema")
        });
    dir.join("layoutschema-en.rng").exists().then_some(dir)
}

fn load_speedata(file: &str) -> Option<Schema> {
    let dir = schema_dir()?;
    let text = std::fs::read_to_string(dir.join(file)).expect("schema file is readable");
    let result = load_rng(&text);
    assert!(
        result.diagnostics.is_empty(),
        "schema {file} should load without diagnostics: {:?}",
        &result.diagnostics[..result.diagnostics.len().min(5)]
    );
    Some(result.schema)
}

macro_rules! require_schema {
    ($file:expr) => {
        match load_speedata($file) {
            Some(schema) => schema,
            None => {
                eprintln!("skipped: speedata schema not found (set SPEEDATA_SCHEMA_DIR)");
                return;
            }
        }
    };
}

#[test]
fn english_schema_loads_cleanly() {
    let schema = require_schema!("layoutschema-en.rng");
    // The schema is generated and large; sanity-check the size.
    assert!(schema.defines().count() >= 100, "expected >= 100 defines");
}

#[test]
fn start_allows_layout_and_include() {
    let schema = require_schema!("layoutschema-en.rng");
    let roots = schema.child_elements(schema.start());
    let names: Vec<String> = roots
        .iter()
        .filter_map(|&id| schema.element_pattern(id))
        .filter_map(|el| el.name.as_name().map(|(_, local)| local.to_string()))
        .collect();
    assert!(names.contains(&"Layout".to_string()), "roots: {names:?}");
    assert!(names.contains(&"Include".to_string()), "roots: {names:?}");
    // Root elements live in the speedata namespace.
    let layout = roots
        .iter()
        .find(|&&id| {
            schema
                .element_pattern(id)
                .is_some_and(|el| el.name.matches(Some(EN_NAMESPACE), "Layout"))
        })
        .copied();
    assert!(layout.is_some());
}

#[test]
fn element_docs_and_attributes_are_available() {
    let schema = require_schema!("layoutschema-en.rng");

    // <A> is the hyperlink element; its href attribute is documented.
    let a = schema
        .find_element(Some(EN_NAMESPACE), "A")
        .expect("element A exists");
    let a_el = schema.element_pattern(a).unwrap();
    assert!(
        a_el.documentation
            .as_deref()
            .is_some_and(|d| d.contains("hyperlink")),
        "docs of <A>: {:?}",
        a_el.documentation
    );

    let attrs = schema.attributes(a_el.content);
    let href = attrs
        .iter()
        .filter_map(|&id| schema.attribute_pattern(id))
        .find(|attr| attr.name.matches(None, "href"))
        .expect("attribute href exists");
    assert!(
        href.documentation
            .as_deref()
            .is_some_and(|d| d.contains("hyperlink")),
        "docs of href: {:?}",
        href.documentation
    );

    // <A> allows nested inline content such as <B> and <I>.
    let children: Vec<String> = schema
        .child_elements(a_el.content)
        .iter()
        .filter_map(|&id| schema.element_pattern(id))
        .filter_map(|el| el.name.as_name().map(|(_, local)| local.to_string()))
        .collect();
    assert!(
        children.contains(&"B".to_string()),
        "children: {children:?}"
    );
    assert!(children.contains(&"I".to_string()));
}

#[test]
fn value_enumerations_are_plentiful() {
    let schema = require_schema!("layoutschema-en.rng");
    // The schema documents hundreds of allowed attribute values; make sure
    // they arrive in the model (completion will feed on these).
    let mut value_count = 0;
    for (_, define) in schema.defines() {
        value_count += schema.value_enum(define.pattern).len();
        for element in schema.child_elements(define.pattern) {
            let content = schema.element_pattern(element).unwrap().content;
            for attr in schema.attributes(content) {
                let attr_content = schema.attribute_pattern(attr).unwrap().content;
                value_count += schema.value_enum(attr_content).len();
            }
        }
    }
    assert!(value_count > 300, "found only {value_count} enum values");
}

#[test]
fn german_schema_loads_cleanly() {
    // The German variant shares the namespace and element names with the
    // English one; only the documentation language differs.
    let schema = require_schema!("layoutschema-de.rng");
    let layout = schema
        .find_element(Some(EN_NAMESPACE), "Layout")
        .expect("element Layout exists");
    let docs = schema
        .element_pattern(layout)
        .unwrap()
        .documentation
        .as_deref();
    assert!(
        docs.is_some_and(|d| d.contains("Wurzelelement")),
        "German docs expected: {docs:?}"
    );
}
