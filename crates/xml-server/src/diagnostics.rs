//! Conversion of parser diagnostics into LSP diagnostics.

use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString, Position, Range};
use xml_ast::LineIndex;
use xml_parser::{ParseDiagnostic, Severity};

use crate::config::DiagnosticsConfig;

/// Converts parser diagnostics into LSP diagnostics, applying the
/// configured limit.
pub fn to_lsp_diagnostics(
    text: &str,
    line_index: &LineIndex,
    diagnostics: &[ParseDiagnostic],
    config: &DiagnosticsConfig,
) -> Vec<Diagnostic> {
    if !config.enable {
        return Vec::new();
    }
    diagnostics
        .iter()
        .take(config.max_count)
        .map(|d| convert(text, line_index, d))
        .collect()
}

fn convert(text: &str, line_index: &LineIndex, diagnostic: &ParseDiagnostic) -> Diagnostic {
    let start = line_index.line_col(text, diagnostic.range.start);
    let end = line_index.line_col(text, diagnostic.range.end);
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
        severity: Some(match diagnostic.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        code: Some(NumberOrString::String(diagnostic.code.as_str().to_string())),
        source: Some("xml".to_string()),
        message: diagnostic.message.clone(),
        ..Diagnostic::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xml_ast::TextRange;
    use xml_parser::DiagnosticCode;

    fn sample_diagnostic(range: TextRange) -> ParseDiagnostic {
        ParseDiagnostic::error(range, DiagnosticCode::SyntaxError, "broken")
    }

    #[test]
    fn converts_positions() {
        let text = "ab\ncd";
        let index = LineIndex::new(text);
        let diags = vec![sample_diagnostic(TextRange::new(3, 5))];
        let lsp = to_lsp_diagnostics(text, &index, &diags, &DiagnosticsConfig::default());
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
    fn respects_disable_and_max_count() {
        let text = "x";
        let index = LineIndex::new(text);
        let diags = vec![
            sample_diagnostic(TextRange::empty(0)),
            sample_diagnostic(TextRange::empty(0)),
            sample_diagnostic(TextRange::empty(0)),
        ];

        let disabled = DiagnosticsConfig {
            enable: false,
            ..Default::default()
        };
        assert!(to_lsp_diagnostics(text, &index, &diags, &disabled).is_empty());

        let capped = DiagnosticsConfig {
            max_count: 2,
            ..Default::default()
        };
        assert_eq!(to_lsp_diagnostics(text, &index, &diags, &capped).len(), 2);
    }
}
