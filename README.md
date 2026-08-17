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

## Representation pipeline

The frontend has one raw syntax representation and one generated typed graph:

```text
source bytes --lex--> CellStream --per cell--> TokenStream --parse--> typed CST
                                                               ^          |
                                                               | ToTokens |
                                                               +----------+

CellStream --Display--> source text
TokenStream --Display--> source fragment
```

`CellStream` is the linear outer layer because a cell is the smallest block of
M2 code that is normally parsed independently. Each `CellBlock` contains a
`TokenStream`; cells cannot occur recursively. `TokenStream` is recursive only
through balanced `Group` token trees.

`Parse` and `ToTokens` are the two directions at the typed boundary. For a
typed value `c`, parsing its emission should recover `c`. Emitting a parsed raw
stream may normalize insignificant layout:

```text
Parse(ToTokens(c)) = c
ToTokens(Parse(tokens)) = normalize(tokens)
```

The two parser backends differ only in how they reach the generated graph.
`NativeParser` constructs it directly from the shared token cursor.
`TreeSitterParser` temporarily projects Tree-sitter's untyped nodes through
`CstNode` and `Reconstruct`. Those adapter traits are not an alternative graph
or a traversal API; typed traversal is generated as `Visit`, `VisitMut`, and
`Fold`.

The ownership boundaries are intentionally narrow:

| Location | Owns |
| --- | --- |
| `token_stream.rs` | raw atoms, recursive groups, and the non-recursive cell layer |
| `lexer.rs` | byte recognition, delimiter balancing, and cell splitting |
| `parse.rs` | the shared `Rc<TokenStream>` cursor and parser traits |
| `native.rs` | precedence and typed-node construction decisions |
| `treesitter.rs` / `cst.rs` | the external-CST compatibility adapter |
| `nodes.rs` | the compact typed grammar declaration |
| `m2-syn-macros/src/` | generation of tokens, nodes, traversal, parsing, and emission |

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
| `Delimiter![()]` | a typed delimiter field | the generated parenthesis delimiter atom |
| `T?`, `[T]`, `punct(T)`, `lines(T)` | optional or repeated children | `Option<T>` or `Vec<T>` with generated reconstruction and normalized separators |
| `paren(...)`, `brace(...)`, etc. | delimited product | a concrete node emitted inside the declared delimiter |

Direct recursive product fields are boxed automatically. Token types and
categories containing only tokens remain inline. Constructors accept the
types written in the declaration and hide that storage choice. Generated
signatures consistently refer to literal token types through `Token![...]`;
their concrete Rust names are an implementation detail of `src/gen/tokens.rs`.
Delimited products likewise store generated `Delimiter![()]`,
`Delimiter![[]]`, `Delimiter![{}]`, or `Delimiter![<||>]` atoms. Their raw
counterpart is the delimiter carried by `TokenTree::Group`.

A named field uses the same CST field name. Prefix a field expression with
`unfielded` when it should consume the next matching unlabelled child. An
unnamed product member receives a stable generated field name such as
`_source_file_1`.

## Generated implementations

The declaration generates the following API without requiring per-node
handwritten boilerplate. Runtime alternatives are represented only by the
grammar-backed enums themselves; there is no parallel global node-kind enum.

| Node form | Generated API |
| --- | --- |
| every node and category | `Spanned`, `Reconstruct<N>`, and traversal dispatch |
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

## Native lexer

`lex` is the first native-parser layer. It accepts any byte iterator and
returns a source-spanned `CellStream`, whose `CellBlock` elements each own the
same `TokenStream` used by quoting and emission. The only raw token categories
are `Group`, `Ident`, `Literal`, `Punct`, and `Trivia`; the lexer does not
synthesize typed nodes or adjacency. Semicolons split
cells only after all delimiters have been paired, while a physical line break
splits unless an operator or mandatory clause component still requires more
input. The terminating semicolon or line break remains in the cell's stream. A
lone carriage return and physical line breaks are both whitespace trivia, with
`Trivia::contains_line_break` carrying the structural distinction. After
greedy token recognition, structural delimiter pairs become recursive `Group`
trees with separate spans for their opening and closing delimiters.

Generated `Token![..]` values are checked typed refinements of these raw atoms,
not a sixth `TokenTree` variant. A punctuation atom retains its raw `Punct`, a
keyword retains its raw `Ident`, and `Token![SPACE]` can retain either implicit
whitespace or the explicit `SPACE` identifier. This makes parsing and emission
use the same object rather than converting through a duplicate `FixedToken`
enum. The lexer applies context-free maximal munch across generated operators,
delimiters, and comment openers. Consequently `|--1` begins with the `|-`
operator rather than a comment, while a token beginning with `--` is a line
comment. Likewise, `(*)` remains the single greedy operator rather than a
parenthesized group. Quotation marks are part of a single literal token, not
delimiter groups.

