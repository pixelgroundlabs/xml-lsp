//! Semantic tokens: syntax classification plus symbol knowledge — values
//! that reference or define named symbols (colors, text formats, …) are
//! reported as variables so editors can highlight them like identifiers.

use tower_lsp::lsp_types::{
    SemanticToken, SemanticTokenModifier, SemanticTokenType, SemanticTokensLegend,
};
use xml_ast::{Document, LineIndex, NodeKind, TextRange};
use xml_languages::{ReferenceRules, SymbolIndex, SymbolRole};

/// Token types, in legend order. The indices below must match.
const TOKEN_TYPES: [SemanticTokenType; 8] = [
    SemanticTokenType::NAMESPACE,
    SemanticTokenType::TYPE,
    SemanticTokenType::PROPERTY,
    SemanticTokenType::STRING,
    SemanticTokenType::COMMENT,
    SemanticTokenType::MACRO,
    SemanticTokenType::KEYWORD,
    SemanticTokenType::VARIABLE,
];

const NAMESPACE: u32 = 0;
const TYPE: u32 = 1;
const PROPERTY: u32 = 2;
const STRING: u32 = 3;
const COMMENT: u32 = 4;
const MACRO: u32 = 5;
const KEYWORD: u32 = 6;
const VARIABLE: u32 = 7;

/// Modifier bits, in legend order.
const TOKEN_MODIFIERS: [SemanticTokenModifier; 1] = [SemanticTokenModifier::DEFINITION];

const DEFINITION_BIT: u32 = 1;

pub fn legend() -> SemanticTokensLegend {
    SemanticTokensLegend {
        token_types: TOKEN_TYPES.to_vec(),
        token_modifiers: TOKEN_MODIFIERS.to_vec(),
    }
}

/// Computes the full semantic token set of a document, delta-encoded as
/// the LSP wants it. `rules` (from the schema's annotations) highlights
/// symbol-bearing attribute values as variables.
pub fn semantic_tokens(
    doc: &Document,
    text: &str,
    line_index: &LineIndex,
    rules: Option<&ReferenceRules>,
) -> Vec<SemanticToken> {
    let mut raw: Vec<(TextRange, u32, u32)> = Vec::new();

    // Symbol occurrences overlay attribute values.
    let occurrences: Vec<(TextRange, u32)> = rules
        .map(|rules| {
            SymbolIndex::build(rules, doc, text)
                .all()
                .iter()
                .map(|occ| {
                    let modifiers = match occ.role {
                        SymbolRole::Definition => DEFINITION_BIT,
                        SymbolRole::Reference => 0,
                    };
                    (occ.range, modifiers)
                })
                .collect()
        })
        .unwrap_or_default();

    for node in doc.descendants(doc.root()) {
        match doc.kind(node) {
            NodeKind::Element(element) => {
                push_name_tokens(&mut raw, element.name_range, text);
                if let Some(close) = element.close_tag {
                    if let Some(range) = end_tag_name_range(element, close, text) {
                        push_name_tokens(&mut raw, range, text);
                    }
                }
                for attr in &element.attributes {
                    let written = attr.name.as_written();
                    let is_xmlns = written == "xmlns" || written.starts_with("xmlns:");
                    raw.push((
                        attr.name_range,
                        if is_xmlns { NAMESPACE } else { PROPERTY },
                        0,
                    ));
                    push_value_tokens(&mut raw, attr.value_range, &occurrences);
                }
            }
            NodeKind::Comment => raw.push((doc.range(node), COMMENT, 0)),
            NodeKind::CData => raw.push((doc.range(node), STRING, 0)),
            NodeKind::ProcessingInstruction { target } => {
                let range = doc.range(node);
                let start = range.start + 2;
                let end = (start + target.len()).min(range.end);
                raw.push((TextRange::new(start, end), MACRO, 0));
            }
            NodeKind::Doctype => raw.push((doc.range(node), KEYWORD, 0)),
            NodeKind::Root | NodeKind::Text => {}
        }
    }

    raw.sort_by_key(|(range, _, _)| range.start);
    encode(&raw, text, line_index)
}

/// Emits `prefix` (namespace) and `local` (type) tokens for a tag name.
fn push_name_tokens(raw: &mut Vec<(TextRange, u32, u32)>, range: TextRange, text: &str) {
    let Some(written) = text.get(range.start..range.end) else {
        return;
    };
    match written.find(':') {
        Some(colon) => {
            raw.push((
                TextRange::new(range.start, range.start + colon),
                NAMESPACE,
                0,
            ));
            raw.push((TextRange::new(range.start + colon + 1, range.end), TYPE, 0));
        }
        None => raw.push((range, TYPE, 0)),
    }
}

