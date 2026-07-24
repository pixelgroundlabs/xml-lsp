//! Context-free queries against the schema model.
//!
//! These walk the pattern graph without tracking how much of a content
//! model has already been consumed — that context-aware part is the job of
//! the validation engine (next milestone). They are the "schema browsing"
//! layer used for documentation lookup and as the basis for completion.

use std::collections::HashSet;

use crate::model::{AttributePattern, ElementPattern, Pattern, PatternId, Schema, ValuePattern};

/// What a traversal is collecting.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Collect {
    Elements,
    Attributes,
    Values,
}

impl Schema {
    /// The element patterns that can occur in the given content pattern,
    /// without descending into nested elements.
    pub fn child_elements(&self, content: PatternId) -> Vec<PatternId> {
        self.collect_reachable(content, Collect::Elements)
    }

    /// The attribute patterns reachable in the given content pattern.
    pub fn attributes(&self, content: PatternId) -> Vec<PatternId> {
        self.collect_reachable(content, Collect::Attributes)
    }

    /// The fixed values (enumerations) reachable in the given content
    /// pattern, e.g. of an attribute.
    pub fn value_enum(&self, content: PatternId) -> Vec<PatternId> {
        self.collect_reachable(content, Collect::Values)
    }

    /// The attribute patterns that are mandatory in the given content
    /// pattern: reachable without passing `optional`, `zeroOrMore`, or a
    /// `choice` branch (attributes only required in some branches are not
    /// reported).
    pub fn required_attributes(&self, content: PatternId) -> Vec<PatternId> {
        let mut visited = HashSet::new();
        let mut out = Vec::new();
        self.walk_required(content, &mut visited, &mut out);
        out
    }

    fn walk_required(
        &self,
        id: PatternId,
        visited: &mut HashSet<PatternId>,
        out: &mut Vec<PatternId>,
    ) {
        if !visited.insert(id) {
            return;
        }
        match self.pattern(id) {
            Pattern::Group(items) | Pattern::Interleave(items) => {
                for &item in items {
                    self.walk_required(item, visited, out);
                }
            }
            Pattern::OneOrMore(inner) => self.walk_required(*inner, visited, out),
            Pattern::Ref(define) => self.walk_required(self.define(*define).pattern, visited, out),
            Pattern::Attribute(_) => out.push(id),
            // Choice, Optional, ZeroOrMore: nothing below is mandatory.
            _ => {}
        }
    }

    /// Whether the content pattern allows anything besides attributes —
    /// child elements, text, or data. `false` means the element is empty
    /// and can be written self-closing.
    pub fn allows_content(&self, content: PatternId) -> bool {
        let mut visited = HashSet::new();
        self.content_reachable(content, &mut visited)
    }

    fn content_reachable(&self, id: PatternId, visited: &mut HashSet<PatternId>) -> bool {
        if !visited.insert(id) {
            return false;
        }
        match self.pattern(id) {
            Pattern::Choice(items) | Pattern::Group(items) | Pattern::Interleave(items) => items
                .iter()
                .any(|&item| self.content_reachable(item, visited)),
            Pattern::Optional(inner)
            | Pattern::ZeroOrMore(inner)
            | Pattern::OneOrMore(inner)
            | Pattern::List(inner) => self.content_reachable(*inner, visited),
            Pattern::Ref(define) => self.content_reachable(self.define(*define).pattern, visited),
            Pattern::Element(_) | Pattern::Text | Pattern::Data(_) | Pattern::Value(_) => true,
            Pattern::Attribute(_) | Pattern::Empty | Pattern::NotAllowed => false,
        }
    }

    /// Whether the content pattern allows character data (text, data, or
    /// fixed values) directly — i.e. the element has text or mixed
    /// content. Element-only content returns `false`.
    pub fn allows_text(&self, content: PatternId) -> bool {
        let mut visited = HashSet::new();
        self.text_reachable(content, &mut visited)
    }

