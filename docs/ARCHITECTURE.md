# Architecture

Status: milestone 7 plus editing comfort — navigation (document
symbols, folding, definition/references, rename), quick fixes,
semantic tokens, inlay hints, symbol completion, tag-skeleton
completion, document formatting, and comment toggling; after hover
(M6), completion (M5), language activation (M4), the validation engine
(M3), the schema model (M2), and the LSP server with parsing and
diagnostics (M1).

## Overview

```
┌────────────────────────────────────────────────────────┐
│ xml-server (binary: xml-lsp)                           │
│  backend      LanguageServer implementation (LSP 3.17) │
│  documents    document cache, incremental text sync    │
│  diagnostics  parser diagnostics → LSP diagnostics     │
│  config       initializationOptions / configuration    │
└──────┬──────────────────────────────┬──────────────────┘
       │ uses internal APIs only      │
┌──────▼────────────────────┐  ┌──────▼──────────────────┐
│ xml-parser                │  │ xml-schema              │
│  parser   error-tolerant  │  │  model   pattern algebra│
│           event loop      │  │  rng     RELAX NG loader│
│  attrs    attribute       │  │  query   schema browsing│
│           scanner         │  │  (XSD frontend planned) │
│  diagnostics              │  └──────┬──────────────────┘
└──────┬────────────────────┘         │
       │              ┌───────────────▼─────┐
       └──────────────▶ xml-ast             │
                      │  tree    arena tree │
                      │  qname   QName + NS │
                      │  text    TextRange, │
                      │          LineIndex  │
                      │  escape  entities   │
                      └─────────────────────┘
```

Ground rule: **dependencies point downwards only.** `xml-ast` and
`xml-parser` know nothing about LSP types or tower-lsp; all LSP features in
the server access only their internal APIs. This keeps the parser and
analysis editor-independent, testable, and reusable.

## Crates

### `xml-ast`

The LSP-independent core representation:

* **`TextRange`** — half-open byte ranges `[start, end)` in the source
  text. The whole tree works with byte offsets; lines/columns only come
  into play at the LSP boundary.
* **`LineIndex`** — conversion between byte offsets and line/column.
  Columns are counted in UTF-16 code units (the LSP's default position
  encoding). Invalid positions are clamped as required by the spec.
* **`QName`** — prefix, local name, and resolved namespace. Namespace URIs
  are `Arc<str>` and interned during parsing because they repeat massively
  in every document.
* **`Document`** — arena tree (`Vec<NodeData>` with `NodeId` handles).
  Node kinds: root (synthetic), element, text, CDATA, comment, processing
  instruction, doctype. Text contents are not copied; every node only
  knows its range in the source text. Only attribute values and names are
  materialized. `node_at_offset` returns the deepest node at a cursor
  position (the basis for completion/hover).

### `xml-parser`

Error-tolerant parser built on quick-xml:

* quick-xml only provides the **event stream** (tag boundaries, text,
  CDATA, …). End-tag checking is disabled (`check_end_names = false`,
  `allow_unmatched_ends = true`); we do the matching ourselves so parsing
  can continue on broken documents.
* **Error strategy:**
  * An end tag matching an element further out → open elements in between
    are closed automatically (`unclosed-element`), the outer element is
    closed properly.
  * An end tag matching nothing at all → `stray-end-tag`, skipped.
  * Open elements at the end of the file → `unclosed-element`.
  * A fatal syntax error (e.g. an incomplete tag while typing) →
    `syntax-error`; the tree built so far is kept and stays usable.
* **Custom attribute scanner** (`attrs.rs`): quick-xml does not report
  positions for individual attributes. For completion, hover, and precise
  diagnostics we scan the tag content ourselves, yielding exact ranges for
  attribute names and values, plus tolerant behavior for missing values,
  missing quotes, and the like.
* **Namespace resolution** during parsing via a scope stack; declarations
  (`xmlns`, `xmlns:p`) are stored per element. Unknown prefixes →
  `undefined-namespace-prefix`.
* **Well-formedness checks:** duplicate attributes (raw and expanded),
  multiple root elements, text outside the root element, missing root
  element, invalid entity references.
* Whitespace-only text nodes are not added to the tree (saves space in
  large, formatted files).

