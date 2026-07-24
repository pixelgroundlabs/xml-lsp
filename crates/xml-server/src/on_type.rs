//! Typing assistance beyond completion:
//!
//! * `documentOnTypeFormatting` — after Enter behind a start tag the
//!   cursor line is indented one step deeper; an end tag sitting right
//!   after the cursor additionally moves to its own line with the
//!   start tag's indentation. Typing the `>` of a start tag without an
//!   end tag inserts the matching end tag.
//! * Deleting the `/` of a self-closing tag (detected in `didChange`)
//!   turns `<Foo />` into `<Foo ></Foo>` via `workspace/applyEdit`.

use xml_ast::{Document, TextRange};

/// Result of the Enter rule: the text edits, and — when the editor's
/// cursor will end up in the wrong place because the edits insert text
/// at the cursor position — the column the cursor belongs at on its
/// line (the caller repositions it via `window/showDocument`).
pub struct EnterEdits {
    pub edits: Vec<(TextRange, String)>,
    pub cursor_col: Option<usize>,
}

/// After Enter directly behind the `>` of a start tag: indents the
/// cursor line one `indent_unit` deeper than the start tag. When the
/// element's end tag follows the cursor on the same line
/// (`<Foo>\n|</Foo>`), it additionally moves onto its own line with
/// the start tag's indentation. `offset` is the cursor position after
/// the newline (and the editor's auto-indent).
pub fn on_enter(
    doc: &Document,
    text: &str,
    offset: usize,
    indent_unit: &str,
) -> Option<EnterEdits> {
    let offset = offset.min(text.len());
    // The cursor must sit at the start of a fresh line: only whitespace
    // between the last newline and the cursor.
    let line_start = text[..offset].rfind('\n')?;
    if !text[line_start + 1..offset]
        .chars()
        .all(|c| c == ' ' || c == '\t')
    {
        return None;
    }
    // Before that newline: the `>` of a (non-self-closing) start tag.
    let before = text[..line_start].trim_end_matches('\r');
    if !before.ends_with('>') {
        return None;
    }
    let gt = before.len() - 1;
    let node = doc.node_at_offset(gt);
    let element = doc.element(node)?;
    if element.open_tag.end != gt + 1 || element.self_closing {
        return None;
    }

    // Indentation of the start tag's line.
    let open_line_start = text[..element.open_tag.start]
        .rfind('\n')
        .map(|p| p + 1)
        .unwrap_or(0);
    let indent: String = text[open_line_start..]
        .chars()
        .take_while(|c| matches!(c, ' ' | '\t'))
        .collect();
    // The cursor line: one unit deeper than the start tag.
    let indent_cursor_line = (
        TextRange::new(line_start + 1, offset),
        format!("{indent}{indent_unit}"),
    );

    // End tag directly after the cursor → block form: end tag onto its
    // own line at the start tag's indentation. The editor drags the
    // cursor past text inserted at the cursor position, so the caller
    // must put it back to `cursor_col` on its line afterwards.
    let ws_end = offset
        + text[offset..]
            .find(|c: char| !matches!(c, ' ' | '\t'))
            .unwrap_or(0);
    if element.close_tag.is_some_and(|close| close.start == ws_end) {
        let close = element.close_tag.expect("checked above");
        let cursor_col = indent.len() + indent_unit.len();
        return Some(EnterEdits {
            edits: vec![
                indent_cursor_line,
                (TextRange::new(offset, close.start), format!("\n{indent}")),
            ],
            cursor_col: Some(cursor_col),
        });
    }

    // Otherwise the rest of the line must be empty (plain Enter after
    // the start tag): only fix the cursor line's indentation. All the
    // edit's text lands before the cursor, so the cursor stays put.
    let eol = text[offset..]
        .find('\n')
        .map(|p| offset + p)
        .unwrap_or(text.len());
    if text[offset..eol].chars().all(|c| matches!(c, ' ' | '\t')) {
        return Some(EnterEdits {
            edits: vec![indent_cursor_line],
            cursor_col: None,
        });
    }
    None
}

/// After typing the `>` of a start tag that has no end tag yet: the
/// edit inserting the matching end tag right after the cursor.
pub fn close_after_gt(doc: &Document, text: &str, offset: usize) -> Option<(TextRange, String)> {
    let offset = offset.min(text.len());
    if !text[..offset].ends_with('>') || text[..offset].ends_with("/>") {
        return None;
    }
    let node = doc.node_at_offset(offset.saturating_sub(1));
    let element = doc.element(node)?;
    if element.open_tag.end != offset || element.self_closing || element.close_tag.is_some() {
        return None;
    }
    Some((
        TextRange::empty(offset),
        format!("</{}>", element.name.as_written()),
    ))
}

/// If a single `didChange` deleted exactly the `/` of a self-closing
/// tag, returns the deletion offset (in the *old* text) — the caller
/// re-checks against the freshly parsed document.
pub fn deleted_self_closing_slash(old_text: &str, removed: &str, offset: usize) -> Option<usize> {
    if removed != "/" {
        return None;
    }
    // After the deletion the tag must still end in `>`: the next
    // non-space character of the old text after the slash.
    let rest = &old_text[offset + 1..];
    let next = rest.trim_start_matches([' ', '\t']).chars().next()?;
    (next == '>').then_some(offset)
}

