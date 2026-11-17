# Tokens and cells

The parsing pipeline separates lexical tokens, cell framing, and typed syntax.
These representations serve different tasks:

| Representation | Contains | Useful for |
| --- | --- | --- |
| `TokenStream` | Raw atoms and recursive groups | Quoting and lexical transformations |
| `CellStream` | Framed cells with inferred adjacency | Parser backends and procedural macros |
| `nodes::SourceFile` | Typed nodes, operators, and boundaries | Analysis, traversal, and rewriting |

```text
source -> lex_str -> TokenStream -> Tokenize -> CellStream -> Parser -> SourceFile
                        ^                          ^                     |
                        +----------- Lex ----------+---------------------+
                                                   +---- Tokenize -------+
```

## Lexical tokens

`TokenTree` distinguishes identifiers, literals, punctuation, and delimiter
groups. A group contains a recursive token stream and retains both boundary
spans. Comments are found from those spans rather than stored. The lexer
recognizes these pieces without constructing expression nodes.

Typed literal tokens such as `Token![+]` refine the raw representation. They
are not extra variants of `TokenTree`. `Token![...]` names every token type by
its spelling, delimiter families included: `Token![else]`, `Token![()]`,
`Token![<||>]`. The spelling can also be a string, which is how a spelling Rust
cannot tokenize, such as `"⊢"`, is written. Whitespace never belongs to a
spelling, so `Token![/ /]`, `Token!["/ /"]`, and `Token!["//"]` are one type,
and nothing written, `Token![]`, is adjacency.

## Cell framing

`Tokenize` partitions lexical input into independently parseable cells.
Semicolons and complete physical lines confirm top-level boundaries. A newline
inside a delimiter group does not split the outer stream into separate cells.

Framing also records application through `CellTokenTree::Adj`. For example,
`f(x)` and `f (x)` both contain inferred adjacency, whereas `return 1` uses the
return syntax. Groups are framed recursively. Converting a framed cell back to
lexical tokens removes the inferred adjacency markers.

End-of-cell and end-of-file are structural facts in a `CellStream`. In the typed
tree, `Cell` retains its `Token![EOC]`, optional semicolon, and content;
`SourceFile` retains the file's `Token![BOF]`, cells, and `Token![EOF]`. These
markers are tokens that are never written: zero-width positions at the end of a
cell and the physical start and end of the file.

## Parsing and emission

`NativeParser` constructs typed syntax from the framed token cursor.
`TreeSitterParser` lowers its grammar's concrete tree to the same types. The
external CST is temporary and is not stored inside the owned result.

`Lex` projects syntax back to tokens; `Tokenize` projects it to framed cells.
Source-backed layout can be recovered from spans between neighboring tokens.
Emission also inserts separation where token boundaries require it. Constructed
or edited syntax may therefore have normalized layout rather than the input's
exact whitespace.

Use `ParsedFile::original_source()` when you need the original bytes. Use
`to_source()` when you need source representing the current tree. A useful
round-trip check is to parse emitted source and verify that emitting it again
produces the same result.
