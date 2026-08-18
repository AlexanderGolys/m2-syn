# Changelog

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
