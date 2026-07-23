# Architecture

Status: milestone 2 — schema model with RELAX NG frontend, on top of the
milestone 1 LSP server with parsing and diagnostics.

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
  paragraphs are joined). Foreign elements such as embedded Schematron
  rules are skipped. `include`/`externalRef`/`parentRef` are reported as
  unsupported for now — the speedata schemas do not use them.
* **`query`** — context-free traversals used for schema browsing:
  child elements, attributes, and value enumerations reachable from a
  content pattern, plus element lookup by qualified name and
  documentation access. Context-aware variants (which of these are valid
  *at this document position*) arrive with the validation engine.

The speedata layout schemas load cleanly: both language variants share
the namespace `urn:speedata.de:2009/publisher/en` and differ only in the
language of their documentation strings.

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
* **`config`** — `ServerConfig`, read from `initializationOptions` and
  `workspace/didChangeConfiguration` (accepts `{ "xml": … }` or the
  options at the top level). Currently: diagnostics on/off and a per-
  document limit.
* **`backend`** — wires everything together; implemented messages:
  `initialize`, `initialized`, `shutdown`, `didOpen`, `didChange`,
  `didClose`, `didChangeConfiguration`, push diagnostics via
  `textDocument/publishDiagnostics`. Important: cache references are never
  held across an await point (deadlock avoidance with DashMap).

## Tests

* Unit tests in every module (LineIndex/UTF-16, QName, tree, attribute
  scanner, config, diagnostics conversion).
* Integration tests: parser (well-formedness, namespaces, recovery cases)
  and server (didOpen/didChange/didClose lifecycle, UTF-16 positions,
  full-document sync fallback).
* Snapshot tests (insta) for diagnostics: wording, codes, and positions;
  changes must be approved deliberately via `cargo insta review`.

## Planned Structure (Roadmap)

Upcoming crates — they are only created once they contain a real
implementation (no placeholders):

* **Validation engine** (next) — derivative-based matching of document
  trees against the schema model, producing diagnostics; also the basis
  for context-aware completion.
* **`xml-languages`** — registry of XML languages with extensible
  activation rules (file name, extension, root element, namespace), schema
  association, optional reference resolution, and quick fixes. New
  languages are added without changes to the core.
* Further feature modules in the server, in this order: completion, hover,
  document symbols, folding ranges, definition/references, rename, code
  actions, semantic tokens, inlay hints.

## Performance Notes

* The tree stores ranges instead of text copies; namespace URIs are
  interned.
* Rebuilding the `LineIndex` and full reparsing per change are the known
  cost centers for very large files (>20 MB). Planned steps: rope-based
  text storage, incremental `LineIndex`, reparsing only the affected
  subtree. The interfaces (`DocumentState`, `ParseResult`) are cut so that
  these optimizations stay local.
