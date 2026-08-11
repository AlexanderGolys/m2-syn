# m2-syn

`m2-syn` is a parser-independent typed syntax graph for Macaulay2. One
`syntax!` declaration describes the algebraic shape of the graph; generated
implementations provide construction, traversal, source provenance, and
conversion from a concrete syntax tree.

The API is experimental and may change while the typed grammar and parser
adapter interfaces are being established.

```rust
use m2_syn::{TokenStream, ToTokens, syntax};

mod grammar {
use super::*;

syntax! {
    tokens {
        Equal [=],
        LeftParenthesis ["("],
        RightParenthesis [")"]
    }

    pub struct Symbol;

    pub struct BinaryExpression {
        pub left: Expr,
        pub operator: Operator,
        pub right: Expr,
    }

    pub enum Expr {
        Binary(BinaryExpression),
        Symbol(Symbol),
    }

    pub enum Operator {
        Equal(Equal),
    }
}

impl ToTokens for BinaryExpression {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.left.to_tokens(output);
        self.operator.to_tokens(output);
        self.right.to_tokens(output);
    }
}
}

# fn main() {}
```

## Declaration model

| Declaration | Syntax role | Stored data |
| --- | --- | --- |
| `Token [spelling]` | anonymous literal token | `Span` |
| unit struct | named text leaf | `String` and `Span` |
| field-bearing struct | product node | declared children |
| one-field enum | coproduct/category node | one declared alternative |
| `Option<T>` | optional child | zero or one `T` |
| `Vec<T>` | repeated child | ordered children |
| `Box<T>` | explicit indirection | one boxed `T` |

Direct recursive product fields are boxed automatically. Token types and
categories containing only tokens remain inline. Constructors accept the
types written in the declaration and hide that storage choice.

A named Rust field uses the same Tree-sitter field name by default. A tuple
field or named field starting with `_` consumes the next matching unfielded
child. The defaults can be overridden with `#[syntax(field = "...")]` and
`#[syntax(unfielded)]`. A named node kind defaults to the snake-case form of
its Rust name and can be overridden with `#[syntax(kind = "...")]`.

## Generated implementations

The declaration generates one `SyntaxKind` and the following API without
requiring per-node handwritten boilerplate.

| Node form | Generated API |
| --- | --- |
| every node and category | `AstNode`, `Spanned`, `Reconstruct<N>`, and traversal dispatch |
| token | `Token`, `ConcreteNode`, constructor, and a `Token![spelling]` arm |
| text leaf | `ConcreteNode` and a text-plus-span constructor |
| product struct | concrete-node matching, storage-aware constructor, child reconstruction, and default child walkers |
| coproduct enum | category matching, alternative reconstruction, default dispatch, and unambiguous direct/transitive `From` conversions |

The traversal traits have ordinary, independently addressable modules:
`visit::Visit`, `visit_mut::VisitMut`, and `fold::Fold`. Their node-specific
method inventories and free default walkers are generated from the syntax
declaration, so adding a node cannot leave one traversal API incomplete. An
override can inspect a node and call its module's free walker to continue into
its children. `VisitMut` edits the owned typed graph in place. `Fold` consumes
nodes and returns the reconstructed graph.

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

Generated reconstruction depends only on `CstNode`. The optional
`tree-sitter` feature supplies `TreeSitterNode`, which adapts Tree-sitter
identity, field names, source text, extras, and ranges to that interface.
Other parsers can implement the same trait without entering the generated
syntax model.

With the `tree-sitter` feature enabled, `parse_file` is the concrete
Macaulay2 parser entry point:

```rust
# #[cfg(feature = "tree-sitter")]
# fn main() -> Result<(), m2_syn::ParseError> {
use m2_syn::{SourceId, ToTokens, parse_file};

let file = parse_file("x = 1\ny = x + 2", SourceId(1))?;
assert_eq!(file.to_m2(), "x = 1\ny = x + 2");
# Ok(())
# }
# #[cfg(not(feature = "tree-sitter"))]
# fn main() {}
```

## Quoting

`quote_m2!` constructs an M2 `TokenStream` from Rust tokens. `$(...)`
interpolates any Rust expression implementing `ToTokens`; `EOC` and `EOF`
represent syntax boundaries rather than ordinary identifier text.

```rust
use m2_syn::{Span, Symbol, quote_m2};

let value = Symbol::new("value", Span::detached());
let tokens = quote_m2! {
    result = $(value) + 1 EOC
    return result EOF
};

assert_eq!(tokens.to_string(), "result=value+1\nreturn result");
```

The rendered text is normalized M2 source, not a lossless reproduction of
whitespace or comments discarded by the typed graph.

## Development

Run the complete workspace checks from the repository root:

```sh
cargo fmt --all --check
cargo check --workspace --all-features
cargo clippy --workspace --all-features --all-targets -- -D warnings
cargo test --workspace --all-features
```

`m2-syn` is licensed under the [GNU General Public License v3.0](LICENSE).
