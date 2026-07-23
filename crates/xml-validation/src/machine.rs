//! The derivative machine: James Clark's algorithm for RELAX NG
//! validation, adapted to the tree-shaped schema model.
//!
//! Validation states are patterns. Consuming a document event (start tag,
//! attribute, text, end tag) turns one pattern into another — its
//! *derivative*. A state of `NotAllowed` means the event was invalid.
//!
//! Patterns are hash-consed: structurally equal states share one id,
//! which makes the memo tables for the derivative functions effective.
//! The same state machinery answers "what is expected here?" — the basis
//! for diagnostics now and for completion later.

use std::collections::HashMap;

use xml_schema::{DefineId, Pattern, PatternId, Schema};

use crate::datatypes::Datatypes;

/// Handle to a validation pattern (a state).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VId(u32);

/// Interned qualified name of a document element or attribute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QId(u32);

/// The binary pattern algebra of the derivative algorithm. Elements,
/// attributes, values, and data keep a link to their schema pattern so
/// diagnostics and completion can reach names and documentation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum VKind {
    Empty,
    NotAllowed,
    Text,
    Choice(VId, VId),
    Group(VId, VId),
    Interleave(VId, VId),
    OneOrMore(VId),
    List(VId),
    /// Content is converted lazily on entry; this keeps recursive schemas
    /// finite.
    Element(PatternId),
    Attribute(PatternId, VId),
    Value(PatternId),
    Data(PatternId, Option<VId>),
    /// `After(body, continuation)`: finish `body`, then continue with the
    /// parent's remaining content model.
    After(VId, VId),
}

struct VNode {
    kind: VKind,
    nullable: bool,
}

pub const EMPTY: VId = VId(0);
pub const NOT_ALLOWED: VId = VId(1);
pub const TEXT: VId = VId(2);

/// The machine owns the state arena and all memo tables for one schema.
/// It can be reused across documents; the tables keep paying off.
pub struct Machine<'s> {
    pub schema: &'s Schema,
    nodes: Vec<VNode>,
    interner: HashMap<VKind, VId>,
    qnames: HashMap<(Option<Box<str>>, Box<str>), QId>,
    /// Schema pattern → converted validation pattern.
    convert_memo: HashMap<PatternId, VId>,
    define_memo: HashMap<DefineId, VId>,
    /// In-progress defines, to catch illegal (non-contracting) recursion.
    define_in_progress: Vec<DefineId>,
    memo_start_tag: HashMap<(VId, QId), VId>,
    memo_start_tag_close: HashMap<VId, VId>,
    memo_end_tag: HashMap<VId, VId>,
    pub datatypes: Datatypes,
}

impl<'s> Machine<'s> {
    pub fn new(schema: &'s Schema) -> Self {
        let mut machine = Self {
            schema,
            nodes: Vec::new(),
            interner: HashMap::new(),
            qnames: HashMap::new(),
            convert_memo: HashMap::new(),
            define_memo: HashMap::new(),
            define_in_progress: Vec::new(),
            memo_start_tag: HashMap::new(),
            memo_start_tag_close: HashMap::new(),
            memo_end_tag: HashMap::new(),
            datatypes: Datatypes::default(),
        };
        // Fixed singletons; their ids are the EMPTY/NOT_ALLOWED/TEXT consts.
        machine.intern(VKind::Empty, true);
        machine.intern(VKind::NotAllowed, false);
        machine.intern(VKind::Text, true);
        machine
    }

    pub fn nullable(&self, id: VId) -> bool {
        self.nodes[id.0 as usize].nullable
    }

    fn kind(&self, id: VId) -> &VKind {
        &self.nodes[id.0 as usize].kind
    }

    pub fn qname(&mut self, ns: Option<&str>, local: &str) -> QId {
        let key = (ns.map(Box::from), Box::from(local));
        if let Some(&id) = self.qnames.get(&key) {
            return id;
        }
        let id = QId(self.qnames.len() as u32);
        self.qnames.insert(key, id);
        id
    }

    fn intern(&mut self, kind: VKind, nullable: bool) -> VId {
        if let Some(&id) = self.interner.get(&kind) {
            return id;
        }
        let id = VId(u32::try_from(self.nodes.len()).expect("too many validation states"));
        self.nodes.push(VNode {
            kind: kind.clone(),
            nullable,
        });
        self.interner.insert(kind, id);
        id
    }

