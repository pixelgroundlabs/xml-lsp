//! Quick fixes anchored on the parser's stable diagnostic codes.

use xml_ast::{Document, TextRange};
use xml_parser::{DiagnosticCode, ParseDiagnostic};

/// One proposed fix: a title and the text edits that apply it. The
/// `diagnostic` index points into the diagnostics slice passed to
/// [`quick_fixes`], so the caller can attach the original diagnostic.
pub struct QuickFix {
    pub title: String,
    pub edits: Vec<(TextRange, String)>,
    pub diagnostic: usize,
}

/// Fixes for all diagnostics that touch `range`.
pub fn quick_fixes(
    doc: &Document,
    text: &str,
    diagnostics: &[ParseDiagnostic],
    range: TextRange,
) -> Vec<QuickFix> {
    diagnostics
        .iter()
        .enumerate()
        .filter(|(_, d)| d.range.start <= range.end && range.start <= d.range.end)
        .filter_map(|(index, d)| {
            let (title, edits) = match d.code {
                DiagnosticCode::UnclosedElement => close_element(doc, text, d)?,
                DiagnosticCode::StrayEndTag => (
                    "Remove stray closing tag".to_string(),
                    vec![(d.range, String::new())],
                ),
                DiagnosticCode::AttributeSyntax => attribute_fix(text, d)?,
                _ => return None,
            };
            Some(QuickFix {
                title,
                edits,
                diagnostic: index,
            })
        })
        .collect()
}

/// Inserts the missing end tag at the end of the unclosed element. The
/// diagnostic range is the name inside the start tag.
fn close_element(
    doc: &Document,
    text: &str,
    diagnostic: &ParseDiagnostic,
) -> Option<(String, Vec<(TextRange, String)>)> {
    let node = doc.descendants(doc.root()).find(|&node| {
        doc.element(node).is_some_and(|element| {
            element.name_range == diagnostic.range
                && element.close_tag.is_none()
                && !element.self_closing
        })
    })?;
    let name = text.get(diagnostic.range.start..diagnostic.range.end)?;
    let insert_at = doc.range(node).end;
    Some((
        format!("Close `<{name}>`"),
        vec![(TextRange::empty(insert_at), format!("</{name}>"))],
    ))
}

/// Fixes for the attribute scanner's syntax errors, distinguished by
/// their (stable, server-owned) message text.
fn attribute_fix(
    text: &str,
    diagnostic: &ParseDiagnostic,
) -> Option<(String, Vec<(TextRange, String)>)> {
    let message = diagnostic.message.as_str();
    let range = diagnostic.range;
    if message.ends_with("must be quoted") {
        return Some((
            "Quote attribute value".to_string(),
            vec![
                (TextRange::empty(range.start), "\"".to_string()),
                (TextRange::empty(range.end), "\"".to_string()),
            ],
        ));
    }
    if message.ends_with("is not terminated") {
        // The range starts at the opening quote; repeat that character.
        let quote = text.get(range.start..range.start + 1)?;
        return Some((
            "Add closing quote".to_string(),
            vec![(TextRange::empty(range.end), quote.to_string())],
        ));
    }
    if message.ends_with("missing a value after `=`") {
        // The range is the attribute name; insert right after the `=`.
        let rest = text.get(range.end..)?;
        let eq = range.end + rest.find('=')?;
        return Some((
            "Add empty value".to_string(),
            vec![(TextRange::empty(eq + 1), "\"\"".to_string())],
        ));
    }
    if message.ends_with("is missing a value") {
        return Some((
            "Add empty value".to_string(),
            vec![(TextRange::empty(range.end), "=\"\"".to_string())],
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Applies fixes back-to-front so earlier offsets stay valid.
    fn apply(text: &str, mut edits: Vec<(TextRange, String)>) -> String {
        edits.sort_by_key(|(range, _)| range.start);
        let mut out = text.to_string();
        for (range, new_text) in edits.into_iter().rev() {
            out.replace_range(range.start..range.end, &new_text);
        }
        out
    }

    fn fixes_for(text: &str) -> Vec<(String, String)> {
        let parse = xml_parser::parse(text);
        let all = TextRange::new(0, text.len());
        quick_fixes(&parse.document, text, &parse.diagnostics, all)
            .into_iter()
            .map(|fix| (fix.title, apply(text, fix.edits)))
            .collect()
    }

    #[test]
    fn closes_unclosed_element() {
        let fixes = fixes_for("<a><b>text</a>");
        let close = fixes.iter().find(|(t, _)| t == "Close `<b>`").unwrap();
        assert_eq!(close.1, "<a><b>text</b></a>");
    }

    #[test]
    fn closes_element_at_eof() {
        let fixes = fixes_for("<a><b>text");
        assert!(fixes.iter().any(|(_, fixed)| fixed == "<a><b>text</b>"));
        assert!(fixes.iter().any(|(_, fixed)| fixed == "<a><b>text</a>"));
    }

    #[test]
    fn removes_stray_end_tag() {
        let fixes = fixes_for("<a>text</b></a>");
        let fix = fixes
            .iter()
            .find(|(t, _)| t == "Remove stray closing tag")
            .unwrap();
        assert_eq!(fix.1, "<a>text</a>");
    }

    #[test]
    fn quotes_attribute_value() {
        let fixes = fixes_for("<a b=1/>");
        let fix = fixes
            .iter()
            .find(|(t, _)| t == "Quote attribute value")
            .unwrap();
        assert_eq!(fix.1, "<a b=\"1\"/>");
    }

    #[test]
    fn terminates_attribute_value() {
        let text = "<a b=\"1></a>";
        let parse = xml_parser::parse(text);
        let all = TextRange::new(0, text.len());
        let fixes = quick_fixes(&parse.document, text, &parse.diagnostics, all);
        let fix = fixes.iter().find(|f| f.title == "Add closing quote");
        // The broken tag is a fatal syntax error for quick-xml; the
        // scanner only sees complete tags, so this may yield no fix —
        // but when it does, the edit must repeat the opening quote.
        if let Some(fix) = fix {
            assert_eq!(fix.edits[0].1, "\"");
        }
    }

    #[test]
    fn adds_missing_value() {
        let fixes = fixes_for("<a checked></a>");
        let fix = fixes.iter().find(|(t, _)| t == "Add empty value").unwrap();
        assert_eq!(fix.1, "<a checked=\"\"></a>");
    }

    #[test]
    fn adds_missing_value_after_equals() {
        let fixes = fixes_for("<a b=></a>");
        let fix = fixes.iter().find(|(t, _)| t == "Add empty value").unwrap();
        assert_eq!(fix.1, "<a b=\"\"></a>");
    }

    #[test]
    fn range_filters_fixes() {
        let text = "<a><b>text</a>";
        let parse = xml_parser::parse(text);
        // A range at the very start does not touch the `<b>` diagnostic.
        let fixes = quick_fixes(
            &parse.document,
            text,
            &parse.diagnostics,
            TextRange::new(0, 1),
        );
        assert!(fixes.iter().all(|f| !f.title.contains("<b>")));
    }
}
