//! Navigation and comfort features through the server: definition,
//! references, rename, quick fixes, semantic tokens, inlay hints, and
//! symbol completion. Symbol knowledge comes from `lsp:*` annotations
//! in a test schema — the server itself has no built-in language rules.

use std::path::PathBuf;

use serde_json::json;
use tower_lsp::lsp_types::{
    CodeActionContext, CodeActionOrCommand, CodeActionParams, CompletionParams, CompletionResponse,
    DidOpenTextDocumentParams, GotoDefinitionParams, GotoDefinitionResponse, InitializeParams,
    InlayHintParams, PartialResultParams, Position, Range, ReferenceContext, ReferenceParams,
    RenameParams, SemanticTokensParams, SemanticTokensResult, TextDocumentIdentifier,
    TextDocumentItem, TextDocumentPositionParams, Url, WorkDoneProgressParams,
};
use tower_lsp::{LanguageServer, LspService};
use xml_server::Backend;

/// Annotated grammar: `<color name>` defines colors, `fill` references
/// them.
const TEST_SCHEMA: &str = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0"
    xmlns:lsp="urn:xml-lsp:annotations" ns="urn:test">
  <start>
    <element name="root">
      <zeroOrMore>
        <choice>
          <element name="color">
            <attribute name="name"><lsp:defines symbol="color"/></attribute>
          </element>
          <element name="box">
            <optional><attribute name="fill"><lsp:references symbol="color"/></attribute></optional>
          </element>
        </choice>
      </zeroOrMore>
    </element>
  </start>
</grammar>"#;

const DOC: &str = r#"<root xmlns="urn:test">
  <color name="accent"/>
  <box fill="accent"/>
</root>"#;

fn schema_path() -> PathBuf {
    // Written exactly once per process: the tests run in parallel, and
    // a concurrent truncate-and-write could hand a reader an empty file.
    static PATH: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    PATH.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("xml-navigation-flow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("annotated.rng");
        std::fs::write(&path, TEST_SCHEMA).unwrap();
        path
    })
    .clone()
}

async fn open(text: &str) -> (LspService<Backend>, Url) {
    let (service, _socket) = LspService::new(Backend::new);
    service
        .inner()
        .initialize(InitializeParams {
            initialization_options: Some(json!({
                "schema": {
                    "languages": [{
                        "schema": schema_path().to_str().unwrap(),
                        "namespace": "urn:test"
                    }]
                }
            })),
            ..InitializeParams::default()
        })
        .await
        .unwrap();
    let uri = Url::parse("file:///project/doc.xml").unwrap();
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
    (service, uri)
}

fn at(uri: &Url, line: u32, character: u32) -> TextDocumentPositionParams {
    TextDocumentPositionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        position: Position { line, character },
    }
}

#[tokio::test]
async fn definition_from_reference() {
    let (service, uri) = open(DOC).await;
    // Cursor inside the `fill="accent"` value on line 2.
    let response = service
        .inner()
        .goto_definition(GotoDefinitionParams {
            text_document_position_params: at(&uri, 2, 15),
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        })
        .await
        .unwrap()
        .expect("definition");
    let GotoDefinitionResponse::Scalar(location) = response else {
        panic!("scalar location expected");
    };
    assert_eq!(location.uri, uri);
    assert_eq!(location.range.start.line, 1);
    assert_eq!(location.range.start.character, 15);
    assert_eq!(location.range.end.character, 21);
}

#[tokio::test]
async fn references_include_definition_on_request() {
    let (service, uri) = open(DOC).await;
    let refs = |include| {
        let uri = uri.clone();
        let service = &service;
        async move {
            service
                .inner()
                .references(ReferenceParams {
                    text_document_position: at(&uri, 1, 17),
                    work_done_progress_params: WorkDoneProgressParams::default(),
                    partial_result_params: PartialResultParams::default(),
                    context: ReferenceContext {
                        include_declaration: include,
                    },
                })
                .await
                .unwrap()
        }
    };
    assert_eq!(refs(true).await.expect("locations").len(), 2);
    assert_eq!(refs(false).await.expect("locations").len(), 1);
}

#[tokio::test]
async fn rename_updates_definition_and_references() {
    let (service, uri) = open(DOC).await;
    let edit = service
        .inner()
        .rename(RenameParams {
            text_document_position: at(&uri, 2, 15),
            new_name: "brand".to_string(),
            work_done_progress_params: WorkDoneProgressParams::default(),
        })
        .await
        .unwrap()
        .expect("workspace edit");
    let changes = edit.changes.expect("changes");
    assert_eq!(changes[&uri].len(), 2);
    assert!(changes[&uri].iter().all(|e| e.new_text == "brand"));
}

#[tokio::test]
async fn completion_offers_defined_symbol_names() {
    let text = r#"<root xmlns="urn:test">
  <color name="accent"/>
  <box fill=""/>
</root>"#;
    let (service, uri) = open(text).await;
    // Cursor inside the empty fill value on line 2 (col 13).
    let response = service
        .inner()
        .completion(CompletionParams {
            text_document_position: at(&uri, 2, 13),
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
            context: None,
        })
        .await
        .unwrap()
        .expect("completions");
    let CompletionResponse::Array(items) = response else {
        panic!("array expected");
    };
    assert!(items.iter().any(|item| item.label == "accent"), "{items:?}");
}

#[tokio::test]
async fn semantic_tokens_mark_symbol_values() {
    let (service, uri) = open(DOC).await;
    let result = service
        .inner()
        .semantic_tokens_full(SemanticTokensParams {
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
            text_document: TextDocumentIdentifier { uri },
        })
        .await
        .unwrap()
        .expect("tokens");
    let SemanticTokensResult::Tokens(tokens) = result else {
        panic!("full tokens expected");
    };
    // `accent` twice: as definition and as reference (type 7 = variable).
    let variables: Vec<_> = tokens.data.iter().filter(|t| t.token_type == 7).collect();
    assert_eq!(variables.len(), 2);
    assert_eq!(variables[0].token_modifiers_bitset, 1);
    assert_eq!(variables[1].token_modifiers_bitset, 0);
}

#[tokio::test]
async fn quick_fix_closes_unclosed_element() {
    let (service, uri) = open("<a><b>text</a>").await;
    let actions = service
        .inner()
        .code_action(CodeActionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 14,
                },
            },
            context: CodeActionContext::default(),
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        })
        .await
        .unwrap()
        .expect("actions");
    let close = actions
        .iter()
        .find_map(|action| match action {
            CodeActionOrCommand::CodeAction(action) if action.title == "Close `<b>`" => {
                Some(action)
            }
            _ => None,
        })
        .expect("close action");
    let changes = close.edit.as_ref().unwrap().changes.as_ref().unwrap();
    assert_eq!(changes[&uri][0].new_text, "</b>");
}

#[tokio::test]
async fn inlay_hint_after_distant_end_tag() {
    let text = format!("<Record element=\"data\">{}</Record>", "\n<Br/>".repeat(10));
    let (service, uri) = open(&text).await;
    let hints = service
        .inner()
        .inlay_hint(InlayHintParams {
            work_done_progress_params: WorkDoneProgressParams::default(),
            text_document: TextDocumentIdentifier { uri },
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 11,
                    character: 0,
                },
            },
        })
        .await
        .unwrap()
        .expect("hints");
    assert_eq!(hints.len(), 1);
    assert_eq!(hints[0].position.line, 10);
}
