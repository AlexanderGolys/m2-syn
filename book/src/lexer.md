# Native lexer

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
