# The schema

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
| `(_)` | a child without a Tree-sitter field name | positional reconstruction from the next matching child |
| `Lines<T>` | a `Vec<T>` printed one element per line | the separator travels with the functor |
| `#[cst(kind = assignment)]` | a Rust name whose external CST node has another name | reconstruction from the named CST kind |
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

Boxing is explicit: put `*` after a declaration name when recursive references
to that type need `Box`. Ordinary fields, token types, and nonrecursive
categories remain inline. Constructors accept the declared types and hide that
storage conversion. Generated signatures consistently refer to literal token
types through `Token![...]`; their concrete Rust names are an implementation
detail of the expansion.

`Delimited<T>` retains any of the four paired delimiter families, while a
second type parameter such as `Delimiter![()]` refines the family statically.
Both spelled boundary spans are stored. The convenient type macros construct
the refined forms:

```rust
use m2_syn::{Lex, Span};

type Parenthesized = m2_syn::paren!(m2_syn::Token![+]);
let value: Parenthesized = m2_syn::paren!(
    m2_syn::Token![+](Span::detached()),
    Span::detached(),
);
assert_eq!(value.to_m2(), "(+)");
```

`paren!`, `brackets!`, `braces!`, and `angle_bars!` cover the four families.
`punct!(T)` names a nonempty comma-punctuated sequence; `punct!(value first)`
and `punct!(pairs first; comma => next)` construct one while retaining every
typed comma and its span. Use `Option<Punctuated<T>>` when the whole sequence
may be absent, or put an empty alternative in `T` when components may be empty.
Every delimiter atom and wrapper implements both `Parse` and `Lex`, so they can
be direct `parse_quote_m2!` targets.

A field normally uses its Rust name as its CST field name. Mark it `(_)`
when it should consume the next matching unlabelled child instead. The
schema deliberately permits only named struct fields and bare-type enum
variants — a variant never spells out a separate name, since it's always
either the wrapped type's own name or, for a variant wrapping a token, that
token's capitalized name — keeping both the declaration and generated API
shaped like ordinary Rust rather than requiring the same fact to be written
twice.