    // ------------------------------------------------------------------
    // Smart constructors (they simplify while building)
    // ------------------------------------------------------------------

    pub fn choice(&mut self, a: VId, b: VId) -> VId {
        if a == NOT_ALLOWED || a == b {
            return b;
        }
        if b == NOT_ALLOWED {
            return a;
        }
        // Normalize so that `choice(a, b)` and `choice(b, a)` intern to the
        // same state.
        let (a, b) = if a.0 <= b.0 { (a, b) } else { (b, a) };
        let nullable = self.nullable(a) || self.nullable(b);
        self.intern(VKind::Choice(a, b), nullable)
    }

    pub fn group(&mut self, a: VId, b: VId) -> VId {
        if a == NOT_ALLOWED || b == NOT_ALLOWED {
            return NOT_ALLOWED;
        }
        if a == EMPTY {
            return b;
        }
        if b == EMPTY {
            return a;
        }
        let nullable = self.nullable(a) && self.nullable(b);
        self.intern(VKind::Group(a, b), nullable)
    }

    pub fn interleave(&mut self, a: VId, b: VId) -> VId {
        if a == NOT_ALLOWED || b == NOT_ALLOWED {
            return NOT_ALLOWED;
        }
        if a == EMPTY {
            return b;
        }
        if b == EMPTY {
            return a;
        }
        let (a, b) = if a.0 <= b.0 { (a, b) } else { (b, a) };
        let nullable = self.nullable(a) && self.nullable(b);
        self.intern(VKind::Interleave(a, b), nullable)
    }

    pub fn one_or_more(&mut self, p: VId) -> VId {
        if p == NOT_ALLOWED || p == EMPTY {
            return p;
        }
        let nullable = self.nullable(p);
        self.intern(VKind::OneOrMore(p), nullable)
    }

    pub fn after(&mut self, body: VId, cont: VId) -> VId {
        if body == NOT_ALLOWED || cont == NOT_ALLOWED {
            return NOT_ALLOWED;
        }
        self.intern(VKind::After(body, cont), false)
    }

    // ------------------------------------------------------------------
    // Schema pattern → validation pattern
    // ------------------------------------------------------------------

    /// Converts a schema pattern into a validation pattern. Element
    /// content is *not* converted here — entering the element does that —
    /// so recursive content models stay finite.
    pub fn convert(&mut self, id: PatternId) -> VId {
        if let Some(&v) = self.convert_memo.get(&id) {
            return v;
        }
        let v = match self.schema.pattern(id).clone() {
            Pattern::Empty => EMPTY,
            Pattern::NotAllowed => NOT_ALLOWED,
            Pattern::Text => TEXT,
            Pattern::Choice(items) => self.fold(&items, Self::choice, NOT_ALLOWED),
            Pattern::Group(items) => self.fold(&items, Self::group, EMPTY),
            Pattern::Interleave(items) => self.fold(&items, Self::interleave, EMPTY),
            Pattern::Optional(inner) => {
                let inner = self.convert(inner);
                self.choice(EMPTY, inner)
            }
            Pattern::ZeroOrMore(inner) => {
                let inner = self.convert(inner);
                let one = self.one_or_more(inner);
                self.choice(EMPTY, one)
            }
            Pattern::OneOrMore(inner) => {
                let inner = self.convert(inner);
                self.one_or_more(inner)
            }
            Pattern::List(inner) => {
                let inner = self.convert(inner);
                self.intern(VKind::List(inner), false)
            }
            Pattern::Ref(define) => self.convert_define(define),
            Pattern::Element(_) => self.intern(VKind::Element(id), false),
            Pattern::Attribute(attr) => {
                let content = self.convert(attr.content);
                self.intern(VKind::Attribute(id, content), false)
            }
            Pattern::Value(_) => self.intern(VKind::Value(id), false),
            Pattern::Data(data) => {
                let except = data.except.map(|e| self.convert(e));
                self.intern(VKind::Data(id, except), false)
            }
        };
        self.convert_memo.insert(id, v);
        v
    }

    fn convert_define(&mut self, define: DefineId) -> VId {
        if let Some(&v) = self.define_memo.get(&define) {
            return v;
        }
        if self.define_in_progress.contains(&define) {
            // Recursion that does not pass through an element is illegal
            // in RELAX NG; break the cycle instead of overflowing.
            return NOT_ALLOWED;
        }
        self.define_in_progress.push(define);
        let v = self.convert(self.schema.define(define).pattern);
        self.define_in_progress.pop();
        self.define_memo.insert(define, v);
        v
    }

