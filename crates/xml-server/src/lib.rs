//! `xml-server`: the LSP server. This library contains all server logic;
//! the `xml-lsp` binary merely connects it to stdin/stdout.
//!
//! The modules access only the internal APIs of `xml-ast` and `xml-parser`
//! — never the other way around.

pub mod backend;
pub mod config;
pub mod diagnostics;
pub mod documents;

pub use backend::Backend;
