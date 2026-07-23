# lsp-speedata

A modern language server (LSP 3.17) for XML written in Rust, focused on XML
languages described by Relax NG. Initially developed for Zed, but
editor-independent — the server works with any LSP client.

## Status

Milestone 1: error-resilient XML parsing with namespace resolution, a
document cache with incremental synchronization, and well-formedness
diagnostics over LSP. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for
details.

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
* `crates/xml-server` — LSP server (tower-lsp), binary `xml-lsp`

The parser and analysis are deliberately LSP-free; all LSP features access
only their internal APIs.
