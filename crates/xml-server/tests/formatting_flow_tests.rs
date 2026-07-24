//! Formatting through the server: didOpen → textDocument/formatting.

use tower_lsp::lsp_types::{
    DidOpenTextDocumentParams, DocumentFormattingParams, FormattingOptions, InitializeParams,
    TextDocumentIdentifier, TextDocumentItem, Url, WorkDoneProgressParams,
};
use tower_lsp::{LanguageServer, LspService};
use xml_server::Backend;

#[tokio::test]
async fn formatting_returns_full_document_edit() {
    let (service, _socket) = LspService::new(Backend::new);
    let init = service
        .inner()
        .initialize(InitializeParams::default())
        .await
        .unwrap();
    assert!(
        init.capabilities.document_formatting_provider.is_some(),
        "formatting capability must be announced"
    );

    let uri = Url::parse("file:///project/doc.xml").unwrap();
    service
        .inner()
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "xml".to_string(),
                version: 1,
                text: "<a><b>text</b><c></c></a>".to_string(),
            },
        })
        .await;

    let edits = service
        .inner()
        .formatting(DocumentFormattingParams {
            text_document: TextDocumentIdentifier { uri },
            options: FormattingOptions {
                tab_size: 2,
                insert_spaces: true,
                ..FormattingOptions::default()
            },
            work_done_progress_params: WorkDoneProgressParams::default(),
        })
        .await
        .unwrap()
        .expect("edits");
    assert_eq!(edits.len(), 1);
    // Depth-1 siblings get blank-line separation.
    assert_eq!(
        edits[0].new_text,
        "<a>\n  <b>\n    text\n  </b>\n\n  <c />\n</a>\n"
    );
    // The edit covers the whole document.
    assert_eq!(edits[0].range.start.line, 0);
    assert_eq!(edits[0].range.start.character, 0);
}
