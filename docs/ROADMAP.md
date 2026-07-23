# Roadmap

Goal: an editor-independent LSP server (initially for Zed) for XML
languages described by RELAX NG — first target: speedata Publisher
layout files — providing validation, completion, documentation, and
navigation from the schema. Designed so that other RELAX NG grammars
work out of the box and an XSD frontend can be added later without
touching the feature code.

One milestone = one squash commit on `main`.

## Done

- **M1 — Foundation** (`5e6e9e4`): Cargo workspace; error-resilient XML
  parser with namespace resolution and precise positions (UTF-16
  mapping); LSP server with incremental sync and well-formedness
  diagnostics.
- **M2 — Schema model** (`b9718fc`): source-neutral pattern model
  (`xml-schema`) with a RELAX NG frontend; documentation and value
  enumerations from the schema; loads the speedata layout schemas
  completely.
- **M3 — Validation** (`7a79a6d`): James Clark's derivative algorithm
  (`xml-validation`) with hash-consing, memoization, error recovery,
  and suggestion-bearing diagnostics (edit-distance ranking).
- **M4 — Language activation** (`cf0ab3b`): `xml-languages` registry —
  `<?xml-model?>` PIs, scored activation profiles, OASIS XML catalogs;
  schema cache with mtime reload; live validation wired into the
  server.
- **M5 — Completion** (`c7f5951`): context-aware, schema-driven
  completion (`xml-completion`) for elements, attributes (required
  first, present ones filtered), attribute values, and closing tags;
  snippet support; schema documentation on every item.

## Next

- **M6 — Hover**: documentation of the element/attribute/value under
  the cursor as Markdown (language variant follows the configured
  schema); allowed values as additional info. The required queries
  (`element_pattern_at`, `attribute_pattern_at`) already exist.
- **M7 — Navigation & comfort**: document symbols, folding ranges,
  go-to-definition/references (IDs, Marks/links via per-language
  extension points), rename, quick fixes (close tag, quote attribute),
  semantic tokens, inlay hints.
- **M8 — Zed integration**: mostly done early (see `editors/zed`):
  a dev extension registers the `xml-lsp` binary, configurable via
  `lsp.xml-lsp` settings. Remaining: publish to the Zed extension
  registry once the feature set has settled. Other editors need only
  their usual LSP client configuration.

## Later (explicitly deferred)

- XSD frontend mapping onto the same schema model (speedata ships
  generated XSDs from the same source).
- RELAX NG compact syntax (`.rnc`).
- Schematron rules (the speedata schema embeds four).
- `include`/`externalRef`/`parentRef` in the RELAX NG loader (reported
  as unsupported today; unused by the target schemas).
- Incremental reparsing / rope-based text storage for very large
  documents (>20 MB); the interfaces are already cut for it.
- Remote schema locations (http URLs in catalogs/xml-model).