/// The end-tag insertion for an element that just lost its `/`:
/// `offset` points into the (new) start tag.
pub fn auto_close_edit(doc: &Document, text: &str, offset: usize) -> Option<(usize, String)> {
    let node = doc.node_at_offset(offset.min(text.len().saturating_sub(1)));
    let element = doc.element(node)?;
    if !element.open_tag.contains(offset) || element.self_closing || element.close_tag.is_some() {
        return None;
    }
    Some((
        element.open_tag.end,
        format!("</{}>", element.name.as_written()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies edits back-to-front.
    fn apply(text: &str, mut edits: Vec<(TextRange, String)>) -> String {
        edits.sort_by_key(|(range, _)| range.start);
        let mut out = text.to_string();
        for (range, new_text) in edits.into_iter().rev() {
            out.replace_range(range.start..range.end, &new_text);
        }
        out
    }

    #[test]
    fn enter_between_tags_builds_indented_block() {
        // After Enter (editor auto-indent gave 4 spaces) in `  <b>|</b>`:
        let text = "<a>\n  <b>\n    </b>\n</a>";
        let offset = text.find("    </b>").unwrap() + 4;
        let result =
            on_enter(&xml_parser::parse(text).document, text, offset, "  ").expect("edits");
        assert_eq!(
            result.cursor_col,
            Some(4),
            "cursor belongs at the end of the middle line's indentation"
        );
        assert_eq!(
            apply(text, result.edits),
            "<a>\n  <b>\n    \n  </b>\n</a>",
            "cursor line one unit deeper, end tag aligned with start tag"
        );
    }

    #[test]
    fn enter_without_editor_indent_still_indents() {
        // Editor did not auto-indent the new line at all.
        let text = "<a>\n  <b>\n</b>\n</a>";
        let offset = text.find("\n</b>").unwrap() + 1;
        let result =
            on_enter(&xml_parser::parse(text).document, text, offset, "  ").expect("edits");
        assert_eq!(apply(text, result.edits), "<a>\n  <b>\n    \n  </b>\n</a>");
    }

    #[test]
    fn enter_after_start_tag_indents_new_line() {
        // Enter at the end of the <b> line; the editor indented the new
        // line only to the start tag's level.
        let text = "<r>\n    <b>\n    \n    </b>\n</r>";
        let offset = text.find("    \n    </b>").unwrap() + 4;
        let result =
            on_enter(&xml_parser::parse(text).document, text, offset, "    ").expect("edits");
        assert_eq!(result.edits.len(), 1, "only the cursor line is re-indented");
        assert_eq!(result.cursor_col, None, "the cursor stays put by itself");
        assert_eq!(
            apply(text, result.edits),
            "<r>\n    <b>\n        \n    </b>\n</r>"
        );
    }

    #[test]
    fn enter_after_self_closing_tag_is_ignored() {
        let text = "<r>\n    <b/>\n    \n</r>";
        let offset = text.find("    \n</r>").unwrap() + 4;
        assert!(on_enter(&xml_parser::parse(text).document, text, offset, "  ").is_none());
    }

    #[test]
    fn enter_after_end_tag_is_ignored() {
        let text = "<r>\n    <b></b>\n    \n</r>";
        let offset = text.find("    \n</r>").unwrap() + 4;
        assert!(on_enter(&xml_parser::parse(text).document, text, offset, "  ").is_none());
    }

    #[test]
    fn enter_inside_content_is_ignored() {
        let text = "<a>\n  <b>text\n  </b>\n</a>";
        let offset = text.find("  </b>").unwrap() + 2;
        assert!(on_enter(&xml_parser::parse(text).document, text, offset, "  ").is_none());
    }

    #[test]
    fn enter_with_content_after_cursor_is_ignored() {
        // The end tag is not directly after the cursor.
        let text = "<a>\n  <b>\n  x</b>\n</a>";
        let offset = text.find("  x").unwrap() + 2;
        assert!(on_enter(&xml_parser::parse(text).document, text, offset, "  ").is_none());
    }

    #[test]
    fn gt_inserts_missing_end_tag() {
        let text = "<a><b></a>";
        let offset = text.find("<b>").unwrap() + 3;
        let (range, insert) =
            close_after_gt(&xml_parser::parse(text).document, text, offset).expect("edit");
        assert_eq!(range, TextRange::empty(offset));
        assert_eq!(insert, "</b>");
    }

    #[test]
    fn gt_leaves_closed_and_self_closing_tags_alone() {
        let text = "<a><b></b></a>";
        let offset = text.find("<b>").unwrap() + 3;
        assert!(close_after_gt(&xml_parser::parse(text).document, text, offset).is_none());
        let text = "<a><b/></a>";
        let offset = text.find("<b/>").unwrap() + 4;
        assert!(close_after_gt(&xml_parser::parse(text).document, text, offset).is_none());
    }

    #[test]
    fn slash_deletion_is_detected() {
        let old = "<a><b /></a>";
        let offset = old.find('/').unwrap();
        assert_eq!(deleted_self_closing_slash(old, "/", offset), Some(offset));
        assert_eq!(deleted_self_closing_slash(old, "b", offset), None);
        // Deleting a slash elsewhere (attribute value) does not count.
        let old = r#"<a href="a/b"></a>"#;
        let offset = old.find("/b").unwrap();
        assert_eq!(deleted_self_closing_slash(old, "/", offset), None);
    }

    #[test]
    fn auto_close_appends_end_tag() {
        // The document after the slash was deleted.
        let text = "<a><b ></a>";
        let offset = text.find("<b ").unwrap() + 3;
        let (at, insert) =
            auto_close_edit(&xml_parser::parse(text).document, text, offset).expect("edit");
        assert_eq!(at, text.find("<b ").unwrap() + 4);
        assert_eq!(insert, "</b>");
        let mut fixed = text.to_string();
        fixed.insert_str(at, &insert);
        assert_eq!(fixed, "<a><b ></b></a>");
    }

    #[test]
    fn auto_close_skips_still_closed_elements() {
        let text = "<a><b ></b></a>";
        let offset = text.find("<b ").unwrap() + 3;
        assert!(auto_close_edit(&xml_parser::parse(text).document, text, offset).is_none());
    }
}
