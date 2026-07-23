//! The parser itself: quick-xml provides the event stream, and we build an
//! error-tolerant tree with namespace resolution from it.
//!
//! Error strategy:
//! * End tags that do not match close open elements automatically (with a
//!   diagnostic) instead of aborting the parse.
//! * Stray end tags are reported and skipped.
//! * On fatal syntax errors (e.g. an incomplete tag at the end of the file)
//!   the tree built so far is kept.

use std::collections::HashMap;
use std::sync::Arc;

use quick_xml::events::Event;
use quick_xml::Reader;

use xml_ast::{
    Attribute, Document, Element, NodeId, NodeKind, NsUri, QName, TextRange, XMLNS_NAMESPACE,
    XML_NAMESPACE,
};

use crate::attrs::{scan_attributes, RawAttr};
use crate::diagnostics::{DiagnosticCode, ParseDiagnostic};

/// Result of a parser run: the (possibly incomplete) tree and all collected
/// diagnostics.
#[derive(Debug)]
pub struct ParseResult {
    pub document: Document,
    pub diagnostics: Vec<ParseDiagnostic>,
}

/// Parses `text` error-tolerantly into a [`ParseResult`].
pub fn parse(text: &str) -> ParseResult {
    Parser::new(text).run()
}

/// An element still open on the parser stack.
struct OpenElement {
    node: NodeId,
    /// Name as written in the start tag (including prefix), for matching
    /// against end tags.
    raw_name: String,
    name_range: TextRange,
    /// Length of the namespace stack before this element's declarations, so
    /// visibility can be restored when the element closes.
    ns_stack_len: usize,
}

