//! Position-aware expectation queries: "what does the schema allow right
//! here?" — the foundation of completion (and of hover lookups).
//!
//! The same derivative machine that validates documents computes the
//! state at a cursor position by consuming everything before it. All
//! steps are lenient: completion must keep working in documents that are
//! currently invalid, which while typing is the normal case.

use xml_ast::{unescape, Document, NodeId, NodeKind, XMLNS_NAMESPACE};
use xml_schema::PatternId;

use crate::machine::{VId, EMPTY, NOT_ALLOWED};
use crate::validator::Validator;

/// What the schema accepts at a queried position.
#[derive(Debug, Default)]
pub struct Expectations {
    /// Element patterns acceptable as the next child.
    pub elements: Vec<PatternId>,
    /// Attribute patterns still available in the start tag.
    pub attributes: Vec<PatternId>,
    /// The subset of `attributes` that is required.
    pub required_attributes: Vec<PatternId>,
    /// Whether text content is acceptable here.
    pub text: bool,
}

impl<'s> Validator<'s> {
    /// Expectations for element content: which children and text are
    /// allowed inside `container` at `offset`? `None` queries the
    /// document's top level (root element candidates).
    pub fn content_expectations(
        &mut self,
        doc: &Document,
        text: &str,
        container: Option<NodeId>,
        offset: usize,
    ) -> Expectations {
        let state = self.content_state(doc, text, container, offset);
        Expectations {
            elements: self.machine.expected_elements(state),
            text: self.machine.expects_text(state),
            ..Expectations::default()
        }
    }

    /// Expectations inside the start tag of `element`: which attributes
    /// can still be added? `exclude` names an attribute index to leave
    /// unconsumed (the one currently being typed).
    pub fn attribute_expectations(
        &mut self,
        doc: &Document,
        text: &str,
        element: NodeId,
        exclude: Option<usize>,
    ) -> Expectations {
        let state = self.tag_state(doc, text, element, exclude);
        let attributes = self.machine.expected_attributes(state);
        let (_, required_attributes) = self.machine.unblock_required_attributes(state);
        Expectations {
            attributes,
            required_attributes,
            ..Expectations::default()
        }
    }

    /// The schema attribute pattern matching attribute `attr_index` of
    /// `element` — the entry point for value completion and hover.
    pub fn attribute_pattern_at(
        &mut self,
        doc: &Document,
        text: &str,
        element: NodeId,
        attr_index: usize,
    ) -> Option<PatternId> {
        let attribute = doc.element(element)?.attributes.get(attr_index)?.clone();
        let state = self.tag_state(doc, text, element, Some(attr_index));
        let ns = attribute.name.namespace.as_deref();
        let local = attribute.name.local.as_str();
        self.machine
            .expected_attributes(state)
            .into_iter()
            .find(|&id| {
                self.machine
                    .schema
                    .attribute_pattern(id)
                    .is_some_and(|a| a.name.matches(ns, local))
            })
    }

    /// The schema element pattern that `element` resolves to in its
    /// context (with a schema-wide name lookup as fallback) — for hover.
    pub fn element_pattern_at(
        &mut self,
        doc: &Document,
        text: &str,
        element: NodeId,
    ) -> Option<PatternId> {
        let el = doc.element(element)?.clone();
        let ns = el.name.namespace.as_deref();
        let local = el.name.local.as_str();
        let container = parent_container(doc, element);
        let parent_state = self.content_state(doc, text, container, doc.range(element).start);
        let candidate = self
            .machine
            .expected_elements(parent_state)
            .into_iter()
            .find(|&id| {
                self.machine
                    .schema
                    .element_pattern(id)
                    .is_some_and(|e| e.name.matches(ns, local))
            });
        candidate.or_else(|| self.machine.schema.find_element(ns, local))
    }

    // ------------------------------------------------------------------
    // State computation
    // ------------------------------------------------------------------

