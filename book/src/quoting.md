# Quoting and expansion

`quote_m2!` constructs an M2 `TokenStream` inside Rust. It produces tokens;
parsing those tokens into a particular syntax type is a separate step.

```rust
use m2_syn::{Lex, parse_native, quote_m2};

let tokens = quote_m2! { answer = 40 + 2 };
let file = parse_native(&tokens.to_string())?;
assert_eq!(file.to_m2(), "answer=40+2");
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Interpolate Rust values

Write `$(expression)` to emit a Rust value implementing `Lex`. Interpolation
uses the value's lexical representation, including real token and delimiter
boundaries.

```rust
use m2_syn::{parse_fragment_str, IdentToken, quote_m2};

let value: IdentToken = parse_fragment_str("value")?;
let tokens = quote_m2! {
    result = $(value) + 1;
    return result
};
assert_eq!(tokens.to_string(), "result=value+1;return result");
# Ok::<(), Box<dyn std::error::Error>>(())
```

Parentheses, brackets, braces, and M2 angle bars become recursive token groups.
The rendered text supplies separation needed to keep adjacent tokens distinct;
it does not reproduce the Rust template's whitespace.

## Repeat a template

`$[pattern in iterator] { ... }` repeats a quoted body with an ordinary Rust
iteration pattern. Put separators in the template explicitly:

```rust
use m2_syn::{parse_fragment_str, IdentToken, quote_m2};

let values = [
    parse_fragment_str::<IdentToken>("left")?,
    parse_fragment_str::<IdentToken>("right")?,
];
let tokens = quote_m2! {
    $[value in &values] { $(value), }
};
assert_eq!(tokens.to_string(), "left,right,");
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Parse a quoted fragment

`parse_quote_m2!` infers the target from its Rust context:

```rust
use m2_syn::{IdentToken, parse_quote_m2};

let symbol: IdentToken = parse_quote_m2!(answer);
assert_eq!(symbol.text(), "answer");
```

The macro panics if its output cannot be parsed as the target type. Use
`parse2::<T>(quote_m2! { ... })` when the caller should receive a `Result`.

## Expand complete cells

`ProcMacro<Args>` is the interface for transformations over a `CellStream`.
An implementation receives input cells and invocation arguments, then returns
expanded cells or its own error. It may parse the input, transform the typed
tree, and frame the emitted output again.

`LiteralReturnTypes` is a supplied example. Given a method such as:

```m2
f = method();
f ZZ := x -> 1
```

it adds a `ZZ` return annotation to the installation. Floating-point and string
literals receive `RR` and `String` annotations respectively; existing annotations
are preserved. The trait defines the expansion boundary, and the caller controls
when and how it is invoked.
