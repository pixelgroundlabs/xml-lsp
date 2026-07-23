//! Diagnostics produced while loading a schema. Ranges refer to the schema
//! source text, so schema problems can be shown in the editor when the
//! user opens the schema file itself.

use xml_ast::TextRange;
pub use xml_parser::Severity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaDiagnosticCode {
    /// The schema file itself is not well-formed XML.
    XmlSyntax,
    /// An element that is not part of the RELAX NG vocabulary appeared in
    /// a pattern position.
    UnexpectedElement,
    /// A construct that requires a name (element/attribute without a name
    /// attribute or name-class child, ref/define without name, …).
    MissingName,
    /// `<data>` without a type attribute.
    MissingType,
    /// A `<ref>` to an undefined name.
    UnknownRef,
    /// More than one `<start>` (without combine).
    DuplicateStart,
    /// More than one `<define>` with the same name (without combine).
    DuplicateDefine,
    /// The grammar has no `<start>`.
    MissingStart,
    /// A pattern that requires content has none.
    MissingContent,
    /// A RELAX NG feature the loader does not support yet
    /// (include/externalRef/parentRef).
    UnsupportedFeature,
    /// A QName whose prefix is not declared in the schema document.
    UnknownPrefix,
}

impl SchemaDiagnosticCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SchemaDiagnosticCode::XmlSyntax => "schema-xml-syntax",
            SchemaDiagnosticCode::UnexpectedElement => "schema-unexpected-element",
            SchemaDiagnosticCode::MissingName => "schema-missing-name",
            SchemaDiagnosticCode::MissingType => "schema-missing-type",
            SchemaDiagnosticCode::UnknownRef => "schema-unknown-ref",
            SchemaDiagnosticCode::DuplicateStart => "schema-duplicate-start",
            SchemaDiagnosticCode::DuplicateDefine => "schema-duplicate-define",
            SchemaDiagnosticCode::MissingStart => "schema-missing-start",
            SchemaDiagnosticCode::MissingContent => "schema-missing-content",
            SchemaDiagnosticCode::UnsupportedFeature => "schema-unsupported-feature",
            SchemaDiagnosticCode::UnknownPrefix => "schema-unknown-prefix",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaDiagnostic {
    pub range: TextRange,
    pub severity: Severity,
    pub code: SchemaDiagnosticCode,
    pub message: String,
}

impl SchemaDiagnostic {
    pub fn error(range: TextRange, code: SchemaDiagnosticCode, message: impl Into<String>) -> Self {
        Self {
            range,
            severity: Severity::Error,
            code,
            message: message.into(),
        }
    }

    pub fn warning(
        range: TextRange,
        code: SchemaDiagnosticCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            range,
            severity: Severity::Warning,
            code,
            message: message.into(),
        }
    }
}
