//! Resolving XML character and entity references.
//!
//! The parser stores text and attribute values as raw slices; consumers
//! that need the interpreted value (e.g. the schema loader reading
//! `name="a&amp;b"`) resolve references with [`unescape`].

use std::borrow::Cow;

/// Resolves the five predefined entities (`&amp;` `&lt;` `&gt;` `&quot;`
/// `&apos;`) and numeric character references (`&#10;`, `&#x1F600;`).
/// Unknown or malformed references are kept verbatim — reporting them is
/// the parser's job, not this function's.
pub fn unescape(text: &str) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        // A reference ends at the next ';'; limit the search so a lone '&'
        // does not swallow the remaining text.
        let end = rest[1..].find(';').map(|i| i + 1);
        let resolved = end.and_then(|end| {
            let entity = &rest[1..end];
            let ch = match entity {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => {
                    let code = entity
                        .strip_prefix("#x")
                        .or_else(|| entity.strip_prefix("#X"))
                        .map(|hex| u32::from_str_radix(hex, 16))
                        .or_else(|| entity.strip_prefix('#').map(str::parse::<u32>));
                    code.and_then(|c| c.ok()).and_then(char::from_u32)
                }
            };
            ch.map(|ch| (ch, end))
        });
        match resolved {
            Some((ch, end)) => {
                out.push(ch);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrows_without_references() {
        assert!(matches!(unescape("plain text"), Cow::Borrowed(_)));
    }

    #[test]
    fn resolves_predefined_entities() {
        assert_eq!(
            unescape("a&amp;b &lt;x&gt; &quot;q&quot; &apos;s&apos;"),
            "a&b <x> \"q\" 's'"
        );
    }

    #[test]
    fn resolves_numeric_references() {
        assert_eq!(unescape("&#65;&#x42;&#x1F600;"), "AB😀");
    }

    #[test]
    fn keeps_unknown_references() {
        assert_eq!(
            unescape("&nbsp; & &#xZZ; &unterminated"),
            "&nbsp; & &#xZZ; &unterminated"
        );
    }
}
