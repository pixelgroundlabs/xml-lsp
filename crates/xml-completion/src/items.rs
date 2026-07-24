//! Building completion items from schema expectations.

use xml_ast::{Document, NodeId, NodeKind, NsUri, TextRange};
use xml_schema::{NameClass, Schema};
use xml_validation::Validator;

use crate::context::{cursor_context, CursorContext};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompletionItemKind {
    Element,
    Attribute,
    Value,
    CloseTag,
}

/// An editor-agnostic completion item. `insert` is plain text; `snippet`
/// is the tab-stop variant for clients that support it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    pub label: String,
    pub kind: CompletionItemKind,
    pub documentation: Option<String>,
    /// The text range the item replaces (the typed fragment).
    pub replace: TextRange,
    pub insert: String,
    pub snippet: Option<String>,
    /// Required attributes sort before optional ones.
    pub required: bool,
}

impl CompletionItem {
    fn new(label: String, kind: CompletionItemKind, replace: TextRange) -> Self {
        Self {
            insert: label.clone(),
            label,
            kind,
            documentation: None,
            replace,
            snippet: None,
            required: false,
        }
    }
}

/// Computes completion items at a byte offset.
pub fn complete(schema: &Schema, doc: &Document, text: &str, offset: usize) -> Vec<CompletionItem> {
    let mut validator = Validator::new(schema);
    match cursor_context(doc, text, offset) {
        CursorContext::ElementName {
            container,
            replace,
            fresh,
        } => {
            let expectations = validator.content_expectations(doc, text, container, offset);
            let mut items = Vec::new();
            for pattern in expectations.elements {
                let Some(element) = schema.element_pattern(pattern) else {
                    continue;
                };
                for (ns, local) in concrete_names(&element.name) {
                    let label = qualified_label(doc, container, ns.as_deref(), &local);
                    let mut item =
                        CompletionItem::new(label.clone(), CompletionItemKind::Element, replace);
                    item.documentation = element.documentation.clone();
                    if fresh {
                        let (insert, snippet) = element_skeleton(schema, pattern, &label, text);
                        item.insert = insert;
                        item.snippet = snippet;
                    }
                    items.push(item);
                }
            }
            dedupe(items)
        }

        CursorContext::CloseTag { element, replace } => {
            let Some(el) = doc.element(element) else {
                return Vec::new();
            };
            let name = el.name.as_written();
            let mut item = CompletionItem::new(name.clone(), CompletionItemKind::CloseTag, replace);
            item.insert = format!("{name}>");
            vec![item]
        }

        CursorContext::AttributeName {
            element,
            attr_index,
            replace,
        } => {
            let expectations = validator.attribute_expectations(doc, text, element, attr_index);
            let mut items = Vec::new();
            for pattern in expectations.attributes {
                let Some(attribute) = schema.attribute_pattern(pattern) else {
                    continue;
                };
                for (_, local) in concrete_names(&attribute.name) {
                    let mut item =
                        CompletionItem::new(local.clone(), CompletionItemKind::Attribute, replace);
                    item.documentation = attribute.documentation.clone();
                    item.insert = format!("{local}=\"\"");
                    item.snippet = Some(format!("{local}=\"$1\""));
                    item.required = expectations.required_attributes.contains(&pattern);
                    items.push(item);
                }
            }
            dedupe(items)
        }

        CursorContext::AttributeValue {
            element,
            attr_index,
            replace,
        } => {
            let Some(pattern) = validator.attribute_pattern_at(doc, text, element, attr_index)
            else {
                return Vec::new();
            };
            let Some(attribute) = schema.attribute_pattern(pattern) else {
                return Vec::new();
            };
            let mut items = Vec::new();
            for value_id in schema.value_enum(attribute.content) {
                let Some(value) = schema.value_pattern(value_id) else {
                    continue;
                };
                let rendered = value.value.trim().to_string();
                if rendered.is_empty() {
                    continue;
                }
                let mut item = CompletionItem::new(rendered, CompletionItemKind::Value, replace);
                item.documentation = value.documentation.clone();
                items.push(item);
            }
            dedupe(items)
        }

        CursorContext::None => Vec::new(),
    }
}

