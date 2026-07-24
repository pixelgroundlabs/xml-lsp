//! Hover through the server: configuration → activation → markdown.

use std::path::PathBuf;

use serde_json::json;
use tower_lsp::lsp_types::{
    DidOpenTextDocumentParams, HoverContents, HoverParams, InitializeParams, Position,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams, Url,
    WorkDoneProgressParams,
};
use tower_lsp::{LanguageServer, LspService};
use xml_server::Backend;

const TEST_SCHEMA: &str = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0"
    xmlns:a="http://relaxng.org/ns/compatibility/annotations/1.0" ns="urn:test:notes">
  <start>
    <element name="notes">
      <a:documentation>The collection of notes.</a:documentation>
      <zeroOrMore>
        <element name="note">
          <attribute name="prio">
            <a:documentation>Priority of the note.</a:documentation>
            <choice><value>low</value><value>high</value></choice>
          </attribute>
          <text/>
        </element>
      </zeroOrMore>
    </element>
  </start>
</grammar>"#;

#[tokio::test]
async fn hover_shows_schema_documentation() {
    let dir = std::env::temp_dir().join(format!("xml-hover-flow-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let schema_path: PathBuf = dir.join("notes.rng");
    std::fs::write(&schema_path, TEST_SCHEMA).unwrap();

    let (service, _socket) = LspService::new(Backend::new);
    service
        .inner()
        .initialize(InitializeParams {
            initialization_options: Some(json!({
                "schema": {
                    "languages": [{
                        "schema": schema_path.to_str().unwrap(),
                        "namespace": "urn:test:notes"
                    }]
                }
            })),
            ..InitializeParams::default()
        })
        .await
        .unwrap();

    let uri = Url::parse("file:///project/notes.xml").unwrap();
    service
        .inner()
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "xml".to_string(),
                version: 1,
                text: r#"<notes xmlns="urn:test:notes"><note prio="low">x</note></notes>"#
                    .to_string(),
            },
        })
        .await;

    // Hover over "prio" (column 37).
    let hover = service
        .inner()
        .hover(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 0,
                    character: 37,
                },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        })
        .await
        .unwrap()
        .expect("hover result");

    let HoverContents::Markup(markup) = hover.contents else {
        panic!("expected markup contents");
    };
    assert!(
        markup.value.contains("Priority of the note."),
        "{}",
        markup.value
    );
    assert!(
        markup.value.contains("Allowed values: `low`, `high`"),
        "{}",
        markup.value
    );
    let range = hover.range.expect("range set");
    assert_eq!(range.start.character, 36);
    assert_eq!(range.end.character, 40);

    // Hover in plain text yields nothing.
    let none = service
        .inner()
        .hover(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position {
                    line: 0,
                    character: 47,
                },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        })
        .await
        .unwrap();
    assert!(none.is_none());
}
