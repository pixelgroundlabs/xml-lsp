# xml-lsp

A modern language server (LSP 3.17) for XML written in Rust, focused on XML
languages described by Relax NG (the speedata Publisher layout language
being the first target). Initially developed for Zed, but
editor-independent — the server works with any LSP client.

## Status

Milestone 7 (in progress): document symbols (hierarchical outline with
identifying attributes as details) and folding ranges (elements,
comments, CDATA). Milestone 6: hover — schema documentation for the element, attribute,
or attribute value under the cursor as Markdown, including the allowed
values of enumerated attributes; the documentation language follows the
configured schema variant. Before that, milestone 5: schema-driven
completion — context-aware proposals for
elements (only what the content model allows at the cursor), attributes
(required ones first, already-present ones filtered out), attribute
values (enumerations), and closing tags, each carrying the documentation
from the schema; snippet support when the client announces it. Earlier
milestones: language activation via xml-model PIs, profiles, and XML
catalogs with live validation (M4), derivative-based validation with
error recovery (M3), the schema model with a RELAX NG frontend (M2), and
the LSP server with error-resilient parsing and incremental
synchronization (M1). See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for details and
[docs/ROADMAP.md](docs/ROADMAP.md) for the milestone plan.

## Building and Testing

```sh
cargo build            # binary: target/debug/xml-lsp
cargo test             # unit, integration, and snapshot tests
cargo insta review     # approve changed diagnostics snapshots
```

The server communicates over stdin/stdout.

**Zed:** install the dev extension from [`editors/zed`](editors/zed/)
first — Zed only launches language servers registered by an extension
(see the instructions there). Then configure the server in
`settings.json`, using the speedata Publisher schemas via their XML
catalog:

```json
{
  "lsp": {
    "xml-lsp": {
      "binary": { "path": "/path/to/xml-lsp" },
      "initialization_options": {
        "diagnostics": { "enable": true, "maxCount": 1000 },
        "schema": {
          "catalogs": ["/path/to/publisher/schema/catalog-schema-en.xml"],
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
    }
  }
}
```

Either mechanism alone is enough: catalogs map namespaces to schemas
(`catalog-schema-de.xml` for German documentation), while language
profiles add activation by root element, file name, or extension. A
document can also pin its schema itself with
`<?xml-model href="schema.rng"?>`, which takes precedence.

## Layout

* `crates/xml-ast` — syntax tree, position mapping (UTF-16), namespaces
* `crates/xml-parser` — error-tolerant parser with diagnostics
* `crates/xml-schema` — schema model with RELAX NG frontend
* `crates/xml-validation` — derivative-based validation engine
* `crates/xml-languages` — language registry: activation and schema association
* `crates/xml-completion` — schema-driven completion
* `crates/xml-hover` — schema documentation under the cursor
* `crates/xml-server` — LSP server (tower-lsp), binary `xml-lsp`

The tests against the real speedata schemas look for them in
`SPEEDATA_SCHEMA_DIR` (default: `~/work/software/publisher/schema`) and
are skipped if the directory is missing.

The parser and analysis are deliberately LSP-free; all LSP features access
only their internal APIs.
