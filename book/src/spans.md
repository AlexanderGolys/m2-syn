# Spans

A `Span` is a byte range and may retain a shared handle to the source buffer it
indexes. Source-backed spans return their text directly; offset-only spans can
still index an explicitly supplied `str` or `String`.

```rust
use m2_syn::Span;

let source = "quotient ring";
let ring = Span::source(source).subspan(9, 13);
assert_eq!(ring.text(), Some("ring"));
assert_eq!(&source[&ring], "ring");
```

Tokens and textual leaves store only spans, never copied spellings. Composite
nodes derive theirs by joining their components, so a node's span is exactly
the extent of the syntax beneath it. Hand-built syntax may use
`Span::detached()`, which names no location; constructors attach a source-backed
fragment when variable text is supplied.

## No line or column

A span stores one offset per boundary. Line and column are deliberately absent:
they are facts about a particular buffer's *contents*, not about the position,
so they belong to whoever holds the buffer and can be recovered from it on
demand. `rustc` makes the same choice — its `Span` holds `lo` and `hi` byte
positions, and `SourceMap` binary-searches a line table when a diagnostic needs
to print `file:line:col`.

This also removes the need for sentinel endpoints. `(line, column)` has no
greatest element, so "end of file" would have to be invented; a byte offset does
not, and an unclosed group simply ends at `source.len()`, which the lexer has in
hand the moment it reaches end of input.

## Edits

A `TextEdit` acts on both halves of the indexing relation — on the buffer, and
on the spans that index it — and the two actions agree:

```rust
use m2_syn::{Span, TextEdit};

let source = "quotient ring";
let ring = Span::source(source).subspan(9, 13);
let edit = TextEdit::insertion(0, "the ");

let edited = edit.apply(source).unwrap();
assert_eq!(&edited[ring.shifted(&edit)], &source[&ring]);
```

A span shifted past an edit keeps naming the syntax it named before. Text landing
inside a span is carried into it, and a span the edit deletes outright collapses
to a zero-width span where it used to be rather than detaching, because it still
records where the syntax was.

## What spans are not

A span is not the identity of a semantic object, and it is not a stable handle
across edits. Incremental dependency identity belongs in the semantic layer.
Macro hygiene likewise is a separate axis — `rustc` keeps it in a `SyntaxContext`
alongside the byte positions rather than inside them — and is not yet modelled
here.
