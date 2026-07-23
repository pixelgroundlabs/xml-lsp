//! Document cache: keeps text, line index, and parse result for every open
//! document. Changes arrive as LSP content changes and are applied
//! incrementally to the text; afterwards the document is reparsed
//! (incremental parsing is a later milestone).

use dashmap::mapref::one::Ref;
use dashmap::DashMap;
use tower_lsp::lsp_types::{TextDocumentContentChangeEvent, Url};
use xml_ast::{LineCol, LineIndex};
use xml_parser::ParseResult;

/// State of an open document.
pub struct DocumentState {
    pub uri: Url,
    pub version: i32,
    pub text: String,
    pub line_index: LineIndex,
    pub parse: ParseResult,
}

impl DocumentState {
    pub fn new(uri: Url, version: i32, text: String) -> Self {
        let line_index = LineIndex::new(&text);
        let parse = xml_parser::parse(&text);
        Self {
            uri,
            version,
            text,
            line_index,
            parse,
        }
    }

    /// Applies a sequence of content changes. Per the LSP specification,
    /// each change refers to the document state after the previous one, so
    /// the line index is rebuilt after every change.
    pub fn apply_changes(&mut self, version: i32, changes: &[TextDocumentContentChangeEvent]) {
        for change in changes {
            match change.range {
                Some(range) => {
                    let start = self.line_index.offset(
                        &self.text,
                        LineCol {
                            line: range.start.line,
                            col: range.start.character,
                        },
                    );
                    let end = self.line_index.offset(
                        &self.text,
                        LineCol {
                            line: range.end.line,
                            col: range.end.character,
                        },
                    );
                    // Defensive clamping in case the client sends invalid
                    // ranges.
                    let (start, end) = (start.min(end), end.max(start));
                    self.text.replace_range(start..end, &change.text);
                }
                None => {
                    self.text = change.text.clone();
                }
            }
            self.line_index = LineIndex::new(&self.text);
        }
        self.version = version;
        self.parse = xml_parser::parse(&self.text);
    }
}

/// Thread-safe cache of all open documents.
#[derive(Default)]
pub struct DocumentStore {
    docs: DashMap<Url, DocumentState>,
}

impl DocumentStore {
    pub fn open(&self, uri: Url, version: i32, text: String) {
        self.docs
            .insert(uri.clone(), DocumentState::new(uri, version, text));
    }

    /// Applies changes; returns `false` if the document is unknown.
    pub fn change(
        &self,
        uri: &Url,
        version: i32,
        changes: &[TextDocumentContentChangeEvent],
    ) -> bool {
        match self.docs.get_mut(uri) {
            Some(mut doc) => {
                doc.apply_changes(version, changes);
                true
            }
            None => false,
        }
    }

    pub fn close(&self, uri: &Url) {
        self.docs.remove(uri);
    }

    pub fn get(&self, uri: &Url) -> Option<Ref<'_, Url, DocumentState>> {
        self.docs.get(uri)
    }

    /// URIs of all open documents (snapshot).
    pub fn uris(&self) -> Vec<Url> {
        self.docs.iter().map(|entry| entry.key().clone()).collect()
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }
}