    fn text_reachable(&self, id: PatternId, visited: &mut HashSet<PatternId>) -> bool {
        if !visited.insert(id) {
            return false;
        }
        match self.pattern(id) {
            Pattern::Choice(items) | Pattern::Group(items) | Pattern::Interleave(items) => {
                items.iter().any(|&item| self.text_reachable(item, visited))
            }
            Pattern::Optional(inner)
            | Pattern::ZeroOrMore(inner)
            | Pattern::OneOrMore(inner)
            | Pattern::List(inner) => self.text_reachable(*inner, visited),
            Pattern::Ref(define) => self.text_reachable(self.define(*define).pattern, visited),
            Pattern::Text | Pattern::Data(_) | Pattern::Value(_) => true,
            Pattern::Element(_) | Pattern::Attribute(_) | Pattern::Empty | Pattern::NotAllowed => {
                false
            }
        }
    }

    /// Finds the first element pattern in the whole schema whose name class
    /// matches, e.g. for documentation lookup by element name.
    pub fn find_element(&self, ns: Option<&str>, local: &str) -> Option<PatternId> {
        (0..self.pattern_count() as u32)
            .map(PatternId::from_raw)
            .find(|&id| {
                matches!(self.pattern(id), Pattern::Element(el) if el.name.matches(ns, local))
            })
    }

    /// Convenience accessor for element patterns.
    pub fn element_pattern(&self, id: PatternId) -> Option<&ElementPattern> {
        match self.pattern(id) {
            Pattern::Element(el) => Some(el),
            _ => None,
        }
    }

    /// Convenience accessor for attribute patterns.
    pub fn attribute_pattern(&self, id: PatternId) -> Option<&AttributePattern> {
        match self.pattern(id) {
            Pattern::Attribute(attr) => Some(attr),
            _ => None,
        }
    }

    /// Convenience accessor for value patterns.
    pub fn value_pattern(&self, id: PatternId) -> Option<&ValuePattern> {
        match self.pattern(id) {
            Pattern::Value(value) => Some(value),
            _ => None,
        }
    }

    /// The documentation attached to a pattern, if any.
    pub fn documentation(&self, id: PatternId) -> Option<&str> {
        match self.pattern(id) {
            Pattern::Element(el) => el.documentation.as_deref(),
            Pattern::Attribute(attr) => attr.documentation.as_deref(),
            Pattern::Value(value) => value.documentation.as_deref(),
            _ => None,
        }
    }

    fn collect_reachable(&self, start: PatternId, what: Collect) -> Vec<PatternId> {
        let mut visited = HashSet::new();
        let mut out = Vec::new();
        self.walk(start, what, &mut visited, &mut out);
        out
    }

    fn walk(
        &self,
        id: PatternId,
        what: Collect,
        visited: &mut HashSet<PatternId>,
        out: &mut Vec<PatternId>,
    ) {
        if !visited.insert(id) {
            return;
        }
        match self.pattern(id) {
            Pattern::Choice(items) | Pattern::Group(items) | Pattern::Interleave(items) => {
                for &item in items {
                    self.walk(item, what, visited, out);
                }
            }
            Pattern::Optional(inner)
            | Pattern::ZeroOrMore(inner)
            | Pattern::OneOrMore(inner)
            | Pattern::List(inner) => self.walk(*inner, what, visited, out),
            Pattern::Ref(define) => self.walk(self.define(*define).pattern, what, visited, out),
            // Elements and attributes are collected but not entered: their
            // content belongs to a deeper level of the document.
            Pattern::Element(_) => {
                if what == Collect::Elements {
                    out.push(id);
                }
            }
            Pattern::Attribute(_) => {
                if what == Collect::Attributes {
                    out.push(id);
                }
            }
            Pattern::Value(_) => {
                if what == Collect::Values {
                    out.push(id);
                }
            }
            Pattern::Data(data) => {
                // Values may appear in a data except clause
                // (e.g. "any string except these").
                if let Some(except) = data.except {
                    self.walk(except, what, visited, out);
                }
            }
            Pattern::Empty | Pattern::Text | Pattern::NotAllowed => {}
        }
    }
}
