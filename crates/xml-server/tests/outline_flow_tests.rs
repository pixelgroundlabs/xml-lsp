//! Document symbols and folding ranges through the server.

use tower_lsp::lsp_types::{
    DidOpenTextDocumentParams, DocumentSymbolParams, DocumentSymbolResponse, FoldingRangeKind,
    FoldingRangeParams, InitializeParams, PartialResultParams, TextDocumentIdentifier,
    TextDocumentItem, Url, WorkDoneProgressParams,
};
use tower_lsp::{LanguageServer, LspService};
use xml_server::Backend;

const TEXT: &str = r#"<Layout xmlns="urn:speedata.de:2009/publisher/en">
  <!-- a
       multi-line comment -->
  <Record element="data">
    <PlaceObject>
      <Textblock/>
    </PlaceObject>
  </Record>
</Layout>
"#;

async fn setup() -> (LspService<Backend>, Url) {
    let (service, _socket) = LspService::new(Backend::new);
    service
        .inner()
        .initialize(InitializeParams::default())
        .await
        .unwrap();
    let uri = Url::parse("file:///project/layout.xml").unwrap();
    service
        .inner()
        .did_open(DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "xml".to_string(),
                version: 1,
                text: TEXT.to_string(),
            },
        })
        .await;
    (service, uri)
}

#[tokio::test]
async fn hierarchical_document_symbols() {
    let (service, uri) = setup().await;
    let response = service
        .inner()
        .document_symbol(DocumentSymbolParams {
            text_document: TextDocumentIdentifier { uri },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        })
        .await
        .unwrap()
        .expect("symbols");

    let DocumentSymbolResponse::Nested(symbols) = response else {
        panic!("expected nested symbols");
    };
    assert_eq!(symbols.len(), 1);
    let layout = &symbols[0];
    assert_eq!(layout.name, "Layout");
    let record = &layout.children.as_ref().unwrap()[0];
    assert_eq!(record.name, "Record");
    assert_eq!(record.detail.as_deref(), Some("element=\"data\""));
    let place = &record.children.as_ref().unwrap()[0];
    assert_eq!(place.name, "PlaceObject");
    // Selection range points at the element name.
    assert_eq!(record.selection_range.start.line, 3);
    assert_eq!(record.selection_range.start.character, 3);
    // Self-closing leaf has no children.
    assert!(place.children.as_ref().unwrap()[0].children.is_none());
}

#[tokio::test]
async fn folding_ranges_for_elements_and_comments() {
    let (service, uri) = setup().await;
    let ranges = service
        .inner()
        .folding_range(FoldingRangeParams {
            text_document: TextDocumentIdentifier { uri },
            work_done_progress_params: WorkDoneProgressParams::default(),
            partial_result_params: PartialResultParams::default(),
        })
        .await
        .unwrap()
        .expect("folding ranges");

    // Elements fold up to the line before their end tag: Layout (0..7),
    // Record (3..6), PlaceObject (4..5); the comment folds entirely.
    let mut summary: Vec<(u32, u32, Option<FoldingRangeKind>)> = ranges
        .iter()
        .map(|r| (r.start_line, r.end_line, r.kind.clone()))
        .collect();
    summary.sort_by_key(|(start, _, _)| *start);
    assert_eq!(summary.len(), 4, "{summary:?}");
    assert_eq!(summary[0], (0, 7, Some(FoldingRangeKind::Region)));
    assert_eq!(summary[1], (1, 2, Some(FoldingRangeKind::Comment)));
    assert_eq!(summary[2], (3, 6, Some(FoldingRangeKind::Region)));
    assert_eq!(summary[3], (4, 5, Some(FoldingRangeKind::Region)));
}
