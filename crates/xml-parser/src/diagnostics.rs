//! Parser diagnostics, independent of LSP types.

use xml_ast::TextRange;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// Stable diagnostic code; shown next to the message in the editor and used
/// later as an anchor for quick fixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticCode {
    SyntaxError,
    UnclosedElement,
    StrayEndTag,
    MultipleRootElements,
    NoRootElement,
    TextOutsideRoot,
    DuplicateAttribute,
    AttributeSyntax,
    UndefinedNamespacePrefix,
    InvalidEntity,
}

impl DiagnosticCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            DiagnosticCode::SyntaxError => "syntax-error",
            DiagnosticCode::UnclosedElement => "unclosed-element",
            DiagnosticCode::StrayEndTag => "stray-end-tag",
            DiagnosticCode::MultipleRootElements => "multiple-root-elements",
            DiagnosticCode::NoRootElement => "no-root-element",
            DiagnosticCode::TextOutsideRoot => "text-outside-root",
            DiagnosticCode::DuplicateAttribute => "duplicate-attribute",
            DiagnosticCode::AttributeSyntax => "attribute-syntax",
            DiagnosticCode::UndefinedNamespacePrefix => "undefined-namespace-prefix",
            DiagnosticCode::InvalidEntity => "invalid-entity",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseDiagnostic {
    pub range: TextRange,
    pub severity: Severity,
    pub code: DiagnosticCode,
    pub message: String,
}

impl ParseDiagnostic {
    pub fn error(range: TextRange, code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            range,
            severity: Severity::Error,
            code,
            message: message.into(),
        }
    }

    pub fn warning(range: TextRange, code: DiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            range,
            severity: Severity::Warning,
            code,
            message: message.into(),
        }
    }
}
