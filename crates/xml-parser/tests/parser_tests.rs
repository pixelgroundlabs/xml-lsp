//! Integration tests for the error-resilient parser.

use xml_ast::{NodeKind, TextRange};
use xml_parser::{parse, DiagnosticCode};

/// Helper: all diagnostic codes of a parse result.
fn codes(result: &xml_parser::ParseResult) -> Vec<DiagnosticCode> {
    result.diagnostics.iter().map(|d| d.code).collect()
}

#[test]
fn well_formed_document() {
    let text = r#"<?xml version="1.0"?>
<root>
  <child attr="wert">text</child>
  <leer/>
</root>
"#;
    let result = parse(text);
    assert!(
        result.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );

    let doc = &result.document;
    let root = doc.root_element().expect("root element present");
    let root_el = doc.element(root).unwrap();
    assert_eq!(root_el.name.local, "root");
    assert!(root_el.close_tag.is_some());

    let children: Vec<_> = doc
        .children(root)
        .iter()
        .filter_map(|&id| doc.element(id))
        .collect();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].name.local, "child");
    assert_eq!(children[1].name.local, "leer");
    assert!(children[1].self_closing);

    let attr = children[0]
        .attribute(None, "attr")
        .expect("attribute present");
    assert_eq!(attr.value, "wert");
    // The value range points exactly at "wert" (without quotes).
    assert_eq!(&text[attr.value_range.start..attr.value_range.end], "wert");
}

#[test]
fn element_ranges_cover_tags() {
    let text = "<a><b>x</b></a>";
    let result = parse(text);
    let doc = &result.document;
    let a = doc.root_element().unwrap();
    assert_eq!(doc.range(a), TextRange::new(0, 15));
    let b = doc.children(a)[0];
    assert_eq!(doc.range(b), TextRange::new(3, 11));
    let a_el = doc.element(a).unwrap();
    assert_eq!(a_el.open_tag, TextRange::new(0, 3));
    assert_eq!(a_el.close_tag, Some(TextRange::new(11, 15)));
}

#[test]
fn namespace_resolution() {
    let text = r#"<root xmlns="urn:default" xmlns:x="urn:x">
  <child x:attr="1"/>
  <x:sub xmlns="urn:inner"><deep/></x:sub>
</root>"#;
    let result = parse(text);
    assert!(
        result.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );

    let doc = &result.document;
    let root = doc.root_element().unwrap();
    assert_eq!(
        doc.element(root).unwrap().name.namespace.as_deref(),
        Some("urn:default")
    );

    let elements: Vec<_> = doc
        .children(root)
        .iter()
        .filter_map(|&id| doc.element(id).map(|el| (id, el)))
        .collect();
    let (_, child) = elements[0];
    assert_eq!(child.name.namespace.as_deref(), Some("urn:default"));
    // Prefixed attributes get the namespace of their prefix …
    assert_eq!(child.attributes[0].name.namespace.as_deref(), Some("urn:x"));

    let (sub_id, sub) = elements[1];
    assert_eq!(sub.name.namespace.as_deref(), Some("urn:x"));
    // … and the default namespace can be overridden further in.
    let deep = doc.children(sub_id)[0];
    assert_eq!(
        doc.element(deep).unwrap().name.namespace.as_deref(),
        Some("urn:inner")
    );
}

#[test]
fn undeclared_prefix_is_reported() {
    let result = parse("<root><x:child/></root>");
    assert_eq!(
        codes(&result),
        vec![DiagnosticCode::UndefinedNamespacePrefix]
    );
    // The element is still in the tree, just without a namespace.
    let doc = &result.document;
    let root = doc.root_element().unwrap();
    let child = doc.element(doc.children(root)[0]).unwrap();
    assert_eq!(child.name.local, "child");
    assert_eq!(child.name.prefix.as_deref(), Some("x"));
    assert!(child.name.namespace.is_none());
}