/// Emits value tokens, splitting around a symbol occurrence when one lies
/// inside the value.
fn push_value_tokens(
    raw: &mut Vec<(TextRange, u32, u32)>,
    value: TextRange,
    occurrences: &[(TextRange, u32)],
) {
    let inside = occurrences
        .iter()
        .find(|(range, _)| value.start <= range.start && range.end <= value.end);
    match inside {
        Some(&(occ, modifiers)) => {
            if occ.start > value.start {
                raw.push((TextRange::new(value.start, occ.start), STRING, 0));
            }
            raw.push((occ, VARIABLE, modifiers));
            if value.end > occ.end {
                raw.push((TextRange::new(occ.end, value.end), STRING, 0));
            }
        }
        None => raw.push((value, STRING, 0)),
    }
}

/// The name range inside an end tag (`</name>`).
fn end_tag_name_range(
    element: &xml_ast::Element,
    close: TextRange,
    text: &str,
) -> Option<TextRange> {
    let written = element.name.as_written();
    let slice = text.get(close.start..close.end)?;
    let pos = slice.find(&written)?;
    Some(TextRange::new(
        close.start + pos,
        close.start + pos + written.len(),
    ))
}

/// Splits ranges at line breaks (tokens must not span lines) and encodes
/// them as LSP delta tokens. Lengths and columns are UTF-16 code units.
fn encode(raw: &[(TextRange, u32, u32)], text: &str, line_index: &LineIndex) -> Vec<SemanticToken> {
    let mut tokens = Vec::new();
    let mut prev_line = 0u32;
    let mut prev_col = 0u32;
    for &(range, token_type, modifiers) in raw {
        let Some(slice) = text.get(range.start..range.end) else {
            continue;
        };
        let mut segment_start = range.start;
        for part in slice.split_inclusive('\n') {
            let content = part.strip_suffix('\n').unwrap_or(part);
            let content = content.strip_suffix('\r').unwrap_or(content);
            if !content.is_empty() {
                let pos = line_index.line_col(text, segment_start);
                let length = content.encode_utf16().count() as u32;
                let delta_line = pos.line - prev_line;
                let delta_start = if delta_line == 0 {
                    pos.col - prev_col
                } else {
                    pos.col
                };
                tokens.push(SemanticToken {
                    delta_line,
                    delta_start,
                    length,
                    token_type,
                    token_modifiers_bitset: modifiers,
                });
                prev_line = pos.line;
                prev_col = pos.col;
            }
            segment_start += part.len();
        }
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(text: &str) -> Vec<SemanticToken> {
        let parse = xml_parser::parse(text);
        let line_index = LineIndex::new(text);
        semantic_tokens(&parse.document, text, &line_index, None)
    }

    #[test]
    fn classifies_basic_syntax() {
        let text = r#"<a b="1"><!-- note --></a>"#;
        let toks = tokens(text);
        // <a>: type; b: property; "1": string; comment; </a>: type.
        let types: Vec<u32> = toks.iter().map(|t| t.token_type).collect();
        assert_eq!(types, vec![TYPE, PROPERTY, STRING, COMMENT, TYPE]);
    }

    #[test]
    fn splits_prefixed_names() {
        let text = r#"<p:a xmlns:p="urn:x"/>"#;
        let toks = tokens(text);
        let types: Vec<u32> = toks.iter().map(|t| t.token_type).collect();
        // p (namespace), a (type), xmlns:p (namespace), value (string).
        assert_eq!(types, vec![NAMESPACE, TYPE, NAMESPACE, STRING]);
    }

    #[test]
    fn marks_symbol_definitions_and_references() {
        let text = r#"<root xmlns="urn:test">
  <color name="accent"/>
  <box fill="accent"/>
</root>"#;
        let parse = xml_parser::parse(text);
        let line_index = LineIndex::new(text);
        let rules = crate::navigation::tests::rules();
        let toks = semantic_tokens(&parse.document, text, &line_index, Some(&rules));
        let variables: Vec<&SemanticToken> =
            toks.iter().filter(|t| t.token_type == VARIABLE).collect();
        assert_eq!(variables.len(), 2);
        assert_eq!(variables[0].token_modifiers_bitset, DEFINITION_BIT);
        assert_eq!(variables[1].token_modifiers_bitset, 0);
    }

    #[test]
    fn multiline_comment_is_split_per_line() {
        let text = "<a><!-- one\ntwo --></a>";
        let toks = tokens(text);
        let comments: Vec<&SemanticToken> =
            toks.iter().filter(|t| t.token_type == COMMENT).collect();
        assert_eq!(comments.len(), 2);
        assert_eq!(comments[1].delta_line, 1);
        assert_eq!(comments[1].delta_start, 0);
    }

    #[test]
    fn delta_encoding_is_relative() {
        let text = "<a>\n<b/>\n</a>";
        let toks = tokens(text);
        assert_eq!(toks[0].delta_line, 0);
        assert_eq!(toks[0].delta_start, 1);
        assert_eq!(toks[1].delta_line, 1);
        assert_eq!(toks[1].delta_start, 1);
    }
}
