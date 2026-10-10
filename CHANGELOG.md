# Changelog

## Unreleased

- Give the end-of-file marker the span of the layout that closes its input, so
  a parsed file emits the source byte for byte, including a final newline and
  any trailing whitespace that belongs to no cell. `Source::trailing_whitespace`
  exposes that range, and emission skips a marker that no longer follows the
  final cell, which keeps it last and inert after nodes are appended.
- Place the tree-sitter end-of-file marker at the physical end of the input
  rather than at the start of the final cell's end-of-cell boundary.
- Retain unfinished ordinary and raw string literals as typed syntax in both
  realtime parsers until their closing delimiter arrives.

## 0.5.0 — 2026-09-04

- Expand each typed node through its own `strutuct!` invocation while keeping
  the shared `Visit`, `VisitMut`, and `Fold` method inventories in one outer
  `syntax!` expansion, making generated nodes substantially easier for
  rust-analyzer to locate and display.
- Generate the default traversal functions directly inside the public
  `visit`, `visit_mut`, and `fold` modules so consumers can call them without
  going through a trait method.
- Make traversal and pretty-tree support unconditional and retain only the
  optional `tree-sitter` Cargo feature.
- Update `tree-sitter-macaulay2` to 6.1.2 and align typed reconstruction with
  its current grammar, including nested lambda blocks, `else continue` inside
  list comprehensions, and newline-sensitive `lib | name | if` forms found in
  the Macaulay2 library corpus.
- Treat a comma-free cell as its contained expression rather than an
  `ExprPack`; packs now require an actual comma.
- Simplify the type-inference example to demonstrate visitor dispatch and a
  small set of expression result rules without inventing an unsupported type
  hierarchy or solver.
- Upgrade `these-macros-should-be-illegal` to 0.7 and use explicit nested type
  names where the typed schema requires stable public names.

### Migration

- Remove `pretty` and `walk` from downstream feature lists. Their APIs are now
  always available; use `default-features = false` only to omit Tree-sitter.
- Code that deliberately matched a singleton `ExprPack` should match the
  contained expression instead. `ExprPack` now represents comma-separated
  syntax only.
- Custom syntax declarations continue to use `syntax!`, but their nested
  `strutuct!` nodes and per-node traversal implementations now expand in later,
  independent macro-expansion stages.

## 0.4.0 — 2026-08-29

- Replace the flat generated CST schema with explicit, arbitrarily nested
  `strutuct!` declarations whose product and sum layers define semantic visitor
  specializations such as operator, assignment, lambda, member-access, and
  indexing families.
- Make syntax containers transparent in semantic presentation and dispatch:
  delimiter boundaries are presented as `Delimiter` leaves, while `Delimited`,
  `Punctuated`, and ordinary vectors remain storage rather than CST node kinds.
- Represent adjacency as a binary operator and add explicit prefix, binary,
  postfix, augmented, member-access, and index-expression specializations.
- Introduce `CellTokenStream`, `CellTokenTree`, and `CellTokenGroup` as the
  parser-facing framed-token layer while retaining the same lexical atoms and
  recursively removing inferred adjacency when projecting back to tokens.
- Require every `Punctuated` value to contain a real separator, keep singleton
  values outside the wrapper, and replace `PunctPair` with the symmetric
  `SeparatedPair` projection.
- Distinguish variadic lambda arguments from delimited argument lists and use an
  argument pack only when a comma is present.
- Canonicalize `Token!` spellings across quoted and unquoted fragments, allowing
  equivalent punctuation to be written with arbitrary Rust token spacing.
- Replace named precedence declarations with the compact numeric operator table
  and generate operator categories directly from each token's roles.
- Add the `ProcMacro` expansion interface and the `LiteralReturnTypes` typed-tree
  transformation example.
- Improve native parsing, Tree-sitter migration, source reconstruction, and
  pretty-tree output for the expanded semantic hierarchy.

### Migration

- Update imports and constructors from `PunctPair<L, P, R>` to
  `SeparatedPair<T, P>`; the two values now deliberately have one shared type.
- Do not construct `Punctuated` for a singleton value. Use its surrounding
  direct-value variant and construct `Punctuated` only after the first
  punctuation/value pair exists.
- Update matches, visitors, folds, and constructors for generated syntax nodes
  to the new nested semantic categories. Generated node names and enum paths
  from 0.3 are not source compatible with 0.4.
- Code consuming framed cells should use `CellTokenStream` and `CellTokenTree`;
  use `to_token_stream` or `into_token_stream` when a lexical projection is
  required.

## 0.3.0 — 2026-08-27

- Split native processing into lexical `TokenStream` construction and
  cell-aware `Tokenize`, with structural cell boundaries and syntactic
  adjacency markers.
- Add `NativeRealtimeParser`, which reparses only the affected cell suffix and
  distinguishes incomplete input from invalid syntax for interactive shells.
- Preserve physical EOF, source identity, delimiter state, and transformed
  cell contents across native and Tree-sitter parsing.
- Make `pretty`, `tree-sitter`, and `walk` independent Cargo features, and
  expose the renderer-independent `PrettyNode` tree for downstream UIs.
- Place generated nodes under `m2_syn::nodes` while keeping construction,
  quoting, parsing, and schema macros at the crate root.
- Export `quote_m2!` through `m2-syn`, leaving `m2-syn-macros` as the internal
  procedural-macro implementation crate.
- Make exported `syntax!` expansions follow the features enabled on `m2-syn`
  without depending on feature names declared by downstream crates.
- Add explicit `#[cell_stream]` schema metadata and preserve feature-reduced
  builds and documentation tests.
- Coalesce complete-line `--` documentation blocks and retain outer and inner
  comments on typed cells or the source-file root across both parser backends.

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