struct Parser<'a> {
    text: &'a str,
    doc: Document,
    diags: Vec<ParseDiagnostic>,
    stack: Vec<OpenElement>,
    /// Visible namespace declarations as a stack; `None` as URI means
    /// "declaration removed" (`xmlns=""`).
    ns_stack: Vec<(Option<String>, Option<NsUri>)>,
    /// Interning cache for namespace URIs.
    ns_cache: HashMap<String, NsUri>,
    seen_root_element: bool,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text,
            doc: Document::new(),
            diags: Vec::new(),
            stack: Vec::new(),
            ns_stack: Vec::new(),
            ns_cache: HashMap::new(),
            seen_root_element: false,
        }
    }

    fn run(mut self) -> ParseResult {
        let mut reader = Reader::from_str(self.text);
        let config = reader.config_mut();
        // We do end-tag matching and error handling ourselves so parsing
        // keeps going on broken documents.
        config.check_end_names = false;
        config.allow_unmatched_ends = true;

        loop {
            let event_start = reader.buffer_position() as usize;
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let end = reader.buffer_position() as usize;
                    let qname = e.name();
                    let name = raw_str(qname.as_ref());
                    self.handle_start_tag(name, TextRange::new(event_start, end), false);
                }
                Ok(Event::Empty(e)) => {
                    let end = reader.buffer_position() as usize;
                    let qname = e.name();
                    let name = raw_str(qname.as_ref());
                    self.handle_start_tag(name, TextRange::new(event_start, end), true);
                }
                Ok(Event::End(e)) => {
                    let end = reader.buffer_position() as usize;
                    let qname = e.name();
                    let name = raw_str(qname.as_ref());
                    self.handle_end_tag(name, TextRange::new(event_start, end));
                }
                Ok(Event::Text(e)) => {
                    let end = reader.buffer_position() as usize;
                    let range = TextRange::new(event_start, end);
                    // Report invalid entity references (without a DTD only
                    // the five predefined entities are allowed).
                    if let Err(err) = e.unescape() {
                        self.diags.push(ParseDiagnostic::error(
                            range,
                            DiagnosticCode::InvalidEntity,
                            err.to_string(),
                        ));
                    }
                    self.handle_text(range);
                }
                Ok(Event::CData(_)) => {
                    let end = reader.buffer_position() as usize;
                    let parent = self.current_parent();
                    self.doc
                        .push(parent, NodeKind::CData, TextRange::new(event_start, end));
                }
                Ok(Event::Comment(_)) => {
                    let end = reader.buffer_position() as usize;
                    let parent = self.current_parent();
                    self.doc
                        .push(parent, NodeKind::Comment, TextRange::new(event_start, end));
                }
                Ok(Event::PI(_)) => {
                    let end = reader.buffer_position() as usize;
                    let range = TextRange::new(event_start, end);
                    let target = self.pi_target(range);
                    let parent = self.current_parent();
                    self.doc
                        .push(parent, NodeKind::ProcessingInstruction { target }, range);
                }
                Ok(Event::DocType(_)) => {
                    let end = reader.buffer_position() as usize;
                    let parent = self.current_parent();
                    self.doc
                        .push(parent, NodeKind::Doctype, TextRange::new(event_start, end));
                }
                Ok(Event::Decl(_)) => {
                    // The XML declaration does not need its own node (yet).
                }
                Ok(Event::Eof) => break,
                Err(err) => {
                    let error_pos = (reader.error_position() as usize).min(self.text.len());
                    let end = (reader.buffer_position() as usize)
                        .min(self.text.len())
                        .max(error_pos);
                    self.diags.push(ParseDiagnostic::error(
                        TextRange::new(error_pos, end),
                        DiagnosticCode::SyntaxError,
                        err.to_string(),
                    ));
                    // After a fatal error the reader state is unreliable;
                    // keep the tree built so far.
                    break;
                }
            }
        }

        self.close_remaining(self.text.len());

        if !self.seen_root_element {
            self.diags.push(ParseDiagnostic::error(
                TextRange::empty(0),
                DiagnosticCode::NoRootElement,
                "document has no root element",
            ));
        }

        ParseResult {
            document: self.doc,
            diagnostics: self.diags,
        }
    }

    /// The node under which new children are attached.
    fn current_parent(&self) -> NodeId {
        self.stack.last().map_or(self.doc.root(), |open| open.node)
    }

    fn handle_start_tag(&mut self, raw_name: &str, span: TextRange, self_closing: bool) {
        let name_range = TextRange::new(span.start + 1, span.start + 1 + raw_name.len());

        // Tag content after the name, without '>' or '/>'.
        let tail = if self_closing { 2 } else { 1 };
        let inner_start = name_range.end.min(span.end);
        let inner_end = span.end.saturating_sub(tail).max(inner_start);
        let raw_attrs = scan_attributes(
            &self.text[inner_start..inner_end],
            inner_start,
            &mut self.diags,
        );

        // Collect this tag's namespace declarations and make them visible.
        let ns_stack_len = self.ns_stack.len();
        let ns_decls = self.collect_ns_decls(&raw_attrs);
        self.ns_stack.extend(ns_decls.iter().cloned());

        // Resolve the element name.
        let (prefix, local) = QName::split(raw_name);
        let namespace = self.resolve_prefix(prefix, false, name_range);
        let name = QName::new(prefix, local, namespace);

        // Resolve attributes and report duplicates.
        let attributes = self.resolve_attributes(&raw_attrs);

        // Multiple root elements are not well-formed.
        if self.stack.is_empty() {
            if self.seen_root_element {
                self.diags.push(ParseDiagnostic::error(
                    name_range,
                    DiagnosticCode::MultipleRootElements,
                    format!("document must have exactly one root element, found additional `<{raw_name}>`"),
                ));
            }
            self.seen_root_element = true;
        }

        let element = Element {
            name,
            name_range,
            attributes,
            open_tag: span,
            close_tag: None,
            self_closing,
            ns_decls,
        };
        let parent = self.current_parent();
        let node = self.doc.push(parent, NodeKind::Element(element), span);

        if self_closing {
            // Declarations only apply within the tag itself.
            self.ns_stack.truncate(ns_stack_len);
        } else {
            self.stack.push(OpenElement {
                node,
                raw_name: raw_name.to_string(),
                name_range,
                ns_stack_len,
            });
        }
    }

    fn handle_end_tag(&mut self, raw_name: &str, span: TextRange) {
        // `</a >` is legal; quick-xml may include the whitespace in the
        // name, so trim it here.
        let raw_name = raw_name.trim_end();

        let Some(match_idx) = self
            .stack
            .iter()
            .rposition(|open| open.raw_name == raw_name)
        else {
            self.diags.push(ParseDiagnostic::error(
                span,
                DiagnosticCode::StrayEndTag,
                format!("closing tag `</{raw_name}>` has no matching opening tag"),
            ));
            return;
        };

        // Auto-close all open elements above the match.
        while self.stack.len() > match_idx + 1 {
            let open = self.stack.pop().expect("stack cannot be empty here");
            self.diags.push(ParseDiagnostic::error(
                open.name_range,
                DiagnosticCode::UnclosedElement,
                format!("element `<{}>` is missing its closing tag", open.raw_name),
            ));
            let start = self.doc.range(open.node).start;
            self.doc
                .set_range(open.node, TextRange::new(start, span.start));
            self.ns_stack.truncate(open.ns_stack_len);
        }

        // Close the matching element regularly.
        let open = self.stack.pop().expect("match index is valid");
        let start = self.doc.range(open.node).start;
        self.doc
            .set_range(open.node, TextRange::new(start, span.end));
        if let Some(element) = self.doc.element_mut(open.node) {
            element.close_tag = Some(span);
        }
        self.ns_stack.truncate(open.ns_stack_len);
    }

    fn handle_text(&mut self, range: TextRange) {
        let raw = &self.text[range.start..range.end];
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            // Whitespace-only text nodes would just bloat the tree.
            return;
        }
        if self.stack.is_empty() {
            // Narrow the range down to the trimmed content.
            let lead = raw.len() - raw.trim_start().len();
            let start = range.start + lead;
            self.diags.push(ParseDiagnostic::error(
                TextRange::new(start, start + trimmed.len()),
                DiagnosticCode::TextOutsideRoot,
                "text is not allowed outside the root element",
            ));
        }
        let parent = self.current_parent();
        self.doc.push(parent, NodeKind::Text, range);
    }

    /// Closes all still-open elements at the end of the file, with
    /// diagnostics.
    fn close_remaining(&mut self, end: usize) {
        while let Some(open) = self.stack.pop() {
            self.diags.push(ParseDiagnostic::error(
                open.name_range,
                DiagnosticCode::UnclosedElement,
                format!("element `<{}>` is missing its closing tag", open.raw_name),
            ));
            let start = self.doc.range(open.node).start;
            self.doc.set_range(open.node, TextRange::new(start, end));
        }
    }

    /// Extracts namespace declarations (`xmlns`, `xmlns:p`) from a start
    /// tag's attributes.
    fn collect_ns_decls(&mut self, attrs: &[RawAttr]) -> Vec<(Option<String>, Option<NsUri>)> {
        let mut decls = Vec::new();
        for attr in attrs {
            let uri = |cache: &mut HashMap<String, NsUri>, value: &Option<String>| {
                value
                    .as_deref()
                    .filter(|v| !v.is_empty())
                    .map(|v| intern(cache, v))
            };
            if attr.name == "xmlns" {
                decls.push((None, uri(&mut self.ns_cache, &attr.value)));
            } else if let Some(prefix) = attr.name.strip_prefix("xmlns:") {
                if !prefix.is_empty() {
                    decls.push((
                        Some(prefix.to_string()),
                        uri(&mut self.ns_cache, &attr.value),
                    ));
                }
            }
        }
        decls
    }

    /// Resolves a prefix against the currently visible declarations and
    /// reports unknown prefixes.
    fn resolve_prefix(
        &mut self,
        prefix: Option<&str>,
        is_attribute: bool,
        name_range: TextRange,
    ) -> Option<NsUri> {
        match prefix {
            // Unprefixed attributes never have a namespace; unprefixed
            // elements get the default namespace.
            None if is_attribute => None,
            None => self
                .ns_stack
                .iter()
                .rev()
                .find(|(p, _)| p.is_none())
                .and_then(|(_, uri)| uri.clone()),
            Some("xml") => Some(intern(&mut self.ns_cache, XML_NAMESPACE)),
            Some("xmlns") => Some(intern(&mut self.ns_cache, XMLNS_NAMESPACE)),
            Some(prefix) => {
                let found = self
                    .ns_stack
                    .iter()
                    .rev()
                    .find(|(p, _)| p.as_deref() == Some(prefix));
                match found {
                    Some((_, Some(uri))) => Some(uri.clone()),
                    // Either never declared or removed via `xmlns:p=""`.
                    _ => {
                        self.diags.push(ParseDiagnostic::error(
                            name_range,
                            DiagnosticCode::UndefinedNamespacePrefix,
                            format!("namespace prefix `{prefix}` is not declared"),
                        ));
                        None
                    }
                }
            }
        }
    }

    /// Resolves attribute names and reports duplicate attributes.
    fn resolve_attributes(&mut self, raw_attrs: &[RawAttr]) -> Vec<Attribute> {
        let mut attributes: Vec<Attribute> = Vec::with_capacity(raw_attrs.len());
        for raw in raw_attrs {
            let (prefix, local) = QName::split(&raw.name);
            let namespace = if raw.name == "xmlns" {
                Some(intern(&mut self.ns_cache, XMLNS_NAMESPACE))
            } else {
                self.resolve_prefix(prefix, true, raw.name_range)
            };
            let name = QName::new(prefix, local, namespace);

            // An attribute is a duplicate if its raw name or expanded name
            // has already been seen.
            let duplicate = attributes.iter().any(|existing| {
                existing.name.as_written() == raw.name
                    || (existing.name.local == name.local
                        && existing.name.namespace == name.namespace
                        && name.namespace.is_some())
            });
            if duplicate {
                self.diags.push(ParseDiagnostic::error(
                    raw.name_range,
                    DiagnosticCode::DuplicateAttribute,
                    format!("duplicate attribute `{}`", raw.name),
                ));
            }

            attributes.push(Attribute {
                name,
                name_range: raw.name_range,
                value: raw.value.clone().unwrap_or_default(),
                value_range: raw
                    .value_range
                    .unwrap_or(TextRange::empty(raw.name_range.end)),
            });
        }
        attributes
    }

    /// Target of a processing instruction: `<?target …?>`.
    fn pi_target(&self, range: TextRange) -> String {
        let inner_start = (range.start + 2).min(range.end);
        let inner_end = range.end.saturating_sub(2).max(inner_start);
        self.text[inner_start..inner_end]
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string()
    }
}

/// quick-xml yields names as bytes; the source text is guaranteed UTF-8.
fn raw_str(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).expect("source text is valid UTF-8")
}

fn intern(cache: &mut HashMap<String, NsUri>, uri: &str) -> NsUri {
    if let Some(interned) = cache.get(uri) {
        return interned.clone();
    }
    let interned: NsUri = Arc::from(uri);
    cache.insert(uri.to_string(), interned.clone());
    interned
}
