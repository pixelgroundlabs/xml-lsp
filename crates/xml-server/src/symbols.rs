//! Document symbols: the element hierarchy as an outline.

use tower_lsp::lsp_types::{DocumentSymbol, Position, Range, SymbolKind};
use xml_ast::{Document, LineIndex, NodeId, NodeKind, TextRange};

/// Attributes that identify an element instance; the first one present
/// becomes the symbol detail (e.g. `Record` → `element="data"`).
const IDENTIFYING_ATTRIBUTES: [&str; 5] = ["name", "id", "element", "match", "mode"];

/// Builds the hierarchical outline of a document.
pub fn document_symbols(doc: &Document, text: &str, line_index: &LineIndex) -> Vec<DocumentSymbol> {
    collect(doc, doc.root(), text, line_index)
}

fn collect(
    doc: &Document,
    node: NodeId,
    text: &str,
    line_index: &LineIndex,
) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();
    for &child in doc.children(node) {
        let NodeKind::Element(element) = doc.kind(child) else {
            continue;
        };
        let detail = IDENTIFYING_ATTRIBUTES.iter().find_map(|name| {
            element
                .attribute(None, name)
                .map(|attr| format!("{name}=\"{}\"", attr.value))
        });
        let children = collect(doc, child, text, line_index);
        #[allow(deprecated)] // `deprecated` field must still be filled
        symbols.push(DocumentSymbol {
            name: element.name.as_written(),
            detail,
            kind: SymbolKind::FIELD,
            tags: None,
            deprecated: None,
            range: to_range(text, line_index, doc.range(child)),
            selection_range: to_range(text, line_index, element.name_range),
            children: (!children.is_empty()).then_some(children),
        });
    }
    symbols
}

fn to_range(text: &str, line_index: &LineIndex, range: TextRange) -> Range {
    let start = line_index.line_col(text, range.start);
    let end = line_index.line_col(text, range.end);
    Range {
        start: Position {
            line: start.line,
            character: start.col,
        },
        end: Position {
            line: end.line,
            character: end.col,
        },
    }
}
