# Representation pipeline

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
