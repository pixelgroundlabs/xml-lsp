//! Validation diagnostics. Ranges refer to the validated document.

use xml_ast::TextRange;
pub use xml_schema::Severity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationDiagnosticCode {
    /// An element that the content model does not allow at this position.
    ElementNotAllowed,
    /// The element ends although required content is still missing.
    IncompleteContent,
    /// Text where the content model allows none.
    TextNotAllowed,
    /// Text content that does not match the expected value or datatype.
    InvalidTextValue,
    /// An attribute the element does not allow.
    AttributeNotAllowed,
    /// An attribute value outside the allowed values or datatype.
    InvalidAttributeValue,
    /// Required attributes are missing on the start tag.
    MissingAttributes,
}

impl ValidationDiagnosticCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ValidationDiagnosticCode::ElementNotAllowed => "element-not-allowed",
            ValidationDiagnosticCode::IncompleteContent => "incomplete-content",
            ValidationDiagnosticCode::TextNotAllowed => "text-not-allowed",
            ValidationDiagnosticCode::InvalidTextValue => "invalid-text-value",
            ValidationDiagnosticCode::AttributeNotAllowed => "attribute-not-allowed",
            ValidationDiagnosticCode::InvalidAttributeValue => "invalid-attribute-value",
            ValidationDiagnosticCode::MissingAttributes => "missing-attributes",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationDiagnostic {
    pub range: TextRange,
    pub severity: Severity,
    pub code: ValidationDiagnosticCode,
    pub message: String,
}

impl ValidationDiagnostic {
    pub fn error(
        range: TextRange,
        code: ValidationDiagnosticCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            range,
            severity: Severity::Error,
            code,
            message: message.into(),
        }
    }
}
