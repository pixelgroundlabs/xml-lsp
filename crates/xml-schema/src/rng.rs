//! RELAX NG frontend: loads a `.rng` schema (XML syntax) into the internal
//! model.
//!
//! The loader uses our own error-tolerant XML parser, so schema problems
//! come with precise source positions. Supported is the full pattern
//! vocabulary used in practice (element, attribute, choice, group,
//! interleave, optional, zeroOrMore, oneOrMore, list, mixed, ref, value,
//! data with params/except, empty, text, notAllowed, name classes with
//! except, ns and datatypeLibrary inheritance, combine on start/define).
//! `include`, `externalRef`, and `parentRef` are reported as unsupported
//! for now.

use std::collections::HashMap;
use std::sync::Arc;

use xml_ast::{unescape, Document, NodeId, NodeKind, NsUri, QName, TextRange};

use crate::diagnostics::{SchemaDiagnostic, SchemaDiagnosticCode};
use crate::model::{
    AttributePattern, DataPattern, DatatypeName, ElementPattern, FormatHints, NameClass, Pattern,
    PatternId, Schema, SymbolAnnotation, SymbolRole, ValueForm, ValuePattern,
};

/// Namespace of the RELAX NG XML syntax.
pub const RNG_NAMESPACE: &str = "http://relaxng.org/ns/structure/1.0";
/// Namespace of the RELAX NG compatibility annotations (`a:documentation`).
pub const RNG_DOC_NAMESPACE: &str = "http://relaxng.org/ns/compatibility/annotations/1.0";
/// Namespace of xml-lsp's own annotations (`lsp:defines`,
/// `lsp:references`, `lsp:format`) — see docs/schema-annotations.md.
pub const LSP_ANNOTATION_NAMESPACE: &str = "urn:xml-lsp:annotations";

/// Result of loading a schema: the model plus all diagnostics. On fatal
/// problems the schema falls back to a `NotAllowed` start pattern.
#[derive(Debug)]
pub struct LoadResult {
    pub schema: Schema,
    pub diagnostics: Vec<SchemaDiagnostic>,
}

/// Loads a RELAX NG schema (XML syntax) from source text.
pub fn load_rng(text: &str) -> LoadResult {
    let parse = xml_parser::parse(text);
    let mut loader = Loader {
        text,
        doc: &parse.document,
        schema: Schema::new(),
        diags: Vec::new(),
    };

    // Schema files are usually machine-generated; if one is not even
    // well-formed the user should hear about it.
    for d in &parse.diagnostics {
        loader.diags.push(SchemaDiagnostic {
            range: d.range,
            severity: d.severity,
            code: SchemaDiagnosticCode::XmlSyntax,
            message: format!("schema is not well-formed XML: {}", d.message),
        });
    }

    loader.load();
    LoadResult {
        schema: loader.schema,
        diagnostics: loader.diags,
    }
}

/// Inherited attributes (`ns`, `datatypeLibrary`) during the tree walk.
#[derive(Clone)]
struct Ctx {
    ns: Option<NsUri>,
    library: Arc<str>,
}

struct Loader<'a> {
    text: &'a str,
    doc: &'a Document,
    schema: Schema,
    diags: Vec<SchemaDiagnostic>,
}

impl<'a> Loader<'a> {
    fn load(&mut self) {
        let Some(root) = self.doc.root_element() else {
            // The XML parser already reported why there is no root.
            return;
        };
        let ctx = Ctx {
            ns: None,
            library: Arc::from(""),
        };
        let root_el = self.doc.element(root).expect("root is an element");
        if root_el.name.namespace.as_deref() != Some(RNG_NAMESPACE) {
            self.error(
                root_el.name_range,
                SchemaDiagnosticCode::UnexpectedElement,
                format!(
                    "expected a RELAX NG schema (namespace {RNG_NAMESPACE}), found `<{}>`",
                    root_el.name.as_written()
                ),
            );
            return;
        }
        let ctx = self.child_ctx(root, &ctx).0;
        if root_el.name.local == "grammar" {
            self.load_grammar(root, &ctx);
        } else {
            // A schema may also consist of a single pattern as its root.
            let start = self.parse_pattern(root, &ctx);
            self.schema.set_start(start);
        }
    }

    // ------------------------------------------------------------------
    // Grammar level
    // ------------------------------------------------------------------

