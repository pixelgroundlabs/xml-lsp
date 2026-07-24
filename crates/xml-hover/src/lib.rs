//! `xml-hover`: schema documentation for the symbol under the cursor,
//! independent of LSP types.
//!
//! Hover targets are element names (start and end tag), attribute names,
//! and attribute values. The content comes from the schema's
//! `a:documentation` annotations; attribute hovers additionally list the
//! allowed values.

use xml_ast::{Document, NodeId, NodeKind, TextRange};
use xml_schema::{PatternId, Schema};
use xml_validation::Validator;

/// A hover result: Markdown content and the range of the hovered token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HoverInfo {
    pub markdown: String,
    pub range: TextRange,
}

/// What the cursor points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// Element name in the start or end tag.
    Element { node: NodeId, range: TextRange },
    AttributeName {
        node: NodeId,
        index: usize,
        range: TextRange,
    },
    AttributeValue {
        node: NodeId,
        index: usize,
        range: TextRange,
    },
}

/// Computes hover information at a byte offset.
pub fn hover(schema: &Schema, doc: &Document, text: &str, offset: usize) -> Option<HoverInfo> {
    let target = hover_target(doc, offset)?;
    let mut validator = Validator::new(schema);

    match target {
        Target::Element { node, range } => {
            let pattern = validator.element_pattern_at(doc, text, node)?;
            let element = schema.element_pattern(pattern)?;
            let documentation = element.documentation.as_deref()?;
            let name = doc.element(node)?.name.as_written();
            Some(HoverInfo {
                markdown: format!("**`<{name}>`**\n\n{documentation}"),
                range,
            })
        }

        Target::AttributeName { node, index, range } => {
            let pattern = validator.attribute_pattern_at(doc, text, node, index)?;
            let attribute = schema.attribute_pattern(pattern)?;
            let name = doc.element(node)?.attributes.get(index)?.name.as_written();
            let mut markdown = format!("**`{name}`**");
            if let Some(documentation) = &attribute.documentation {
                markdown.push_str("\n\n");
                markdown.push_str(documentation);
            }
            if let Some(values) = allowed_values(schema, attribute.content) {
                markdown.push_str("\n\n");
                markdown.push_str(&values);
            }
            if attribute.documentation.is_none() && !markdown.contains("Allowed values") {
                return None;
            }
            Some(HoverInfo { markdown, range })
        }

        Target::AttributeValue { node, index, range } => {
            let pattern = validator.attribute_pattern_at(doc, text, node, index)?;
            let attribute = schema.attribute_pattern(pattern)?;
            let attr = doc.element(node)?.attributes.get(index)?.clone();
            let written = xml_ast::unescape(&attr.value);

            // Documentation of the matching enumeration value, if any.
            let value_docs = schema
                .value_enum(attribute.content)
                .into_iter()
                .filter_map(|id| schema.value_pattern(id))
                .find(|v| collapse(&v.value) == collapse(&written))
                .and_then(|v| {
                    v.documentation
                        .as_ref()
                        .map(|d| (v.value.trim().to_string(), d.clone()))
                });
            if let Some((value, documentation)) = value_docs {
                return Some(HoverInfo {
                    markdown: format!("**`{value}`**\n\n{documentation}"),
                    range,
                });
            }

            // Fall back to the attribute documentation.
            let documentation = attribute.documentation.as_deref()?;
            Some(HoverInfo {
                markdown: format!("**`{}`**\n\n{documentation}", attr.name.as_written()),
                range,
            })
        }
    }
}

/// Renders the allowed values of an attribute content pattern.
fn allowed_values(schema: &Schema, content: PatternId) -> Option<String> {
    let values: Vec<String> = schema
        .value_enum(content)
        .into_iter()
        .filter_map(|id| schema.value_pattern(id))
        .map(|v| format!("`{}`", v.value.trim()))
        .filter(|v| v != "``")
        .collect();
    if values.is_empty() {
        return None;
    }
    Some(format!("Allowed values: {}", values.join(", ")))
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Determines the hover target at an offset.
fn hover_target(doc: &Document, offset: usize) -> Option<Target> {
    let mut node = doc.node_at_offset(offset);
    loop {
        match doc.kind(node) {
            NodeKind::Element(element) => {
                // Element name in the start tag.
                if element.name_range.contains_inclusive(offset) {
                    return Some(Target::Element {
                        node,
                        range: element.name_range,
                    });
                }
                // Element name in the end tag (`</name>`).
                if let Some(close_tag) = element.close_tag {
                    let name_start = close_tag.start + 2;
                    let name_range = TextRange::new(
                        name_start,
                        (name_start + element.name.as_written().len()).min(close_tag.end),
                    );
                    if name_range.contains_inclusive(offset) {
                        return Some(Target::Element {
                            node,
                            range: name_range,
                        });
                    }
                }
                // Attributes.
                for (index, attribute) in element.attributes.iter().enumerate() {
                    if attribute.name_range.contains_inclusive(offset) {
                        return Some(Target::AttributeName {
                            node,
                            index,
                            range: attribute.name_range,
                        });
                    }
                    if attribute.value_range.contains_inclusive(offset)
                        && !attribute.value_range.is_empty()
                    {
                        return Some(Target::AttributeValue {
                            node,
                            index,
                            range: attribute.value_range,
                        });
                    }
                }
                return None;
            }
            NodeKind::Root => return None,
            _ => node = doc.parent(node)?,
        }
    }
}
