# Parser adapters

Generated migration depends only on `ExternalCstNode`.
`TreeSitterNode` adapts Tree-sitter identity, field names, source text, extras,
and ranges to that interface for schema-compatible leaves, operators, and
tokens. Complete-file `TreeSitterParser` lowers Tree-sitter's `muted`, pack,
delimiter, operator, assignment, and control nodes directly into the cleaned
typed graph; it does not re-lex the source or call the native parser. The
external node view ends at the adapter and is never stored in a typed node.

`Parser<T>` is the parser-backend boundary. It receives a `CellStream` and
returns a generated syntax target. `T` defaults to
`SourceFile`, while parsers may implement the trait again for more specific
targets such as `Expr`. Both built-in backends implement this same cell-stream
API. A parser with its own CST can implement `ExternalCstNode` and call `migrate`;
a parser whose native model already matches the generated graph can construct
the target directly.

The target-owned `Parse` trait is the dual of `Lex`: `parse2::<T>` consumes
one complete `TokenStream` as `T`. Generated `Token![..]` atoms, raw token
categories, token streams, boxes, delimiters, comma-punctuated collections, and
terminated bodies implement it. `ParseStream` shares immutable token storage
through the persistent `TokenStream` handle; end-of-input is cursor state rather
than a synthetic token. A `fork` has an independent index and `advance_to`
explicitly commits successful speculation.
`parse_quote_m2!` composes quoting with type-inferred parsing.

`Lex::to_token_stream` is the lexical projection, while `Tokenize` produces
framed cells. The native parser supports the normalized round trip
`CellStream -> SourceFile -> CellStream -> SourceFile`. Generated concrete token
structs are deliberately underscore-prefixed; use `Token![..]`
to name their types and inspect the underlying `TokenTree` after `Lex`
when working with a heterogeneous stream. Delimiter structs follow the same
rule through `Delimiter![..]`.

Cell framing is shared infrastructure rather than a parser side effect.
Semicolons and complete physical lines confirm top-level boundaries; delimiter
groups are framed recursively but their separators never become top-level
boundaries. The result uses `CellTokenTree::Adj` instead of manufacturing empty
punctuation inside the lexical stream.

```rust
# use std::convert::Infallible;
use m2_syn::nodes::{EOF, SourceFile};
use m2_syn::{CellStream, Parser, Span, Tokenize, lex_str, parse_with};

struct EmptyParser;

impl Parser for EmptyParser {
    type Error = Infallible;

    fn parse_cells(&mut self, _cells: CellStream) -> Result<SourceFile, Self::Error> {
        Ok(SourceFile::new(Vec::new(), EOF::new("", Span::for_text(""))))
    }
}

let tokens = lex_str("").unwrap();
let cells = tokens.to_cell_stream().unwrap();
let file = parse_with(&mut EmptyParser, cells).unwrap();
assert!(file.cells.is_empty());
```

`TreeSitterParser` is the built-in implementation and can be retained across
calls. `parse_file` remains the convenience function that creates one for a
single parse.

`parse_file` is the concrete Macaulay2 parser entry point:

```rust
# fn main() -> Result<(), m2_syn::ParseError> {
use m2_syn::{Lex, parse_file};

let file = parse_file("x + 1\ny + x * 2")?;
let normalized = file.to_m2();
assert_eq!(parse_file(&normalized)?.to_m2(), normalized);
# Ok(())
# }
```

`parse_tokens` connects generated or quoted M2 back to typed syntax:

```rust
# fn main() -> Result<(), m2_syn::ParseError> {
use m2_syn::{Lex, parse_tokens, quote_m2};

let quoted = quote_m2! { left + (right) };
let file = parse_tokens(&quoted)?;
assert_eq!(file.to_m2(), "left + (right)");
# Ok(())
# }
```

`ParsedFile` provides the convenient source-to-source interface over these
lower-level pieces. It owns one mutable typed CST and computes every current
output from that tree, so a token projection cannot silently become stale after
an edit. The original source remains available separately as an immutable
snapshot. `from_source` uses Tree-sitter; `from_source_native` selects the direct
precedence parser.

```rust
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use m2_syn::{ParsedFile, Symbol, visit_mut::VisitMut};

struct Rename;

impl VisitMut for Rename {
    fn visit_symbol_mut(&mut self, symbol: &mut Symbol) {
        if symbol.text() == "left" {
            symbol.set_text("renamed");
        }
    }
}

let file = ParsedFile::from_source("left + right")?.edit(|cst| {
    Rename.visit_source_file_mut(cst);
});

assert_eq!(file.original_source(), "left + right");
assert_eq!(file.to_source(), "renamed + right");

// Stable, uncolored projections are useful in tests and diagnostics.
println!("{}", file.pretty_tokens());
println!("{}", file.pretty_cst());

// One terminal-aware report includes the retained input, recursively flattened
// tokens, and the generated typed CST.
file.print_pretty()?;
# Ok(())
# }
```

The CST view is generated from the same schema as node construction and
traversal. Struct fields become labelled edges, category enums remain invisible
implementation details, punctuated sequences expose both values and comma
tokens, leaf source spellings precede their node types, and delimiters show
separate opening and closing boundaries. The token
view rotates primitive sequences into compact horizontal bands: centered token
text occupies the first row and token type the second, with recursive group
openings and closings represented as ordinary columns. End-of-cell markers are
explicit, always end a band, and group delimiters provide the only soft wrapping
boundaries. Trivia is hidden by default and coalesced when enabled. Source
ranges can be disabled and ANSI styling can be selected explicitly through
`PrettyReport`.

Run the inspector with its built-in example or give it an M2 source file:

```sh
cargo run --example inspect
cargo run --example inspect -- path/to/file.m2
```

Tree-sitter extras are not reconstructed yet. `ChildCursor` currently removes
extra children, so comments are discarded instead of being retained as
ordered trivia. They must be anchored relative to semantic children before
comment-preserving round trips can be claimed.