    fn load_grammar(&mut self, grammar: NodeId, ctx: &Ctx) {
        // First pass: register all define names so forward references
        // resolve to stable ids.
        self.prescan_defines(grammar);

        let mut starts: Vec<(PatternId, Option<String>, TextRange)> = Vec::new();
        let mut bodies: HashMap<String, Vec<(PatternId, Option<String>, TextRange)>> =
            HashMap::new();
        self.load_grammar_content(grammar, ctx, &mut starts, &mut bodies);

        // Merge starts (combine="choice|interleave").
        match self.merge_combinable(starts, "start", SchemaDiagnosticCode::DuplicateStart) {
            Some(start) => self.schema.set_start(start),
            None => {
                let range = self.doc.range(grammar);
                self.error(
                    TextRange::empty(range.start),
                    SchemaDiagnosticCode::MissingStart,
                    "grammar has no <start> pattern",
                );
            }
        }

        // Merge define bodies.
        for (name, list) in bodies {
            let id = self
                .schema
                .define_by_name(&name)
                .expect("define was registered in prescan");
            if let Some(pattern) =
                self.merge_combinable(list, &name, SchemaDiagnosticCode::DuplicateDefine)
            {
                self.schema.set_define_pattern(id, pattern);
            }
        }
    }

    fn prescan_defines(&mut self, node: NodeId) {
        for &child in self.doc.children(node) {
            let Some(element) = self.doc.element(child) else {
                continue;
            };
            if element.name.namespace.as_deref() != Some(RNG_NAMESPACE) {
                continue;
            }
            match element.name.local.as_str() {
                "define" => {
                    if let Some(name) = self.attr_value(child, "name") {
                        self.schema.add_define(&name, self.doc.range(child));
                    }
                }
                "div" => self.prescan_defines(child),
                _ => {}
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn load_grammar_content(
        &mut self,
        node: NodeId,
        ctx: &Ctx,
        starts: &mut Vec<(PatternId, Option<String>, TextRange)>,
        bodies: &mut HashMap<String, Vec<(PatternId, Option<String>, TextRange)>>,
    ) {
        for &child in self.doc.children(node) {
            let Some(element) = self.doc.element(child) else {
                continue;
            };
            if element.name.namespace.as_deref() != Some(RNG_NAMESPACE) {
                continue; // annotations and foreign elements
            }
            let name_range = element.name_range;
            let combine = self.attr_value(child, "combine");
            let ctx = self.child_ctx(child, ctx).0;
            match element.name.local.as_str() {
                "start" => {
                    let pattern = self.parse_content(child, &ctx, ContentDefault::Required);
                    starts.push((pattern, combine, name_range));
                }
                "define" => {
                    let Some(name) = self.attr_value(child, "name") else {
                        self.error(
                            name_range,
                            SchemaDiagnosticCode::MissingName,
                            "<define> is missing a name attribute",
                        );
                        continue;
                    };
                    let pattern = self.parse_content(child, &ctx, ContentDefault::Required);
                    bodies
                        .entry(name)
                        .or_default()
                        .push((pattern, combine, name_range));
                }
                "div" => self.load_grammar_content(child, &ctx, starts, bodies),
                "include" => {
                    self.error(
                        name_range,
                        SchemaDiagnosticCode::UnsupportedFeature,
                        "<include> is not supported yet",
                    );
                }
                other => {
                    self.error(
                        name_range,
                        SchemaDiagnosticCode::UnexpectedElement,
                        format!("`<{other}>` is not allowed directly inside <grammar>"),
                    );
                }
            }
        }
    }

    /// Merges multiple starts or defines of the same name according to
    /// their `combine` attributes.
    fn merge_combinable(
        &mut self,
        mut list: Vec<(PatternId, Option<String>, TextRange)>,
        what: &str,
        code: SchemaDiagnosticCode,
    ) -> Option<PatternId> {
        match list.len() {
            0 => None,
            1 => Some(list.remove(0).0),
            _ => {
                let combine = list
                    .iter()
                    .find_map(|(_, c, _)| c.clone())
                    .unwrap_or_default();
                let without_combine = list.iter().filter(|(_, c, _)| c.is_none()).count();
                let consistent = list
                    .iter()
                    .all(|(_, c, _)| c.as_deref().is_none_or(|c| c == combine));
                if without_combine > 1 || !consistent || combine.is_empty() {
                    let range = list[1].2;
                    self.error(
                        range,
                        code,
                        format!("`{what}` is defined more than once without a valid combine"),
                    );
                    return Some(list.remove(0).0);
                }
                let ids: Vec<PatternId> = list.iter().map(|(p, _, _)| *p).collect();
                let range = list[0].2;
                let merged = match combine.as_str() {
                    "choice" => Pattern::Choice(ids),
                    "interleave" => Pattern::Interleave(ids),
                    other => {
                        self.error(range, code, format!("unknown combine method `{other}`"));
                        return Some(list.remove(0).0);
                    }
                };
                Some(self.schema.alloc(merged, range))
            }
        }
    }

    // ------------------------------------------------------------------
    // Patterns
    // ------------------------------------------------------------------

    /// Parses one RELAX NG pattern element.
    fn parse_pattern(&mut self, node: NodeId, ctx: &Ctx) -> PatternId {
        let (ctx, explicit_ns) = self.child_ctx(node, ctx);
        let element = self.doc.element(node).expect("pattern node is an element");
        let range = self.doc.range(node);
        let name_range = element.name_range;

        match element.name.local.as_str() {
            "empty" => self.schema.alloc(Pattern::Empty, range),
            "text" => self.schema.alloc(Pattern::Text, range),
            "notAllowed" => self.schema.alloc(Pattern::NotAllowed, range),

            "group" | "choice" | "interleave" => {
                let items = self.parse_child_patterns(node, &ctx, None);
                if items.is_empty() {
                    self.error(
                        name_range,
                        SchemaDiagnosticCode::MissingContent,
                        format!("`<{}>` has no content pattern", element.name.local),
                    );
                    return self.schema.alloc(Pattern::Empty, range);
                }
                if items.len() == 1 {
                    return items[0];
                }
                let pattern = match element.name.local.as_str() {
                    "group" => Pattern::Group(items),
                    "choice" => Pattern::Choice(items),
                    _ => Pattern::Interleave(items),
                };
                self.schema.alloc(pattern, range)
            }

            "optional" | "zeroOrMore" | "oneOrMore" | "list" | "mixed" => {
                let inner = self.parse_content(node, &ctx, ContentDefault::Required);
                let pattern = match element.name.local.as_str() {
                    "optional" => Pattern::Optional(inner),
                    "zeroOrMore" => Pattern::ZeroOrMore(inner),
                    "oneOrMore" => Pattern::OneOrMore(inner),
                    "list" => Pattern::List(inner),
                    _ => {
                        // mixed p == interleave(text, p)
                        let text = self.schema.alloc(Pattern::Text, range);
                        Pattern::Interleave(vec![text, inner])
                    }
                };
                self.schema.alloc(pattern, range)
            }

            "element" => {
                let (name, skip_first) = self.parse_name(node, &ctx, ctx.ns.clone());
                let content = match skip_first {
                    NameSource::Missing => self.schema.alloc(Pattern::Empty, range),
                    _ => self.parse_content_after_name(
                        node,
                        &ctx,
                        skip_first,
                        ContentDefault::Required,
                    ),
                };
                let documentation = self.collect_documentation(node);
                let format = self.collect_format_hints(node);
                self.schema.alloc(
                    Pattern::Element(ElementPattern {
                        name,
                        content,
                        documentation,
                        format,
                    }),
                    range,
                )
            }

            "attribute" => {
                // Unprefixed attribute names have no namespace unless an ns
                // attribute is given explicitly (RELAX NG spec, 4.9/4.10).
                let default_ns = if explicit_ns { ctx.ns.clone() } else { None };
                let (name, skip_first) = self.parse_name(node, &ctx, default_ns);
                // An attribute without a content pattern accepts any text.
                let content = match skip_first {
                    NameSource::Missing => self.schema.alloc(Pattern::Text, range),
                    _ => {
                        self.parse_content_after_name(node, &ctx, skip_first, ContentDefault::Text)
                    }
                };
                let documentation = self.collect_documentation(node);
                let symbol = self.collect_symbol_annotation(node);
                self.schema.alloc(
                    Pattern::Attribute(AttributePattern {
                        name,
                        content,
                        documentation,
                        symbol,
                    }),
                    range,
                )
            }

            "ref" => {
                let Some(name) = self.attr_value(node, "name") else {
                    self.error(
                        name_range,
                        SchemaDiagnosticCode::MissingName,
                        "<ref> is missing a name attribute",
                    );
                    return self.schema.alloc(Pattern::Empty, range);
                };
                match self.schema.define_by_name(&name) {
                    Some(define) => self.schema.alloc(Pattern::Ref(define), range),
                    None => {
                        self.error(
                            name_range,
                            SchemaDiagnosticCode::UnknownRef,
                            format!("reference to undefined pattern `{name}`"),
                        );
                        self.schema.alloc(Pattern::Empty, range)
                    }
                }
            }

            "value" => {
                let datatype = self.attr_value(node, "type").map(|name| DatatypeName {
                    library: ctx.library.to_string(),
                    name,
                });
                let value = self.text_content(node);
                let documentation = self.collect_documentation(node);
                self.schema.alloc(
                    Pattern::Value(ValuePattern {
                        value,
                        datatype,
                        documentation,
                    }),
                    range,
                )
            }

            "data" => {
                let Some(type_name) = self.attr_value(node, "type") else {
                    self.error(
                        name_range,
                        SchemaDiagnosticCode::MissingType,
                        "<data> is missing a type attribute",
                    );
                    return self.schema.alloc(Pattern::Empty, range);
                };
                let mut params = Vec::new();
                let mut except = None;
                for &child in self.doc.children(node) {
                    let Some(el) = self.doc.element(child) else {
                        continue;
                    };
                    if el.name.namespace.as_deref() != Some(RNG_NAMESPACE) {
                        continue;
                    }
                    match el.name.local.as_str() {
                        "param" => {
                            let name = self.attr_value(child, "name").unwrap_or_else(|| {
                                self.error(
                                    el.name_range,
                                    SchemaDiagnosticCode::MissingName,
                                    "<param> is missing a name attribute",
                                );
                                String::new()
                            });
                            let value = self.text_content(child);
                            params.push((name, value));
                        }
                        "except" => {
                            except =
                                Some(self.parse_content(child, &ctx, ContentDefault::Required));
                        }
                        other => self.error(
                            el.name_range,
                            SchemaDiagnosticCode::UnexpectedElement,
                            format!("`<{other}>` is not allowed inside <data>"),
                        ),
                    }
                }
                self.schema.alloc(
                    Pattern::Data(DataPattern {
                        datatype: DatatypeName {
                            library: ctx.library.to_string(),
                            name: type_name,
                        },
                        params,
                        except,
                    }),
                    range,
                )
            }

            "externalRef" | "parentRef" | "include" | "grammar" => {
                // grammar-in-pattern is legal RELAX NG but unused in
                // practice for our targets; report it honestly instead of
                // mis-parsing.
                self.error(
                    name_range,
                    SchemaDiagnosticCode::UnsupportedFeature,
                    format!("`<{}>` is not supported yet", element.name.local),
                );
                self.schema.alloc(Pattern::Empty, range)
            }

            other => {
                self.error(
                    name_range,
                    SchemaDiagnosticCode::UnexpectedElement,
                    format!("unknown RELAX NG pattern `<{other}>`"),
                );
                self.schema.alloc(Pattern::Empty, range)
            }
        }
    }

    /// Parses all RNG pattern children of `node` (optionally skipping the
    /// first one, which was consumed as a name class).
    fn parse_child_patterns(
        &mut self,
        node: NodeId,
        ctx: &Ctx,
        skip: Option<NodeId>,
    ) -> Vec<PatternId> {
        let mut items = Vec::new();
        for &child in self.doc.children(node) {
            if Some(child) == skip {
                continue;
            }
            let Some(el) = self.doc.element(child) else {
                continue; // text/comments between patterns
            };
            if el.name.namespace.as_deref() != Some(RNG_NAMESPACE) {
                continue; // a:documentation, schematron rules, …
            }
            items.push(self.parse_pattern(child, ctx));
        }
        items
    }

    /// Parses the content of a container node into a single pattern,
    /// wrapping multiple children in an implicit group.
    fn parse_content(&mut self, node: NodeId, ctx: &Ctx, default: ContentDefault) -> PatternId {
        self.parse_content_after_name(node, ctx, NameSource::Attribute, default)
    }

    fn parse_content_after_name(
        &mut self,
        node: NodeId,
        ctx: &Ctx,
        name_source: NameSource,
        default: ContentDefault,
    ) -> PatternId {
        let skip = match name_source {
            NameSource::Child(id) => Some(id),
            _ => None,
        };
        let items = self.parse_child_patterns(node, ctx, skip);
        let range = self.doc.range(node);
        match items.len() {
            0 => match default {
                ContentDefault::Text => self.schema.alloc(Pattern::Text, range),
                ContentDefault::Required => {
                    let element = self.doc.element(node).expect("node is an element");
                    self.warning(
                        element.name_range,
                        SchemaDiagnosticCode::MissingContent,
                        format!("`<{}>` has no content pattern", element.name.local),
                    );
                    self.schema.alloc(Pattern::Empty, range)
                }
            },
            1 => items[0],
            _ => self.schema.alloc(Pattern::Group(items), range),
        }
    }

    // ------------------------------------------------------------------
    // Name classes
    // ------------------------------------------------------------------

    /// Determines the name class of an element/attribute pattern: either
    /// from the `name` attribute or from the first RNG child element.
    fn parse_name(
        &mut self,
        node: NodeId,
        ctx: &Ctx,
        default_ns: Option<NsUri>,
    ) -> (NameClass, NameSource) {
        let element = self.doc.element(node).expect("node is an element");
        if let Some(raw) = self.attr_value(node, "name") {
            let name = self.name_from_qname(node, &raw, default_ns, element.name_range);
            return (name, NameSource::Attribute);
        }
        // No name attribute: the first RNG child must be a name class.
        let first = self.doc.children(node).iter().copied().find(|&c| {
            self.doc
                .element(c)
                .is_some_and(|el| el.name.namespace.as_deref() == Some(RNG_NAMESPACE))
        });
        match first {
            Some(child)
                if matches!(
                    self.doc.element(child).unwrap().name.local.as_str(),
                    "name" | "anyName" | "nsName" | "choice"
                ) =>
            {
                let name = self.parse_name_class(child, ctx, default_ns);
                (name, NameSource::Child(child))
            }
            _ => {
                self.error(
                    element.name_range,
                    SchemaDiagnosticCode::MissingName,
                    format!(
                        "`<{}>` needs a name attribute or a name class child",
                        element.name.local
                    ),
                );
                (NameClass::AnyName { except: None }, NameSource::Missing)
            }
        }
    }

    fn parse_name_class(
        &mut self,
        node: NodeId,
        ctx: &Ctx,
        default_ns: Option<NsUri>,
    ) -> NameClass {
        let (ctx, explicit_ns) = self.child_ctx(node, ctx);
        // An explicit ns attribute on the name class overrides the default
        // (this is how attribute names get a namespace after all).
        let default_ns = if explicit_ns {
            ctx.ns.clone()
        } else {
            default_ns
        };
        let element = self.doc.element(node).expect("name class is an element");

        match element.name.local.as_str() {
            "name" => {
                let raw = self.text_content(node);
                self.name_from_qname(node, raw.trim(), default_ns, element.name_range)
            }
            "anyName" => NameClass::AnyName {
                except: self.parse_name_class_except(node, &ctx, default_ns.clone()),
            },
            "nsName" => NameClass::NsName {
                ns: default_ns.clone(),
                except: self.parse_name_class_except(node, &ctx, default_ns),
            },
            "choice" => {
                let mut choices = Vec::new();
                for &child in self.doc.children(node) {
                    let Some(el) = self.doc.element(child) else {
                        continue;
                    };
                    if el.name.namespace.as_deref() != Some(RNG_NAMESPACE) {
                        continue;
                    }
                    choices.push(self.parse_name_class(child, &ctx, default_ns.clone()));
                }
                if choices.is_empty() {
                    self.error(
                        element.name_range,
                        SchemaDiagnosticCode::MissingContent,
                        "name class <choice> has no alternatives",
                    );
                    NameClass::AnyName { except: None }
                } else {
                    NameClass::Choice(choices)
                }
            }
            other => {
                self.error(
                    element.name_range,
                    SchemaDiagnosticCode::UnexpectedElement,
                    format!("`<{other}>` is not a name class"),
                );
                NameClass::AnyName { except: None }
            }
        }
    }

    /// Parses the optional `<except>` child of anyName/nsName.
    fn parse_name_class_except(
        &mut self,
        node: NodeId,
        ctx: &Ctx,
        default_ns: Option<NsUri>,
    ) -> Option<Box<NameClass>> {
        let except = self.doc.children(node).iter().copied().find(|&c| {
            self.doc.element(c).is_some_and(|el| {
                el.name.namespace.as_deref() == Some(RNG_NAMESPACE) && el.name.local == "except"
            })
        })?;
        let mut classes = Vec::new();
        for &child in self.doc.children(except) {
            let Some(el) = self.doc.element(child) else {
                continue;
            };
            if el.name.namespace.as_deref() != Some(RNG_NAMESPACE) {
                continue;
            }
            classes.push(self.parse_name_class(child, ctx, default_ns.clone()));
        }
        match classes.len() {
            0 => None,
            1 => Some(Box::new(classes.remove(0))),
            _ => Some(Box::new(NameClass::Choice(classes))),
        }
    }

    fn name_from_qname(
        &mut self,
        node: NodeId,
        raw: &str,
        default_ns: Option<NsUri>,
        range: TextRange,
    ) -> NameClass {
        match QName::split(raw) {
            (Some(prefix), local) => {
                let ns = self.resolve_schema_prefix(node, prefix, range);
                NameClass::Name {
                    ns,
                    local: local.to_string(),
                }
            }
            (None, local) => NameClass::Name {
                ns: default_ns,
                local: local.to_string(),
            },
        }
    }

    /// Resolves a QName prefix against the xmlns declarations of the
    /// schema document itself.
    fn resolve_schema_prefix(
        &mut self,
        node: NodeId,
        prefix: &str,
        range: TextRange,
    ) -> Option<NsUri> {
        // The xml prefix is bound implicitly (e.g. name="xml:lang").
        if prefix == "xml" {
            return Some(Arc::from(xml_ast::XML_NAMESPACE));
        }
        let mut current = Some(node);
        while let Some(id) = current {
            if let Some(element) = self.doc.element(id) {
                for (decl_prefix, uri) in &element.ns_decls {
                    if decl_prefix.as_deref() == Some(prefix) {
                        return uri.clone();
                    }
                }
            }
            current = self.doc.parent(id);
        }
        self.error(
            range,
            SchemaDiagnosticCode::UnknownPrefix,
            format!("prefix `{prefix}` is not declared in the schema"),
        );
        None
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    /// Applies `ns` and `datatypeLibrary` attributes of `node` to the
    /// inherited context. The bool reports whether `ns` was explicit.
    fn child_ctx(&self, node: NodeId, ctx: &Ctx) -> (Ctx, bool) {
        let mut next = ctx.clone();
        let mut explicit_ns = false;
        if let Some(ns) = self.attr_value(node, "ns") {
            next.ns = if ns.is_empty() {
                None
            } else {
                Some(Arc::from(ns.as_str()))
            };
            explicit_ns = true;
        }
        if let Some(library) = self.attr_value(node, "datatypeLibrary") {
            next.library = Arc::from(library.as_str());
        }
        (next, explicit_ns)
    }

    /// The unescaped value of an unprefixed attribute.
    fn attr_value(&self, node: NodeId, name: &str) -> Option<String> {
        let element = self.doc.element(node)?;
        let attribute = element.attribute(None, name)?;
        Some(unescape(&attribute.value).into_owned())
    }

    /// The unescaped text content of a node (text and CDATA children).
    fn text_content(&self, node: NodeId) -> String {
        let mut out = String::new();
        for &child in self.doc.children(node) {
            let range = self.doc.range(child);
            match self.doc.kind(child) {
                NodeKind::Text => {
                    out.push_str(&unescape(&self.text[range.start..range.end]));
                }
                NodeKind::CData => {
                    // Strip `<![CDATA[` and `]]>`.
                    let inner_start = (range.start + 9).min(range.end);
                    let inner_end = range.end.saturating_sub(3).max(inner_start);
                    out.push_str(&self.text[inner_start..inner_end]);
                }
                _ => {}
            }
        }
        out
    }

    /// Collects `a:documentation` children into one documentation string.
    fn collect_documentation(&self, node: NodeId) -> Option<String> {
        let mut parts = Vec::new();
        for &child in self.doc.children(node) {
            let Some(el) = self.doc.element(child) else {
                continue;
            };
            if el.name.namespace.as_deref() == Some(RNG_DOC_NAMESPACE)
                && el.name.local == "documentation"
            {
                let text = self.text_content(child);
                let text = text.trim();
                if !text.is_empty() {
                    parts.push(text.to_string());
                }
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        }
    }

    /// Collects an `lsp:defines`/`lsp:references` annotation from the
    /// children of an attribute pattern. The first one wins; malformed
    /// annotations are reported and skipped.
    fn collect_symbol_annotation(&mut self, node: NodeId) -> Option<SymbolAnnotation> {
        let mut result = None;
        for &child in self.doc.children(node) {
            let Some(el) = self.doc.element(child) else {
                continue;
            };
            if el.name.namespace.as_deref() != Some(LSP_ANNOTATION_NAMESPACE) {
                continue;
            }
            let range = self.doc.range(child);
            let role = match el.name.local.as_str() {
                "defines" => SymbolRole::Definition,
                "references" => SymbolRole::Reference,
                "format" => {
                    self.warning(
                        range,
                        SchemaDiagnosticCode::Annotation,
                        "lsp:format belongs on an element pattern, not an attribute",
                    );
                    continue;
                }
                other => {
                    self.warning(
                        range,
                        SchemaDiagnosticCode::Annotation,
                        format!("unknown annotation lsp:{other}"),
                    );
                    continue;
                }
            };
            let Some(kind) = el
                .attribute(None, "symbol")
                .map(|attr| attr.value.trim().to_string())
                .filter(|kind| !kind.is_empty())
            else {
                self.warning(
                    range,
                    SchemaDiagnosticCode::Annotation,
                    format!("lsp:{} needs a symbol attribute", el.name.local),
                );
                continue;
            };
            let form = match el.attribute(None, "form").map(|attr| attr.value.trim()) {
                None | Some("literal") => ValueForm::Literal,
                Some("xpath-string") => ValueForm::XPathString,
                Some("xpath-variable") => ValueForm::XPathVariable,
                Some(other) => {
                    self.warning(
                        range,
                        SchemaDiagnosticCode::Annotation,
                        format!("unknown annotation form `{other}` (expected literal, xpath-string, or xpath-variable)"),
                    );
                    ValueForm::Literal
                }
            };
            if result.is_none() {
                result = Some(SymbolAnnotation { role, kind, form });
            } else {
                self.warning(
                    range,
                    SchemaDiagnosticCode::Annotation,
                    "more than one symbol annotation on this attribute; the first one wins",
                );
            }
        }
        result
    }

    /// Collects an `lsp:format` annotation from the children of an
    /// element pattern.
    fn collect_format_hints(&mut self, node: NodeId) -> Option<FormatHints> {
        let mut result: Option<FormatHints> = None;
        for &child in self.doc.children(node) {
            let Some(el) = self.doc.element(child) else {
                continue;
            };
            if el.name.namespace.as_deref() != Some(LSP_ANNOTATION_NAMESPACE)
                || el.name.local != "format"
            {
                continue;
            }
            let truthy = |name: &str| {
                el.attribute(None, name)
                    .is_some_and(|attr| attr.value.trim() == "true")
            };
            let hints = FormatHints {
                preserve: truthy("preserve"),
                blank_lines: truthy("blank-lines"),
                inline: truthy("inline"),
            };
            if hints == FormatHints::default() {
                self.warning(
                    self.doc.range(child),
                    SchemaDiagnosticCode::Annotation,
                    "lsp:format without any hint (preserve, blank-lines, inline)",
                );
                continue;
            }
            result = Some(result.map_or(hints, |prev| FormatHints {
                preserve: prev.preserve || hints.preserve,
                blank_lines: prev.blank_lines || hints.blank_lines,
                inline: prev.inline || hints.inline,
            }));
        }
        result
    }

    fn error(&mut self, range: TextRange, code: SchemaDiagnosticCode, message: impl Into<String>) {
        self.diags
            .push(SchemaDiagnostic::error(range, code, message));
    }

    fn warning(
        &mut self,
        range: TextRange,
        code: SchemaDiagnosticCode,
        message: impl Into<String>,
    ) {
        self.diags
            .push(SchemaDiagnostic::warning(range, code, message));
    }
}

/// How the name of an element/attribute pattern was given.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NameSource {
    /// Via the `name` attribute — all children are content.
    Attribute,
    /// Via a name class child, which must be skipped for content parsing.
    Child(NodeId),
    /// Missing entirely (already reported).
    Missing,
}

/// What an empty content model means.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ContentDefault {
    /// Content is required; emit a warning and fall back to Empty.
    Required,
    /// Missing content means "any text" (attributes).
    Text,
}
