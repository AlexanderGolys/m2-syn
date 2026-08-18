# Spans

`Span` answers where syntax originated; it is not the identity of a semantic
object and does not make coordinates persistent across edits. Located tokens
and text leaves retain a `SourceId` and `TextRange`. Product and category spans
are derived from their children. Parser-independent syntax uses
`Span::detached()` when no source location exists.

This provenance is what diagnostics, hovers, semantic tokens, navigation, and
source edits use to translate a typed node back to a document. Incremental
dependency identity belongs in the semantic layer rather than in `Span`.
