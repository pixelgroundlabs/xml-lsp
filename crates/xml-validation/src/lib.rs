//! `xml-validation`: derivative-based validation of XML documents against
//! the internal schema model.
//!
//! The core is a faithful implementation of James Clark's derivative
//! algorithm for RELAX NG ([`machine`]), wrapped in a tree walker with
//! error recovery ([`validator`]) that produces editor-friendly
//! diagnostics. The same machinery answers "what is expected at this
//! position?", which later powers completion.

mod datatypes;
mod diagnostics;
mod machine;
mod validator;

pub use diagnostics::{Severity, ValidationDiagnostic, ValidationDiagnosticCode};
pub use validator::{validate, Validator};
