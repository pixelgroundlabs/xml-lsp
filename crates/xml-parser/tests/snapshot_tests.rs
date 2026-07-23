//! Snapshot tests for diagnostics: they check wording, codes, and positions
//! in one place, so any change to diagnostic behavior has to be reviewed
//! deliberately (`cargo insta review`).

use xml_parser::parse;

/// Renders diagnostics in a compact, stable format for snapshots.
fn render(text: &str) -> String {
    let result = parse(text);
    if result.diagnostics.is_empty() {
        return "(no diagnostics)".to_string();
    }
    result
        .diagnostics
        .iter()
        .map(|d| {
            format!(
                "{:?} [{}] {}..{}: {}",
                d.severity,
                d.code.as_str(),
                d.range.start,
                d.range.end,
                d.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn snapshot_well_formed() {
    insta::assert_snapshot!(render("<a><b>ok</b></a>"));
}

#[test]
fn snapshot_broken_nesting() {
    insta::assert_snapshot!(render("<a><b><c></b></a>"));
}

#[test]
fn snapshot_attribute_errors() {
    insta::assert_snapshot!(render(r#"<a x=1 y a="1" a="2"></a>"#));
}

#[test]
fn snapshot_namespace_and_roots() {
    insta::assert_snapshot!(render("<x:a/><b/>text"));
}
