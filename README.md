# lsp-speedata

A modern language server (LSP 3.17) for XML written in Rust, focused on XML
languages described by Relax NG. Initially developed for Zed, but
editor-independent — the server works with any LSP client.

## Status

Milestone 3: schema validation via James Clark's derivative algorithm —
documents are checked against the schema model with error recovery and
precise, suggestion-bearing diagnostics (typos get "did you mean"-style
expected lists). Earlier milestones: the schema model with a RELAX NG
frontend that loads the speedata Publisher layout schemas completely
(M2), and the LSP server with error-resilient parsing, incremental
synchronization, and well-formedness diagnostics (M1). See
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for details.

## Building and Testing

```sh
cargo build            # binary: target/debug/xml-lsp
cargo test             # unit, integration, and snapshot tests
cargo insta review     # approve changed diagnostics snapshots
```

The server communicates over stdin/stdout. Example configuration for Zed
(`settings.json`):

```json
{
  "lsp": {
    "xml-lsp": {
      "binary": { "path": "/path/to/xml-lsp" },
      "initialization_options": {
        "diagnostics": { "enable": true, "maxCount": 1000 }
      }
    }
  }
}
```

## Layout

* `crates/xml-ast` — syntax tree, position mapping (UTF-16), namespaces
* `crates/xml-parser` — error-tolerant parser with diagnostics
* `crates/xml-schema` — schema model with RELAX NG frontend
* `crates/xml-validation` — derivative-based validation engine
* `crates/xml-server` — LSP server (tower-lsp), binary `xml-lsp`

The tests against the real speedata schemas look for them in
`SPEEDATA_SCHEMA_DIR` (default: `~/work/software/publisher/schema`) and
are skipped if the directory is missing.

The parser and analysis are deliberately LSP-free; all LSP features access
only their internal APIs.