    fn fold(
        &mut self,
        items: &[PatternId],
        combine: fn(&mut Self, VId, VId) -> VId,
        empty: VId,
    ) -> VId {
        let mut iter = items.iter().rev();
        let Some(&last) = iter.next() else {
            return empty;
        };
        let mut acc = self.convert(last);
        for &item in iter {
            let lhs = self.convert(item);
            acc = combine(self, lhs, acc);
        }
        acc
    }

    /// The content pattern of an element, converted on entry.
    pub fn element_content(&mut self, element: PatternId) -> VId {
        let content = self
            .schema
            .element_pattern(element)
            .expect("id refers to an element pattern")
            .content;
        self.convert(content)
    }

    // ------------------------------------------------------------------
    // Derivatives
    // ------------------------------------------------------------------

    /// Applies `f` to the continuation of every `After` in `p`
    /// (Clark's `applyAfter`).
    fn apply_after(&mut self, p: VId, f: &mut dyn FnMut(&mut Self, VId) -> VId) -> VId {
        match *self.kind(p) {
            VKind::After(body, cont) => {
                let cont = f(self, cont);
                self.after(body, cont)
            }
            VKind::Choice(a, b) => {
                let a = self.apply_after(a, f);
                let b = self.apply_after(b, f);
                self.choice(a, b)
            }
            VKind::NotAllowed => NOT_ALLOWED,
            _ => NOT_ALLOWED,
        }
    }

    /// Derivative for an opening start tag `<name …`.
    pub fn start_tag_open(&mut self, p: VId, name: QId, ns: Option<&str>, local: &str) -> VId {
        if let Some(&v) = self.memo_start_tag.get(&(p, name)) {
            return v;
        }
        let v = match *self.kind(p) {
            VKind::Choice(a, b) => {
                let a = self.start_tag_open(a, name, ns, local);
                let b = self.start_tag_open(b, name, ns, local);
                self.choice(a, b)
            }
            VKind::Element(element) => {
                let matches = self
                    .schema
                    .element_pattern(element)
                    .expect("element pattern")
                    .name
                    .matches(ns, local);
                if matches {
                    let content = self.element_content(element);
                    self.after(content, EMPTY)
                } else {
                    NOT_ALLOWED
                }
            }
            VKind::Group(a, b) => {
                let da = self.start_tag_open(a, name, ns, local);
                let x = self.apply_after(da, &mut |m, d| m.group(d, b));
                if self.nullable(a) {
                    let db = self.start_tag_open(b, name, ns, local);
                    self.choice(x, db)
                } else {
                    x
                }
            }
            VKind::Interleave(a, b) => {
                let da = self.start_tag_open(a, name, ns, local);
                let x = self.apply_after(da, &mut |m, d| m.interleave(d, b));
                let db = self.start_tag_open(b, name, ns, local);
                let y = self.apply_after(db, &mut |m, d| m.interleave(a, d));
                self.choice(x, y)
            }
            VKind::OneOrMore(inner) => {
                let d = self.start_tag_open(inner, name, ns, local);
                self.apply_after(d, &mut |m, cont| {
                    let more = m.choice(EMPTY, p);
                    m.group(cont, more)
                })
            }
            VKind::After(a, b) => {
                let d = self.start_tag_open(a, name, ns, local);
                self.apply_after(d, &mut |m, cont| m.after(cont, b))
            }
            _ => NOT_ALLOWED,
        };
        self.memo_start_tag.insert((p, name), v);
        v
    }

