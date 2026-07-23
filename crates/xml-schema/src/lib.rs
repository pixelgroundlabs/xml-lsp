//! `xml-schema`: the internal schema model and its frontends.
//!
//! The model ([`Schema`], [`Pattern`], [`NameClass`]) is the single source
//! of truth for everything the server knows about an XML language:
//! allowed elements, attributes, values, and their documentation. All
//! features (validation, completion, hover) work against this model only.
//!
//! Frontends produce the model from a concrete schema language. The first
//! frontend is RELAX NG (XML syntax, [`load_rng`]); XSD and RELAX NG
//! compact syntax are planned as additional frontends that map onto the
//! same model without touching its consumers.

mod diagnostics;
mod model;
mod query;
mod rng;

pub use diagnostics::{SchemaDiagnostic, SchemaDiagnosticCode, Severity};
pub use model::{
    AttributePattern, DataPattern, DatatypeName, Define, DefineId, ElementPattern, NameClass,
    Pattern, PatternId, Schema, ValuePattern,
};
pub use rng::{load_rng, LoadResult, RNG_DOC_NAMESPACE, RNG_NAMESPACE};
