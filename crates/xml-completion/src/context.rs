//! Cursor context analysis: what is being typed at this offset?
//!
//! The analysis combines a backwards text scan (robust while the tag
//! being typed still breaks the parse) with the error-tolerant syntax
//! tree (precise for attribute and value positions).

use xml_ast::{Document, NodeId, NodeKind, TextRange};

/// The syntactic position of the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorContext {
    /// Typing an element name after `<`. `container` is the element whose
    /// content the new tag belongs to (`None` at document level).
    /// `fresh` is true when the tag is being typed from scratch (no `>`,
    /// `/>`, or attributes yet) — only then may completion insert a whole
    /// tag skeleton instead of just the name.
    ElementName {
        container: Option<NodeId>,
        replace: TextRange,
        fresh: bool,
    },
    /// Typing a closing tag after `</`; `element` is the innermost open
    /// element.
    CloseTag { element: NodeId, replace: TextRange },
    /// Inside a start tag, at attribute-name position. `attr_index` is
    /// the attribute under the cursor, if any.
    AttributeName {
        element: NodeId,
        attr_index: Option<usize>,
        replace: TextRange,
    },
    /// Inside the quotes of an attribute value.
    AttributeValue {
        element: NodeId,
        attr_index: usize,
        replace: TextRange,
    },
    /// Nothing completable here.
    None,
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')
}

/// End offset of the XML name fragment starting at `offset`.
fn word_end(text: &str, offset: usize) -> usize {
    text[offset..]
        .char_indices()
        .find(|(_, c)| !is_name_char(*c))
        .map(|(idx, _)| offset + idx)
        .unwrap_or(text.len())
}

/// Start offset of the XML name fragment ending at `offset`.
fn word_start(text: &str, offset: usize) -> usize {
    let mut start = offset;
    for (idx, c) in text[..offset].char_indices().rev() {
        if !is_name_char(c) {
            break;
        }
        start = idx;
    }
    start
}