    /// Derivative for one attribute `name="value"`.
    pub fn attribute(&mut self, p: VId, ns: Option<&str>, local: &str, value: &str) -> VId {
        match *self.kind(p) {
            VKind::After(a, b) => {
                let da = self.attribute(a, ns, local, value);
                self.after(da, b)
            }
            VKind::Choice(a, b) => {
                let da = self.attribute(a, ns, local, value);
                let db = self.attribute(b, ns, local, value);
                self.choice(da, db)
            }
            VKind::Group(a, b) => {
                let da = self.attribute(a, ns, local, value);
                let left = self.group(da, b);
                let db = self.attribute(b, ns, local, value);
                let right = self.group(a, db);
                self.choice(left, right)
            }
            VKind::Interleave(a, b) => {
                let da = self.attribute(a, ns, local, value);
                let left = self.interleave(da, b);
                let db = self.attribute(b, ns, local, value);
                let right = self.interleave(a, db);
                self.choice(left, right)
            }
            VKind::OneOrMore(inner) => {
                let d = self.attribute(inner, ns, local, value);
                let more = self.choice(EMPTY, p);
                self.group(d, more)
            }
            VKind::Attribute(attr, content) => {
                let matches = self
                    .schema
                    .attribute_pattern(attr)
                    .expect("attribute pattern")
                    .name
                    .matches(ns, local);
                if matches && self.value_matches(content, value) {
                    EMPTY
                } else {
                    NOT_ALLOWED
                }
            }
            _ => NOT_ALLOWED,
        }
    }

    /// Like [`Self::attribute`], but matches exactly one attribute pattern
    /// and ignores its value — used to recover and to compute which
    /// required attributes are missing.
    pub fn attribute_lenient(&mut self, p: VId, target: PatternId) -> VId {
        match *self.kind(p) {
            VKind::After(a, b) => {
                let da = self.attribute_lenient(a, target);
                self.after(da, b)
            }
            VKind::Choice(a, b) => {
                let da = self.attribute_lenient(a, target);
                let db = self.attribute_lenient(b, target);
                self.choice(da, db)
            }
            VKind::Group(a, b) => {
                let da = self.attribute_lenient(a, target);
                let left = self.group(da, b);
                let db = self.attribute_lenient(b, target);
                let right = self.group(a, db);
                self.choice(left, right)
            }
            VKind::Interleave(a, b) => {
                let da = self.attribute_lenient(a, target);
                let left = self.interleave(da, b);
                let db = self.attribute_lenient(b, target);
                let right = self.interleave(a, db);
                self.choice(left, right)
            }
            VKind::OneOrMore(inner) => {
                let d = self.attribute_lenient(inner, target);
                let more = self.choice(EMPTY, p);
                self.group(d, more)
            }
            VKind::Attribute(attr, _) if attr == target => EMPTY,
            _ => NOT_ALLOWED,
        }
    }

    /// Derivative for the end of a start tag (`>`): all remaining required
    /// attributes make the state invalid.
    pub fn start_tag_close(&mut self, p: VId) -> VId {
        if let Some(&v) = self.memo_start_tag_close.get(&p) {
            return v;
        }
        let v = match *self.kind(p) {
            VKind::After(a, b) => {
                let da = self.start_tag_close(a);
                self.after(da, b)
            }
            VKind::Choice(a, b) => {
                let da = self.start_tag_close(a);
                let db = self.start_tag_close(b);
                self.choice(da, db)
            }
            VKind::Group(a, b) => {
                let da = self.start_tag_close(a);
                let db = self.start_tag_close(b);
                self.group(da, db)
            }
            VKind::Interleave(a, b) => {
                let da = self.start_tag_close(a);
                let db = self.start_tag_close(b);
                self.interleave(da, db)
            }
            VKind::OneOrMore(inner) => {
                let d = self.start_tag_close(inner);
                self.one_or_more(d)
            }
            VKind::Attribute(..) => NOT_ALLOWED,
            _ => p,
        };
        self.memo_start_tag_close.insert(p, v);
        v
    }

