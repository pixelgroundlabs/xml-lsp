//! The internal schema model.
//!
//! This is the central data structure the rest of the server works
//! against: patterns describe which elements, attributes, and values are
//! allowed, and carry the documentation shown in hover and completion.
//! The model is deliberately independent of the schema source — RELAX NG
//! is the first frontend that produces it; XSD is planned as a second one.

use std::collections::HashMap;

use xml_ast::{NsUri, TextRange};

/// Handle to a pattern in the schema's arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PatternId(u32);

impl PatternId {
    pub fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) fn from_raw(raw: u32) -> Self {
        Self(raw)
    }
}

/// Handle to a named definition (`<define>` in RELAX NG).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefineId(u32);

impl DefineId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A name class decides which qualified names are matched by an element or
/// attribute pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameClass {
    /// Exactly one name; `ns: None` means "no namespace".
    Name { ns: Option<NsUri>, local: String },
    /// Any name in one namespace, minus the exceptions.
    NsName {
        ns: Option<NsUri>,
        except: Option<Box<NameClass>>,
    },
    /// Any name at all, minus the exceptions.
    AnyName { except: Option<Box<NameClass>> },
    /// Any of the listed name classes.
    Choice(Vec<NameClass>),
}

impl NameClass {
    /// Whether the name class matches the given namespace and local name.
    pub fn matches(&self, ns: Option<&str>, local: &str) -> bool {
        match self {
            NameClass::Name {
                ns: own_ns,
                local: own_local,
            } => own_ns.as_deref() == ns && own_local == local,
            NameClass::NsName { ns: own_ns, except } => {
                own_ns.as_deref() == ns && !except.as_ref().is_some_and(|e| e.matches(ns, local))
            }
            NameClass::AnyName { except } => !except.as_ref().is_some_and(|e| e.matches(ns, local)),
            NameClass::Choice(choices) => choices.iter().any(|c| c.matches(ns, local)),
        }
    }

    /// The single concrete name, if the class is a plain `Name`.
    pub fn as_name(&self) -> Option<(Option<&str>, &str)> {
        match self {
            NameClass::Name { ns, local } => Some((ns.as_deref(), local)),
            _ => None,
        }
    }
}

/// A datatype reference: library URI (empty for the RELAX NG built-in
/// library) plus the type name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatatypeName {
    pub library: String,
    pub name: String,
}

/// Whether a symbol annotation marks a definition or a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolRole {
    Definition,
    Reference,
}

/// How an annotated attribute value carries a symbol name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ValueForm {
    /// The attribute value is the name itself.
    #[default]
    Literal,
    /// The value is an XPath expression; only a quoted string literal
    /// (`'name'`) carries a name.
    XPathString,
    /// The value is an XPath expression; only a plain variable
    /// reference (`$name`) carries a name.
    XPathVariable,
}

/// An `lsp:defines` / `lsp:references` annotation on an attribute
/// pattern (namespace `urn:xml-lsp:annotations`): this attribute
/// defines or references a named symbol of the given kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolAnnotation {
    pub role: SymbolRole,
    /// Symbol kind, the grouping key (e.g. "color", "fontfile").
    pub kind: String,
    pub form: ValueForm,
}

/// An `lsp:format` annotation on an element pattern: how the formatter
/// treats this element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FormatHints {
    /// Content is emitted verbatim (whitespace preserved).
    pub preserve: bool,
    /// Direct children are separated into blank-line groups.
    pub blank_lines: bool,
    /// Below this element, text-only elements stay on a single line.
    pub inline: bool,
}

/// `<element>` pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ElementPattern {
    pub name: NameClass,
    pub content: PatternId,
    pub documentation: Option<String>,
    /// Formatting hints from `lsp:format`, if annotated.
    pub format: Option<FormatHints>,
}

/// `<attribute>` pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributePattern {
    pub name: NameClass,
    pub content: PatternId,
    pub documentation: Option<String>,
    /// Symbol site from `lsp:defines`/`lsp:references`, if annotated.
    pub symbol: Option<SymbolAnnotation>,
}

/// `<value>` pattern: a fixed, allowed value (typically part of an
/// enumeration).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValuePattern {
    pub value: String,
    /// `None` means the built-in `token` type (whitespace-insensitive
    /// comparison).
    pub datatype: Option<DatatypeName>,
    pub documentation: Option<String>,
}