    /// The machine state inside `container`, after consuming all children
    /// that end at or before `offset`.
    fn content_state(
        &mut self,
        doc: &Document,
        text: &str,
        container: Option<NodeId>,
        offset: usize,
    ) -> VId {
        let Some(element) = container else {
            return self.machine.convert(self.machine.schema.start());
        };

        let state = self.tag_state(doc, text, element, None);
        let mut state = self.machine.start_tag_close_lenient(state);

        for &child in doc.children(element) {
            if doc.range(child).end > offset {
                break;
            }
            match doc.kind(child) {
                NodeKind::Element(el) => {
                    let el = el.clone();
                    let ns = el.name.namespace.as_deref();
                    let qid = self.machine.qname(ns, &el.name.local);
                    let d = self.machine.start_tag_open(state, qid, ns, &el.name.local);
                    if d != NOT_ALLOWED {
                        // Assume the sibling's content was fine; only its
                        // effect on the sequence matters here.
                        let after = self.machine.end_tag_forced(d);
                        if after != NOT_ALLOWED {
                            state = after;
                        }
                    }
                    // Unknown sibling: ignore it, like validation recovery.
                }
                NodeKind::Text | NodeKind::CData => {
                    let range = doc.range(child);
                    let raw = unescape(&text[range.start..range.end]);
                    let next = self.machine.text(state, &raw);
                    if next != NOT_ALLOWED {
                        state = next;
                    }
                }
                _ => {}
            }
        }
        state
    }

    /// The machine state inside the start tag of `element`, after the
    /// element name and all attributes except `exclude`.
    fn tag_state(
        &mut self,
        doc: &Document,
        text: &str,
        element: NodeId,
        exclude: Option<usize>,
    ) -> VId {
        let Some(el) = doc.element(element).cloned() else {
            return NOT_ALLOWED;
        };
        let ns = el.name.namespace.as_deref();
        let local = el.name.local.as_str();

        let container = parent_container(doc, element);
        let parent_state = self.content_state(doc, text, container, doc.range(element).start);

        let qid = self.machine.qname(ns, local);
        let mut state = self.machine.start_tag_open(parent_state, qid, ns, local);
        if state == NOT_ALLOWED {
            // The element is invalid here — fall back to its schema-wide
            // definition so completion still helps fixing it.
            match self.machine.schema.find_element(ns, local) {
                Some(known) => {
                    let content = self.machine.element_content(known);
                    state = self.machine.after(content, EMPTY);
                }
                None => return NOT_ALLOWED,
            }
        }

        for (i, attribute) in el.attributes.iter().enumerate() {
            if Some(i) == exclude || attribute.name.namespace.as_deref() == Some(XMLNS_NAMESPACE) {
                continue;
            }
            let ns = attribute.name.namespace.as_deref();
            let local = attribute.name.local.as_str();
            let value = unescape(&attribute.value);
            let next = self.machine.attribute(state, ns, local, &value);
            if next != NOT_ALLOWED {
                state = next;
                continue;
            }
            // Bad value or misplaced attribute: consume it leniently by
            // name so the remaining expectations stay accurate.
            let by_name = self
                .machine
                .expected_attributes(state)
                .into_iter()
                .find(|&id| {
                    self.machine
                        .schema
                        .attribute_pattern(id)
                        .is_some_and(|a| a.name.matches(ns, local))
                });
            if let Some(pattern) = by_name {
                let next = self.machine.attribute_lenient(state, pattern);
                if next != NOT_ALLOWED {
                    state = next;
                }
            }
        }
        state
    }
}

/// The container to query for an element's own position: its parent
/// element, or `None` at document level.
fn parent_container(doc: &Document, element: NodeId) -> Option<NodeId> {
    let parent = doc.parent(element)?;
    match doc.kind(parent) {
        NodeKind::Element(_) => Some(parent),
        _ => None,
    }
}
