# xml-lsp

A language server (LSP 3.17) for XML, written in Rust. It understands
XML languages described by a RELAX NG schema and turns the schema into
editor features: validation as you type, context-aware completion,
documentation on hover, navigation, and more. The first target is the
[speedata Publisher](https://www.speedata.de/) layout language, but any
RELAX NG grammar works — new languages are configuration, not code.

## Features

* **Validation** — well-formedness and schema validation on every
  keystroke, with precise positions, stable error codes, and "did you
  mean" suggestions for misspelled elements.
* **Completion** — elements valid *at the cursor position* (not just
  anywhere in the parent), attributes (required first, present ones
  filtered), and attribute values. Accepting a new element inserts the
  whole tag: required attributes as tab stops, `<Options />`
  self-closing for empty elements (cursor before `/>` when optional
  attributes exist), and the closing tag for container elements.
* **Symbol-aware values** — attributes that reference named things
  (`color=`, `fontfamily=`, `select="$…"`, …) complete to the names
  defined in the document. Which attributes define and reference which
  symbols is declared *in the schema* via small annotations — see
  [docs/schema-annotations.md](docs/schema-annotations.md).
* **Hover** — the schema documentation for the element, attribute, or
  value under the cursor, including allowed values.
* **Navigation** — go-to-definition and find-references for annotated
  symbols (`DefineColor` ↔ `color=`, marks ↔ links, variables, …),
  document outline, folding.
* **Rename** — symbols (definition plus all references) and element
  tag pairs (start and end tag together).
* **Quick fixes** — close an unclosed element, remove a stray end tag,
  quote or terminate attribute values, add missing values.
* **Comment toggle** — code actions to comment/uncomment; nested
  comments are escaped (`<!--` ↔ `<!-/-`, `-->` ↔ `-/->`) and restored
  losslessly.
* **Formatting** — whole-document formatting: indentation, empty
  elements become self-closing, blank lines between top-level blocks.
  Per-element conventions (verbatim content, blank-line groups, inline
  text flow) come from `lsp:format` annotations in the schema.
* **Semantic tokens & inlay hints** — symbol references highlighted
  like identifiers; the identifying attribute shown after distant end
  tags (`</Record> element="data"`).

## Installation

```sh
cargo build --release    # binary: target/release/xml-lsp
```

The server speaks LSP over stdin/stdout; any LSP client can use it.

## Editor setup

### Zed

Install the dev extension from [`editors/zed`](editors/zed/) — Zed only
launches language servers registered by an extension (instructions in
that directory). Then point Zed at the binary and your schemas in
`settings.json`:

```json
{
  "lsp": {
    "xml-lsp": {
      "binary": { "path": "/path/to/xml-lsp/target/release/xml-lsp" },
      "initialization_options": {
        "schema": {
          "catalogs": ["/path/to/publisher/schema/catalog-schema-en.xml"]
        }
      }
    }
  }
}
```

### Other editors

Configure your editor's LSP client to run the `xml-lsp` binary for XML
files and pass the configuration shown below as
`initializationOptions`.

## Telling the server about your schema

Three mechanisms, in order of precedence:

1. **`<?xml-model?>`** in the document itself (RELAX NG XML syntax):

   ```xml
   <?xml-model href="schema.rng"?>
   ```

2. **Language profiles** in the configuration — activation by
   namespace, root element, file name, or extension:

   ```json
   {
     "schema": {
       "languages": [
         {
           "name": "speedata-layout",
           "schema": "/path/to/publisher/schema/layoutschema-en.rng",
           "namespace": "urn:speedata.de:2009/publisher/en",
           "rootElement": "Layout",
           "fileNames": ["layout.xml"]
         }
       ]
     }
   }
   ```

3. **OASIS XML catalogs** mapping a namespace to a schema — exactly
   what speedata ships (`catalog-schema-en.xml`; use
   `catalog-schema-de.xml` for German documentation):

   ```json
   { "schema": { "catalogs": ["/path/to/catalog-schema-en.xml"] } }
   ```

Relative paths resolve against the workspace root. Edited schemas are
picked up automatically (mtime check) — no server restart needed.

Further options: `"diagnostics": { "enable": true, "maxCount": 1000 }`.
All options are also accepted via `workspace/didChangeConfiguration`,
either at the top level or wrapped in `{ "xml": … }`.

## Symbol navigation and formatter conventions

Go-to-definition, rename, symbol completion, and per-element formatter
behavior are driven by small annotations in the RELAX NG schema
(namespace `urn:xml-lsp:annotations`), so any grammar can opt in
without touching the server:

```xml
<attribute name="name"><lsp:defines symbol="color"/></attribute>
<attribute name="color"><lsp:references symbol="color"/></attribute>
```

See [docs/schema-annotations.md](docs/schema-annotations.md) for the
vocabulary and the migration guide for the speedata schemas.

## Development

```sh
cargo test             # unit, integration, and snapshot tests
cargo insta review     # approve changed diagnostics snapshots
```

The workspace is split into LSP-independent analysis crates and a thin
server binding — see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for
the crate layout and [docs/ROADMAP.md](docs/ROADMAP.md) for the
milestone plan.

Tests against the real speedata schemas look for them in
`SPEEDATA_SCHEMA_DIR` (default: `~/work/software/publisher/schema`) and
are skipped if the directory is missing.
