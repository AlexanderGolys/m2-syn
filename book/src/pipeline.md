# Representation pipeline

The frontend has a lexical representation, a framed cell representation, and
one generated typed graph:

```text
source --Lex--> TokenStream --Tokenize--> CellStream<CellTokenStream> --Parser--> typed CST
   |                                                                            |
   +------------------> Tree-sitter CST --------------------Migrate--------------+
                                                                                |
source <-----------------------Display/Lex---------------------------------------+
```

`TokenStream` contains only lexical token trees. `Tokenize` performs the first
grammar-aware step: it confirms top-level cell boundaries and recursively adds
`CellTokenTree::Adj` markers. Each resulting `CellTokenStream` is therefore a
partially parsed cell, and `CellStream` records EOC and EOF structurally through
the sequence rather than virtual tokens. Converting a cell back into a raw
`TokenStream` removes adjacency recursively.

`Parse` and `Lex` are the two directions at the typed boundary. For a
typed value `c`, parsing its emission should recover `c`. Emitting a parsed raw
stream may normalize insignificant layout:

```text
Parse(Lex(c)) = c
Lex(Parse(tokens)) = normalize(tokens)
```

The two parser backends differ only in how they reach the generated graph.
`NativeParser` consumes framed cells and constructs it directly from the shared
token cursor.
`TreeSitterParser` lowers Tree-sitter's concrete grammar layout directly and
uses `ExternalCstNode` plus generated `Migrate` for compatible leaves,
operators, and tokens. It never routes the source through `NativeParser`.
Those adapter traits are not an alternative graph or a traversal API; typed
traversal is generated as `Visit`, `VisitMut`, and `Fold`.

The ownership boundaries are intentionally narrow:

| Location | Owns |
| --- | --- |
| `lex/token_stream.rs` | raw atoms and recursive groups |
| `lex/lexer.rs` | byte recognition and delimiter balancing |
| `lex/cell_stream.rs` | framed cells, inferred adjacency, and cell boundaries |
| `parser/parse.rs` | the shared token cursor and parser traits |
| `parser/native.rs` | precedence and typed-node construction |
| `migration/` | the external-CST reconstruction adapter |
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

The canonical declaration lives in `src/schema/nodes.rs`, where the `syntax!`
procedural macro expands it into the token, node, traversal, and fold items,
and into the `visit`, `visit_mut`, and `fold` modules re-exported at the crate
root. The expansion is items only — no `macro_rules!` — so rust-analyzer
resolves every generated name.

`Token!` and `Delimiter!` are ordinary source in `src/schema/macros.rs` instead of
part of that expansion. Neither depends on the schema: `Token!` forwards the
spelling to the `token!` procedural macro, and the delimiter families are a
fixed set.
