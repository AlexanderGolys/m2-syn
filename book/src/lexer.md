# Lexing Macaulay2

`lex_str` reads source text into a `TokenStream`. `lex` accepts a finite byte
iterator and first collects and validates its input. Each atom refers to the
shared source snapshot through a span, so ordinary spellings need not be copied
into separate strings.

```rust
use m2_syn::lex_str;

let tokens = lex_str("f(x) + 1")?;
assert_eq!(tokens.to_string(), "f(x) + 1");
# Ok::<(), Box<dyn std::error::Error>>(())
```

The lexer recognizes identifiers, literals, punctuation, comments, and balanced
delimiter groups. Expressions, application, and cell boundaries are handled by
later stages described in [Tokens and cells](./pipeline.md).

## Greedy recognition

Token recognition uses maximal munch across operators, delimiters, and comment
openers. This matters for spellings that look like several smaller tokens:

| Source | Recognition |
| --- | --- |
| `***1` | `**`, `*`, then the integer `1` |
| `|--1` | `|-`, `-`, then `1` |
| `-- comment` | One line comment |
| `(*)` | One operator, rather than a parenthesized group |

Comments are neither tokens nor expression nodes. Quotation marks belong to
literal tokens rather than ordinary delimiter groups.

## Unicode characters

Outside ASCII, almost every character continues an identifier, emoji
included: `a€b` and `a😀` are single symbols. The exception is the blocks M2
reserves for operators: U+00A0–U+00BF (`§`, `©`, `°`, `½`, …), `×`, `÷`,
U+2180–U+23FF, U+27C0–U+27FF, and U+2900–U+2BFF. Each character there is a
token of its own, together with an `=` right after it:

| Source | Tokens |
| --- | --- |
| `a§b` | `a`, `§`, `b`, with adjacency between them |
| `§§` | `§`, `§`, with adjacency |
| `§= = 5` | the symbol `§=`, then `=` and `5` |
| `x ⊠= y` | `x`, the augmented operator `⊠=`, `y` |

Such a token is an operator when its spelling is declared, and otherwise a
one-character symbol, as in M2. So `≠=` and `⊢=` are symbols, because neither
operator has an augmented form.

M2's Unicode synonyms are the operators they stand for, written differently:
`→`, `←`, `⇒` for `->`, `<-`, `=>`; `∧`, `∨` for `and`, `or`; `≠`, `≤`, `≥`
for `!=`, `<=`, `>=`; `≪`, `≫`, `⊢` for `<<`, `>>`, `|-`; and `⊕`, `⊗` for
`++`, `**`. `x → x + 1` is an `ExprFunction` whose `Token![->]` prints `→`.
A synonym has no augmented form: `⊕=` is a symbol.

The bundled tree-sitter grammar knows only the declared spellings, so it reads
each synonym and `Core$` alias as the declared spelling put at the same
offsets. `∧` has exactly the three bytes of `and` and no room around it, so
`a∧b` cannot be read as `a and b` and is reported as unsupported there; write
`a ∧ b`, or use the native parser.

## Qualified identifiers

`Package$name$12` is one identifier token. The typed `nodes::Symbol` can also
retain subspans for its package qualifier and decimal index. `$` is part of the
identifier spelling, so consumers should not split every occurrence into
separate tokens.

Every M2 keyword also has a `Core$` spelling, but they don't all work alike:

Word operators and construct heads
: `Core$not`, `Core$and`, `Core$if`, `Core$for`, `Core$symbol` … behave as the
  bare word: `Core$if x then y` is an `if`, and `Core$symbol (` quotes the `(`.

Construct continuations
: `then`, `else`, `do`, `list`, `from`, `to`, `in`, `when`, `of`, and `except`
  are matched by their exact spelling, so `if x Core$then y`, and even
  `x = Core$then` alone, are syntax errors, as in M2.

Punctuation
: `Core$+` is not one token: it lexes as the symbol `Core$` followed by `+`.

`Core$Core$not` remains an ordinary symbol.

## Numeric literals

Precision comes before the exponent in a decimal real literal. `1p53`, `.5p20`,
and `1.25p53e3` are each single floating-point literals. In `1e3p20`, the exponent
ends the literal, leaving `p20` as an adjacent symbol.

## Recovering missing delimiters

`lex_str_recovering` and `lex_recovering` can retain usable tokens when a closing
delimiter is missing at end of input. The recovered closer has a zero-width span
at EOF, and the returned diagnostics describe the missing delimiter. Recovery
does not make arbitrary malformed source valid; later framing and parsing may
still fail.

## Comments { #comments }

The lexer records where each comment lies in the source, and a span answers
which comments are next to it. A comment that starts on the line where an atom
ends follows that atom; every other comment precedes the next atom:

```text
x = 1 -- after `1`
-- before `y`
y = 2
```

`Span::comments_after` and `Span::comments_before` ask a single span.
`Lex::comments` asks any syntax, a cell or a whole file, and sorts the answer:

Outer comments
: Complete-line `--` comments right above the first token. Consecutive lines
  coalesce into one block, so a blank line starts a new one. These document the
  syntax.

Inner comments
: Every other comment above the first token, the comments within the syntax,
  and those after it on its last line.

Because nothing is stored on tokens or nodes, a rewritten tree keeps each
comment next to the syntax whose spans it kept, and a renamed token keeps the
comments of the token it replaced.
