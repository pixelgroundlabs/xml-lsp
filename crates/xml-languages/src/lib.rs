//! `xml-languages`: which XML dialect is this document, and which schema
//! applies?
//!
//! A [`LanguageRegistry`] resolves a document to a schema file using, in
//! order of precedence:
//!
//! 1. an `<?xml-model?>` processing instruction in the document itself,
//! 2. configured language profiles with activation rules (namespace,
//!    root element, file name, extension),
//! 3. OASIS XML catalogs mapping a namespace to a schema.
//!
//! New XML languages are added purely through profiles and catalogs — no
//! core code changes. The [`SchemaRepository`] loads and caches the
//! resolved schema files, reloading them when they change on disk.

mod catalog;
mod references;
mod registry;
mod repository;
mod xml_model;

pub use catalog::CatalogError;
pub use references::{
    ReferenceRules, SymbolIndex, SymbolOccurrence, SymbolRole, SymbolSite, ValueForm,
};
pub use registry::{
    Activation, LanguageProfile, LanguageRegistry, ResolutionOrigin, ResolvedSchema,
};
pub use repository::{LoadedSchema, SchemaRepository};
