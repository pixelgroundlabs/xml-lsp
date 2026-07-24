//! Integration tests for schema-driven completion. The `|` marker in the
//! test input marks the cursor.

use xml_completion::{complete, CompletionItem, CompletionItemKind};
use xml_schema::{load_rng, Schema};

const LIBRARY: &str = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0"
    xmlns:a="http://relaxng.org/ns/compatibility/annotations/1.0">
  <start><ref name="library"/></start>
  <define name="library">
    <element name="library">
      <a:documentation>A collection of books.</a:documentation>
      <oneOrMore><ref name="book"/></oneOrMore>
    </element>
  </define>
  <define name="book">
    <element name="book">
      <a:documentation>One book.</a:documentation>
      <attribute name="id">
        <a:documentation>Unique identifier.</a:documentation>
      </attribute>
      <optional>
        <attribute name="lang">
          <choice>
            <value>de<a:documentation>German</a:documentation></value>
            <value>en</value>
          </choice>
        </attribute>
      </optional>
      <element name="title"><text/></element>
      <optional><element name="author"><text/></element></optional>
    </element>
  </define>
</grammar>"#;

fn schema() -> Schema {
    let result = load_rng(LIBRARY);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    result.schema
}

/// Runs completion at the `|` marker.
fn complete_at(marked: &str) -> Vec<CompletionItem> {
    let offset = marked.find('|').expect("marker in input");
    let text = marked.replacen('|', "", 1);
    let schema = schema();
    let parse = xml_parser::parse(&text);
    complete(&schema, &parse.document, &text, offset)
}

fn labels(items: &[CompletionItem]) -> Vec<String> {
    items.iter().map(|i| i.label.clone()).collect()
}

#[test]
fn root_element_completion() {
    let items = complete_at("<|");
    assert_eq!(labels(&items), vec!["library"]);
    assert_eq!(items[0].kind, CompletionItemKind::Element);
    assert_eq!(
        items[0].documentation.as_deref(),
        Some("A collection of books.")
    );
}