The lexer reads through a lazy byte cursor with arbitrary finite lookahead.
Lookahead fills only the demanded prefix, so accepting an arbitrary byte
iterator does not force the complete source into memory before lexing begins.

```rust
use m2_syn::{SourceId, TokenTree, lex_str};

let tokens = lex_str("***1", SourceId(1))?;
let mut outer = tokens.into_iter();
let cell = outer.next().unwrap();
let spellings = cell
    .into_stream()
    .into_iter()
    .map(|token| match token {
        TokenTree::Literal(token) => token.text().to_owned(),
        token => token.spelling().unwrap().to_owned(),
    })
    .collect::<Vec<_>>();

assert_eq!(spellings, ["**", "*", "1"]);
assert!(outer.next().is_none());
# Ok::<(), m2_syn::LexError>(())
```

## Parser adapters

Generated compatibility reconstruction depends only on `CstNode`.
`TreeSitterNode` adapts Tree-sitter identity, field names, source text, extras,
and ranges to that interface. This homogeneous node view ends at the adapter:
it is never stored inside a generated typed node. Parsers that already know the
typed grammar should construct the graph directly, as `NativeParser` does.

`Parser<T>` is the parser-backend boundary. It receives the `CellStream`
produced by lexing and returns a generated syntax target. `T` defaults to
`SourceFile`, while parsers may implement the trait again for more specific
targets such as `Expr`. Both built-in backends implement this same token-stream
API. A parser with its own CST can implement `CstNode` and call `reconstruct`;
a parser whose native model already matches the generated graph can construct
the target directly.

The target-owned `Parse` trait is the dual of `ToTokens`: `parse2::<T>` consumes
one complete `TokenStream` as `T`. It currently forms a complete vertical slice
for every generated `Token![..]` atom; composite-node parsing will be generated
from the syntax schema next. `ParseStream` owns a
`std::io::Cursor<Rc<TokenStream>>`. A `fork` shares immutable token storage but
has an independent position; `advance_to` explicitly commits successful
speculation. No `unsafe`, interior mutability, or parser-private token vector is
involved.
`parse_quote_m2!` composes quoting with type-inferred parsing.

`ToTokens::to_token_stream` is the inverse boundary for every typed node.
`ToCellStream::to_cell_stream` performs the corresponding top-level conversion
for `SourceFile`, preserving cells as a linear outer layer rather than making
them recursive token trees. The native parser therefore supports the normalized
round trip `CellStream -> SourceFile -> CellStream -> SourceFile`. Generated
concrete token structs are deliberately underscore-prefixed; use `Token![..]`
to name their types and inspect the underlying `TokenTree` after `ToTokens`
when working with a heterogeneous stream. Delimiter structs follow the same
rule through `Delimiter![..]`.

```rust
# use std::convert::Infallible;
use m2_syn::{CellStream, Parser, SourceFile, SourceId, lex_str, parse_with};

struct EmptyParser;

impl Parser for EmptyParser {
    type Error = Infallible;

    fn parse(&mut self, _tokens: CellStream) -> Result<SourceFile, Self::Error> {
        Ok(SourceFile::new(Vec::new()))
    }
}

let tokens = lex_str("", SourceId(1)).unwrap();
let file = parse_with(&mut EmptyParser, tokens).unwrap();
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

`ParsedFile` provides the convenient source-to-source interface over these
lower-level pieces. It owns one mutable typed CST and computes every current
output from that tree, so a token projection cannot silently become stale after
an edit. The original source remains available separately as an immutable
snapshot. `parse` uses Tree-sitter; `parse_native` selects the direct precedence
parser.

```rust
# fn main() -> Result<(), m2_syn::ParseError> {
use m2_syn::{ParsedFile, Symbol, visit_mut::VisitMut};

struct Rename;

impl VisitMut for Rename {
    fn visit_symbol_mut(&mut self, symbol: &mut Symbol) {
        if symbol.text == "left" {
            symbol.text = "renamed".into();
        }
    }
}

let file = ParsedFile::parse("left + right")?.edit(|cst| {
    Rename.visit_source_file_mut(cst);
});

assert_eq!(file.original_source(), "left + right");
assert_eq!(file.to_source(), "renamed + right");

// Indented views are useful in tests, examples, and diagnostics.
println!("{}", file.pretty_tokens());
println!("{}", file.pretty_cst());
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
