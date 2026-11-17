# Parsing source

Choose the entry point that matches the input you have:

| Input and purpose | Entry point | Required feature |
| --- | --- | --- |
| Source with an owned tree and convenient output | `ParsedFile::from_source_native` | none |
| Source to a typed `nodes::SourceFile` | `parse_native` | none |
| Source through Tree-sitter | `parse_file` or `ParsedFile::from_source` | `tree-sitter` |
| Quoted tokens through Tree-sitter | `parse_tokens` | `tree-sitter` |
| A typed fragment from tokens | `parse2::<T>` | none |
| A typed fragment from source | `parse_fragment_str::<T>` | none |
| Framed cells with an explicit backend | `parse_with` | depends on backend |

## Whole files

Both backends produce the same owned syntax types. Once parsing finishes,
traversal and source emission use that tree independently of the backend.

```rust
use m2_syn::{Lex, parse_native};

let file = parse_native("x + 1\ny + x * 2")?;
assert_eq!(file.cells.len(), 2);
let emitted = file.to_m2();
assert_eq!(parse_native(&emitted)?.to_m2(), emitted);
# Ok::<(), Box<dyn std::error::Error>>(())
```

With the default features, `parse_file` selects Tree-sitter instead. Retain a
`TreeSitterParser` when you want to reuse that backend across calls.

For applications that edit source, `ParsedFile` supplies `cst()` and `cst_mut()`
to access the tree, `original_source()` to inspect the retained input, and
`to_source()` to emit the current tree. Token, cell, and pretty-print projections
are computed from the current tree, so they reflect subsequent edits.

## Fragments and parser backends

`Parse` belongs to the type being read. `parse2::<T>` requires the entire input
to be consumed; `parse_fragment_str::<T>` first lexes source and then performs
that same complete parse. `parse_quote_m2!` combines quoting with type-inferred
fragment parsing.

`Parser<T>` belongs to a backend. Its `parse_cells` method accepts a
`CellStream` and returns `T`, which defaults to `nodes::SourceFile`:

```rust
use m2_syn::{NativeParser, Tokenize, lex_str, parse_with};

let tokens = lex_str("f(x); y = 2")?;
let cells = tokens.to_cell_stream()?;
let file = parse_with(&mut NativeParser, cells)?;
assert_eq!(file.cells.len(), 2);
# Ok::<(), Box<dyn std::error::Error>>(())
```

An external parser can implement `ExternalCstNode` and use `Migrate` for
schema-compatible nodes. A backend whose tree has a different shape must lower
those differences explicitly. Tree-sitter's adapter does this for assignments,
control forms, packs, and delimiters; it does not route parsing through the
native backend.

## Incomplete input and recovery

The strict entry points return an error when they cannot construct the requested
syntax. Recovery is explicit: `parse_native_recovering` and, with Tree-sitter,
`parse_file_recovering` return a `Recovered` value with syntax and diagnostics
when recovery succeeds. Native recovery can supply missing closing delimiters
at end of input; other lexical or grammatical failures can still be fatal.

For interactive input, choose the parser that matches your editing model:

| Parser | Use |
| --- | --- |
| `NativeCellParser` | Submit lines and collect completed cells |
| `NativeRealtimeParser` | Retain a document and apply insertions or deletions |
| `RealtimeParser` | Retain Tree-sitter parsing state; requires `tree-sitter` |

`NativeRealtimeParser` exposes each cell's state and diagnostics. A confirmed
newline or semicolon provides a restart boundary. An EOF-only boundary remains
speculative, so appending text can reopen the final cell. Earlier edits reframe
forwards until an existing boundary is restored, allowing the remaining suffix
to be reused.

Cell spans refer to their own immutable source snapshots. Use a retained cell's
`document_range()` when you need its position in the current document.

## Inspecting a parse

`ParsedFile::pretty_tokens()` prints the lexical structure, while `pretty_cst()`
prints the typed tree. `print_pretty()` combines the original source and both
views, with terminal-aware coloring. `PrettyReport` and `PrettyOptions` expose
more control over ranges, comments, and styling.

The repository includes a command-line inspector:

```sh
cargo run --example inspect
cargo run --example inspect -- path/to/file.m2
```
