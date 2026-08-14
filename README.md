# m2-syn

`m2-syn` is a parser-independent typed syntax graph for Macaulay2. One
`syntax!` declaration describes the algebraic shape of the graph; generated
implementations provide construction, traversal, source provenance, and
conversion from a concrete syntax tree.

The API is experimental and may change while the typed grammar and parser
adapter interfaces are being established.

## Project direction

This crate began as the syntax and analysis layer for the Macaulay2 language
server. It grew into a standalone, `syn`-like foundation for M2: an owned,
typed representation that can be constructed from source, traversed and
rewritten, and emitted as code.

The current priority is the basic source pipeline:

1. read M2 source and parse it with Tree-sitter;
2. reconstruct the complete typed AST;
3. emit normalized M2 that parses back to the same structure.

The syntax model and development dependency follow the current grammar in the
sibling `../tree-sitter-macaulay2` checkout. Published version `5.0.0` is
substantially older; once the current grammar is released, its registry version
can replace the sibling path without changing the parser API.

Once broad round-tripping works, the next steps are to stabilize a primitive
`quote_m2!`, then consider either an M2 `Core`-parser converter or another
parser implementation. Those frontends may expose different concrete trees,
so the typed graph and its generated APIs must remain parser-independent even
while Tree-sitter is an unconditional dependency.

Longer term, `m2-syn` should support semantic and typechecking walks,
incremental analysis after referenced code changes, selected node metadata,
and procedural macros for M2. `Visit`, `VisitMut`, and `Fold` are intended as
the basic traversal and transformation vocabulary for that work. The guiding
design rule is to describe as little grammar as possible in `syntax!` and
generate the repetitive construction, traversal, reconstruction, and printing
machinery.

```text
syntax! {
    tokens {
        [+] {pref, bin, aug}
        [=] {}
    }
    keywords: { [if] [then] [else] }
    markers: {}
    punct: { [,] }

    Symbol ::= leaf

    OperatorExpr ::= {
        BinaryExpression: (
            left: Expr,
            operator: BinaryOperator,
            right: Expr,
        ),
    }

    Expr ::= {
        OperatorExpr,
        Symbol,
    }
}
```

The canonical declaration lives in `src/nodes.rs`. Run the generator after
editing it; the checked-in expansion is split across `src/gen/`. Generated
files are reviewable build inputs and must not be edited directly.

## Declaration model

| Declaration | Syntax role | Generated shape |
| --- | --- | --- |
| `[+] {pref, bin, aug}` | operator and augmented token declarations | token types plus operator-enum membership |
| `Name ::= leaf` | named text leaf | `String` and `Span` |
| `Name ::= (left: Expr, ...)` | concrete product node | declared children |
| `Name ::= { Expr, Variant: (...) }` | grouping category | one declared alternative; inline products become concrete structs |
| `Token![=]` | anonymous literal token field | the type selected by the generated `Token!` macro |
| `T?`, `[T]`, `punct(T)`, `lines(T)` | optional or repeated children | `Option<T>` or `Vec<T>` with generated reconstruction and normalized separators |
| `paren(...)`, `brace(...)`, etc. | delimited product | a concrete node emitted inside the declared delimiter |

Direct recursive product fields are boxed automatically. Token types and
categories containing only tokens remain inline. Constructors accept the
types written in the declaration and hide that storage choice. Generated
signatures consistently refer to literal token types through `Token![...]`;
their concrete Rust names are an implementation detail of `src/gen/tokens.rs`.

A named field uses the same CST field name. Prefix a field expression with
`unfielded` when it should consume the next matching unlabelled child. An
unnamed product member receives a stable generated field name such as
`_source_file_1`.

## Generated implementations

The declaration generates one `SyntaxKind` and the following API without
requiring per-node handwritten boilerplate.

| Node form | Generated API |
| --- | --- |
| every node and category | `AstNode`, `Spanned`, `Reconstruct<N>`, and traversal dispatch |
| token | `Token`, constructor, reconstruction, and a `Token![spelling]` arm |
| text leaf | text-plus-span constructor and exact CST reconstruction |
| product struct | concrete-node matching, storage-aware constructor, child reconstruction, and default child walkers |
| coproduct enum | category matching, alternative reconstruction, default dispatch, and unambiguous direct/transitive `From` conversions |

