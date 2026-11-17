# m2-syn

[![crates.io](https://img.shields.io/crates/v/m2-syn.svg)](https://crates.io/crates/m2-syn)
[![docs.rs](https://img.shields.io/docsrs/m2-syn)](https://docs.rs/m2-syn)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

`m2-syn` is a parser-independent typed syntax graph for Macaulay2. Every
syntax type is a plain Rust declaration with `#[derive(Syntax)]`, the operators and keywords
come from one `tokens!` table, and a `traversal!` inventory of the type names
generates the visitors. Generated implementations provide construction,
traversal, source provenance, and conversion from a concrete syntax tree.

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
design rule is to describe as little grammar as possible in the schema and
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
| `schema/tokens.rs` | the token table: spellings, parser roles, and binding strengths |
| `schema/nodes.rs` | one plain declaration per syntax type |
| `schema/traversal.rs` | the inventory of types the visitors cover |
| `m2-syn-macros/src/` | generation of tokens, nodes, traversal, parsing, and emission |

```rust,ignore
// src/schema/tokens.rs
crate::tokens! {
    [+] [-]     { bin, pref, aug } (50, L)
    [=]         { infix }          (14, R)
    [if]
    [else] [then]                  (12)
    [,]                            (10)
}

// src/schema/nodes.rs
/// A binary operation: `a + b`, `x == y`, `L_0`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprBinary {
    pub left: Box<Expr>,
    pub op: BinaryOperator,
    pub right: Box<Expr>,
}

/// A conditional: `if c then x else y`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprIf {
    pub if_token: Token![if],
    pub cond: Box<Expr>,
    pub then_clause: ThenClause,
    pub else_clause: Option<ElseClause>,
}

// src/schema/traversal.rs
crate::traversal! {
    BinaryOperator, Expr, ExprBinary, ExprIf, ThenClause, ElseClause,
}
```

Every type is declared exactly as it is stored, with every field and every
name written out. `#[derive(Syntax)]` adds the constructor, printing,
migration, pretty tree, and child walkers from the field types it reads; it
never adds fields or names. The token table generates
the token types and the four operator enums. The traversal inventory only
lists names: it generates the `visit`, `visit_mut`, and `fold` modules, which
are re-exported at the crate root, and a type left out of it fails to compile.

`Expr` is one flat enum whose variants hold `Expr<Kind>` structs such as
`ExprBinary`, `ExprField`, and `ExprAssign`. Construct them directly and use
`Expr::from(node)` to wrap them. No checked-in expansion output is required.

`Token!` is ordinary source in `src/schema/macros.rs` instead of part of that
expansion. It does not depend on the schema: it forwards the spelling to the
`token!` procedural macro, which names a token or, for `()`, `[]`, `{}`, and
`<||>`, a delimiter atom.

## Declaration model

| Declaration | Syntax role | Generated shape |
| --- | --- | --- |
| `[+] { bin, pref, aug } (50, L)` | left-associative binary and prefix binding plus augmented-token generation | token types, parser metadata, and inferred operator-enum membership |
| `[=] { infix } (14, R)` | right-associative syntax-specific infix binding without generic operator membership | a typed token and parser metadata |
| `[else] (12)` | a keyword and its parser strength | a typed keyword token and parser metadata |
| `["≠"] = [!=]` | another spelling of a declared token, as M2's `findSynonyms` lists them | no new type: `Token![!=]` also matches and keeps `≠` |
| `[EOF] { marker }` | a position that is never written; its spelling stays an ordinary name in source | a typed zero-width token that emits nothing |
| `struct Name { left: Box<Expr>, ... }` | concrete product node | declared children |
| `enum Name { Variant(Box<Expr>), ... }` | a sum of alternatives | one variant per payload, with `From` for each payload no other variant shares |
| `Token![=]` | literal token field | the type selected by the `Token!` macro |
| `Box<T>`, `Option<T>`, `Vec<T>`, `Punctuated<T>`, `Delimited<T>` | boxed, optional, repeated, nonempty comma-punctuated, or arbitrarily delimited children | the corresponding public wrapper type |
| `delimiter: Token![()]` | a product enclosed by a delimiter family | the typed delimiter pair, printed around the other fields |
| `#[syntax(leaf = text)]` with a `span` field | a terminal retaining its text | text and span; `symbol`, `integer`, `float`, and `empty` select other terminals |
| `#[syntax(literal = string)]` with a `span` field | a product spelled as a string literal | emission as one literal token, with no delimiter atom |
| `#[syntax(cst = assignment)]` | a Rust name whose external CST node has another name | migration from the named CST kind |

Every token is declared in `tokens!`; adjacent spellings share the
metadata written on their right. A binary role writes `L` or `R`, while a
literal strength remains available for a non-associative rule. A prefix role
binds at the token's precedence unless a third slot overrides it, and `post`
writes no strength at all. The `aug` tag generates the corresponding assignment
token, which binds exactly like the `=` it abbreviates and so takes its rule
from that declaration.

Boxing is explicit rather than inferred: a `*` after a field type stores it
through `Box`. Only recursive cycles need it; ordinary fields, token types,
and nonrecursive categories stay inline. Generated signatures consistently refer to literal token
types through `Token![...]`; their concrete Rust names are an implementation
detail of the expansion.

Generated concrete syntax nodes and literal-token structs live under
`m2_syn::nodes`. Construction and type macros such as `Token!`, `paren!`, and
`punct!` remain available from the crate root, as do generic
containers such as `Delimited` and `Punctuated`.

`Delimited<T>` retains an arbitrary one of the four paired delimiter families;
`Delimited<T, Token![()]>` and the other concrete delimiter atoms refine
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

A field uses its Rust name as its CST field name, and every variant is named
in the declaration, so the generated API reads like the declaration itself.

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
| coproduct enum | category matching, alternative migration, default dispatch, and `From` for each unshared payload |

The traversal traits have ordinary, independently addressable modules:
`visit::Visit`, `visit_mut::VisitMut`, and `fold::Fold`. Their node-specific
method inventories and free default walkers are generated from the inventory
in `src/schema/traversal.rs`, so adding a node cannot leave one traversal API
incomplete. Tokens have no methods of their own: `Visit::visit_terminal` sees
every token, leaf, and literal as a `&dyn Spanned`. An
override can inspect a node and call its module's free walker to continue into
its children. `VisitMut` edits the owned typed graph in place. `Fold` consumes
nodes and returns the migrated graph.

`cargo run --example type_inference` demonstrates the intended analysis path:
generated dispatch selects one node-specific rule per syntax form instead of a
hand-written match, and a rule calls its own free walker before reading the
classes its operands recorded. Literals, the four bracketing classes, prefix and
binary operators, simple assignment, parenthesization, conditionals, and cell
value suppression each own one override; a conditional joins its branches by
walking the Macaulay2 class hierarchy, so `if p then 1 else 1.5` infers `Number`
and an unmodelled expression keeps the `Thing` bound. The example deliberately
has no symbol registry or solver; those belong in the analysis/LSP layer rather
than the syntax crate.

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
are `Group`, `Ident`, `Literal`, and `Punct`. Comments are not tokens and are
stored nowhere in the stream: like ordinary layout, they are recovered from
source spans. The lexer does not synthesize typed nodes, adjacency, or cells. After
greedy token recognition, structural delimiter pairs become recursive `Group`
trees that retain exact opening and closing spans.

Generated `Token![..]` values are checked typed refinements of these raw atoms,
not a sixth `TokenTree` variant. A punctuation atom retains its raw `Punct`, and
a keyword retains its raw `Ident`. Adjacency is the binary operator `SPACE`,
`Token![SPACE]`, also named `Token![]`, so `f x` is an `ExprBinary`. Written out, as in `f SPACE x` or
`f Core$SPACE x`, it retains its `Ident`; implied, as in `f x`, it is an empty
atom at the right operand's boundary that emits nothing. It is independent of
physical whitespace (`f(x)` and `f (x)` have the same operator), and absent
from `return 1`. The lexer applies context-free maximal
munch across generated operators, delimiters, and comment openers.
Consequently `|--1` begins with the `|-` operator rather than a comment, while
a token beginning with `--` is a line comment. Likewise, `(*)` remains the
single greedy operator rather than a parenthesized group. Quotation marks are
part of a single literal token, not delimiter groups.

The lexer records where every comment lies in the `Source`, which keeps the
index next to its text (a source built some other way indexes itself on first
use). Any span then answers which comments are next to it: a comment starting
on the line where an atom ends follows that atom (`Span::comments_after`), and
every other comment precedes the next atom (`Span::comments_before`).

```text
x = 1 -- after `1`
-- before `y`
y = 2
```

`Lex::comments` collects the comments of any syntax on demand. Consecutive
complete-line `--` comments right above its first token coalesce into outer
documentation blocks; every other comment above it, within it, or after it on
its last line is inner. A comment on its own line below the last cell belongs
to no cell, only to `SourceFile::comments`.

`$` remains an identifier-continuation character. `Package$name$12` is one
`IdentToken` with one overall span, which additionally records
`package_qualifier = Package` and `index_qualifier = 12` as optional subspans;
the typed `Symbol` wraps that token.
Only `package$name`, `name$decimal`, and their combined form are marked, so the
representation permits at most one qualifier of each kind. This is metadata,
not grammar-level tokenization. Exact `Core$word` spellings of declared word
tokens behave as the word wherever M2 acts on the word itself: `Core$not` is
`not` and `Core$if x then y` is an `if`, while `Core$Core$not` remains an
ordinary symbol. A word that only continues a construct (`then`, `else`, `do`,
`list`, `from`, `to`, `in`, `when`, `of`, `except`) is matched by its exact
spelling, so its alias is a syntax error everywhere, as in M2: `if x Core$then y`
and even `x = Core$then` are rejected.

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
rule through `Token![..]`.

The lexer emits one flat `TokenStream`; `Tokenize` splits it into the cells
evaluated at global scope. A `;`
ends a cell unless a quote specifier took it literally, and a line break ends
one only where the tokens so far already form a complete expression. Wrapping a
region in a delimiter therefore never rewrites its tokens.

```rust
# use std::convert::Infallible;
use m2_syn::nodes::SourceFile;
use m2_syn::{CellStream, Parser, Span, Token, Tokenize, lex_str, parse_with};

struct EmptyParser;

impl Parser for EmptyParser {
    type Error = Infallible;

    fn parse_cells(&mut self, _cells: CellStream) -> Result<SourceFile, Self::Error> {
        Ok(SourceFile::new(
            Token![BOF](Span::for_text("")),
            Vec::new(),
            Token![EOF](Span::for_text("")),
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
boundaries. Comments are hidden by default and adjacent ones coalesce when
enabled. Source
ranges can be disabled and ANSI styling can be selected explicitly through
`PrettyReport`.

Run the inspector with its built-in example or give it an M2 source file:

```sh
cargo run --example inspect
cargo run --example inspect -- path/to/file.m2
```

Neither parser adapter places comments: both keep the spans of the source
they parse, so the same comments are next to the same syntax. Emission writes
the gap between two atoms verbatim when it holds only whitespace and comments,
and otherwise the comments next to each atom, so a typed tree reproduces its
comments in place, and an edited tree keeps them beside the syntax they
annotate.

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

The schema is expanded by the `tokens!` and `traversal!` procedural macros and
`#[derive(Syntax)]`, so changes to `src/schema/` are picked up by the next compile; there is no generated source
to refresh or commit.

## License

`m2-syn` and `m2-syn-macros` are licensed under either the
[MIT License](LICENSE-MIT) or the [Apache License, Version 2.0](LICENSE-APACHE),
at your option.
