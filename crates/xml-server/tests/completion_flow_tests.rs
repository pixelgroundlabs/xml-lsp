//! Completion through the server: configuration → activation → items,
//! including snippet capability handling.

use std::path::PathBuf;

use serde_json::json;
use tower_lsp::lsp_types::{
    ClientCapabilities, CompletionClientCapabilities, CompletionItemCapability, CompletionParams,
    CompletionResponse, CompletionTextEdit, DidOpenTextDocumentParams, InitializeParams,
    PartialResultParams, Position, TextDocumentClientCapabilities, TextDocumentIdentifier,
    TextDocumentItem, TextDocumentPositionParams, Url, WorkDoneProgressParams,
};
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

fn setup_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("xml-completion-flow-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.rng"), TEST_SCHEMA).unwrap();
    dir
}

fn snippet_capabilities() -> ClientCapabilities {
    ClientCapabilities {
        text_document: Some(TextDocumentClientCapabilities {
            completion: Some(CompletionClientCapabilities {
                completion_item: Some(CompletionItemCapability {
                    snippet_support: Some(true),
                    ..CompletionItemCapability::default()
                }),
                ..CompletionClientCapabilities::default()
            }),
            ..TextDocumentClientCapabilities::default()
        }),
        ..ClientCapabilities::default()
    }
}

async fn setup(snippets: bool) -> (LspService<Backend>, Url) {
    let dir = setup_dir();
    let (service, _socket) = LspService::new(Backend::new);
    let capabilities = if snippets {
        snippet_capabilities()
    } else {
        ClientCapabilities::default()
    };
    service
        .inner()
        .initialize(InitializeParams {
            capabilities,
            initialization_options: Some(json!({
                "schema": {
                    "languages": [{
                        "schema": dir.join("notes.rng").to_str().unwrap(),
                        "namespace": "urn:test:notes"
                    }]
                }
            })),
            ..InitializeParams::default()
        })
        .await
        .unwrap();

    let uri = Url::parse("file:///project/notes.xml").unwrap();
    // Cursor target: inside <note …>, attribute position.
    service
        .inner()
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "xml".to_string(),
                version: 1,
                text: r#"<notes xmlns="urn:test:notes"><note ></note></notes>"#.to_string(),
            },
        })
        .await;
    (service, uri)
}

async fn complete_at(
    service: &LspService<Backend>,
    uri: &Url,
    line: u32,
    character: u32,
) -> Vec<tower_lsp::lsp_types::CompletionItem> {
    let response = service
        .inner()
        .completion(CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line, character },
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
            context: None,
        })
        .await
        .unwrap();
    match response {
        Some(CompletionResponse::Array(items)) => items,
        other => panic!("expected item array, got {other:?}"),
    }
}

#[tokio::test]
async fn attribute_completion_with_snippets() {
    let (service, uri) = setup(true).await;
    // Position inside `<note |>` (column 36 = after the space).
    let items = complete_at(&service, &uri, 0, 36).await;
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.label, "prio");
    // Required attribute sorts first.
    assert_eq!(item.sort_text.as_deref(), Some("0prio"));
    match &item.text_edit {
        Some(CompletionTextEdit::Edit(edit)) => {
            assert_eq!(edit.new_text, "prio=\"$1\"");
        }
        other => panic!("expected text edit, got {other:?}"),
    }
    assert_eq!(
        item.insert_text_format,
        Some(tower_lsp::lsp_types::InsertTextFormat::SNIPPET)
    );
}

#[tokio::test]
async fn attribute_completion_without_snippets_is_plain() {
    let (service, uri) = setup(false).await;
    let items = complete_at(&service, &uri, 0, 36).await;
    match &items[0].text_edit {
        Some(CompletionTextEdit::Edit(edit)) => {
            assert_eq!(edit.new_text, "prio=\"\"");
        }
        other => panic!("expected text edit, got {other:?}"),
    }
    assert_eq!(items[0].insert_text_format, None);
}
