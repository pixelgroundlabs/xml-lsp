//! Conversion of parser and validation diagnostics into LSP diagnostics.

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};
use xml_ast::{LineIndex, TextRange};
use xml_parser::{ParseDiagnostic, Severity};
use xml_validation::ValidationDiagnostic;

/// Converts parser diagnostics into LSP diagnostics.
pub fn parse_to_lsp(
    text: &str,
    line_index: &LineIndex,
    diagnostics: &[ParseDiagnostic],
) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .map(|d| {
            convert(
                text,
                line_index,
                d.range,
                d.severity,
                d.code.as_str(),
                &d.message,
            )
        })
        .collect()
}

/// Converts validation diagnostics into LSP diagnostics.
pub fn validation_to_lsp(
    text: &str,
    line_index: &LineIndex,
    diagnostics: &[ValidationDiagnostic],
) -> Vec<Diagnostic> {
    diagnostics
        .iter()
        .map(|d| {
            convert(
                text,
                line_index,
                d.range,
                d.severity,
                d.code.as_str(),
                &d.message,
            )
        })
        .collect()
}

/// A free-form diagnostic, e.g. "schema could not be loaded".
pub fn server_notice(
    text: &str,
    line_index: &LineIndex,
    severity: Severity,
    code: &str,
    message: &str,
) -> Diagnostic {
    convert(
        text,
        line_index,
        TextRange::empty(0),
        severity,
        code,
        message,
    )
}

fn convert(
    text: &str,
    line_index: &LineIndex,
    range: TextRange,
    severity: Severity,
    code: &str,
    message: &str,
) -> Diagnostic {
    let start = line_index.line_col(text, range.start);
    let end = line_index.line_col(text, range.end);
    Diagnostic {
        range: Range {
            start: Position {
                line: start.line,
                character: start.col,
            },
            end: Position {
                line: end.line,
                character: end.col,
            },
        },
        severity: Some(match severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        code: Some(NumberOrString::String(code.to_string())),
        source: Some("xml".to_string()),
        message: message.to_string(),
        ..Diagnostic::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xml_parser::DiagnosticCode;

    #[test]
    fn converts_positions() {
        let text = "ab\ncd";
        let index = LineIndex::new(text);
        let diags = vec![ParseDiagnostic::error(
            TextRange::new(3, 5),
            DiagnosticCode::SyntaxError,
            "broken",
        )];
        let lsp = parse_to_lsp(text, &index, &diags);
        assert_eq!(lsp.len(), 1);
        assert_eq!(
            lsp[0].range.start,
            Position {
                line: 1,
                character: 0
            }
        );
        assert_eq!(
            lsp[0].range.end,
            Position {
                line: 1,
                character: 2
            }
        );
        assert_eq!(lsp[0].severity, Some(DiagnosticSeverity::ERROR));
        assert_eq!(lsp[0].source.as_deref(), Some("xml"));
    }

    #[test]
    fn server_notice_at_document_start() {
        let text = "x";
        let index = LineIndex::new(text);
        let notice = server_notice(text, &index, Severity::Warning, "schema-load", "oh no");
        assert_eq!(
            notice.range.start,
            Position {
                line: 0,
                character: 0
            }
        );
        assert_eq!(notice.severity, Some(DiagnosticSeverity::WARNING));
    }
}
