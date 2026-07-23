//! `xml-ast` provides the LSP-independent core representation of XML
//! documents: text ranges and position mapping, qualified names with
//! namespace resolution, and the syntax tree itself.
//!
//! This crate deliberately has no dependencies on LSP types or the parser.

mod escape;
mod qname;
mod text;
mod tree;

pub use escape::unescape;
pub use qname::{NsUri, QName, XMLNS_NAMESPACE, XML_NAMESPACE};
pub use text::{LineCol, LineIndex, TextRange};
pub use tree::{Attribute, Document, Element, NodeId, NodeKind};
