//! Whole-document XML formatting (ported from the speedata VS Code
//! extension's algorithm): one element per line with indentation,
//! self-closing normalization (`<Foo></Foo>` → `<Foo />`), and blank
//! lines between sibling groups at the top level.
//!
//! Language-specific conventions come from `lsp:format` annotations in
//! the schema (see docs/schema-annotations.md), not from the server:
//! `preserve` keeps element content verbatim, `blank-lines` separates
//! an element's children like top-level siblings, and `inline` keeps
//! text-only elements below it on a single line (HTML/MathML flow).

use std::collections::HashSet;

use xml_schema::{Pattern, Schema};

/// Formatting conventions extracted from the schema's `lsp:format`
/// annotations, keyed by element local name.
#[derive(Debug, Clone, Default)]
pub struct FormatStyle {
    /// Content is emitted verbatim (whitespace preserved).
    pub preserved: HashSet<String>,
    /// Direct children get blank-line separation like the top level.
    pub blank_line_groups: HashSet<String>,
    /// Below these elements, text-only elements stay on a single line.
    pub inline_flow: HashSet<String>,
}

impl FormatStyle {
    /// Collects the format hints of all annotated element patterns.
    pub fn from_schema(schema: &Schema) -> Self {
        let mut style = Self::default();
        for id in schema.pattern_ids() {
            let Pattern::Element(element) = schema.pattern(id) else {
                continue;
            };
            let Some(hints) = element.format else {
                continue;
            };
            let Some((_, local)) = element.name.as_name() else {
                continue;
            };
            if hints.preserve {
                style.preserved.insert(local.to_string());
            }
            if hints.blank_lines {
                style.blank_line_groups.insert(local.to_string());
            }
            if hints.inline {
                style.inline_flow.insert(local.to_string());
            }
        }
        style
    }
}

/// The local part of a possibly prefixed tag name.
fn local_name(tag_name: &str) -> &str {
    tag_name.rsplit(':').next().unwrap_or(tag_name)
}