    /// Derivative for text content. `s` is the raw (entity-resolved) text.
    pub fn text(&mut self, p: VId, s: &str) -> VId {
        match *self.kind(p) {
            VKind::Choice(a, b) => {
                let da = self.text(a, s);
                let db = self.text(b, s);
                self.choice(da, db)
            }
            VKind::Interleave(a, b) => {
                let da = self.text(a, s);
                let left = self.interleave(da, b);
                let db = self.text(b, s);
                let right = self.interleave(a, db);
                self.choice(left, right)
            }
            VKind::Group(a, b) => {
                let da = self.text(a, s);
                let x = self.group(da, b);
                if self.nullable(a) {
                    let db = self.text(b, s);
                    self.choice(x, db)
                } else {
                    x
                }
            }
            VKind::After(a, b) => {
                let da = self.text(a, s);
                self.after(da, b)
            }
            VKind::OneOrMore(inner) => {
                let d = self.text(inner, s);
                let more = self.choice(EMPTY, p);
                self.group(d, more)
            }
            // Text absorbs any amount of text and stays available.
            VKind::Text => TEXT,
            VKind::Value(value) => {
                let pattern = self
                    .schema
                    .value_pattern(value)
                    .expect("value pattern")
                    .clone();
                if self.datatypes.value_equal(&pattern, s) {
                    EMPTY
                } else {
                    NOT_ALLOWED
                }
            }
            VKind::Data(data, except) => {
                let pattern = match self.schema.pattern(data) {
                    Pattern::Data(d) => d.clone(),
                    _ => unreachable!("id refers to a data pattern"),
                };
                let mut ok = self.datatypes.data_allows(&pattern, s);
                if ok {
                    if let Some(except) = except {
                        let d = self.text(except, s);
                        if self.nullable(d) {
                            ok = false;
                        }
                    }
                }
                if ok {
                    EMPTY
                } else {
                    NOT_ALLOWED
                }
            }
            VKind::List(inner) => {
                let mut state = inner;
                for token in s.split_whitespace() {
                    state = self.text(state, token);
                    if state == NOT_ALLOWED {
                        break;
                    }
                }
                if self.nullable(state) {
                    EMPTY
                } else {
                    NOT_ALLOWED
                }
            }
            _ => NOT_ALLOWED,
        }
    }

    /// Greedily consumes required attributes until the start tag could
    /// close. Returns the unblocked state and the consumed attribute
    /// patterns — the "missing required attributes" set.
    pub fn unblock_required_attributes(&mut self, state: VId) -> (VId, Vec<PatternId>) {
        let mut missing = Vec::new();
        let mut current = state;
        // Every round must consume one more attribute; the candidate list
        // is finite, so this terminates.
        loop {
            if self.start_tag_close(current) != NOT_ALLOWED {
                break;
            }
            let candidates = self.expected_attributes(current);
            let mut advanced = false;
            for candidate in candidates {
                let next = self.attribute_lenient(current, candidate);
                if next != NOT_ALLOWED {
                    missing.push(candidate);
                    current = next;
                    advanced = true;
                    break;
                }
            }
            if !advanced {
                break;
            }
        }
        (current, missing)
    }

    /// Closes the start tag even if required attributes are missing —
    /// recovery and state computation for completion.
    pub fn start_tag_close_lenient(&mut self, state: VId) -> VId {
        let closed = self.start_tag_close(state);
        if closed != NOT_ALLOWED {
            return closed;
        }
        let (unblocked, _) = self.unblock_required_attributes(state);
        let closed = self.start_tag_close(unblocked);
        if closed != NOT_ALLOWED {
            closed
        } else {
            unblocked
        }
    }

    /// Recovery variant of [`Self::text`]: consumes text as if it matched
    /// whatever value or data pattern was expected.
    pub fn text_lenient(&mut self, p: VId) -> VId {
        match *self.kind(p) {
            VKind::Choice(a, b) => {
                let da = self.text_lenient(a);
                let db = self.text_lenient(b);
                self.choice(da, db)
            }
            VKind::Interleave(a, b) => {
                let da = self.text_lenient(a);
                let left = self.interleave(da, b);
                let db = self.text_lenient(b);
                let right = self.interleave(a, db);
                self.choice(left, right)
            }
            VKind::Group(a, b) => {
                let da = self.text_lenient(a);
                let x = self.group(da, b);
                if self.nullable(a) {
                    let db = self.text_lenient(b);
                    self.choice(x, db)
                } else {
                    x
                }
            }
            VKind::After(a, b) => {
                let da = self.text_lenient(a);
                self.after(da, b)
            }
            VKind::OneOrMore(inner) => {
                let d = self.text_lenient(inner);
                let more = self.choice(EMPTY, p);
                self.group(d, more)
            }
            VKind::Text => TEXT,
            VKind::Value(_) | VKind::Data(..) | VKind::List(_) => EMPTY,
            _ => NOT_ALLOWED,
        }
    }

    /// Derivative for the end tag of the current element.
    pub fn end_tag(&mut self, p: VId) -> VId {
        if let Some(&v) = self.memo_end_tag.get(&p) {
            return v;
        }
        let v = match *self.kind(p) {
            VKind::Choice(a, b) => {
                let da = self.end_tag(a);
                let db = self.end_tag(b);
                self.choice(da, db)
            }
            VKind::After(a, b) => {
                if self.nullable(a) {
                    b
                } else {
                    NOT_ALLOWED
                }
            }
            _ => NOT_ALLOWED,
        };
        self.memo_end_tag.insert(p, v);
        v
    }

