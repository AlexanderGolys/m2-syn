# Generated implementations

The declaration generates the following API without requiring per-node
handwritten boilerplate. Runtime alternatives are represented only by the
grammar-backed enums themselves; there is no parallel global node-kind enum.

| Node form | Generated API |
| --- | --- |
| every node and category | `Spanned`, `Reconstruct<N>`, and traversal dispatch |
| token | `Token`, constructor, reconstruction, and a `Token![spelling]` arm |
| text leaf | text-plus-span constructor and exact CST reconstruction |
| product struct | concrete-node matching, storage-aware constructor, child reconstruction, and default child walkers |
| coproduct enum | category matching, alternative reconstruction, default dispatch, and unambiguous direct/transitive `From` conversions |

The traversal traits have ordinary, independently addressable modules:
`visit::Visit`, `visit_mut::VisitMut`, and `fold::Fold`. Their node-specific
method inventories and free default walkers are generated from the syntax
declaration, so adding a node cannot leave one traversal API incomplete. An
override can inspect a node and call its module's free walker to continue into
its children. `VisitMut` edits the owned typed graph in place. `Fold` consumes
nodes and returns the reconstructed graph.

`cargo run --example type_inference` demonstrates the intended analysis path:
generated `Visit` dispatch selects node-specific rules, and the binary rule
consumes the neighbouring operand facts to compute an upward-closed result
range. Graph storage and fixed-point solving belong in the analysis/LSP layer;
this example only verifies the syntax traversal boundary. Its generic
`InferenceContext` leaves concrete types, values, and deferred database or
signature queries under the control of the consuming analysis layer.