All diagnostics carry stable codes (`unclosed-element`, …) that will later
serve as anchors for quick fixes.

### `xml-schema`

The central schema model and its frontends. Everything the server knows
about an XML language — allowed elements, attributes, values, and their
documentation — lives in this model; validation, completion, and hover
work against it exclusively.

* **`model`** — an arena of patterns (`PatternId` handles) forming RELAX
  NG's pattern algebra: element/attribute with name classes (including
  `nsName`/`anyName` with `except`), choice/group/interleave, cardinality
  wrappers, `value` enumerations, `data` with datatype parameters, and
  named defines with references. Every pattern keeps its source range in
  the schema file (for future navigation), and element, attribute, and
  value patterns carry their documentation. The algebra is source-neutral:
  DTD and most of XSD can be mapped onto it, so additional schema
  languages become new frontends without touching consumers.
* **`rng`** — the RELAX NG frontend (XML syntax). It parses the schema
  with our own error-tolerant XML parser, so schema problems are reported
  with precise positions and a broken schema still yields a partial
  model. Handles `ns`/`datatypeLibrary` inheritance (including the rule
  that unprefixed attribute names carry no namespace), QName prefixes via
  the schema's own `xmlns` declarations, `combine` on start/define,
  implicit groups, `mixed`, and `a:documentation` collection (multiple
  paragraphs are joined). xml-lsp's own annotations
  (`urn:xml-lsp:annotations`: `lsp:defines`/`lsp:references` on
  attributes, `lsp:format` on elements — see
  [schema-annotations.md](schema-annotations.md)) are collected into
  the model; malformed ones warn with code `schema-annotation`. Other
  foreign elements such as embedded Schematron rules are skipped.
  `include`/`externalRef`/`parentRef` are reported as unsupported for
  now — the speedata schemas do not use them.
* **`query`** — context-free traversals used for schema browsing:
  child elements, attributes, and value enumerations reachable from a
  content pattern, plus element lookup by qualified name and
  documentation access. `required_attributes` (mandatory attributes of
  a content pattern) and `allows_content` (can the element hold
  children or text, or is it empty?) drive the tag-skeleton snippets in
  completion. Context-aware variants (which of these are valid *at this
  document position*) live in the validation engine.

The speedata layout schemas load cleanly: both language variants share
the namespace `urn:speedata.de:2009/publisher/en` and differ only in the
language of their documentation strings.

### `xml-validation`

Validation of documents against the schema model, implementing James
Clark's derivative algorithm for RELAX NG.

* **`machine`** — the state machine. Validation states are patterns;
  consuming a document event (start tag, attribute, text, end tag) yields
  the derivative state, `NotAllowed` meaning "invalid here". States are
  hash-consed (structurally equal states share one id), and the
  derivative functions are memoized, so revalidating a document full of
  repeated structures stays cheap; the machine survives across runs and
  keeps its tables. Element content is converted lazily on entry, which
  keeps recursive content models finite. The machine also answers
  expectation queries — which elements, attributes, or text are
  acceptable in a state — used for messages today and for completion
  next.
* **`validator`** — the tree walker with error recovery. Each failed
  derivative becomes one diagnostic with a precise range (element name,
  attribute name/value, text node), and validation continues: unknown
  elements are skipped (their subtrees still checked via name lookup),
  bad values are accepted leniently after reporting, missing required
  attributes are computed by greedily unblocking the start tag. Expected
  element names in messages are ranked by edit distance to the written
  name, so typos get useful suggestions first.
* **`datatypes`** — value comparison (`token` collapse vs. `string`
  exact) and data facets: `pattern` (XSD-style anchored regexes, cached),
  `minLength`/`maxLength`. Unknown types are accepted permissively;
  tightening is additive.

Diagnostics carry stable codes: `element-not-allowed`,
`incomplete-content`, `text-not-allowed`, `invalid-text-value`,
`attribute-not-allowed`, `invalid-attribute-value`,
`missing-attributes`.

### `xml-languages`

Which XML dialect is a document, and which schema applies? The
[`LanguageRegistry`] resolves this with three mechanisms, in order of
precedence:

1. **`<?xml-model?>`** processing instructions in the document (W3C
   schema association); relative hrefs resolve against the document.
   Only RELAX NG XML syntax is accepted — XSD and `.rnc` models are
   recognized and skipped.
