//! End-to-end flow inside the server: configuration → language
//! activation → schema loading → validation diagnostics.

use std::path::PathBuf;

use serde_json::json;
use tower_lsp::lsp_types::{DidOpenTextDocumentParams, InitializeParams, TextDocumentItem, Url};
use tower_lsp::{LanguageServer, LspService};
use xml_server::Backend;

const TEST_SCHEMA: &str = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0" ns="urn:test:notes">
  <start>
    <element name="notes">
      <zeroOrMore>
        <element name="note">
          <attribute name="prio">
            <choice><value>low</value><value>high</value></choice>
          </attribute>
          <text/>
        </element>
      </zeroOrMore>
    </element>
  </start>
</grammar>"#;

fn setup_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xml-server-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.rng"), TEST_SCHEMA).unwrap();
    std::fs::write(
        dir.join("catalog.xml"),
        r#"<catalog xmlns="urn:oasis:names:tc:entity:xmlns:xml:catalog">
  <uri name="urn:test:notes" uri="notes.rng"/>
</catalog>"#,
    )
    .unwrap();
    dir
}

async fn initialized_backend(options: serde_json::Value) -> (LspService<Backend>, Url) {
    let (service, _socket) = LspService::new(Backend::new);
    // A real, platform-valid path: file:///project/... has no drive
    // letter and cannot be converted back to a path on Windows, which
    // would break file-name/extension based activation.
    let uri = Url::from_file_path(std::env::temp_dir().join("notes.xml")).unwrap();
    service
        .inner()
        .initialize(InitializeParams {
            initialization_options: Some(options),
            ..InitializeParams::default()
        })
        .await
        .expect("initialize succeeds");
    (service, uri)
}

async fn open(service: &LspService<Backend>, uri: &Url, text: &str) {
    service
        .inner()
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "xml".to_string(),
                version: 1,
                text: text.to_string(),
            },
        })
        .await;
}

#[tokio::test]
async fn catalog_activation_produces_validation_diagnostics() {
    let dir = setup_dir("catalog");
    let options = json!({
        "schema": { "catalogs": [dir.join("catalog.xml").to_str().unwrap()] }
    });
    let (service, uri) = initialized_backend(options).await;

    // Invalid: prio has a bad value, and <todo> is unknown.
    open(
        &service,
        &uri,
        r#"<notes xmlns="urn:test:notes"><note prio="urgent">x</note><todo/></notes>"#,
    )
    .await;

    let (_, diags) = service.inner().compute_diagnostics(&uri).unwrap();
    let codes: Vec<String> = diags
        .iter()
        .filter_map(|d| match &d.code {
            Some(tower_lsp::lsp_types::NumberOrString::String(s)) => Some(s.clone()),
            _ => None,
        })
        .collect();
    assert!(
        codes.contains(&"invalid-attribute-value".to_string()),
        "{codes:?}"
    );
    assert!(
        codes.contains(&"element-not-allowed".to_string()),
        "{codes:?}"
    );
}

#[tokio::test]
async fn language_profile_activation_by_file_name() {
    let dir = setup_dir("profile");
    let options = json!({
        "schema": {
            "languages": [{
                "name": "notes",
                "schema": dir.join("notes.rng").to_str().unwrap(),
                "fileNames": ["notes.xml"]
            }]
        }
    });
    let (service, uri) = initialized_backend(options).await;

    // Valid document: no diagnostics at all.
    open(
        &service,
        &uri,
        r#"<notes xmlns="urn:test:notes"><note prio="low">x</note></notes>"#,
    )
    .await;
    let (_, diags) = service.inner().compute_diagnostics(&uri).unwrap();
    assert!(diags.is_empty(), "{diags:?}");
}

#[tokio::test]
async fn missing_schema_yields_notice_not_silence() {
    let options = json!({
        "schema": {
            "languages": [{
                "schema": "/nonexistent/schema.rng",
                "extensions": [".xml"]
            }]
        }
    });
    let (service, uri) = initialized_backend(options).await;
    open(&service, &uri, "<anything/>").await;

    let (_, diags) = service.inner().compute_diagnostics(&uri).unwrap();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(diags[0].message.contains("cannot load schema"));
}

#[tokio::test]
async fn no_schema_means_parser_diagnostics_only() {
    let (service, uri) = initialized_backend(json!({})).await;
    open(&service, &uri, "<a><b></a>").await;
    let (_, diags) = service.inner().compute_diagnostics(&uri).unwrap();
    // Well-formedness errors are still reported, nothing schema-related.
    assert_eq!(diags.len(), 1);
    assert!(diags[0].message.contains("closing tag"));
}

#[tokio::test]
async fn empty_did_change_configuration_keeps_settings() {
    // Zed sends `workspace/didChangeConfiguration` with empty settings
    // right after startup; that must not wipe the configuration from
    // initializationOptions (regression test).
    use tower_lsp::lsp_types::DidChangeConfigurationParams;

    let dir = setup_dir("keep-config");
    let options = json!({
        "schema": { "catalogs": [dir.join("catalog.xml").to_str().unwrap()] }
    });
    let (service, uri) = initialized_backend(options).await;

    service
        .inner()
        .did_change_configuration(DidChangeConfigurationParams {
            settings: json!({}),
        })
        .await;
    service
        .inner()
        .did_change_configuration(DidChangeConfigurationParams {
            settings: serde_json::Value::Null,
        })
        .await;

    open(
        &service,
        &uri,
        r#"<notes xmlns="urn:test:notes"><bad/></notes>"#,
    )
    .await;
    let (_, diags) = service.inner().compute_diagnostics(&uri).unwrap();
    assert_eq!(diags.len(), 1, "validation must still run: {diags:?}");

    // A meaningful configuration change still applies.
    service
        .inner()
        .did_change_configuration(DidChangeConfigurationParams {
            settings: json!({ "diagnostics": { "enable": false } }),
        })
        .await;
    let (_, diags) = service.inner().compute_diagnostics(&uri).unwrap();
    assert!(diags.is_empty(), "diagnostics disabled: {diags:?}");
}

#[tokio::test]
async fn xml_model_pi_activates_schema() {
    let dir = setup_dir("xmlmodel");
    let (service, _) = initialized_backend(json!({})).await;
    // Document points at its schema itself; URI must be inside `dir` so
    // the relative href resolves.
    let uri = Url::from_file_path(dir.join("doc.xml")).unwrap();
    open(
        &service,
        &uri,
        r#"<?xml-model href="notes.rng"?><notes xmlns="urn:test:notes"><bad/></notes>"#,
    )
    .await;
    let (_, diags) = service.inner().compute_diagnostics(&uri).unwrap();
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert!(diags[0].message.contains("`bad`"));
}