/// Formats a whole document. `indent` is one indentation step (spaces or
/// a tab), as requested by the client's formatting options.
pub fn format_xml(text: &str, indent: &str, style: &FormatStyle) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut depth: usize = 0;
    let mut i: usize = 0;
    let mut preserve_depth: usize = 0;
    let mut element_stack: Vec<String> = Vec::new();
    let mut inline_depth: usize = 0;

    // Blank lines are not emitted immediately; `pending_blank` remembers
    // that the next sibling may need one, and comments are buffered so
    // the blank line lands before an attached comment block.
    let mut pending_blank = false;
    let mut prev_sibling_self_closing = false;
    let mut comment_buffer: Vec<String> = Vec::new();
    let mut saw_blank_line = false;

    macro_rules! flush_before_element {
        ($current_self_closing:expr) => {{
            if !is_blank_line_depth(depth, &element_stack, style) {
                out.append(&mut comment_buffer);
            } else {
                let both_self_closing = prev_sibling_self_closing && $current_self_closing;
                if pending_blank && !both_self_closing {
                    out.push("\n".to_string());
                }
                out.append(&mut comment_buffer);
            }
        }};
    }

    while i < bytes.len() {
        // Skip whitespace between tokens (when not preserving).
        if preserve_depth == 0 && bytes[i].is_ascii_whitespace() {
            let mut newlines = 0;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                if bytes[i] == b'\n' {
                    newlines += 1;
                }
                i += 1;
            }
            if newlines >= 2 {
                saw_blank_line = true;
            }
            continue;
        }

        // Processing instruction: <?...?>
        if text[i..].starts_with("<?") {
            saw_blank_line = false;
            let Some(end) = text[i..].find("?>").map(|p| i + p) else {
                out.push(text[i..].to_string());
                break;
            };
            let pi = &text[i..end + 2];
            out.push(pi.to_string());
            if preserve_depth == 0 {
                out.push("\n".to_string());
            }
            i = end + 2;
            continue;
        }

        // Comment: <!--...-->
        if text[i..].starts_with("<!--") {
            let Some(end) = text[i..].find("-->").map(|p| i + p) else {
                if preserve_depth > 0 {
                    out.push(text[i..].to_string());
                } else {
                    out.push(format!("{}{}", indent_str(indent, depth), &text[i..]));
                }
                break;
            };
            let comment = &text[i..end + 3];
            if preserve_depth > 0 {
                out.push(comment.to_string());
            } else if is_blank_line_depth(depth, &element_stack, style) {
                if has_blank_line_after(text, end + 3) {
                    // Standalone comment: emit immediately with blank-line
                    // logic; it separates from what follows.
                    flush_before_element!(false);
                    if saw_blank_line {
                        out.push("\n".to_string());
                    }
                    out.push(format!("{}{comment}\n", indent_str(indent, depth)));
                    pending_blank = true;
                    prev_sibling_self_closing = false;
                } else {
                    // Attached to the next element: buffer it.
                    if saw_blank_line {
                        comment_buffer.push("\n".to_string());
                    }
                    comment_buffer.push(format!("{}{comment}\n", indent_str(indent, depth)));
                }
            } else {
                if saw_blank_line && !out.is_empty() {
                    out.push("\n".to_string());
                }
                out.push(format!("{}{comment}\n", indent_str(indent, depth)));
            }
            saw_blank_line = false;
            i = end + 3;
            continue;
        }

        // CDATA: <![CDATA[...]]>
        if text[i..].starts_with("<![CDATA[") {
            saw_blank_line = false;
            let Some(end) = text[i..].find("]]>").map(|p| i + p) else {
                if preserve_depth > 0 {
                    out.push(text[i..].to_string());
                } else {
                    out.push(format!("{}{}", indent_str(indent, depth), &text[i..]));
                }
                break;
            };
            let cdata = &text[i..end + 3];
            if preserve_depth > 0 {
                out.push(cdata.to_string());
            } else {
                out.push(format!("{}{cdata}\n", indent_str(indent, depth)));
            }
            i = end + 3;
            continue;
        }

        // Closing tag: </...>
        if text[i..].starts_with("</") {
            saw_blank_line = false;
            let Some(end) = text[i..].find('>').map(|p| i + p) else {
                out.push(text[i..].to_string());
                break;
            };
            let tag = &text[i..end + 1];
            let tag_name = extract_tag_name(&tag[2..]);

            if preserve_depth > 0 {
                if style.preserved.contains(local_name(tag_name)) {
                    preserve_depth -= 1;
                    if preserve_depth == 0 {
                        depth = depth.saturating_sub(1);
                        element_stack.pop();
                        out.push(format!("{}{tag}\n", indent_str(indent, depth)));
                        pending_blank = true;
                        prev_sibling_self_closing = false;
                        i = end + 1;
                        continue;
                    }
                }
                out.push(tag.to_string());
                i = end + 1;
                continue;
            }

            depth = depth.saturating_sub(1);
            element_stack.pop();
            if style.inline_flow.contains(local_name(tag_name)) && inline_depth > 0 {
                inline_depth -= 1;
            }
            // Flush buffered comments before the closing tag.
            out.append(&mut comment_buffer);
            out.push(format!("{}{tag}\n", indent_str(indent, depth)));
            pending_blank = true;
            prev_sibling_self_closing = false;
            i = end + 1;
            continue;
        }

        // Opening or self-closing tag.
        saw_blank_line = false;
        if bytes[i] == b'<' && i + 1 < bytes.len() && is_name_start_char(bytes[i + 1]) {
            let Some(end) = find_tag_end(text, i) else {
                out.push(text[i..].to_string());
                break;
            };
            let tag = &text[i..end + 1];
            let tag_name = extract_tag_name(&tag[1..]).to_string();
            let self_closing = tag.ends_with("/>");

            if preserve_depth > 0 {
                out.push(tag.to_string());
                if !self_closing && style.preserved.contains(local_name(&tag_name)) {
                    preserve_depth += 1;
                }
                i = end + 1;
                continue;
            }

            if self_closing {
                flush_before_element!(true);
                out.push(format!(
                    "{}{}\n",
                    indent_str(indent, depth),
                    normalize_tag_spaces(tag)
                ));
                pending_blank = true;
                prev_sibling_self_closing = true;
            } else if style.preserved.contains(local_name(&tag_name)) {
                flush_before_element!(false);
                i = end + 1;
                let close_tag = format!("</{tag_name}>");
                match capture_preserved_content(text, i, &close_tag) {
                    Some((content, end_index)) => {
                        if content.is_empty() {
                            // Empty preserved element → self-closing tag.
                            out.push(format!(
                                "{}{}\n",
                                indent_str(indent, depth),
                                to_self_closing_tag(tag)
                            ));
                            pending_blank = true;
                            prev_sibling_self_closing = true;
                        } else {
                            out.push(format!(
                                "{}{}{content}{close_tag}\n",
                                indent_str(indent, depth),
                                normalize_tag_spaces(tag)
                            ));
                            pending_blank = true;
                            prev_sibling_self_closing = false;
                        }
                        i = end_index;
                    }
                    None => {
                        // No matching close tag (unterminated while
                        // editing): fall back to normal formatting.
                        out.push(format!(
                            "{}{}\n",
                            indent_str(indent, depth),
                            normalize_tag_spaces(tag)
                        ));
                        element_stack.push(tag_name);
                        depth += 1;
                        pending_blank = false;
                        prev_sibling_self_closing = false;
                    }
                }
                continue;
            } else {
                let close_tag = format!("</{tag_name}>");
                // Empty element (`<Foo></Foo>`) → self-closing tag.
                if let Some(after_close) = empty_element_end(text, end + 1, &close_tag) {
                    flush_before_element!(true);
                    out.push(format!(
                        "{}{}\n",
                        indent_str(indent, depth),
                        to_self_closing_tag(tag)
                    ));
                    pending_blank = true;
                    prev_sibling_self_closing = true;
                    i = after_close;
                    continue;
                }
                // Inside HTML/MathML: keep text-only elements on one line.
                if inline_depth > 0 {
                    if let Some((content, end_index)) =
                        inline_text_content(text, end + 1, &close_tag)
                    {
                        flush_before_element!(false);
                        out.push(format!(
                            "{}{}{content}{close_tag}\n",
                            indent_str(indent, depth),
                            normalize_tag_spaces(tag)
                        ));
                        pending_blank = true;
                        prev_sibling_self_closing = false;
                        i = end_index;
                        continue;
                    }
                }
                flush_before_element!(false);
                out.push(format!(
                    "{}{}\n",
                    indent_str(indent, depth),
                    normalize_tag_spaces(tag)
                ));
                if style.inline_flow.contains(local_name(&tag_name)) {
                    inline_depth += 1;
                }
                element_stack.push(tag_name);
                depth += 1;
                pending_blank = false;
                prev_sibling_self_closing = false;
            }
            i = end + 1;
            continue;
        }

        // Text content.
        if preserve_depth > 0 {
            let next = text[i..]
                .char_indices()
                .nth(1)
                .map(|(idx, _)| i + idx)
                .unwrap_or(text.len());
            out.push(text[i..next].to_string());
            i = next;
            continue;
        }

        let text_start = i;
        while i < bytes.len() && bytes[i] != b'<' {
            i += 1;
        }
        let text_content = &text[text_start..i];
        // Inside HTML, collapse whitespace runs (inline flow); elsewhere
        // keep the text apart from trimming (e.g. CSS in <Stylesheet>).
        let trimmed = if inline_depth > 0 {
            collapse_whitespace(text_content)
        } else {
            text_content.trim().to_string()
        };
        if !trimmed.is_empty() {
            let mut glued = false;
            // In HTML, text following a sibling element (e.g. the "."
            // after </math>) is appended to that element's line so no
            // stray space appears before punctuation.
            if inline_depth > 0 {
                if let Some(idx) = last_non_blank_index(&out) {
                    if ends_with_element(&out[idx]) {
                        let had_leading_ws =
                            text_start > 0 && text.as_bytes()[text_start - 1].is_ascii_whitespace();
                        let sep = if had_leading_ws { " " } else { "" };
                        let base = out[idx].trim_end().to_string();
                        out[idx] = format!("{base}{sep}{trimmed}\n");
                        out.truncate(idx + 1);
                        glued = true;
                    }
                }
            }
            if !glued {
                out.push(format!("{}{trimmed}\n", indent_str(indent, depth)));
            }
        }
    }

    out.append(&mut comment_buffer);

    let mut result: String = out.concat();
    result = remove_blank_lines_before_close(&result);
    let trimmed = result.trim_end_matches('\n');
    format!("{trimmed}\n")
}

