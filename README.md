# m2-syn

[![crates.io](https://img.shields.io/crates/v/m2-syn.svg)](https://crates.io/crates/m2-syn)
[![docs.rs](https://img.shields.io/docsrs/m2-syn)](https://docs.rs/m2-syn)
[![license](https://img.shields.io/crates/l/m2-syn.svg)](https://github.com/AlexanderGolys/m2-syn/blob/master/LICENSE)

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

The syntax model follows the grammar published as `tree-sitter-macaulay2`
`6.0.0`; `Cargo.toml` pins that version but resolves it against the sibling
`../tree-sitter-macaulay2` checkout during local development, so the two stay
in lockstep without a registry round trip on every grammar change.

Round-tripping through both the Tree-sitter and native parser backends works
end to end, and `quote_m2!`/`parse_quote_m2!` are stable enough for everyday
use (see [Quoting](#quoting)). The next steps are a converter from M2's own
`Core` parser output and a macro-expansion engine built on the typed graph;
`Core`'s own concrete tree is considerably less structured than either parser
backend here (no named fields, several statement forms share one flat node
shape distinguished only by which positional slots are populated), so the
typed graph and its generated APIs must stay parser-independent rather than
special-cased around it.

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
`ExternalCstNode` and `Reconstruct`. Those adapter traits are not an alternative graph
or a traversal API; typed traversal is generated as `Visit`, `VisitMut`, and
`Fold`.

The ownership boundaries are intentionally narrow:

| Location | Owns |
| --- | --- |
| `token_stream.rs` | raw atoms, recursive groups, and the non-recursive cell layer |
| `lexer.rs` | byte recognition, delimiter balancing, and cell splitting |
| `parse.rs` | the shared `Rc<TokenStream>` parser position and parser traits |
| `native.rs` | precedence execution and typed-node construction decisions |
| `treesitter.rs` / `cst.rs` | the external-CST compatibility adapter |
| `nodes.rs` | the compact typed grammar and token parser metadata declaration |
| `m2-syn-macros/src/` | generation of tokens, nodes, traversal, parsing, and emission |

```text
syntax! {
    // Structural precedences with no single owning token; every operator
    // below carries its own precedence inline instead.
    precedence: {
        PREC_CONTROL = 12,
    }

    augmented: (14, 13)

    tokens {
        // (precedence, binary_strength, unary_strength); `_` marks a slot
        // the row's flags don't use.
        [+] { bin, pref, aug } (50, 50, 50)
        [=] { infix }          (14, 13, _)
    }
    keywords: { [if] [then] [else] }
    markers: {}
    punct: { [,] }

    struct Symbol;

    struct BinaryExpression {
        left: Expr,
        operator: BinaryOperator,
        right: Expr,
    }

    enum Expr {
        BinaryExpression,
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
| `PREC_CONTROL = 12` | a structural precedence with no single owning token, consumed directly by the native parser | a crate-visible precedence constant |
| `[+] { bin, pref, aug } (50, 50, 50)` | binary and prefix binding plus augmented-token generation, precedence inline | token types, parser metadata, and inferred operator-enum membership |
| `[=] { infix } (14, 13, _)` | syntax-specific infix binding without generic operator membership | a typed token and parser metadata |
| `[else]` | a keyword already identified by its typed token | a typed keyword token |
| `struct Name;` | named text leaf | `String` and `Span` |
| `struct Name { left: Expr, ... }` | concrete product node | declared children |
| `enum Name { Expr, ... }` | grouping category; each bare variant names the type it wraps, so the wrapped name doubles as the variant name | one declared alternative per variant |
| `Token![=]` | literal token field | the type selected by the generated `Token!` macro |
| `X?`, `Vec<T>`, `Punctuated<T>` | optional, repeated, or comma-punctuated children | the corresponding public wrapper type |
| `(_)` | a child without a Tree-sitter field name | positional reconstruction from the next matching child |
| `(_ lines)` | a positional `Vec<T>` child that's newline- rather than space-separated | positional reconstruction with line-separated emission |
| `#[cst(kind = assignment)]` | a Rust name whose external CST node has another name | reconstruction from the named CST kind |
| `#[delimiter(parenthesis)]` | a product enclosed by a delimiter family | a concrete node with its typed delimiter atom |

Each operator token carries its own `(precedence, binary_strength,
unary_strength)` triple directly in `tokens { ... }`, with `_` for any slot
its flags don't use — `bin`/`infix` use the binary-strength slot, `pref` uses
the unary-strength slot, `post` uses neither. The separate `precedence` stage
only holds names for structural precedences with no single owning operator
token (delimiter/control-clause stoppers the native parser references
directly); it is not a shared table operators look their numbers up in, so a
level used by exactly one token is just written as a literal there and
nowhere else. The `aug` tag generates the corresponding assignment token
using the binding declared once in `augmented: (precedence,
binary_strength)`.

Direct recursive product fields are boxed automatically. Token types and
categories containing only tokens remain inline. Constructors accept the
types written in the declaration and hide that storage choice. Generated
signatures consistently refer to literal token types through `Token![...]`;
their concrete Rust names are an implementation detail of `src/gen/tokens.rs`.
Delimited products likewise store generated delimiter atoms. The six families
are the four paired collection delimiters plus the implicit empty cell
delimiter and the semicolon cell delimiter. Their convenient type macros also
construct values:

```rust
use m2_syn::{Span, ToTokens};

type Parenthesized = m2_syn::paren!(m2_syn::Token![+]);
let value: Parenthesized = m2_syn::paren!(
    m2_syn::Token![+](Span::detached()),
    Span::detached(),
);
assert_eq!(value.to_m2(), "(+)");
```

`naked!`, `semicolon!`, `paren!`, `brackets!`, `braces!`, and `angle_bars!`
cover all six families. `punct!(T)` names a comma-punctuated sequence;
`punct!()`, `punct!(value value)`, and `punct!(pairs first; comma => next)`
construct one while retaining every typed comma and its span. Every delimiter
atom and each of these wrapper types implements both `Parse` and `ToTokens`, so
they can be direct `parse_quote_m2!` targets.

A field normally uses its Rust name as its CST field name. Mark it `(_)`
when it should consume the next matching unlabelled child instead. The
schema deliberately permits only named struct fields and bare-type enum
variants — a variant never spells out a separate name, since it's always
either the wrapped type's own name or, for a variant wrapping a token, that
token's capitalized name — keeping both the declaration and generated API
shaped like ordinary Rust rather than requiring the same fact to be written
twice.

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
input. A top-level semicolon becomes the cell's `Semicolon` delimiter rather
than remaining in its interior token stream; an ordinary cell has an implicit
`Empty` delimiter. A lone carriage return and physical line breaks are both whitespace trivia, with
`Trivia::contains_line_break` carrying the structural distinction. After
greedy token recognition, structural delimiter pairs become recursive `Group`
trees whose span covers the complete group. Exact opening and closing spans are
derived from that range and the delimiter family's fixed spelling width.

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

Generated compatibility reconstruction depends only on `ExternalCstNode`.
`TreeSitterNode` adapts Tree-sitter identity, field names, source text, extras,
and ranges to that interface. This homogeneous node view ends at the adapter:
it is never stored inside a generated typed node. Parsers that already know the
typed grammar should construct the graph directly, as `NativeParser` does.

`Parser<T>` is the parser-backend boundary. It receives the `CellStream`
produced by lexing and returns a generated syntax target. `T` defaults to
`SourceFile`, while parsers may implement the trait again for more specific
targets such as `Expr`. Both built-in backends implement this same token-stream
API. A parser with its own CST can implement `ExternalCstNode` and call `reconstruct`;
a parser whose native model already matches the generated graph can construct
the target directly.

The target-owned `Parse` trait is the dual of `ToTokens`: `parse1::<T>` consumes
one complete `TokenStream` as `T`. Generated `Token![..]` atoms, raw token
categories, token streams, boxes, delimiters, comma-punctuated collections, and
terminated bodies implement it. `ParseStream` shares immutable token storage
through `std::io::Cursor<Rc<TokenStream>>`. Its final stored token is an
internal EOF marker, which iteration does not yield. A `fork` has
an independent index; `advance_to` explicitly commits successful speculation.
No `unsafe`, interior mutability, or parser-private token vector is involved.
`parse_quote_m2!` composes quoting with type-inferred parsing.

`ToTokens::to_token_stream` is the inverse boundary for local syntax.
`ToCells` is the corresponding global-scope emission interface. Its `to_cells`
method appends to an existing `CellStream`, while `to_cell_stream` creates one
with a chosen `SourceId`. Complete expressions, typed cells, `SourceFile`, raw
token streams, and groups implement both scope projections where meaningful.
The native parser therefore supports the normalized round trip
`CellStream -> SourceFile -> CellStream -> SourceFile`. Generated
concrete token structs are deliberately underscore-prefixed; use `Token![..]`
to name their types and inspect the underlying `TokenTree` after `ToTokens`
when working with a heterogeneous stream. Delimiter structs follow the same
rule through `Delimiter![..]`.

`CellBlock` and `CellStream` implement `ToTokens`, so placing them inside a
`Delimited<S, D>` lowers global cells into one local token-tree group. In the
other direction, `Group::to_cells` removes the outer delimiter and promotes
the group's contents to global cells; top-level semicolons and terminating
newlines regain their cell-boundary meaning during that promotion.

```rust
# use std::convert::Infallible;
use m2_syn::{CellStream, Parser, SourceFile, SourceId, lex_str, parse_with};

struct EmptyParser;

impl Parser for EmptyParser {
    type Error = Infallible;

    fn parse_cells(&mut self, _tokens: CellStream) -> Result<SourceFile, Self::Error> {
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
snapshot. `from_source` uses Tree-sitter; `from_source_native` selects the direct
precedence parser.

```rust
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use m2_syn::{ParsedFile, Symbol, visit_mut::VisitMut};

struct Rename;

impl VisitMut for Rename {
    fn visit_symbol_mut(&mut self, symbol: &mut Symbol) {
        if symbol.text == "left" {
            symbol.text = "renamed".into();
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

## Quoting

`quote_m2!` constructs an M2 `TokenStream` from Rust tokens. `$(...)`
interpolates any Rust expression implementing `ToTokens`. Each interpolation is
first emitted as a real fragment, so punctuation, identifiers, literals, and
groups compose according to their actual token boundaries rather than the Rust
expression's type. Parentheses, brackets, braces, and M2 angle bars are emitted
as recursive raw groups.

`$[pattern in iterator] { ... }` repeats a recursively quoted template using an
ordinary Rust `for` loop. The iterator and pattern are explicit; interpolations
that do not depend on the pattern are reused normally on each iteration.

```rust
use m2_syn::{Span, Symbol, quote_m2};

let value = Symbol::new("value", Span::detached());
let tokens = quote_m2! {
    result = $(value) + 1;
    return result
};

assert_eq!(tokens.to_string(), "result=value+1;return result");

let values = [
    Symbol::new("left", Span::detached()),
    Symbol::new("right", Span::detached()),
];
let comma = m2_syn::Token![,](Span::detached());
let repeated = quote_m2! {
    $[value in &values] { $(value) $(comma) }
};
assert_eq!(repeated.to_string(), "left,right,");
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

`m2-syn` is licensed under the [GNU General Public License v3.0](https://github.com/AlexanderGolys/m2-syn/blob/master/LICENSE).
