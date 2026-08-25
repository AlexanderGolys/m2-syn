# Native lexer

`lex` is the first native-parser layer. It accepts any byte iterator and
returns the same source-spanned `TokenStream` used by quoting and emission. The raw token categories
are `Eoc`, `Group`, `Ident`, `Literal`, `Punct`, and `Trivia`; the lexer does not
synthesize typed nodes, adjacency, or typed cells. It inserts zero-width `Eoc`
markers after unquoted top-level semicolons, complete top-level lines, and the
final nonempty cell. A lone carriage return and
physical line breaks are both whitespace trivia, with
`Trivia::contains_line_break` carrying the structural distinction. After
greedy token recognition, structural delimiter pairs become recursive `Group`
trees that retain exact opening and closing spans.

Generated `Token![..]` values are checked typed refinements of these raw atoms,
not a sixth `TokenTree` variant. A punctuation atom retains its raw `Punct`, and
a keyword retains its raw `Ident`. Adjacency is instead a parser-owned empty
`Adj` node at the right operand boundary: `f(x)` and `f (x)` have the same
operator, `return 1` has none, and emission produces no token for it. The lexer
applies context-free maximal munch across generated operators, delimiters, and
comment openers. Consequently `|--1` begins with the `|-` operator rather than
a comment, while a token beginning with `--` is a line comment. Likewise,
`(*)` remains the single greedy operator rather than a parenthesized group.
Quotation marks are part of a single literal token, not delimiter groups.

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
