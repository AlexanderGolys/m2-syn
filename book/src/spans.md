# Source locations

A `Span` pairs a shared immutable `Source` with an optional half-open byte range.
A located span can return its source text directly. An unlocated span retains
its source identity without claiming a byte range.

```rust
use m2_syn::{Source, Span};

let source = Source::new("quotient ring");
let ring = Span::ranged(source, 9..13);
assert_eq!(ring.text(), Some("ring"));
assert_eq!(ring.line_column_range().unwrap().start.column, 10);
```

`Source::new` creates an ordinary source snapshot. `Source::synthetic` creates
one for constructed syntax, and `Span::for_text` conveniently covers an entire
synthetic fragment. Use `Span::unlocated(source)` when the source is known but a
location is not.

Typed leaves retain their spelling through spans. Composite nodes implement
`Spanned` using their components. After a transformation, a tree may contain
spans from the original source and from newly constructed fragments; it need
not occupy one contiguous range in one buffer.

## Byte offsets and displayed positions

Ranges use UTF-8 byte offsets. `Source::line_column` and
`Span::line_column_range` convert valid positions to one-based line and column
values for diagnostics. Columns count Unicode scalar values, rather than bytes
or terminal display cells. The source owns the line-start table and computes
it when needed.

A source snapshot remains immutable. The same offset in two different snapshots
can refer to different text, so retain the source identity together with a range.

## Applying edits

`TextEdit` represents an insertion or deletion. It can apply the change to a
string and shift an existing span's boundaries:

```rust
use m2_syn::{Span, TextEdit};

let source = "quotient ring";
let ring = Span::for_text(source).subspan(9, 13);
let edit = TextEdit::insertion(0, "the ");
let edited = edit.apply(source).unwrap();
let shifted = ring.shifted(&edit);
assert_eq!(&edited[shifted.range().unwrap()], "ring");
```

`shifted` adjusts offsets; it retains the original source snapshot. Index the
edited buffer explicitly when using those adjusted offsets. Shifting spans
alone does not reparse syntax or update the original source text.

An insertion shifts boundaries at or after its position. A deletion collapses
covered boundaries onto its start. A completely deleted span becomes zero-width.
Neither these rules nor a span's source location provide semantic identity
across edits; applications must track that separately.
