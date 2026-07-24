//! Folding ranges: multi-line elements, comments, and CDATA sections.

use tower_lsp::lsp_types::{FoldingRange, FoldingRangeKind};
use xml_ast::{Document, LineIndex, NodeKind};

/// Computes the folding ranges of a document.
pub fn folding_ranges(doc: &Document, text: &str, line_index: &LineIndex) -> Vec<FoldingRange> {
    let mut ranges = Vec::new();
    for node in doc.descendants(doc.root()) {
        match doc.kind(node) {
            NodeKind::Element(element) => {
                // Foldable from the start tag down to the line before the
                // end tag, so the closing line stays visible.
                let Some(close_tag) = element.close_tag else {
                    continue;
                };
                let start = line_index.line_col(text, element.open_tag.start).line;
                let end = line_index.line_col(text, close_tag.start).line;
                if end > start {
                    ranges.push(FoldingRange {
                        start_line: start,
                        end_line: end - 1,
                        kind: Some(FoldingRangeKind::Region),
                        ..FoldingRange::default()
                    });
                }
            }
            NodeKind::Comment | NodeKind::CData => {
                let range = doc.range(node);
                let start = line_index.line_col(text, range.start).line;
                let end = line_index.line_col(text, range.end).line;
                if end > start {
                    ranges.push(FoldingRange {
                        start_line: start,
                        end_line: end,
                        kind: Some(match doc.kind(node) {
                            NodeKind::Comment => FoldingRangeKind::Comment,
                            _ => FoldingRangeKind::Region,
                        }),
                        ..FoldingRange::default()
                    });
                }
            }
            _ => {}
        }
    }
    ranges
}