2. **Language profiles** with activation rules: namespace, root element,
   file names, extensions. Every matching rule adds to a score (specific
   signals outweigh generic ones), the best-scoring profile wins. New XML
   languages are pure configuration — no core changes.
3. **OASIS XML catalogs** (`<uri name="…" uri="…"/>`), mapping the root
   namespace to a schema; exactly what speedata ships
   (`catalog-schema-en.xml`/`-de.xml`, which also selects the
   documentation language).

The `SchemaRepository` caches loaded schemas by path and reloads them
when the file's mtime changes — editing the schema is picked up on the
next validation without restarting the server.

The crate also hosts the **reference rules** (`references`):
`ReferenceRules::from_schema` collects every annotated symbol site
(element × attribute × kind × role) from the loaded schema's
`lsp:defines`/`lsp:references` annotations — see
[schema-annotations.md](schema-annotations.md). The server contains no
language-specific knowledge; a grammar that wants navigation annotates
its attribute patterns. A `SymbolIndex` built from the rules and the
AST lists every occurrence (definition or reference) with its exact
value range; XPath values are only indexed when they are plain string
literals (`form="xpath-string"`) or plain variable references
(`form="xpath-variable"`, the `$` excluded so rename works on the bare
name). `sites_for` answers "is this element/attribute pair a symbol
site?" — the basis for symbol completion.

### `xml-completion`

Schema-driven completion, independent of LSP types.

* **Cursor context** — a backwards text scan (robust while the tag being
  typed still breaks the parse) combined with the error-tolerant tree
  determines what is being typed: an element name after `<`, a closing
  tag after `</`, an attribute name inside a start tag, or a value inside
  quotes.
* **Expectations** — the validation engine computes the machine state at
  the cursor by consuming everything before it (leniently, since
  documents are invalid most of the time while typing). Element proposals
  therefore respect the position in the content model: after `<title>`,
  only `<author>` is offered, not everything the parent allows somewhere.
  The expectation walker distinguishes sequence semantics for elements
  from the order-independence of attributes.
* **Items** — labels are namespace-aware (prefixes from the document's
  own declarations), attributes come as `name="$1"` snippets with
  required ones marked and already-present ones filtered out (they were
  consumed by the state computation), values list the schema's
  enumerations, and every item carries the `a:documentation` text from
  the schema.
* **Tag skeletons** — accepting a *freshly typed* element inserts the
  whole tag: required attributes as tab stops, `<Foo />` self-closing
  when the schema says the element is empty (final cursor before the
  `/>` if optional attributes exist, after the tag otherwise), and
  `>$0</Foo>` when it takes content. The cursor context distinguishes
  fresh typing from a cursor inside an existing tag (where only the
  name is replaced) — a broken tag that swallowed following text still
  counts as fresh because its parsed name is garbage.

The same expectation API (`content_expectations`,
`attribute_expectations`, `attribute_pattern_at`, `element_pattern_at`
in `xml-validation`) is the basis for hover in the next milestone.

### `xml-hover`

Schema documentation for the symbol under the cursor, independent of
LSP types. Targets are element names (start *and* end tag), attribute
names, and attribute values. Content is the `a:documentation` from the
schema as Markdown; attribute hovers additionally list the allowed
values, and hovering an enumeration value shows that value's own
documentation when it has one. Lookup reuses the expectation API from
`xml-validation` (`element_pattern_at`, `attribute_pattern_at`), so the
result respects the cursor's context in the content model.

### `xml-server`

The LSP server itself (tower-lsp, LSP 3.17):

* **`documents`** — `DocumentStore` (DashMap) keeps text, version,
  `LineIndex`, and parse result per open document. Text changes arrive as
  incremental LSP content changes and are applied via range splicing
  (sync kind: `Incremental`). After every change the document is currently
  reparsed in full — incremental *parsing* is a planned milestone; the
  document cache already isolates the interface for it.
* **`diagnostics`** — pure translation layer `ParseDiagnostic` →
  `lsp_types::Diagnostic` (severity, code, source `xml`, positions via
  `LineIndex`).
