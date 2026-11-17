# Introduction

`m2-syn` provides Rust types for Macaulay2 syntax. It can parse source into an
owned concrete syntax tree, walk or edit that tree, and emit M2 source again.
The same types serve both the native parser and the optional Tree-sitter
backend.

A concrete syntax tree retains syntax that matters when writing code back out:
operators, delimiters, comma tokens, statement boundaries, and comments. Its
Rust types distinguish an assignment from a function call or a literal, so a
transformation can work with those structures directly.

Add the published crate to your project:

```sh
cargo add m2-syn
```

## Start with a parsed file

`ParsedFile` owns the typed tree and retains the original input for diagnostics.
Use the native parser to get started without configuring a backend:

```rust
use m2_syn::ParsedFile;

let file = ParsedFile::from_source_native("answer = 40 + 2")?;
assert_eq!(file.cst().cells.len(), 1);
assert_eq!(file.to_source(), "answer = 40 + 2");
# Ok::<(), Box<dyn std::error::Error>>(())
```

From there, you can [choose a parser](./parsers.md),
[walk and edit syntax](./generated-api.md), or
[construct fragments with quoting](./quoting.md). The later chapters explain
the token pipeline, source locations, and schema used to generate the tree.

## Features and API layout

The default `tree-sitter` Cargo feature enables the alternative parser backend,
including `parse_file`, `parse_tokens`, `ParsedFile::from_source`, and
`RealtimeParser`. The native parser, quoting, traversal, source emission, and
inspection remain available with `default-features = false`.

Concrete syntax types live in `m2_syn::nodes`. Shared types and macros such as
`ParsedFile`, `TokenStream`, `Span`, `quote_m2!`, and `Token!` are exported from
the crate root. Traversal lives in `visit`, `visit_mut`, and `fold`.

The API is experimental. This book describes the workspace API; when using a
published version, consult that version's API reference for exact signatures.
