//! Custom attribute scanner for start tags.
//!
//! quick-xml does not report positions for individual attributes, but
//! completion, hover, and precise diagnostics need the exact ranges of
//! attribute names and values. The scanner works on the raw tag content
//! (after the element name, before the closing `>`) and is tolerant of
//! typical typos.

use crate::diagnostics::{DiagnosticCode, ParseDiagnostic};
use xml_ast::TextRange;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawAttr {
    pub name: String,
    pub name_range: TextRange,
    /// `None` if the attribute has no value (syntax error).
    pub value: Option<String>,
    pub value_range: Option<TextRange>,
}

/// Scans attributes from `src`, where `base` is the byte offset of `src`
/// within the whole document. Syntax errors are reported into `diags`.
pub(crate) fn scan_attributes(
    src: &str,
    base: usize,
    diags: &mut Vec<ParseDiagnostic>,
) -> Vec<RawAttr> {
    let bytes = src.as_bytes();
    let mut i = 0usize;
    let mut out = Vec::new();

    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() || bytes[i] == b'/' {
            i += 1;
            continue;
        }

        // Read the attribute name.
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && !matches!(bytes[i], b'=' | b'/' | b'"' | b'\'')
        {
            i += 1;
        }
        if i == name_start {
            // A character that cannot start a name (e.g. a stray quote) —
            // report and skip it.
            diags.push(ParseDiagnostic::error(
                TextRange::new(base + i, base + i + 1),
                DiagnosticCode::AttributeSyntax,
                "unexpected character in start tag",
            ));
            i += 1;
            continue;
        }
        let name = src[name_start..i].to_string();
        let name_range = TextRange::new(base + name_start, base + i);

        // Optional whitespace before the '='.
        let mut lookahead = i;
        while lookahead < bytes.len() && bytes[lookahead].is_ascii_whitespace() {
            lookahead += 1;
        }

        if lookahead >= bytes.len() || bytes[lookahead] != b'=' {
            // Attribute without '=': not allowed in XML.
            diags.push(ParseDiagnostic::error(
                name_range,
                DiagnosticCode::AttributeSyntax,
                format!("attribute `{name}` is missing a value"),
            ));
            out.push(RawAttr {
                name,
                name_range,
                value: None,
                value_range: None,
            });
            continue;
        }
        i = lookahead + 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }

        if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
            // Regular quoted value.
            let quote = bytes[i];
            i += 1;
            let value_start = i;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let value_end = i;
            if i < bytes.len() {
                i += 1;
            } else {
                diags.push(ParseDiagnostic::error(
                    TextRange::new(base + value_start - 1, base + value_end),
                    DiagnosticCode::AttributeSyntax,
                    format!("attribute value of `{name}` is not terminated"),
                ));
            }
            out.push(RawAttr {
                name,
                name_range,
                value: Some(src[value_start..value_end].to_string()),
                value_range: Some(TextRange::new(base + value_start, base + value_end)),
            });
        } else {
            // Unquoted or missing value.
            let value_start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'/' {
                i += 1;
            }
            if value_start == i {
                diags.push(ParseDiagnostic::error(
                    name_range,
                    DiagnosticCode::AttributeSyntax,
                    format!("attribute `{name}` is missing a value after `=`"),
                ));
                out.push(RawAttr {
                    name,
                    name_range,
                    value: None,
                    value_range: None,
                });
            } else {
                diags.push(ParseDiagnostic::error(
                    TextRange::new(base + value_start, base + i),
                    DiagnosticCode::AttributeSyntax,
                    format!("attribute value of `{name}` must be quoted"),
                ));
                out.push(RawAttr {
                    name,
                    name_range,
                    value: Some(src[value_start..i].to_string()),
                    value_range: Some(TextRange::new(base + value_start, base + i)),
                });
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(src: &str) -> (Vec<RawAttr>, Vec<ParseDiagnostic>) {
        let mut diags = Vec::new();
        let attrs = scan_attributes(src, 0, &mut diags);
        (attrs, diags)
    }

    #[test]
    fn simple_attributes() {
        let (attrs, diags) = scan(r#" a="1" b='two' "#);
        assert!(diags.is_empty());
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].name, "a");
        assert_eq!(attrs[0].value.as_deref(), Some("1"));
        assert_eq!(attrs[0].name_range, TextRange::new(1, 2));
        assert_eq!(attrs[0].value_range, Some(TextRange::new(4, 5)));
        assert_eq!(attrs[1].value.as_deref(), Some("two"));
    }

    #[test]
    fn whitespace_around_equals() {
        let (attrs, diags) = scan(r#"a = "1""#);
        assert!(diags.is_empty());
        assert_eq!(attrs[0].value.as_deref(), Some("1"));
    }

    #[test]
    fn missing_value() {
        let (attrs, diags) = scan("checked");
        assert_eq!(attrs.len(), 1);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, DiagnosticCode::AttributeSyntax);
        assert!(attrs[0].value.is_none());
    }

    #[test]
    fn unquoted_value() {
        let (attrs, diags) = scan("a=1");
        assert_eq!(attrs[0].value.as_deref(), Some("1"));
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn unterminated_value() {
        let (attrs, diags) = scan(r#"a="open"#);
        assert_eq!(attrs[0].value.as_deref(), Some("open"));
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn value_with_slash_and_quote_chars() {
        let (attrs, diags) = scan(r#"href="a/b" title="it's""#);
        assert!(diags.is_empty());
        assert_eq!(attrs[0].value.as_deref(), Some("a/b"));
        assert_eq!(attrs[1].value.as_deref(), Some("it's"));
    }
}