/// Determines the cursor context at a byte offset.
pub fn cursor_context(doc: &Document, text: &str, offset: usize) -> CursorContext {
    let offset = offset.min(text.len());

    // --- Backwards scan: freshly typed `<name` or `</name` -------------
    let start = word_start(text, offset);
    let before = &text[..start];
    if before.ends_with("</") {
        // The `<` position anchors the context lookup.
        let lt = start - 2;
        if let Some(element) = unclosed_element_at(doc, lt) {
            // Cover the rest of the name and a directly following `>`
            // (editors auto-pair `<` with `>`), so the inserted `name>`
            // does not duplicate it.
            let end = word_end(text, offset);
            let end = if text.as_bytes().get(end) == Some(&b'>') {
                end + 1
            } else {
                end
            };
            return CursorContext::CloseTag {
                element,
                replace: TextRange::new(start, end),
            };
        }
        return CursorContext::None;
    }
    if before.ends_with('<') {
        let lt = start - 1;
        // Extend the replace range over the rest of a name the cursor
        // sits in (`<Optio|ns`), so accepting replaces the whole word.
        let end = word_end(text, offset);
        let next_is_gt = text.as_bytes().get(end) == Some(&b'>');
        // Is the cursor inside an *existing* start tag's name? A tag
        // parses as existing when its name is exactly the typed word —
        // except for the bare `<word>` an editor's bracket auto-pairing
        // produces while typing a new tag: no attributes, not
        // self-closing, no end tag. (A broken tag that swallowed
        // following text parses too, but with a garbage name — that is
        // still fresh typing.)
        let existing = {
            let node = doc.node_at_offset(lt);
            match doc.kind(node) {
                NodeKind::Element(el)
                    if el.open_tag.contains(lt) && el.name_range == TextRange::new(start, end) =>
                {
                    // A `<` inside the rest of the start tag means the
                    // broken fresh tag swallowed following markup — a
                    // real start tag cannot contain a raw `<`.
                    let swallowed = text
                        .get(end..el.open_tag.end)
                        .is_some_and(|rest| rest.contains('<'));
                    let bare_autopair = next_is_gt
                        && el.attributes.is_empty()
                        && !el.self_closing
                        && el.close_tag.is_none()
                        && el.open_tag == TextRange::new(lt, end + 1);
                    !swallowed && !bare_autopair
                }
                _ => false,
            }
        };
        // The tag also counts as existing when it visibly continues
        // (`/>` or a `>` further away than the auto-pair position).
        let rest = text[end..].trim_start();
        let continues = !next_is_gt && (rest.starts_with('>') || rest.starts_with('/'));
        let fresh = !existing && !continues;
        // A fresh tag skeleton replaces the auto-paired `>` as well.
        let replace_end = if fresh && next_is_gt { end + 1 } else { end };
        return CursorContext::ElementName {
            container: content_container(doc, lt),
            replace: TextRange::new(start, replace_end),
            fresh,
        };
    }

    // --- Tree-based: inside an existing start tag ----------------------
    let mut node = doc.node_at_offset(offset.saturating_sub(1).max(0));
    loop {
        match doc.kind(node) {
            NodeKind::Element(element) => {
                let element = element.clone();
                if !element.open_tag.contains_inclusive(offset)
                    || offset <= element.name_range.start
                {
                    return CursorContext::None;
                }
                // Value position?
                for (i, attribute) in element.attributes.iter().enumerate() {
                    if attribute.value_range.contains_inclusive(offset)
                        && offset >= attribute.value_range.start
                        && attribute.name_range.end < offset
                    {
                        return CursorContext::AttributeValue {
                            element: node,
                            attr_index: i,
                            replace: attribute.value_range,
                        };
                    }
                }
                // Attribute-name position? (Cursor in the name itself or
                // in whitespace between attributes.)
                if offset <= element.name_range.end {
                    return CursorContext::None; // element name handled above
                }
                for (i, attribute) in element.attributes.iter().enumerate() {
                    if attribute.name_range.contains_inclusive(offset) {
                        return CursorContext::AttributeName {
                            element: node,
                            attr_index: Some(i),
                            replace: attribute.name_range,
                        };
                    }
                }
                // In whitespace inside the tag — but not inside a value
                // and not right at the closing `>`.
                let tag_inner_end = element.open_tag.end.saturating_sub(1);
                let tag_inner_end = if element.self_closing {
                    tag_inner_end.saturating_sub(1)
                } else {
                    tag_inner_end
                };
                if offset <= tag_inner_end && text[..offset].ends_with(|c: char| c.is_whitespace())
                {
                    return CursorContext::AttributeName {
                        element: node,
                        attr_index: None,
                        replace: TextRange::empty(offset),
                    };
                }
                return CursorContext::None;
            }
            NodeKind::Root => return CursorContext::None,
            _ => {
                let Some(parent) = doc.parent(node) else {
                    return CursorContext::None;
                };
                node = parent;
            }
        }
    }
}

/// The element whose *content* contains the position (for a tag being
/// typed at `lt`). A node whose own start tag covers the position is the
/// tag being typed itself — its parent is the container.
fn content_container(doc: &Document, lt: usize) -> Option<NodeId> {
    let mut node = doc.node_at_offset(lt);
    loop {
        match doc.kind(node) {
            NodeKind::Root => return None,
            NodeKind::Element(element) => {
                if element.open_tag.contains(lt) {
                    // The broken tag itself; continue with its parent.
                    node = doc.parent(node)?;
                } else {
                    return Some(node);
                }
            }
            _ => node = doc.parent(node)?,
        }
    }
}

/// The innermost element around the position that still lacks its end
/// tag.
fn unclosed_element_at(doc: &Document, position: usize) -> Option<NodeId> {
    let mut node = doc.node_at_offset(position.saturating_sub(1));
    loop {
        if let NodeKind::Element(element) = doc.kind(node) {
            if element.close_tag.is_none() && !element.self_closing {
                return Some(node);
            }
        }
        if matches!(doc.kind(node), NodeKind::Root) {
            return None;
        }
        node = doc.parent(node)?;
    }
}
