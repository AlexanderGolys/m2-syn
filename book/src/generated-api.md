# Walking and editing syntax

The tree has three generated traversal interfaces. Each provides a method for
every syntax node and a free walker with the default child traversal.

| Trait | Access | Typical use |
| --- | --- | --- |
| `visit::Visit<'ast>` | Shared references | Collect names or compute facts |
| `visit_mut::VisitMut` | Mutable references | Rename or update syntax in place |
| `fold::Fold` | Owned nodes | Replace and rebuild subtrees |

Override the methods relevant to your task. Call the corresponding free walker
when the override should continue into that node's children; omitting the call
stops traversal at that node.

## Collect names

```rust
use m2_syn::{nodes::Symbol, parse_native, visit::Visit};

#[derive(Default)]
struct Names(Vec<String>);

impl<'ast> Visit<'ast> for Names {
    fn visit_symbol(&mut self, symbol: &'ast Symbol) {
        self.0.push(symbol.text().to_owned());
    }
}

let file = parse_native("left + right")?;
let mut names = Names::default();
names.visit_source_file(&file);
assert_eq!(names.0, ["left", "right"]);
# Ok::<(), Box<dyn std::error::Error>>(())
```

A symbol is a leaf, so this override has no children to visit. For a composite
node, the free walker visits its fields in the generated order.

## Rename a symbol

```rust
use m2_syn::{ParsedFile, nodes::Symbol, visit_mut::VisitMut};

struct Rename;

impl VisitMut for Rename {
    fn visit_symbol_mut(&mut self, symbol: &mut Symbol) {
        if symbol.text() == "left" {
            symbol.set_text("renamed");
        }
    }
}

let file = ParsedFile::from_source_native("left+right")?.edit(|tree| {
    Rename.visit_source_file_mut(tree);
});
assert_eq!(file.original_source(), "left+right");
assert_eq!(file.to_source(), "renamed+right");
# Ok::<(), Box<dyn std::error::Error>>(())
```

This changes matching spellings throughout the tree. Scope-aware renaming also
needs name resolution, which belongs in the application using the syntax tree.
The retained original source remains available even after the edit.

## Consuming transformations

A `Fold` method takes a node by value and returns its replacement. Call
`fold::fold_<node>` to transform its children before applying your own rule.
This is useful when a rewrite changes a variant or moves children between nodes.
The `LiteralReturnTypes` transformation demonstrates this approach in the crate:
it folds method installations and adds return annotations for direct literals.

The schema also supplies constructors, `Spanned`, lexical emission, pretty-tree
inspection, and migration support where applicable. Category enums represent
grammar alternatives directly. Consult the API reference for a node's exact
fields and available conversions.

For a larger read-only traversal, run `cargo run --example type_inference`.
It demonstrates collecting child facts and combining them at parent expressions.
