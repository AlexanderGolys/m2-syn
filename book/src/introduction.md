# Introduction

`m2-syn` is a parser-independent typed syntax graph for Macaulay2. One
`syntax!` declaration describes the algebraic shape of the graph; generated
implementations provide construction, traversal, source provenance, and
conversion from a concrete syntax tree.

The API is experimental and may change while the typed grammar and parser
adapter interfaces are being established.

## Project direction

This crate began as the syntax and analysis layer for the Macaulay2 language
server. It grew into a standalone, `syn`-like foundation for M2: an owned,
typed representation that can be constructed from source, traversed and
rewritten, and emitted as code.

The current priority is the basic source pipeline:

1. read M2 source and parse it with Tree-sitter;
2. reconstruct the complete typed AST;
3. emit normalized M2 that parses back to the same structure.

The syntax model follows the grammar published as `tree-sitter-macaulay2`
`6.0.0`; `Cargo.toml` pins that version but resolves it against the sibling
`../tree-sitter-macaulay2` checkout during local development, so the two stay
in lockstep without a registry round trip on every grammar change.

Round-tripping through both the Tree-sitter and native parser backends works
end to end, and `quote_m2!`/`parse_quote_m2!` are stable enough for everyday
use (see [Quoting](./quoting.md)). The next steps are a converter from M2's
own `Core` parser output and a macro-expansion engine built on the typed
graph; `Core`'s own concrete tree is considerably less structured than either
parser backend here (no named fields, several statement forms share one flat
node shape distinguished only by which positional slots are populated), so
the typed graph and its generated APIs must stay parser-independent rather
than special-cased around it.

Longer term, `m2-syn` should support semantic and typechecking walks,
incremental analysis after referenced code changes, selected node metadata,
and procedural macros for M2. `Visit`, `VisitMut`, and `Fold` are intended as
the basic traversal and transformation vocabulary for that work. The guiding
design rule is to describe as little grammar as possible in `syntax!` and
generate the repetitive construction, traversal, reconstruction, and printing
machinery.
