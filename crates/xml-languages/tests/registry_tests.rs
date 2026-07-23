//! Integration tests for language resolution.

use std::path::{Path, PathBuf};

use xml_languages::{Activation, LanguageProfile, LanguageRegistry, ResolutionOrigin};

fn parse(text: &str) -> xml_parser::ParseResult {
    xml_parser::parse(text)
}

fn profile(name: &str, schema: &str, activation: Activation) -> LanguageProfile {
    LanguageProfile {
        name: name.to_string(),
        schema: PathBuf::from(schema),
        activation,
    }
}

#[test]
fn resolves_by_namespace_profile() {
    let mut registry = LanguageRegistry::new();
    registry.add_profile(profile(
        "speedata",
        "/schemas/layout.rng",
        Activation {
            namespace: Some("urn:speedata.de:2009/publisher/en".to_string()),
            root_element: Some("Layout".to_string()),
            file_names: vec!["layout.xml".to_string()],
            extensions: Vec::new(),
        },
    ));

    let doc = parse(r#"<Layout xmlns="urn:speedata.de:2009/publisher/en"/>"#);
    let resolved = registry
        .resolve(&doc.document, "", Some(Path::new("/tmp/anything.xml")))
        .expect("resolves via namespace");
    assert_eq!(resolved.language.as_deref(), Some("speedata"));
    assert_eq!(resolved.origin, ResolutionOrigin::Profile);

    // File name alone also activates (e.g. missing xmlns while typing).
    let doc = parse("<Layout/>");
    let resolved = registry
        .resolve(&doc.document, "", Some(Path::new("/project/layout.xml")))
        .expect("resolves via file name");
    assert_eq!(resolved.language.as_deref(), Some("speedata"));
}

#[test]
fn best_scoring_profile_wins() {
    let mut registry = LanguageRegistry::new();
    registry.add_profile(profile(
        "generic-xml",
        "/schemas/generic.rng",
        Activation {
            extensions: vec![".xml".to_string()],
            ..Activation::default()
        },
    ));
    registry.add_profile(profile(
        "specific",
        "/schemas/specific.rng",
        Activation {
            namespace: Some("urn:specific".to_string()),
            ..Activation::default()
        },
    ));

    let doc = parse(r#"<r xmlns="urn:specific"/>"#);
    let resolved = registry
        .resolve(&doc.document, "", Some(Path::new("/x/data.xml")))
        .unwrap();
    assert_eq!(resolved.language.as_deref(), Some("specific"));
}

#[test]
fn xml_model_pi_takes_precedence() {
    let mut registry = LanguageRegistry::new();
    registry.add_profile(profile(
        "by-extension",
        "/schemas/other.rng",
        Activation {
            extensions: vec![".xml".to_string()],
            ..Activation::default()
        },
    ));

    let text = r#"<?xml-model href="local-schema.rng"?><root/>"#;
    let doc = parse(text);
    let resolved = registry
        .resolve(&doc.document, text, Some(Path::new("/project/doc.xml")))
        .unwrap();
    assert_eq!(resolved.origin, ResolutionOrigin::XmlModelPi);
    assert_eq!(
        resolved.schema_path,
        PathBuf::from("/project/local-schema.rng")
    );
}

#[test]
fn catalog_fallback_by_namespace() {
    let dir = std::env::temp_dir().join(format!("xml-registry-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let catalog = dir.join("catalog.xml");
    std::fs::write(
        &catalog,
        r#"<catalog xmlns="urn:oasis:names:tc:entity:xmlns:xml:catalog">
  <uri name="urn:example:cat" uri="cat.rng"/>
</catalog>"#,
    )
    .unwrap();

    let mut registry = LanguageRegistry::new();
    assert_eq!(registry.load_catalog(&catalog).unwrap(), 1);

    let doc = parse(r#"<r xmlns="urn:example:cat"/>"#);
    let resolved = registry.resolve(&doc.document, "", None).unwrap();
    assert_eq!(resolved.origin, ResolutionOrigin::Catalog);
    assert_eq!(resolved.schema_path, dir.join("cat.rng"));

    // Unknown namespace: no resolution.
    let doc = parse(r#"<r xmlns="urn:example:unknown"/>"#);
    assert!(registry.resolve(&doc.document, "", None).is_none());
}

#[test]
fn no_match_yields_none() {
    let registry = LanguageRegistry::new();
    let doc = parse("<plain/>");
    assert!(registry
        .resolve(&doc.document, "", Some(Path::new("/x/plain.xml")))
        .is_none());
}