    /// Recovery variant of [`Self::end_tag`]: closes the element even if
    /// required content is missing, so validation can continue in the
    /// parent.
    pub fn end_tag_forced(&mut self, p: VId) -> VId {
        match *self.kind(p) {
            VKind::Choice(a, b) => {
                let da = self.end_tag_forced(a);
                let db = self.end_tag_forced(b);
                self.choice(da, db)
            }
            VKind::After(_, b) => b,
            _ => NOT_ALLOWED,
        }
    }

    /// Whether `value` is accepted by an attribute content pattern.
    fn value_matches(&mut self, content: VId, value: &str) -> bool {
        if self.nullable(content) && value.trim().is_empty() {
            return true;
        }
        let d = self.text(content, value);
        self.nullable(d)
    }

    // ------------------------------------------------------------------
    // Expectations (for messages, recovery, and later completion)
    // ------------------------------------------------------------------

    /// The element patterns acceptable as the next start tag in `p`.
    pub fn expected_elements(&self, p: VId) -> Vec<PatternId> {
        let mut out = Vec::new();
        let mut visited = std::collections::HashSet::new();
        self.expect_walk(
            p,
            &mut visited,
            &mut |kind| match *kind {
                VKind::Element(id) => Some(id),
                _ => None,
            },
            // Elements follow sequence order: the right half of a group
            // is only reachable once the left half can be complete.
            false,
            &mut out,
        );
        out
    }

    /// The attribute patterns still acceptable in `p`.
    pub fn expected_attributes(&self, p: VId) -> Vec<PatternId> {
        let mut out = Vec::new();
        let mut visited = std::collections::HashSet::new();
        self.expect_walk(
            p,
            &mut visited,
            &mut |kind| match *kind {
                VKind::Attribute(id, _) => Some(id),
                _ => None,
            },
            // Attributes are order-independent: attDeriv descends into
            // both halves of a group unconditionally.
            true,
            &mut out,
        );
        out
    }

    fn expect_walk(
        &self,
        p: VId,
        visited: &mut std::collections::HashSet<VId>,
        collect: &mut dyn FnMut(&VKind) -> Option<PatternId>,
        through_groups: bool,
        out: &mut Vec<PatternId>,
    ) {
        if !visited.insert(p) {
            return;
        }
        if let Some(id) = collect(self.kind(p)) {
            if !out.contains(&id) {
                out.push(id);
            }
            return;
        }
        match *self.kind(p) {
            VKind::Choice(a, b) | VKind::Interleave(a, b) => {
                self.expect_walk(a, visited, collect, through_groups, out);
                self.expect_walk(b, visited, collect, through_groups, out);
            }
            VKind::Group(a, b) => {
                self.expect_walk(a, visited, collect, through_groups, out);
                if through_groups || self.nullable(a) {
                    self.expect_walk(b, visited, collect, through_groups, out);
                }
            }
            VKind::OneOrMore(inner) => {
                self.expect_walk(inner, visited, collect, through_groups, out)
            }
            // Only the current body counts; the continuation comes after
            // the end tag.
            VKind::After(a, _) => self.expect_walk(a, visited, collect, through_groups, out),
            _ => {}
        }
    }

    /// Whether text (or data) content is acceptable next.
    pub fn expects_text(&self, p: VId) -> bool {
        let mut visited = std::collections::HashSet::new();
        self.expects_text_walk(p, &mut visited)
    }

    fn expects_text_walk(&self, p: VId, visited: &mut std::collections::HashSet<VId>) -> bool {
        if !visited.insert(p) {
            return false;
        }
        match *self.kind(p) {
            VKind::Text | VKind::Value(_) | VKind::Data(..) | VKind::List(_) => true,
            VKind::Choice(a, b) | VKind::Interleave(a, b) => {
                self.expects_text_walk(a, visited) || self.expects_text_walk(b, visited)
            }
            VKind::Group(a, b) => {
                self.expects_text_walk(a, visited)
                    || (self.nullable(a) && self.expects_text_walk(b, visited))
            }
            VKind::OneOrMore(inner) => self.expects_text_walk(inner, visited),
            VKind::After(a, _) => self.expects_text_walk(a, visited),
            _ => false,
        }
    }
}
