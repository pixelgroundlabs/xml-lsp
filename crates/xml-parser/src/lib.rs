//! `xml-parser`: error-resilient XML parser built on quick-xml.
//!
//! The parser does not abort on broken documents; it builds as much tree as
//! possible and collects diagnostics. This is the foundation for all LSP
//! features that must keep working while the user is typing.

mod attrs;
mod diagnostics;
mod parser;

pub use diagnostics::{DiagnosticCode, ParseDiagnostic, Severity};
pub use parser::{parse, ParseResult};