/// Builds the full-tag insertion for a freshly typed element: required
/// attributes pre-filled as tab stops, then either the end tag when the
/// element takes content, or a self-closing tag. Content elements get
/// an extra tab stop inside the start tag when the element has
/// attributes: the cursor starts before the `>` for further attributes,
/// Tab moves on into the content. For elements that can hold text the
/// content stop is inline (`Foo$1>$0</Foo>`); element-only content
/// expands as an indented block (`Foo$1>\n␣␣$0\n</Foo>`) — the snippet
/// carries only relative indentation (one document-derived step),
/// editors prepend the insertion line's own indentation to snippet
/// lines. With optional attributes on an empty element the final
/// cursor sits before the `/>`, otherwise after the tag. Returns
/// `(plain insert, snippet)`; the snippet is `None` when it would
/// contain no tab stops.
fn element_skeleton(
    schema: &Schema,
    pattern: xml_schema::PatternId,
    label: &str,
    text: &str,
) -> (String, Option<String>) {
    let Some(element) = schema.element_pattern(pattern) else {
        return (label.to_string(), None);
    };
    let required = schema.required_attributes(element.content);
    let required_names: Vec<String> = required
        .iter()
        .filter_map(|&id| schema.attribute_pattern(id))
        .filter_map(|attr| concrete_names(&attr.name).into_iter().next())
        .map(|(_, local)| local)
        .collect();
    let attribute_count = schema.attributes(element.content).len();
    let has_optional_attributes = attribute_count > required_names.len();
    let has_content = schema.allows_content(element.content);

    let mut insert = label.to_string();
    let mut snippet = label.to_string();
    let mut tab_stops = false;
    let mut tab = 0;
    for name in &required_names {
        tab += 1;
        insert.push_str(&format!(" {name}=\"\""));
        snippet.push_str(&format!(" {name}=\"${{{tab}}}\""));
        tab_stops = true;
    }
    if has_content {
        insert.push_str(&format!("></{label}>"));
        if attribute_count > 0 {
            // A stop inside the start tag for (further) attributes.
            tab += 1;
            snippet.push_str(&format!("${{{tab}}}"));
        }
        if schema.allows_text(element.content) {
            snippet.push_str(&format!(">$0</{label}>"));
        } else {
            let unit = infer_indent_unit(text);
            snippet.push_str(&format!(">\n{unit}$0\n</{label}>"));
        }
        tab_stops = true;
    } else if has_optional_attributes {
        insert.push_str(" />");
        snippet.push_str(" $0/>");
        tab_stops = true;
    } else {
        insert.push_str(" />");
        snippet.push_str(" />");
    }
    (insert, tab_stops.then_some(snippet))
}

/// The document's indentation step: the shortest non-empty leading
/// whitespace of any line; two spaces when nothing is indented yet.
fn infer_indent_unit(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| &line[..line.len() - line.trim_start().len()])
        .filter(|ws| !ws.is_empty())
        .min_by_key(|ws| ws.len())
        .unwrap_or("  ")
        .to_string()
}

/// Expands a name class into its concrete names; wildcards cannot be
/// completed and yield nothing.
fn concrete_names(name: &NameClass) -> Vec<(Option<NsUri>, String)> {
    match name {
        NameClass::Name { ns, local } => vec![(ns.clone(), local.clone())],
        NameClass::Choice(choices) => choices.iter().flat_map(concrete_names).collect(),
        NameClass::NsName { .. } | NameClass::AnyName { .. } => Vec::new(),
    }
}

/// Renders an element name as it should be written at this position:
/// plain if the namespace is the default one in scope, prefixed if a
/// prefix for it is declared.
fn qualified_label(
    doc: &Document,
    context: Option<NodeId>,
    ns: Option<&str>,
    local: &str,
) -> String {
    let Some(ns) = ns else {
        return local.to_string();
    };
    // Walk the ancestor chain; innermost declarations win.
    let mut node = context;
    while let Some(current) = node {
        if let NodeKind::Element(element) = doc.kind(current) {
            for (prefix, uri) in &element.ns_decls {
                if uri.as_deref() == Some(ns) {
                    return match prefix {
                        Some(prefix) => format!("{prefix}:{local}"),
                        None => local.to_string(),
                    };
                }
            }
        }
        node = doc.parent(current);
    }
    local.to_string()
}

fn dedupe(items: Vec<CompletionItem>) -> Vec<CompletionItem> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for item in items {
        if seen.insert(item.label.clone()) {
            out.push(item);
        }
    }
    out
}