The traversal traits have ordinary, independently addressable modules:
`visit::Visit`, `visit_mut::VisitMut`, and `fold::Fold`. Their node-specific
method inventories and free default walkers are generated from the syntax
declaration, so adding a node cannot leave one traversal API incomplete. An
override can inspect a node and call its module's free walker to continue into
its children. `VisitMut` edits the owned typed graph in place. `Fold` consumes
nodes and returns the reconstructed graph.

`cargo run --example type_inference` demonstrates the intended analysis path:
generated `Visit` dispatch selects node-specific rules, and the binary rule
consumes the neighbouring operand facts to compute an upward-closed result
range. Graph storage and fixed-point solving belong in the analysis/LSP layer;
this example only verifies the syntax traversal boundary. Its generic
`InferenceContext` leaves concrete types, values, and deferred database or
signature queries under the control of the consuming analysis layer.

## Spans

`Span` answers where syntax originated; it is not the identity of a semantic
object and does not make coordinates persistent across edits. Located tokens
and text leaves retain a `SourceId` and `TextRange`. Product and category spans
are derived from their children. Parser-independent syntax uses
`Span::detached()` when no source location exists.

This provenance is what diagnostics, hovers, semantic tokens, navigation, and
source edits use to translate a typed node back to a document. Incremental
dependency identity belongs in the semantic layer rather than in `Span`.

## Parser adapters

Generated reconstruction depends only on `CstNode`. `TreeSitterNode` adapts
Tree-sitter identity, field names, source text, extras, and ranges to that
interface. Other parsers can implement the same trait without entering the
generated syntax model.

`Parser<T>` is the high-level ecosystem boundary. It receives a `ParseInput`
containing source text and its `SourceId`, and returns a typed target. `T`
defaults to `SourceFile`, while parsers may implement the trait again for
specific targets such as `Expr`. A parser with its own CST can implement
`CstNode` and call `reconstruct`; a parser whose native model already matches
the typed graph can construct the target directly.

```rust
# use std::convert::Infallible;
use m2_syn::{ParseInput, Parser, SourceFile, SourceId, parse_with};

struct EmptyParser;

impl Parser for EmptyParser {
    type Error = Infallible;

    fn parse(&mut self, _input: ParseInput<'_>) -> Result<SourceFile, Self::Error> {
        Ok(SourceFile::new(Vec::new()))
    }
}

let file = parse_with(&mut EmptyParser, "", SourceId(1)).unwrap();
assert!(file.elements.is_empty());
```

`TreeSitterParser` is the built-in implementation and can be retained across
calls. `parse_file` remains the convenience function that creates one for a
single parse.

`parse_file` is the concrete Macaulay2 parser entry point:

```rust
# fn main() -> Result<(), m2_syn::ParseError> {
use m2_syn::{SourceId, ToTokens, parse_file};

let file = parse_file("x + 1\ny + x * 2", SourceId(1))?;
let normalized = file.to_m2();
assert_eq!(parse_file(&normalized, SourceId(2))?.to_m2(), normalized);
# Ok(())
# }
```

`parse_tokens` connects generated or quoted M2 back to typed syntax:

```rust
# fn main() -> Result<(), m2_syn::ParseError> {
use m2_syn::{SourceId, ToTokens, parse_tokens, quote_m2};

let quoted = quote_m2! { left + (right) };
let file = parse_tokens(&quoted, SourceId(1))?;
assert_eq!(file.to_m2(), "left + (right)");
# Ok(())
# }
```

Tree-sitter extras are not reconstructed yet. `ChildCursor` currently removes
extra children, so comments are discarded instead of being retained as
ordered trivia. They must be anchored relative to semantic children before
comment-preserving round trips can be claimed.

## Quoting

`quote_m2!` constructs an M2 `TokenStream` from Rust tokens. `$(...)`
interpolates any Rust expression implementing `ToTokens`. The macro contains
only M2 tokens; the end of its input is the end of the resulting source.

```rust
use m2_syn::{Span, Symbol, quote_m2};

let value = Symbol::new("value", Span::detached());
let tokens = quote_m2! {
    result = $(value) + 1;
    return result
};

assert_eq!(tokens.to_string(), "result=value+1;return result");
```

The rendered text is normalized M2 source, not a lossless reproduction of
whitespace or comments discarded by the typed graph.

## Development

Run the complete workspace checks from the repository root:

```sh
cargo run -p m2-syn-macros --bin generate -- --check
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

After changing `src/nodes.rs` or the generator, refresh the expansion with
`cargo run -p m2-syn-macros --bin generate` and include the resulting
`src/gen/` diff.

`m2-syn` is licensed under the [GNU General Public License v3.0](LICENSE).