* **`symbols`**/**`folding`** — document outline (element hierarchy
  with identifying attributes such as `name`/`id`/`element` as detail)
  and folding ranges (multi-line elements fold up to the line before
  their end tag; comments and CDATA fold entirely). Both are pure AST
  consumers and work without a schema.
* **`navigation`** — go-to-definition and find-references over the
  symbol index from `xml-languages`; built per request (walking the
  tree is cheap compared to reparsing).
* **`rename`** — `prepareRename` + `rename`: a symbol rename updates
  the definition and every reference; a rename on an element name
  updates start and end tag together. New names are validated (XML
  `Name` for tags, quote-safe strings for symbol values).
* **`code_actions`** — quick fixes anchored on the stable parser
  diagnostic codes: insert the missing end tag of an unclosed element,
  remove a stray end tag, quote or terminate an attribute value, add a
  missing value. Also the comment toggle (Comment out / Uncomment):
  inner `<!--`/`-->` are escaped to `<!-/-`/`-/->` and restored, the
  speedata VS Code extension's convention; an empty selection targets
  the current line.
* **`formatting`** — whole-document formatting (algorithm ported from
  the speedata VS Code extension and verified byte-identical against
  it): indentation, self-closing normalization, blank lines between
  top-level sibling groups. Language conventions come from `lsp:format`
  annotations in the schema (`FormatStyle::from_schema`): `preserve`
  (verbatim content), `blank-lines` (blank-line groups inside an
  element), `inline` (single-line text elements below, HTML/MathML
  flow).
* **`semantic_tokens`** — full-document tokens: element names (prefix
  reported separately as `namespace`), attribute names (`xmlns` as
  `namespace`), values, comments, CDATA, PI targets, doctype.
  Attribute values matching a symbol rule become `variable` tokens;
  definitions carry the `definition` modifier, so references and
  definitions are distinguishable in the editor. Multi-line tokens are
  split per line as the spec demands.
* **`inlay_hints`** — after the end tag of an element spanning eight
  or more lines, the identifying attribute of its start tag is shown
  (`</Record> element="data"`), keeping deeply nested layout files
  navigable.
* **`config`** — `ServerConfig`, read from `initializationOptions` and
  `workspace/didChangeConfiguration` (accepts `{ "xml": … }` or the
  options at the top level). Currently: diagnostics on/off and a per-
  document limit.
* **`backend`** — wires everything together; implemented messages:
  `initialize`, `initialized`, `shutdown`, `didOpen`, `didChange`,
  `didClose`, `didChangeConfiguration`, completion, hover,
  documentSymbol, foldingRange, definition, references,
  prepareRename/rename, codeAction, formatting, semanticTokens/full,
  inlayHint, push diagnostics via `textDocument/publishDiagnostics`.
  Completion merges the schema-driven items with symbol-name items
  from the reference rules (the latter work without a schema). On every open/change the backend
  resolves the document's language, loads the schema through the
  repository, and merges well-formedness and validation diagnostics into
  one publish; schema problems surface as a single `schema-load` warning
  instead of failing silently. Relative configuration paths resolve
  against the workspace root. Important: cache references are never held
  across an await point (deadlock avoidance with DashMap).

## Tests

* Unit tests in every module (LineIndex/UTF-16, QName, tree, attribute
  scanner, config, diagnostics conversion).
* Integration tests: parser (well-formedness, namespaces, recovery cases)
  and server (didOpen/didChange/didClose lifecycle, UTF-16 positions,
  full-document sync fallback).
* Snapshot tests (insta) for diagnostics: wording, codes, and positions;
  changes must be approved deliberately via `cargo insta review`.

## Planned Structure (Roadmap)

The server-side feature set (M1–M7 plus editing comfort: symbol
completion, tag skeletons, formatting, comment toggle) is complete.
Next up is publishing the Zed extension (M8); the reference rules in
`xml-languages` are the extension point for additional per-language
navigation (more symbol kinds, more languages) and, later,
configuration-driven rules.

## Performance Notes

* The tree stores ranges instead of text copies; namespace URIs are
  interned.
* Rebuilding the `LineIndex` and full reparsing per change are the known
  cost centers for very large files (>20 MB). Planned steps: rope-based
  text storage, incremental `LineIndex`, reparsing only the affected
  subtree. The interfaces (`DocumentState`, `ParseResult`) are cut so that
  these optimizations stay local.
