# m2-syn

[![crates.io](https://img.shields.io/crates/v/m2-syn.svg)](https://crates.io/crates/m2-syn)
[![docs.rs](https://img.shields.io/docsrs/m2-syn)](https://docs.rs/m2-syn)
[![license](https://img.shields.io/crates/l/m2-syn.svg)](https://github.com/AlexanderGolys/m2-syn/blob/master/LICENSE)

`m2-syn` is a parser-independent typed syntax graph for Macaulay2. One
`syntax!` declaration inventories the algebraic graph while its nested
`strutuct!` declarations expand each node independently. Generated
implementations provide construction, traversal, source provenance, and
conversion from a concrete syntax tree.

The API is experimental and may change while the typed grammar and parser
adapter interfaces are being established.

## Cargo features

Traversal, typed-tree inspection, and the native parser are always available.
Only the alternative parser backend is optional:

- `tree-sitter` enables the Tree-sitter parser adapter, incremental parser,
  `parse_file`, `parse_tokens`, and `ParsedFile::from_source`.

The native lexer, native precedence parser, typed nodes, traversal, pretty
inspection, quoting, emission, spans, and parser-independent migration traits
remain available with `default-features = false`.

## Project direction

This crate began as the syntax and analysis layer for the Macaulay2 language
server. It grew into a standalone, `syn`-like foundation for M2: an owned,
typed representation that can be constructed from source, traversed and
rewritten, and emitted as code.

The current priority is the basic source pipeline:

1. parse M2 source with Tree-sitter and migrate its typed graph directly;
2. parse the lexer's token stream with the independent native precedence parser;
3. emit normalized M2 that parses back to the same structure.

The syntax model follows the grammar published as `tree-sitter-macaulay2`
`6.1.2`, resolved from the registry like any other dependency. Local builds and
tests therefore exercise the same grammar downstream users get, rather than
whatever happens to be checked out beside the repository.

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
generate the repetitive construction, traversal, migration, and printing
machinery.

## Representation pipeline

The frontend has one lexical representation, one cell-partitioned
representation, and one generated typed graph:

```text
source --Lex--> TokenStream --Tokenize--> CellStream<CellTokenStream> --Parser--> typed CST
                       ^                                  ^                         |
                       |                                  +--------Tokenize---------+
                       +-------------------------------------Lex---------------------+
```

`TokenStream` is the lexical representation. It is flat at the top level and
recursive only through balanced `Group` token trees. `Tokenize` partitions it
into independently parseable `CellTokenStream` values and inserts inferred
`CellTokenTree::Adj` markers recursively. `CellStream` collects those partially
parsed cells and expresses end-of-cell and end-of-file structurally rather than
as virtual token variants. Converting a framed cell back to `TokenStream`
removes the non-lexical adjacency markers. Both parser backends consume the
framed representation.

These explicit phases are also the intended macro seams. A future
function-like macro may consume and produce the less constrained
`TokenStream` before tokenisation. Transformations that should preserve normal
M2 cell structure can instead implement `ProcMacro` over `CellStream`.

`Parse` and `Lex` are the two directions at the typed boundary. For a
typed value `c`, parsing its emission should recover `c`. Emitting a parsed raw
stream may normalize insignificant layout:

```text
Parse(Lex(c)) = c
Lex(Parse(tokens)) = normalize(tokens)
```

`NativeParser` constructs the graph directly from the shared token cursor.
`TreeSitterParser` lowers the bundled grammar's CST directly, including its
`muted`, comma-pack, delimiter, operator, and control-form layouts. It does not
re-lex or invoke `NativeParser`. `TreeSitterNode`, `ExternalCstNode`, and
`Migrate` remain the parser-independent seam used by that adapter for
schema-compatible leaves, operators, and tokens; they are not an alternative
graph or traversal API. Typed traversal is generated as `Visit`, `VisitMut`,
and `Fold`.

`NativeRealtimeParser` retains this pipeline at cell granularity. A newline or
semicolon-confirmed cell is a restart point, while a cell ending only at the
current EOF stays speculative and is reopened by an append. Earlier edits are
reframed strictly forwards until the next old EOC is restored; the remaining
cell suffix is reused without lexing or parsing it. Every reparsed region
regenerates `Adj` together with its cell boundaries, and each cell reports
`Complete`, `Incomplete`, or `Invalid` independently. Cell spans are local to
the cell's immutable source allocation, with `document_range()` providing its
position in the current buffer; this permits unchanged cells to move without
rewriting every leaf span.

The ownership boundaries are intentionally narrow:

| Location | Owns |
| --- | --- |
| `lex/token_stream.rs` | raw atoms and recursive groups |
| `lex/lexer.rs` | byte recognition and delimiter balancing |
| `lex/cell_stream.rs` | cell boundaries and inferred adjacency |
| `parser/parse.rs` | the per-cell token cursor and parser traits |
| `parser/native.rs` | precedence and typed-node construction |
| `parser/realtime.rs` | retained native cells and forward reframing |
| `migration/` | the external-CST migration adapter |
| `schema/nodes.rs` | the compact grammar and token parser metadata declaration |
| `m2-syn-macros/src/` | generation of tokens, nodes, traversal, parsing, and emission |

```text
syntax! {
    // Structural precedences with no single owning token; every operator
    // below carries its own precedence inline instead.
    precedence: {
        PREC_CONTROL = 12,
    }

    tokens {
        // L/R is the binary associativity. A third slot overrides the
        // prefix strength when it differs from the precedence.
        [+] { bin, pref, aug } (50, L)
        [=] { infix }          (14, R)
        [if] [then] [else]     (PREC_CONTROL)
        [,]
    }

    #[operator(prefix)]
    enum PrefixOperator {}
    #[operator(binary)]
    enum BinaryOperator {}
    #[operator(postfix)]
    enum PostfixOperator {}

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

The canonical declaration lives in `src/schema/nodes.rs`. The outer `syntax!`
pass reads the unexpanded declarations to build the shared method inventories,
then re-emits every `strutuct!` invocation. Rust expands those invocations and
their per-type child walkers separately. The documented `visit`, `visit_mut`,
and `fold` modules are re-exported at the crate root without placing hundreds
of free walker functions directly in the root namespace.

`Token!` and `Delimiter!` are ordinary source in `src/schema/macros.rs` instead of
part of that expansion. Neither depends on the schema: `Token!` forwards the
spelling to the `token!` procedural macro, and the delimiter families are a
fixed set.

## Declaration model

| Declaration | Syntax role | Generated shape |
| --- | --- | --- |
| `PREC_CONTROL = 12` | a structural precedence with no single owning token, consumed directly by the native parser | a crate-visible precedence constant |
| `[+] { bin, pref, aug } (50, L)` | left-associative binary and prefix binding plus augmented-token generation | token types, parser metadata, and inferred operator-enum membership |
| `[=] { infix } (14, R)` | right-associative syntax-specific infix binding without generic operator membership | a typed token and parser metadata |
| `[else] (PREC_CONTROL)` | a keyword and both of its parser strengths | a typed keyword token and parser metadata |
| `struct Name;` | named text leaf | `String` and `Span` |
| `#[leaf(qualified)] struct Name;` | one identifier with optional qualification metadata | text plus optional package and decimal-index subspans |
| `struct Name { left: Expr, ... }` | concrete product node | declared children |
| `field: Nested { value: Expr }` or `field: Pair(Expr, Symbol)` | a field whose type is declared in place | a hoisted `Nested` or `Pair` declaration plus a field of that type |
| `enum Name { Expr, ... }` | grouping category; each bare variant names the type it wraps, so the wrapped name doubles as the variant name | one declared alternative per variant |
| `Variant(Expr, Symbol)` or `Variant { value: Expr }` | an enum alternative whose product type is declared in place | a hoisted product and one alternative wrapping it |
| `struct Name* ...` or `enum Name* ...` | a declaration whose recursive references require indirection | `Box<Name>` at references to that type |
| `Token![=]` | literal token field | the type selected by the generated `Token!` macro |
| `X?`, `Vec<T>`, `Punctuated<T>`, `Delimited<T>` | optional, repeated, nonempty comma-punctuated, or arbitrarily delimited children | the corresponding public wrapper type |
| `(_)` | a child without a Tree-sitter field name | positional migration from the next matching child |
| `Lines<T>` | a `Vec<T>` printed one element per line | the separator travels with the functor |
| `#[cst(kind = assignment)]` | a Rust name whose external CST node has another name | migration from the named CST kind |
| `#[delimiter(parenthesis)]` | a product enclosed by a delimiter family | a concrete node with its typed delimiter atom |
| `#[literal(string)]` | a product spelled as a string literal | emission as one literal token, with no delimiter atom |

Every token is declared in `tokens { ... }`; adjacent spellings share the
metadata written on their right. A binary role writes `L` or `R`, while a
literal strength remains available for a non-associative rule. A prefix role
binds at the token's precedence unless a third slot overrides it, and `post`
writes no strength at all. The separate `precedence` stage
only holds names for structural precedences with no single owning operator
token (delimiter/control-clause stoppers the native parser references
directly); it is not a shared table operators look their numbers up in, so a
level used by exactly one token is just written as a literal there and
nowhere else. The `aug` tag generates the corresponding assignment
token, which binds exactly like the `=` it abbreviates and so takes its rule
from that declaration.

Boxing is explicit rather than inferred: put `*` after a declaration name when
references to that type must be stored through `Box`. Only recursive cycles
need it; ordinary fields, token types, and nonrecursive categories stay inline.
Constructors still accept the types written in the declaration and hide the
storage conversion. Generated signatures consistently refer to literal token
types through `Token![...]`; their concrete Rust names are an implementation
detail of the expansion.

Generated concrete syntax nodes and literal-token structs live under
`m2_syn::nodes`. Construction and type macros such as `Token!`, `Delimiter!`,
`paren!`, and `punct!` remain available from the crate root, as do generic
containers such as `Delimited` and `Punctuated`.

`Delimited<T>` retains an arbitrary one of the four paired delimiter families;
`Delimited<T, Delimiter![()]>` and the other concrete delimiter atoms refine
the family statically. Every delimiter is spelled in source, so both boundary
spans are read rather than inferred. The convenient type macros construct the
statically refined forms:

```rust
use m2_syn::{Lex, Source, Span};

type Parenthesized = m2_syn::paren!(m2_syn::Token![+]);
let span = Span::unlocated(Source::synthetic(""));
let value: Parenthesized = m2_syn::paren!(
    m2_syn::Token![+](&span),
    &span,
);
assert_eq!(value.to_m2(), "(+)");
```

`paren!`, `brackets!`, `braces!`, and `angle_bars!` cover all four families.
`punct!(T)` names a nonempty comma-punctuated sequence; `punct!(value first)`
and `punct!(pairs first; comma => next)` construct one while retaining every
typed comma and its span. Absence belongs outside it as
`Option<Punctuated<T>>`, while an empty component belongs in `T` itself. Every
delimiter atom and each wrapper type implements both `Parse` and `Lex`, so they
can be direct `parse_quote_m2!` targets.

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
| every node and category | `Spanned`, `Migrate<N>`, and traversal dispatch |
| token | `Token`, constructor, migration, and a `Token![spelling]` arm |
| text leaf | text-plus-span constructor and exact CST migration |
| product struct | concrete-node matching, storage-aware constructor, child migration, and default child walkers |
| coproduct enum | category matching, alternative migration, default dispatch, and unambiguous direct/transitive `From` conversions |

The traversal traits have ordinary, independently addressable modules:
`visit::Visit`, `visit_mut::VisitMut`, and `fold::Fold`. Their node-specific
method inventories and free default walkers are generated from the syntax
declaration, so adding a node cannot leave one traversal API incomplete. An
override can inspect a node and call its module's free walker to continue into
its children. `VisitMut` edits the owned typed graph in place. `Fold` consumes
nodes and returns the migrated graph.

`cargo run --example type_inference` demonstrates the intended analysis path:
an expression visitor returns simple names for a few obvious syntax forms,
passes assignment and parenthesized-expression types through, and joins the
two result types of a conditional. Unsupported expressions return `Unknown`.
The example deliberately has no type hierarchy, symbol registry, or solver;
those belong in the analysis/LSP layer rather than the syntax crate.

## Spans

A `Span` is an `Arc<Source>` and an optional standard byte `Range`. The source
allocation contains an immutable text snapshot and a compact `File` enum. Every
token cloned from one parse therefore shares one source allocation, while the
token types themselves stay concrete and require no source-id generic.

```rust
use std::sync::Arc;
use m2_syn::{File, Source, Span, TextEdit};

let source = Source::with_file(File::from_raw(7), "quotient ring");
let ring = Span::ranged(Arc::clone(&source), 9..13);
assert_eq!(ring.text(), Some("ring"));

// An edit acts on the buffer and on the spans indexing it, and the two agree.
let edit = TextEdit::insertion(0, "the ");
let edited = edit.apply(source.text()).unwrap();
let edited_source = Source::with_file(source.file(), edited);
let shifted = ring.shifted(&edit).with_source(edited_source);
assert_eq!(shifted.text(), ring.text());
```

Textual leaves derive their spelling directly from the source snapshot. Product
and category spans are derived from their children. A span always knows its
source; when only its exact position is unavailable, its range is `None`.
Programmatically generated syntax uses an explicit synthetic `Source`, and
joining spans from different source snapshots returns `None`.

Line and column remain derived facts rather than stored span coordinates.
`Source::line_column` resolves a byte boundary through the immutable source's
line-start index, and `Span::line_column_range` resolves both half-open
boundaries for diagnostics. A span is not the identity of a semantic object;
incremental dependency identity belongs in the semantic layer.

## Native lexer

`lex` is the first native-parser layer. It accepts any byte iterator and
returns the same source-spanned `TokenStream` used by quoting and emission. The only raw token categories
are `Group`, `Ident`, `Literal`, and `Punct`; comments are retained as trivia
beside those atoms, while ordinary layout remains recoverable from source
spans. The lexer does not synthesize typed nodes, adjacency, or cells. After
greedy token recognition, structural delimiter pairs become recursive `Group`
trees that retain exact opening and closing spans.

Generated `Token![..]` values are checked typed refinements of these raw atoms,
not a sixth `TokenTree` variant. A punctuation atom retains its raw `Punct`, and
a keyword retains its raw `Ident`. Adjacency is different: it is a parser-owned
empty `Adj` node placed at the right operand's boundary. It is independent of
physical whitespace (`f(x)` and `f (x)` have the same operator), remains absent
from `return 1`, and emits no token. The lexer applies context-free maximal
munch across generated operators, delimiters, and comment openers.
Consequently `|--1` begins with the `|-` operator rather than a comment, while
a token beginning with `--` is a line comment. Likewise, `(*)` remains the
single greedy operator rather than a parenthesized group. Quotation marks are
part of a single literal token, not delimiter groups.

Consecutive `--` comments occupying complete physical lines coalesce into one
outer comment block. Every comment is stored where it occurs: before a cell's
first token it lives in that cell's `leading_trivia`, after an opening
delimiter in the delimiter's opening trivia, and anywhere else in the trailing
trivia of the token it follows, so comments after the last token belong to the
last cell. A file without cells keeps its comments in its own
`leading_trivia`. `Cell::comments` collects a cell's comments on demand: the
complete-line `--` block before its first token is the outer documentation
comment, and every other comment is inner. Token leaves never become the
accidental owners of cell documentation.

`$` remains an identifier-continuation character. `Package$name$12` is one
`IdentToken` with one overall span; the typed `Symbol` additionally records
`package_qualifier = Package` and `index_qualifier = 12` as optional subspans.
Only `package$name`, `name$decimal`, and their combined form are marked, so the
representation permits at most one qualifier of each kind. This is metadata,
not grammar-level tokenization. Exact `Core$word` spellings are a separate
hardcoded renaming hook for declared word tokens: `Core$not` behaves as `not`,
while `Core$Core$not` remains an ordinary symbol.

Decimal real literals retain precision before exponent: `1p53`, `.5p20`, and
`1.25p53e3` are each one `FloatLiteral`. Once an exponent has been consumed,
`p` starts the following identifier instead, so `1e3p20` is adjacency between
`1e3` and `p20`.

The lexer validates one source buffer and gives every lexical atom the same
source identity with its own byte range. Each atom separately retains the
spelling needed for parser-independent token processing. The byte-iterator
entry point first materializes its finite input; `lex_str` starts directly from
source text.

```rust
use m2_syn::{TokenTree, lex_str};

let tokens = lex_str("***1")?;
let spellings = tokens
    .into_iter()
    .filter_map(|token| match token {
        TokenTree::Literal(token) => Some(token.text().to_owned()),
        token => token.spelling().map(str::to_owned),
    })
    .collect::<Vec<_>>();

assert_eq!(spellings, ["**", "*", "1"]);
# Ok::<(), m2_syn::LexError>(())
```

## Parser adapters

Generated migration depends only on `ExternalCstNode`.
`TreeSitterNode` adapts Tree-sitter identity, field names, source text, extras,
and ranges to that interface. This homogeneous node view ends at the adapter:
it is never stored inside a generated typed node. Parsers that already know the
typed grammar should construct the graph directly, as `NativeParser` does.

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
terminated bodies implement it. `ParseStream` clones the persistent
`TokenStream` handle, so forks share its immutable backing allocation;
end-of-input is cursor state rather than a synthetic token. A `fork` has an
independent index and `advance_to` explicitly commits successful speculation.
`parse_quote_m2!` composes quoting with type-inferred parsing.

`Lex::to_token_stream` is the lexical projection, while `Tokenize` projects
raw or typed syntax into cells. The native parser supports the normalized
round trip `CellStream -> SourceFile -> CellStream -> SourceFile`. Generated
concrete token structs are deliberately underscore-prefixed; use `Token![..]`
to name their types and inspect the underlying `TokenTree` after `Lex`
when working with a heterogeneous stream. Delimiter structs follow the same
rule through `Delimiter![..]`.

The lexer emits one flat `TokenStream`; `Tokenize` splits it into the cells
evaluated at global scope. A `;`
ends a cell unless a quote specifier took it literally, and a line break ends
one only where the tokens so far already form a complete expression. Wrapping a
region in a delimiter therefore never rewrites its tokens.

```rust
# use std::convert::Infallible;
use m2_syn::nodes::{BOF, EOF, SourceFile};
use m2_syn::{CellStream, Parser, Span, Tokenize, lex_str, parse_with};

struct EmptyParser;

impl Parser for EmptyParser {
    type Error = Infallible;

    fn parse_cells(&mut self, _cells: CellStream) -> Result<SourceFile, Self::Error> {
        Ok(SourceFile::new(
            BOF::new("", Span::for_text("")),
            Vec::new(),
            EOF::new("", Span::for_text("")),
        ))
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
# #[cfg(feature = "tree-sitter")]
# fn main() -> Result<(), m2_syn::ParseError> {
use m2_syn::{Lex, parse_file};

let file = parse_file("x + 1\ny + x * 2")?;
let normalized = file.to_m2();
assert_eq!(parse_file(&normalized)?.to_m2(), normalized);
# Ok(())
# }
# #[cfg(not(feature = "tree-sitter"))]
# fn main() {}
```

`parse_tokens` connects generated or quoted M2 back to typed syntax:

```rust
# #[cfg(feature = "tree-sitter")]
# fn main() -> Result<(), m2_syn::ParseError> {
use m2_syn::{Lex, parse_tokens, quote_m2};

let quoted = quote_m2! { left + (right) };
let file = parse_tokens(&quoted)?;
assert_eq!(file.to_m2(), "left+(right)");
# Ok(())
# }
# #[cfg(not(feature = "tree-sitter"))]
# fn main() {}
```

`ParsedFile` provides the convenient source-to-source interface over these
lower-level pieces. It owns one mutable typed CST and computes every current
output from that tree, so a token projection cannot silently become stale after
an edit. The original source remains available separately as an immutable
snapshot. `from_source` uses Tree-sitter; `from_source_native` selects the direct
precedence parser.

```rust
# #[cfg(feature = "tree-sitter")]
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use m2_syn::nodes::Symbol;
use m2_syn::{ParsedFile, visit_mut::VisitMut};

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
assert_eq!(file.to_source(), "renamed+ right");

// Stable, uncolored projections are useful in tests and diagnostics.
println!("{}", file.pretty_tokens());
println!("{}", file.pretty_cst());

// One terminal-aware report includes the retained input, recursively flattened
// tokens, and the generated typed CST.
file.print_pretty()?;
# Ok(())
# }
# #[cfg(not(feature = "tree-sitter"))]
# fn main() {}
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

Both parser adapters place comments by the same rule. The native parser reads
them from the trivia of the tokens it consumes; the Tree-sitter adapter lexes
the same source and moves each raw atom's trivia onto the typed terminal or
delimiter covering exactly the same bytes. Emission writes every comment from
the slot that stores it, so a typed tree reproduces its comments in place
without any positional reconstruction.

## Quoting

`quote_m2!` constructs an M2 `TokenStream` from Rust tokens. `$(...)`
interpolates any Rust expression implementing `Lex`. Each interpolation is
first emitted as a real fragment, so punctuation, identifiers, literals, and
groups compose according to their actual token boundaries rather than the Rust
expression's type. Parentheses, brackets, braces, and M2 angle bars are emitted
as recursive raw groups.

`$[pattern in iterator] { ... }` repeats a recursively quoted template using an
ordinary Rust `for` loop. The iterator and pattern are explicit; interpolations
that do not depend on the pattern are reused normally on each iteration.

```rust
use m2_syn::nodes::Symbol;
use m2_syn::{Span, quote_m2};

let value = Symbol::new("value", Span::for_text("value"));
let tokens = quote_m2! {
    result = $(value) + 1;
    return result
};

assert_eq!(tokens.to_string(), "result=value+1;return result");

let values = [
    Symbol::new("left", Span::for_text("left")),
    Symbol::new("right", Span::for_text("right")),
];
let comma = m2_syn::Token![,](Span::for_text(","));
let repeated = quote_m2! {
    $[value in &values] { $(value) $(comma) }
};
assert_eq!(repeated.to_string(), "left,right,");
```

The rendered text preserves retained comments and source-backed layout.
Generated or moved tokens whose spans no longer share one source snapshot use
normalized spacing where the original gap cannot be recovered.

## Development

Run the complete workspace checks from the repository root:

```sh
cargo fmt --all --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The schema is expanded by the `syntax!` procedural macro, so changes to
`src/schema/nodes.rs` are picked up by the next compile; there is no generated source
to refresh or commit.

`m2-syn` is licensed under the [GNU General Public License v3.0](https://github.com/AlexanderGolys/m2-syn/blob/master/LICENSE).
