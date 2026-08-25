# Changelog

## 0.2.0 — 2026-08-25

- Expand the typed schema directly from `syntax!`; generated Rust files and the
  generator binary are no longer part of the workflow.
- Rename the token-to-syntax backend trait to `Parser` and simplify
  `ParseStream` to shared tokens plus a cursor position.
- Preserve token and delimiter boundary spans through native parsing and
  Tree-sitter reconstruction.
- Retain qualified names as one identifier while exposing optional package and
  decimal-index subspans on `Symbol`; apply exact `Core$word` aliases once.
- Keep precision and exponent suffixes in one typed float consistently across
  both parser backends.
- Correct quote-specifier state, cell parsing, edit ranges, and adjacent span
  intersections.
- Add recovering native parsing and retained incremental Tree-sitter parsing
  for editor-style incomplete source updates.
- Align both backends for `new` installations, literal delimiters and escapes,
  empty components, source-aware span identity, and quoted punctuation boundaries.
- Remove redundant source-provider, EOF-token, bespoke punctuation-iterator,
  and parser compatibility layers; use one token description and delimiter
  decoder.
- Document the complete public and macro-generated API, enforced by the
  workspace warning gate.

## 0.1.0 — 2026-08-18

First published release.

- Typed syntax graph generated from one `syntax!` declaration in `src/nodes.rs`,
  covering the full Macaulay2 expression, statement, and collection grammar.
- Dual parser backends: `NativeParser` (a direct precedence parser over the
  shared token cursor) and `TreeSitterParser` (reconstruction from
  `tree-sitter-macaulay2` via `ExternalCstNode`/`Reconstruct`).
- Generated `Visit`, `VisitMut`, and `Fold` traversal traits with default
  walkers for every node and category.
- `quote_m2!`/`parse_quote_m2!` for constructing and parsing typed M2 syntax
  from Rust.
- `ParsedFile` source-to-source pipeline, and a pretty-tree printer
  (`cargo run --example inspect`).
