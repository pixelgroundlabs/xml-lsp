//! `xml-completion`: schema-driven completion, independent of LSP types.
//!
//! [`complete`] analyzes the cursor position (element name, attribute
//! name, attribute value, closing tag), asks the validation engine what
//! the schema expects there, and returns ready-to-insert items with the
//! documentation from the schema.

mod context;
mod items;

pub use context::{cursor_context, CursorContext};
pub use items::{complete, CompletionItem, CompletionItemKind};