#[test]
fn unprefixed_attribute_has_no_namespace() {
    let result = parse(r#"<root xmlns="urn:default" a="1"/>"#);
    assert!(result.diagnostics.is_empty());
    let doc = &result.document;
    let root = doc.element(doc.root_element().unwrap()).unwrap();
    let attr = root.attribute_by_written_name("a").unwrap();
    assert!(attr.name.namespace.is_none());
}

#[test]
fn duplicate_attributes_are_reported() {
    let result = parse(r#"<root a="1" a="2"/>"#);
    assert_eq!(codes(&result), vec![DiagnosticCode::DuplicateAttribute]);

    // Also for equal expanded names with different prefixes.
    let result = parse(r#"<r xmlns:a="urn:x" xmlns:b="urn:x" a:k="1" b:k="2"/>"#);
    assert_eq!(codes(&result), vec![DiagnosticCode::DuplicateAttribute]);
}

#[test]
fn mismatched_end_tag_recovers() {
    // <b> is never closed; </a> auto-closes <b> with a diagnostic.
    let text = "<a><b>x</a>";
    let result = parse(text);
    assert_eq!(codes(&result), vec![DiagnosticCode::UnclosedElement]);

    let doc = &result.document;
    let a = doc.root_element().unwrap();
    let a_el = doc.element(a).unwrap();
    // <a> itself is closed cleanly.
    assert_eq!(a_el.close_tag, Some(TextRange::new(7, 11)));
    // <b> ends before the end tag of <a>.
    let b = doc.children(a)[0];
    assert_eq!(doc.range(b), TextRange::new(3, 7));
    assert!(doc.element(b).unwrap().close_tag.is_none());
}

#[test]
fn stray_end_tag_is_reported() {
    let result = parse("<a></b></a>");
    assert_eq!(codes(&result), vec![DiagnosticCode::StrayEndTag]);
    let doc = &result.document;
    let a = doc.root_element().unwrap();
    assert!(doc.element(a).unwrap().close_tag.is_some());
}

#[test]
fn unclosed_elements_at_eof() {
    let result = parse("<a><b>");
    let mut found = codes(&result);
    found.sort_by_key(|c| c.as_str());
    assert_eq!(
        found,
        vec![
            DiagnosticCode::UnclosedElement,
            DiagnosticCode::UnclosedElement
        ]
    );
    // Both elements are in the tree and extend to the end of the file.
    let doc = &result.document;
    let a = doc.root_element().unwrap();
    assert_eq!(doc.range(a), TextRange::new(0, 6));
}

#[test]
fn multiple_root_elements() {
    let result = parse("<a/><b/>");
    assert_eq!(codes(&result), vec![DiagnosticCode::MultipleRootElements]);
    assert_eq!(result.document.top_level_elements().count(), 2);
}

#[test]
fn text_outside_root() {
    let result = parse("<a/>hallo");
    assert_eq!(codes(&result), vec![DiagnosticCode::TextOutsideRoot]);
}

#[test]
fn empty_document_has_no_root() {
    let result = parse("");
    assert_eq!(codes(&result), vec![DiagnosticCode::NoRootElement]);

    let result = parse("<!-- just a comment -->");
    assert_eq!(codes(&result), vec![DiagnosticCode::NoRootElement]);
}

#[test]
fn unfinished_start_tag_keeps_tree() {
    // Typical state while typing: incomplete tag at the end of the file.
    let text = "<root><child attr=\"1\">ok</child><unfinish";
    let result = parse(text);
    assert!(codes(&result).contains(&DiagnosticCode::SyntaxError));

    // The tree built so far stays usable.
    let doc = &result.document;
    let root = doc.root_element().expect("root element is kept");
    let child = doc.element(doc.children(root)[0]).unwrap();
    assert_eq!(child.name.local, "child");
    assert!(child.close_tag.is_some());
}

#[test]
fn invalid_entity_is_reported() {
    let result = parse("<a>&nbsp;</a>");
    assert_eq!(codes(&result), vec![DiagnosticCode::InvalidEntity]);

    let result = parse("<a>&amp; &lt; &gt; &quot; &apos;</a>");
    assert!(
        result.diagnostics.is_empty(),
        "predefined entities are allowed"
    );
}

#[test]
fn cdata_comment_pi_doctype_nodes() {
    let text = r#"<?xml version="1.0"?>
<!DOCTYPE root>
<root><![CDATA[<raw>]]><!-- note --><?php echo ?></root>"#;
    let result = parse(text);
    assert!(
        result.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        result.diagnostics
    );

    let doc = &result.document;
    let root = doc.root_element().unwrap();
    let kinds: Vec<_> = doc
        .children(root)
        .iter()
        .map(|&id| doc.kind(id).clone())
        .collect();
    assert!(matches!(kinds[0], NodeKind::CData));
    assert!(matches!(kinds[1], NodeKind::Comment));
    match &kinds[2] {
        NodeKind::ProcessingInstruction { target } => assert_eq!(target, "php"),
        other => panic!("expected PI, found: {other:?}"),
    }
    // The DOCTYPE sits at the top level.
    assert!(doc
        .children(doc.root())
        .iter()
        .any(|&id| matches!(doc.kind(id), NodeKind::Doctype)));
}

#[test]
fn node_at_offset_finds_deepest_element() {
    let text = "<a><b>hallo</b></a>";
    let result = parse(text);
    let doc = &result.document;
    // Offset 8 is inside the text "hallo".
    let node = doc.node_at_offset(8);
    assert!(matches!(doc.kind(node), NodeKind::Text));
    let b = doc.parent(node).unwrap();
    assert_eq!(doc.element(b).unwrap().name.local, "b");
}

#[test]
fn large_document_parses() {
    // Not a benchmark, just a plausibility test for many nodes.
    let mut text = String::from("<root>");
    for i in 0..10_000 {
        text.push_str(&format!("<item id=\"{i}\"><name>Number {i}</name></item>"));
    }
    text.push_str("</root>");
    let result = parse(&text);
    assert!(result.diagnostics.is_empty());
    let doc = &result.document;
    let root = doc.root_element().unwrap();
    assert_eq!(doc.children(root).len(), 10_000);
}
