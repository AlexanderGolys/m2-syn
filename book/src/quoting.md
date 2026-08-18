# Quoting

`quote_m2!` constructs an M2 `TokenStream` from Rust tokens. `$(...)`
interpolates any Rust expression implementing `ToTokens`. Each interpolation is
first emitted as a real fragment, so punctuation, identifiers, literals, and
groups compose according to their actual token boundaries rather than the Rust
expression's type. Parentheses, brackets, braces, and M2 angle bars are emitted
as recursive raw groups.

`$[pattern in iterator] { ... }` repeats a recursively quoted template using an
ordinary Rust `for` loop. The iterator and pattern are explicit; interpolations
that do not depend on the pattern are reused normally on each iteration.

```rust
use m2_syn::{Span, Symbol, quote_m2};

let value = Symbol::new("value", Span::detached());
let tokens = quote_m2! {
    result = $(value) + 1;
    return result
};

assert_eq!(tokens.to_string(), "result=value+1;return result");

let values = [
    Symbol::new("left", Span::detached()),
    Symbol::new("right", Span::detached()),
];
let comma = m2_syn::Token![,](Span::detached());
let repeated = quote_m2! {
    $[value in &values] { $(value) $(comma) }
};
assert_eq!(repeated.to_string(), "left,right,");
```

The rendered text is normalized M2 source, not a lossless reproduction of
whitespace or comments discarded by the typed graph.
