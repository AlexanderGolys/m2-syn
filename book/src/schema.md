# The schema

| Declaration | Syntax role | Generated shape |
| --- | --- | --- |
| `PREC_CONTROL = 12` | a structural precedence with no single owning token, consumed directly by the native parser | a crate-visible precedence constant |
| `[+] { bin, pref, aug } (50, 50, 50)` | binary and prefix binding plus augmented-token generation, precedence inline | token types, parser metadata, and inferred operator-enum membership |
| `[=] { infix } (14, 13, _)` | syntax-specific infix binding without generic operator membership | a typed token and parser metadata |
| `[else]` | a keyword already identified by its typed token | a typed keyword token |
| `struct Name;` | named text leaf | `String` and `Span` |
| `struct Name { left: Expr, ... }` | concrete product node | declared children |
| `enum Name { Expr, ... }` | grouping category; each bare variant names the type it wraps, so the wrapped name doubles as the variant name | one declared alternative per variant |
| `Token![=]` | literal token field | the type selected by the generated `Token!` macro |
| `X?`, `Vec<T>`, `Punctuated<T>` | optional, repeated, or comma-punctuated children | the corresponding public wrapper type |
| `(_)` | a child without a Tree-sitter field name | positional reconstruction from the next matching child |
| `(_ lines)` | a positional `Vec<T>` child that's newline- rather than space-separated | positional reconstruction with line-separated emission |
| `#[cst(kind = assignment)]` | a Rust name whose external CST node has another name | reconstruction from the named CST kind |
| `#[delimiter(parenthesis)]` | a product enclosed by a delimiter family | a concrete node with its typed delimiter atom |

Each operator token carries its own `(precedence, binary_strength,
unary_strength)` triple directly in `tokens { ... }`, with `_` for any slot
its flags don't use — `bin`/`infix` use the binary-strength slot, `pref` uses
the unary-strength slot, `post` uses neither. The separate `precedence` stage
only holds names for structural precedences with no single owning operator
token (delimiter/control-clause stoppers the native parser references
directly); it is not a shared table operators look their numbers up in, so a
level used by exactly one token is just written as a literal there and
nowhere else. The `aug` tag generates the corresponding assignment token
using the binding declared once in `augmented: (precedence,
binary_strength)`.

Direct recursive product fields are boxed automatically. Token types and
categories containing only tokens remain inline. Constructors accept the
types written in the declaration and hide that storage choice. Generated
signatures consistently refer to literal token types through `Token![...]`;
their concrete Rust names are an implementation detail of `src/gen/tokens.rs`.
Delimited products likewise store generated delimiter atoms. The six families
are the four paired collection delimiters plus the implicit empty cell
delimiter and the semicolon cell delimiter. Their convenient type macros also
construct values:

```rust
use m2_syn::{Span, ToTokens};

type Parenthesized = m2_syn::paren!(m2_syn::Token![+]);
let value: Parenthesized = m2_syn::paren!(
    m2_syn::Token![+](Span::detached()),
    Span::detached(),
);
assert_eq!(value.to_m2(), "(+)");
```

`naked!`, `semicolon!`, `paren!`, `brackets!`, `braces!`, and `angle_bars!`
cover all six families. `punct!(T)` names a comma-punctuated sequence;
`punct!()`, `punct!(value value)`, and `punct!(pairs first; comma => next)`
construct one while retaining every typed comma and its span. Every delimiter
atom and each of these wrapper types implements both `Parse` and `ToTokens`, so
they can be direct `parse_quote_m2!` targets.

A field normally uses its Rust name as its CST field name. Mark it `(_)`
when it should consume the next matching unlabelled child instead. The
schema deliberately permits only named struct fields and bare-type enum
variants — a variant never spells out a separate name, since it's always
either the wrapped type's own name or, for a variant wrapping a token, that
token's capitalized name — keeping both the declaration and generated API
shaped like ordinary Rust rather than requiring the same fact to be written
twice.
