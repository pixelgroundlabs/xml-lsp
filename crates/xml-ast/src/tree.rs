//! The syntax tree: an arena of nodes with parent/child relations.
//!
//! Text contents are not copied; every node knows its byte range in the
//! source text, so contents can be sliced from the original text on demand.
//! Only attribute values are materialized because they are needed all the
//! time for namespace resolution and upcoming features.

use crate::qname::QName;
use crate::text::TextRange;

/// Handle to a node in the arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u32);

impl NodeId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// An attribute of an element, including position information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub name: QName,
    pub name_range: TextRange,
    /// The raw value without quotes (entities not resolved).
    pub value: String,
    /// Range of the value without quotes.
    pub value_range: TextRange,
}

/// Element data. The node range spans from the start tag through the end
/// tag; `open_tag`/`close_tag` denote the tags themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub name: QName,
    /// Range of the name inside the start tag.
    pub name_range: TextRange,
    pub attributes: Vec<Attribute>,
    /// Range of the complete start tag `<name …>`.
    pub open_tag: TextRange,
    /// Range of the end tag `</name>`; `None` for self-closing or unclosed
    /// elements.
    pub close_tag: Option<TextRange>,
    pub self_closing: bool,
    /// Namespaces declared in this start tag (prefix → URI).
    /// `None` as prefix stands for the default namespace.
    pub ns_decls: Vec<(Option<String>, Option<crate::qname::NsUri>)>,
}

impl Element {
    pub fn attribute(&self, namespace: Option<&str>, local: &str) -> Option<&Attribute> {
        self.attributes
            .iter()
            .find(|a| a.name.matches(namespace, local))
    }

    pub fn attribute_by_written_name(&self, written: &str) -> Option<&Attribute> {
        self.attributes
            .iter()
            .find(|a| a.name.as_written() == written)
    }
}

/// The different node kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    /// Synthetic root node of the tree (not the document's root element!).
    Root,
    Element(Element),
    /// Raw text; the content is given by the node range in the source text.
    Text,
    /// `<![CDATA[…]]>`, range including the brackets.
    CData,
    /// `<!-- … -->`, range including the brackets.
    Comment,
    /// `<?target …?>`.
    ProcessingInstruction {
        target: String,
    },
    /// `<!DOCTYPE …>`.
    Doctype,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NodeData {
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    range: TextRange,
    kind: NodeKind,
}

/// A parsed XML document as an arena tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    nodes: Vec<NodeData>,
}

impl Document {
    pub fn new() -> Self {
        Self {
            nodes: vec![NodeData {
                parent: None,
                children: Vec::new(),
                range: TextRange::default(),
                kind: NodeKind::Root,
            }],
        }
    }

    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    /// Inserts a new node as the last child of `parent`.
    pub fn push(&mut self, parent: NodeId, kind: NodeKind, range: TextRange) -> NodeId {
        let id = NodeId(u32::try_from(self.nodes.len()).expect("too many nodes"));
        self.nodes.push(NodeData {
            parent: Some(parent),
            children: Vec::new(),
            range,
            kind,
        });
        self.nodes[parent.index()].children.push(id);
        id
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn kind(&self, id: NodeId) -> &NodeKind {
        &self.nodes[id.index()].kind
    }

    pub fn kind_mut(&mut self, id: NodeId) -> &mut NodeKind {
        &mut self.nodes[id.index()].kind
    }

    pub fn range(&self, id: NodeId) -> TextRange {
        self.nodes[id.index()].range
    }

    pub fn set_range(&mut self, id: NodeId, range: TextRange) {
        self.nodes[id.index()].range = range;
    }

    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.index()].parent
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id.index()].children
    }

    pub fn element(&self, id: NodeId) -> Option<&Element> {
        match &self.nodes[id.index()].kind {
            NodeKind::Element(element) => Some(element),
            _ => None,
        }
    }

    pub fn element_mut(&mut self, id: NodeId) -> Option<&mut Element> {
        match &mut self.nodes[id.index()].kind {
            NodeKind::Element(element) => Some(element),
            _ => None,
        }
    }

    /// The first top-level element (the document's root element).
    pub fn root_element(&self) -> Option<NodeId> {
        self.children(self.root())
            .iter()
            .copied()
            .find(|&id| matches!(self.kind(id), NodeKind::Element(_)))
    }

    /// All top-level elements (documents that are not well-formed can have
    /// more than one).
    pub fn top_level_elements(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.children(self.root())
            .iter()
            .copied()
            .filter(|&id| matches!(self.kind(id), NodeKind::Element(_)))
    }

    /// The deepest node whose range contains the offset; falls back to the
    /// synthetic root node.
    pub fn node_at_offset(&self, offset: usize) -> NodeId {
        let mut current = self.root();
        'descend: loop {
            for &child in self.children(current) {
                if self.range(child).contains(offset) {
                    current = child;
                    continue 'descend;
                }
            }
            return current;
        }
    }

    /// All nodes below `id` (preorder), `id` itself first.
    pub fn descendants(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let mut stack = vec![id];
        std::iter::from_fn(move || {
            let next = stack.pop()?;
            stack.extend(self.children(next).iter().rev().copied());
            Some(next)
        })
    }
}

impl Default for Document {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::qname::QName;

    fn element(name: &str) -> NodeKind {
        NodeKind::Element(Element {
            name: QName::new(None, name, None),
            name_range: TextRange::default(),
            attributes: Vec::new(),
            open_tag: TextRange::default(),
            close_tag: None,
            self_closing: false,
            ns_decls: Vec::new(),
        })
    }

    #[test]
    fn tree_structure() {
        let mut doc = Document::new();
        let a = doc.push(doc.root(), element("a"), TextRange::new(0, 20));
        let b = doc.push(a, element("b"), TextRange::new(3, 10));
        let text = doc.push(b, NodeKind::Text, TextRange::new(6, 7));

        assert_eq!(doc.parent(text), Some(b));
        assert_eq!(doc.children(a), &[b]);
        assert_eq!(doc.root_element(), Some(a));
        assert_eq!(doc.descendants(doc.root()).count(), 4);
    }

    #[test]
    fn node_at_offset_descends() {
        let mut doc = Document::new();
        let a = doc.push(doc.root(), element("a"), TextRange::new(0, 20));
        let b = doc.push(a, element("b"), TextRange::new(3, 10));
        doc.push(a, element("c"), TextRange::new(10, 17));

        assert_eq!(doc.node_at_offset(5), b);
        assert_eq!(doc.node_at_offset(1), a);
        assert_eq!(doc.node_at_offset(50), doc.root());
    }
}
