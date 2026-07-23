//! Integration tests for the server: document lifecycle and initialize
//! handshake through the public API.

use tower_lsp::lsp_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    InitializeParams, Position, Range, TextDocumentContentChangeEvent,
    TextDocumentContentChangeEvent as Change, TextDocumentIdentifier, TextDocumentItem,
    TextDocumentSyncCapability, TextDocumentSyncKind, Url, VersionedTextDocumentIdentifier,
};
use tower_lsp::{LanguageServer, LspService};
use xml_server::Backend;

fn test_uri() -> Url {
    Url::parse("file:///test/example.xml").unwrap()
}

fn incremental_change(
    start: (u32, u32),
    end: (u32, u32),
    text: &str,
) -> TextDocumentContentChangeEvent {
    Change {
        range: Some(Range {
            start: Position {
                line: start.0,
                character: start.1,
            },
            end: Position {
                line: end.0,
                character: end.1,
            },
        }),
        range_length: None,
        text: text.to_string(),
    }
}

#[tokio::test]
async fn initialize_advertises_incremental_sync() {
    let (service, _socket) = LspService::new(Backend::new);
    let result = service
        .inner()
        .initialize(InitializeParams::default())
        .await
        .expect("initialize succeeds");

    assert_eq!(
        result.capabilities.text_document_sync,
        Some(TextDocumentSyncCapability::Kind(
            TextDocumentSyncKind::INCREMENTAL
        ))
    );
    let info = result.server_info.expect("server_info is set");
    assert_eq!(info.name, "xml-lsp");
}

#[tokio::test]
async fn open_change_close_lifecycle() {
    let (service, _socket) = LspService::new(Backend::new);
    let backend = service.inner();
    let uri = test_uri();

    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "xml".to_string(),
                version: 1,
                text: "<root>\n</root>\n".to_string(),
            },
        })
        .await;

    {
        let doc = backend.store().get(&uri).expect("document is cached");
        assert_eq!(doc.version, 1);
        assert!(doc.parse.diagnostics.is_empty());
    }

    // Incremental change: insert "<kind/>" at the start of line 1.
    backend
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version: 2,
            },
            content_changes: vec![incremental_change((1, 0), (1, 0), "  <kind/>\n")],
        })
        .await;

    {
        let doc = backend.store().get(&uri).expect("document is cached");
        assert_eq!(doc.version, 2);
        assert_eq!(doc.text, "<root>\n  <kind/>\n</root>\n");
        assert!(doc.parse.diagnostics.is_empty());
        let tree = &doc.parse.document;
        let root = tree.root_element().expect("root element");
        assert_eq!(tree.children(root).len(), 1);
    }

    // A breaking change produces diagnostics.
    backend
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version: 3,
            },
            content_changes: vec![incremental_change((1, 2), (1, 9), "<open>")],
        })
        .await;

    {
        let doc = backend.store().get(&uri).expect("document is cached");
        assert_eq!(doc.text, "<root>\n  <open>\n</root>\n");
        assert!(
            !doc.parse.diagnostics.is_empty(),
            "expected unclosed-element"
        );
    }

    backend
        .did_close(DidCloseTextDocumentParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
        })
        .await;
    assert!(backend.store().get(&uri).is_none());
}

#[tokio::test]
async fn multibyte_positions_are_utf16() {
    let (service, _socket) = LspService::new(Backend::new);
    let backend = service.inner();
    let uri = test_uri();

    // "😀" occupies 2 UTF-16 units; the attribute value continues after it.
    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "xml".to_string(),
                version: 1,
                text: "<a t=\"😀x\"/>".to_string(),
            },
        })
        .await;

    // Replace the "x" after the emoji (UTF-16 columns 8..9).
    backend
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version: 2,
            },
            content_changes: vec![incremental_change((0, 8), (0, 9), "y")],
        })
        .await;

    let doc = backend.store().get(&uri).unwrap();
    assert_eq!(doc.text, "<a t=\"😀y\"/>");
    assert!(doc.parse.diagnostics.is_empty());
}

#[tokio::test]
async fn full_document_change() {
    let (service, _socket) = LspService::new(Backend::new);
    let backend = service.inner();
    let uri = test_uri();

    backend
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "xml".to_string(),
                version: 1,
                text: "<old/>".to_string(),
            },
        })
        .await;

    backend
        .did_change(DidChangeTextDocumentParams {
            text_document: VersionedTextDocumentIdentifier {
                uri: uri.clone(),
                version: 2,
            },
            content_changes: vec![Change {
                range: None,
                range_length: None,
                text: "<new/>".to_string(),
            }],
        })
        .await;

    let doc = backend.store().get(&uri).unwrap();
    assert_eq!(doc.text, "<new/>");
    let tree = &doc.parse.document;
    let root = tree.root_element().unwrap();
    assert_eq!(tree.element(root).unwrap().name.local, "new");
}