#[test]
fn child_element_completion_respects_position() {
    // Inside an empty <book>: first expected child is <title>.
    let items = complete_at(r#"<library><book id="1"><|</book></library>"#);
    assert_eq!(labels(&items), vec!["title"]);

    // After <title>: only <author> remains.
    let items = complete_at(r#"<library><book id="1"><title>T</title><|</book></library>"#);
    assert_eq!(labels(&items), vec!["author"]);
}

#[test]
fn element_completion_with_typed_prefix() {
    // The typed fragment breaks the parse; completion still works and
    // the replace range covers the fragment.
    let marked = r#"<library><book id="1"><tit|</book></library>"#;
    let offset = marked.find('|').unwrap();
    let items = complete_at(marked);
    assert_eq!(labels(&items), vec!["title"]);
    let replace = items[0].replace;
    assert_eq!(replace.start, offset - 3);
    assert_eq!(replace.end, offset);
}

#[test]
fn attribute_completion_excludes_present_attributes() {
    // id is present, so only lang remains; typing position is fresh
    // whitespace inside the tag.
    let items = complete_at(r#"<library><book id="1" |>"#);
    assert_eq!(labels(&items), vec!["lang"]);
    assert_eq!(items[0].kind, CompletionItemKind::Attribute);
    assert_eq!(items[0].insert, "lang=\"\"");
    assert_eq!(items[0].snippet.as_deref(), Some("lang=\"$1\""));
    assert!(!items[0].required);
}

#[test]
fn required_attributes_are_marked() {
    let items = complete_at(r#"<library><book |>"#);
    let id = items.iter().find(|i| i.label == "id").expect("id offered");
    assert!(id.required);
    assert_eq!(id.documentation.as_deref(), Some("Unique identifier."));
    let lang = items.iter().find(|i| i.label == "lang").unwrap();
    assert!(!lang.required);
}

#[test]
fn attribute_completion_while_typing_a_name() {
    // The half-typed attribute (parsed as attribute-without-value) is
    // excluded from the "already present" set and its range is replaced.
    let items = complete_at(r#"<library><book la|>"#);
    let lang = items
        .iter()
        .find(|i| i.label == "lang")
        .expect("lang offered");
    assert_eq!(lang.replace.len(), 2, "replaces the typed fragment");
    // id is still offered too (not yet present).
    assert!(items.iter().any(|i| i.label == "id"));
}

#[test]
fn attribute_value_completion() {
    let items = complete_at(r#"<library><book id="1" lang="|">"#);
    assert_eq!(labels(&items), vec!["de", "en"]);
    assert_eq!(items[0].kind, CompletionItemKind::Value);
    assert_eq!(items[0].documentation.as_deref(), Some("German"));

    // With a typed value the whole value is the replace range.
    let marked = r#"<library><book id="1" lang="d|e">"#;
    let items = complete_at(marked);
    assert_eq!(labels(&items), vec!["de", "en"]);
    assert_eq!(items[0].replace.len(), 2);
}

#[test]
fn close_tag_completion() {
    let items = complete_at(r#"<library><book id="1"><title>T</title></|"#);
    assert_eq!(labels(&items), vec!["book"]);
    assert_eq!(items[0].insert, "book>");
    assert_eq!(items[0].kind, CompletionItemKind::CloseTag);
}

#[test]
fn fresh_element_gets_full_skeleton() {
    // <book> has a required id, an optional lang, and element content:
    // required attribute pre-filled, end tag included.
    let items = complete_at("<library><boo|");
    let book = items.iter().find(|i| i.label == "book").expect("book");
    assert_eq!(book.insert, "book id=\"\"></book>");
    assert_eq!(
        book.snippet.as_deref(),
        Some("book id=\"${1}\"${2}>\n  $0\n</book>"),
        "attribute stop in the start tag, then the indented block"
    );

    // <title> has no attributes but text content.
    let items = complete_at(r#"<library><book id="1"><ti|"#);
    let title = items.iter().find(|i| i.label == "title").expect("title");
    assert_eq!(title.insert, "title></title>");
    assert_eq!(title.snippet.as_deref(), Some("title>$0</title>"));
}

#[test]
fn empty_element_completes_self_closing() {
    let schema_src = r#"<grammar xmlns="http://relaxng.org/ns/structure/1.0">
  <start>
    <element name="root">
      <optional><element name="plain"><empty/></element></optional>
      <optional>
        <element name="tunable">
          <optional><attribute name="level"/></optional>
          <empty/>
        </element>
      </optional>
    </element>
  </start>
</grammar>"#;
    let result = xml_schema::load_rng(schema_src);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    let marked = "<root><|</root>";
    let offset = marked.find('|').unwrap();
    let text = marked.replacen('|', "", 1);
    let parse = xml_parser::parse(&text);
    let items = complete(&result.schema, &parse.document, &text, offset);

    // No attributes at all: cursor ends up after the tag.
    let plain = items.iter().find(|i| i.label == "plain").expect("plain");
    assert_eq!(plain.insert, "plain />");
    assert_eq!(plain.snippet, None);

    // Optional attributes: the cursor stops before `/>`.
    let tunable = items
        .iter()
        .find(|i| i.label == "tunable")
        .expect("tunable");
    assert_eq!(tunable.insert, "tunable />");
    assert_eq!(tunable.snippet.as_deref(), Some("tunable $0/>"));
}

#[test]
fn cursor_in_existing_tag_completes_name_only() {
    // The cursor sits in the name of an intact start tag — completion
    // must replace the whole name and nothing else.
    let marked = r#"<library><boo|k id="1"><title>T</title></book></library>"#;
    let offset = marked.find('|').unwrap();
    let items = complete_at(marked);
    let book = items.iter().find(|i| i.label == "book").expect("book");
    assert_eq!(book.insert, "book");
    assert_eq!(book.snippet, None);
    assert_eq!(book.replace.start, offset - 3);
    assert_eq!(book.replace.end, offset + 1, "covers the rest of the word");
}

/// Applies an item to the text (replace range → insert).
fn apply(marked: &str, item: &CompletionItem) -> String {
    let text = marked.replacen('|', "", 1);
    let mut out = text.clone();
    out.replace_range(item.replace.start..item.replace.end, &item.insert);
    out
}

#[test]
fn close_tag_completion_consumes_autopaired_bracket() {
    // Editors auto-pair `<` with `>`: typing `</` inside <book> yields
    // `</|>`. Accepting the close-tag item must not double the `>`.
    let marked = r#"<library><book id="1"><title>T</title></|></library>"#;
    let items = complete_at(marked);
    assert_eq!(labels(&items), vec!["book"]);
    assert_eq!(
        apply(marked, &items[0]),
        r#"<library><book id="1"><title>T</title></book></library>"#
    );
}

#[test]
fn element_skeleton_consumes_autopaired_bracket() {
    // `<boo|>` with the auto-paired `>`: the skeleton replaces it.
    let marked = r#"<library><boo|></library>"#;
    let items = complete_at(marked);
    let book = items.iter().find(|i| i.label == "book").expect("book");
    assert_eq!(
        book.snippet.as_deref(),
        Some("book id=\"${1}\"${2}>\n  $0\n</book>"),
        "attribute stop in the start tag, then the indented block"
    );
    assert_eq!(
        apply(marked, book),
        r#"<library><book id=""></book></library>"#
    );
}

#[test]
fn no_completion_in_plain_text() {
    let items = complete_at(r#"<library><book id="1"><title>Tex|t</title></book></library>"#);
    assert!(items.is_empty(), "{items:?}");
}

#[test]
fn completion_in_invalid_document_still_works() {
    // <book> is misplaced (directly under book), but attribute
    // completion falls back to the schema-wide definition.
    let items = complete_at(r#"<library><book id="1"><book |</library>"#);
    assert!(items.iter().any(|i| i.label == "id"), "{items:?}");
}
