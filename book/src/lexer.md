# Native lexer

`lex` is the first native-parser layer. It accepts any byte iterator and
returns the same source-spanned `TokenStream` used by quoting and emission. The
raw token categories are `Group`, `Ident`, `Literal`, and `Punct`; comments are
retained as trivia attached to those atoms or to the surrounding stream. The
lexer does not synthesize typed nodes, adjacency, EOC, or EOF. Physical layout
remains recoverable from source gaps. After greedy token recognition,
structural delimiter pairs become recursive `Group` trees that retain exact
opening and closing spans.

Generated `Token![..]` values are checked typed refinements of these raw atoms,
not another `TokenTree` variant. `Tokenize` performs the later framing step and
represents adjacency explicitly as `CellTokenTree::Adj`; converting the cell
back to lexical tokens removes it. Thus `f(x)` and `f (x)` have the same framed
operator, while `return 1` has none. The lexer applies context-free maximal
munch across generated operators, delimiters, and comment openers.
Consequently `|--1` begins with the `|-` operator rather than a comment, while
a token beginning with `--` is a line comment. Likewise, `(*)` remains the
single greedy operator rather than a parenthesized group. Quotation marks are
part of a single literal token, not delimiter groups.

`$` remains part of an identifier. `Package$name$12` is one `IdentToken`; its
typed `Symbol` merely adds optional subspans for `Package` and `12`. Only one
package qualifier and one decimal index can be marked. Exact `Core$word`
spellings use a separate hardcoded renaming hook, so `Core$not` behaves as
`not`, but `Core$Core$not` is an ordinary symbol.

Precision precedes exponent in a decimal real literal. `1p53`, `.5p20`, and
`1.25p53e3` are single `FloatLiteral` nodes. In `1e3p20`, the completed
exponent ends the literal and `p20` is the adjacent symbol.

The lexer owns one validated source buffer and gives every atom a span into the
same allocation. Variable spellings are therefore sliced from source instead
of copied into token nodes. The byte-iterator entry point first materializes
its finite input; `lex_str` starts directly from source text.

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