/// `<data>` pattern: a value constrained by a datatype and its parameters
/// (e.g. a regex via `<param name="pattern">`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPattern {
    pub datatype: DatatypeName,
    pub params: Vec<(String, String)>,
    pub except: Option<PatternId>,
}

/// The pattern language. This is a faithful representation of RELAX NG's
/// pattern algebra, which is general enough to express DTD and large parts
/// of XSD content models as well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Empty,
    Text,
    NotAllowed,
    Choice(Vec<PatternId>),
    Group(Vec<PatternId>),
    Interleave(Vec<PatternId>),
    Optional(PatternId),
    ZeroOrMore(PatternId),
    OneOrMore(PatternId),
    List(PatternId),
    Ref(DefineId),
    Element(ElementPattern),
    Attribute(AttributePattern),
    Value(ValuePattern),
    Data(DataPattern),
}

/// A named definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Define {
    pub name: String,
    pub pattern: PatternId,
    /// Range of the definition in the schema source (for navigation).
    pub range: TextRange,
}

/// A loaded schema: pattern arena, named definitions, and the start
/// pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    patterns: Vec<Pattern>,
    /// Source range per pattern, parallel to `patterns`.
    ranges: Vec<TextRange>,
    defines: Vec<Define>,
    define_index: HashMap<String, DefineId>,
    start: PatternId,
}

impl Schema {
    pub(crate) fn new() -> Self {
        // Slot 0 is a fixed NotAllowed pattern used as the fallback start
        // for schemas that fail to load.
        Self {
            patterns: vec![Pattern::NotAllowed],
            ranges: vec![TextRange::default()],
            defines: Vec::new(),
            define_index: HashMap::new(),
            start: PatternId(0),
        }
    }

    pub(crate) fn alloc(&mut self, pattern: Pattern, range: TextRange) -> PatternId {
        let id = PatternId(u32::try_from(self.patterns.len()).expect("too many patterns"));
        self.patterns.push(pattern);
        self.ranges.push(range);
        id
    }

    pub(crate) fn set_start(&mut self, start: PatternId) {
        self.start = start;
    }

    pub(crate) fn add_define(&mut self, name: &str, range: TextRange) -> DefineId {
        if let Some(&id) = self.define_index.get(name) {
            return id;
        }
        let id = DefineId(u32::try_from(self.defines.len()).expect("too many defines"));
        self.defines.push(Define {
            name: name.to_string(),
            // Placeholder until the body is parsed; stays Empty if the
            // schema never provides one.
            pattern: PatternId(0),
            range,
        });
        self.define_index.insert(name.to_string(), id);
        id
    }

    pub(crate) fn set_define_pattern(&mut self, id: DefineId, pattern: PatternId) {
        self.defines[id.index()].pattern = pattern;
    }

    /// The start pattern (entry point of the schema).
    pub fn start(&self) -> PatternId {
        self.start
    }

    pub fn pattern(&self, id: PatternId) -> &Pattern {
        &self.patterns[id.index()]
    }

    /// Source range of a pattern in the schema document.
    pub fn range(&self, id: PatternId) -> TextRange {
        self.ranges[id.index()]
    }

    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }

    /// All pattern ids in the arena (for schema-wide scans, e.g.
    /// collecting annotated symbol sites).
    pub fn pattern_ids(&self) -> impl Iterator<Item = PatternId> {
        (0..self.patterns.len() as u32).map(PatternId)
    }

    pub fn define(&self, id: DefineId) -> &Define {
        &self.defines[id.index()]
    }

    pub fn define_by_name(&self, name: &str) -> Option<DefineId> {
        self.define_index.get(name).copied()
    }

    pub fn defines(&self) -> impl Iterator<Item = (DefineId, &Define)> {
        self.defines
            .iter()
            .enumerate()
            .map(|(i, d)| (DefineId(i as u32), d))
    }

    /// Follows `Ref` chains until a non-reference pattern is reached.
    /// Guards against reference cycles by giving up after as many steps as
    /// there are defines.
    pub fn resolve(&self, mut id: PatternId) -> PatternId {
        let mut budget = self.defines.len() + 1;
        while let Pattern::Ref(define) = self.pattern(id) {
            if budget == 0 {
                return id;
            }
            budget -= 1;
            id = self.define(*define).pattern;
        }
        id
    }
}