/// Removes blank lines directly before closing tags (a trailing blank
/// line inside an element).
fn remove_blank_lines_before_close(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut lines = text.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        if line.trim_end_matches(['\n', '\r']).is_empty() {
            if let Some(next) = lines.peek() {
                if next.trim_start().starts_with("</") {
                    continue;
                }
            }
        }
        result.push_str(line);
    }
    result
}

/// Content of a preserved element up to its matching close tag, tracking
/// nested elements of the same name.
fn capture_preserved_content<'a>(
    text: &'a str,
    start: usize,
    close_tag: &str,
) -> Option<(&'a str, usize)> {
    let tag_name = &close_tag[2..close_tag.len() - 1];
    let open_pattern = format!("<{tag_name}");
    let mut depth = 1;
    let mut i = start;
    let bytes = text.as_bytes();

    // Byte-wise scan: `i` may sit inside a multi-byte character, so all
    // comparisons work on bytes, never on `&text[i..]`.
    while i < bytes.len() {
        if bytes[i..].starts_with(close_tag.as_bytes()) {
            depth -= 1;
            if depth == 0 {
                return Some((&text[start..i], i + close_tag.len()));
            }
            i += close_tag.len();
            continue;
        }
        if bytes[i..].starts_with(open_pattern.as_bytes()) {
            let after = i + open_pattern.len();
            if after < bytes.len()
                && (bytes[after] == b'>'
                    || bytes[after] == b'/'
                    || bytes[after].is_ascii_whitespace())
            {
                if let Some(tag_end) = find_tag_end(text, i) {
                    if !text[i..tag_end + 1].ends_with("/>") {
                        depth += 1;
                    }
                    i = tag_end + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    None
}

/// Index of the last emitted entry that is not whitespace-only.
fn last_non_blank_index(out: &[String]) -> Option<usize> {
    out.iter().rposition(|entry| !entry.trim().is_empty())
}

/// Whether an emitted line ends with a closing (`</x>`) or self-closing
/// (`<x/>`) tag.
fn ends_with_element(entry: &str) -> bool {
    let t = entry.trim_end();
    if !t.ends_with('>') {
        return false;
    }
    if t.ends_with("/>") {
        return true;
    }
    match t.rfind('<') {
        Some(lt) => t[lt + 1..].starts_with('/'),
        None => false,
    }
}

/// Byte index of the `>` closing the tag starting at `start`, respecting
/// quoted attribute values.
fn find_tag_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut in_quote: Option<u8> = None;
    for (i, &b) in bytes.iter().enumerate().skip(start + 1) {
        match in_quote {
            Some(q) => {
                if b == q {
                    in_quote = None;
                }
            }
            None => match b {
                b'"' | b'\'' => in_quote = Some(b),
                b'>' => return Some(i),
                _ => {}
            },
        }
    }
    None
}

/// The tag name at the start of `s` (up to whitespace, `>`, or `/`).
fn extract_tag_name(s: &str) -> &str {
    let end = s
        .find(|c: char| c.is_ascii_whitespace() || c == '>' || c == '/')
        .unwrap_or(s.len());
    &s[..end]
}

fn indent_str(indent: &str, depth: usize) -> String {
    indent.repeat(depth)
}

/// Collapses whitespace runs between name, attributes, and bracket to
/// single spaces; quoted values stay untouched. Self-closing tags end in
/// ` />`, normal tags have no space before `>`.
fn normalize_tag_spaces(tag: &str) -> String {
    let mut result = String::with_capacity(tag.len());
    let mut in_quote: Option<char> = None;
    let mut prev_was_space = false;
    for ch in tag.chars() {
        match in_quote {
            Some(q) => {
                result.push(ch);
                prev_was_space = false;
                if ch == q {
                    in_quote = None;
                }
            }
            None => {
                if ch == '"' || ch == '\'' {
                    result.push(ch);
                    prev_was_space = false;
                    in_quote = Some(ch);
                } else if ch.is_ascii_whitespace() {
                    if !prev_was_space {
                        result.push(' ');
                        prev_was_space = true;
                    }
                } else {
                    result.push(ch);
                    prev_was_space = false;
                }
            }
        }
    }
    if result.ends_with("/>") {
        let head = result[..result.len() - 2].trim_end();
        result = format!("{head} />");
    } else if result.ends_with('>') {
        let head = result[..result.len() - 1].trim_end();
        result = format!("{head}>");
    }
    result
}

/// Converts `<Foo attr="x">` into `<Foo attr="x" />`.
fn to_self_closing_tag(open_tag: &str) -> String {
    let stripped = open_tag.trim_end_matches('>').trim_end();
    normalize_tag_spaces(&format!("{stripped} />"))
}

/// If only whitespace sits between the opening tag and the close tag,
/// returns the index just past the close tag.
fn empty_element_end(text: &str, after_open: usize, close_tag: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut j = after_open;
    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
    }
    text[j..]
        .starts_with(close_tag)
        .then_some(j + close_tag.len())
}

/// Text-only content of an element (no child nodes), whitespace runs
/// collapsed, plus the index past the close tag.
fn inline_text_content(text: &str, after_open: usize, close_tag: &str) -> Option<(String, usize)> {
    let rest = &text[after_open..];
    let lt = rest.find('<')?;
    if !rest[lt..].starts_with(close_tag) {
        return None; // a child element / comment / CDATA → not text-only
    }
    let content = collapse_whitespace(&rest[..lt]);
    if content.is_empty() {
        return None; // empty → handled as self-closing
    }
    Some((content, after_open + lt + close_tag.len()))
}

fn collapse_whitespace(text: &str) -> String {
    text.split_ascii_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_name_start_char(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// Whether a blank line (two newlines) follows before the next
/// non-whitespace character.
fn has_blank_line_after(text: &str, pos: usize) -> bool {
    let mut newlines = 0;
    for &b in text.as_bytes()[pos.min(text.len())..].iter() {
        if b == b'\n' {
            newlines += 1;
            if newlines >= 2 {
                return true;
            }
        } else if !b.is_ascii_whitespace() {
            break;
        }
    }
    false
}

/// Blank lines separate siblings at the top level (depth 1) and inside
/// `Section` elements (depth 2).
fn is_blank_line_depth(depth: usize, element_stack: &[String], style: &FormatStyle) -> bool {
    if depth == 1 {
        return true;
    }
    element_stack
        .last()
        .is_some_and(|name| style.blank_line_groups.contains(local_name(name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(text: &str) -> String {
        format_xml(text, "  ", &FormatStyle::default())
    }

    /// A speedata-like style, as its schema annotations would produce.
    fn styled() -> FormatStyle {
        FormatStyle {
            preserved: ["Value".to_string()].into(),
            blank_line_groups: ["Section".to_string()].into(),
            inline_flow: ["HTML".to_string()].into(),
        }
    }

    fn fmt_styled(text: &str) -> String {
        format_xml(text, "  ", &styled())
    }

    #[test]
    fn indents_nested_elements() {
        let input = "<Layout><Record element=\"data\"><Paragraph><Value select=\"1\"/></Paragraph></Record></Layout>";
        let expected = "<Layout>\n  <Record element=\"data\">\n    <Paragraph>\n      <Value select=\"1\" />\n    </Paragraph>\n  </Record>\n</Layout>\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn empty_elements_become_self_closing() {
        assert_eq!(
            fmt("<Layout><Options></Options></Layout>"),
            "<Layout>\n  <Options />\n</Layout>\n"
        );
        assert_eq!(
            fmt("<Layout><Options>   </Options></Layout>"),
            "<Layout>\n  <Options />\n</Layout>\n"
        );
    }

    #[test]
    fn normalizes_tag_whitespace() {
        assert_eq!(
            fmt("<Layout>\n<Options   mainlanguage = \"de\"    /></Layout>"),
            "<Layout>\n  <Options mainlanguage = \"de\" />\n</Layout>\n"
        );
    }

    #[test]
    fn blank_lines_between_top_level_siblings() {
        let input = "<Layout><Record element=\"a\"><Paragraph/></Record><Record element=\"b\"><Paragraph/></Record></Layout>";
        let formatted = fmt(input);
        assert!(
            formatted.contains("</Record>\n\n  <Record element=\"b\">"),
            "{formatted}"
        );
    }

    #[test]
    fn consecutive_self_closing_siblings_stay_together() {
        let input = "<Layout><LoadFontfile name=\"a\"/><LoadFontfile name=\"b\"/></Layout>";
        let formatted = fmt(input);
        assert!(
            formatted.contains("<LoadFontfile name=\"a\" />\n  <LoadFontfile name=\"b\" />"),
            "{formatted}"
        );
    }

    #[test]
    fn value_content_is_preserved() {
        let input = "<Layout><Record><Value>  keep   this\n  spacing </Value></Record></Layout>";
        let formatted = fmt_styled(input);
        assert!(
            formatted.contains("<Value>  keep   this\n  spacing </Value>"),
            "{formatted}"
        );
    }

    #[test]
    fn empty_value_becomes_self_closing() {
        let formatted = fmt_styled("<Layout><Record><Value></Value></Record></Layout>");
        assert!(formatted.contains("<Value />"), "{formatted}");
    }

    #[test]
    fn html_text_elements_stay_inline() {
        let input = "<Layout><HTML><p>some   text</p></HTML></Layout>";
        let formatted = fmt_styled(input);
        assert!(formatted.contains("<p>some text</p>"), "{formatted}");
    }

    #[test]
    fn comment_stays_attached_to_next_element() {
        let input = "<Layout><Record element=\"a\"/>\n<!-- about b -->\n<Record element=\"b\"><P/></Record></Layout>";
        let formatted = fmt(input);
        assert!(
            formatted.contains("\n\n  <!-- about b -->\n  <Record element=\"b\">"),
            "{formatted}"
        );
    }

    #[test]
    fn processing_instruction_kept_on_own_line() {
        let formatted = fmt("<?xml version=\"1.0\"?><Layout><Options/></Layout>");
        assert!(
            formatted.starts_with("<?xml version=\"1.0\"?>\n<Layout>"),
            "{formatted}"
        );
    }

    #[test]
    fn style_comes_from_schema_annotations() {
        let schema_src = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0"
    xmlns:lsp="urn:xml-lsp:annotations">
  <start>
    <element name="root">
      <lsp:format blank-lines="true"/>
      <optional>
        <element name="code"><lsp:format preserve="true"/><text/></element>
      </optional>
      <optional>
        <element name="p"><lsp:format inline="true"/><text/></element>
      </optional>
    </element>
  </start>
</grammar>"#;
        let result = xml_schema::load_rng(schema_src);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let style = FormatStyle::from_schema(&result.schema);
        assert!(style.blank_line_groups.contains("root"));
        assert!(style.preserved.contains("code"));
        assert!(style.inline_flow.contains("p"));

        let formatted = format_xml("<root><code>  a\n b </code></root>", "  ", &style);
        assert!(formatted.contains("<code>  a\n b </code>"), "{formatted}");
    }

    #[test]
    fn idempotent_on_formatted_output() {
        let input = "<Layout><Record element=\"a\"><Paragraph><Value select=\"1\"/></Paragraph></Record><Record element=\"b\"/></Layout>";
        let once = fmt(input);
        assert_eq!(fmt(&once), once);
    }

    #[test]
    fn non_ascii_value_content_survives() {
        let input = "<Layout><Record><Value>Ä quälendes Prüfstück — ößü</Value></Record></Layout>";
        let formatted = fmt_styled(input);
        assert!(
            formatted.contains("<Value>Ä quälendes Prüfstück — ößü</Value>"),
            "{formatted}"
        );
    }

    #[test]
    fn text_outside_html_is_trimmed_not_collapsed() {
        let input = "<Layout><Stylesheet>\n  td {\n    color: red;\n  }\n</Stylesheet></Layout>";
        let formatted = fmt(input);
        assert!(
            formatted.contains("td {\n    color: red;\n  }"),
            "{formatted}"
        );
    }
}
