# The syntax schema

The grammar lives in three files under `src/schema/`, each expanded by its
own macro. There is no checked-in generated source to refresh.

`tokens.rs`
: The `tokens!` table: every operator, keyword, and separator with its parser
  roles and binding strengths.

`nodes.rs`
: One plain Rust declaration per syntax type, with `#[derive(Syntax)]`.

`traversal.rs`
: The `traversal!` inventory of the type names the visitors cover.

Most users can work entirely with `m2_syn::nodes`. Read this chapter when
extending the grammar or understanding how its public types are organized.

## Node declarations

Every type is declared as it is stored: each field, each variant, and each
name is written out. These are the declarations of a file and of one element
of a comma pack:

```rust,ignore
/// A parsed source file: the cells between its start and end markers.
#[derive(Debug, Spanned, Syntax)]
#[syntax(cell_stream)]
pub struct SourceFile {
    pub bof: Token![BOF],
    pub cells: Vec<Cell>,
    pub eof: Token![EOF],
}

/// One element of a comma-separated pack: `a` or the empty slot in `a, , b`.
#[derive(Debug, Spanned, Syntax)]
pub enum Element {
    Expr(Box<Expr>),
    Empty(Empty),
}
```

`#[derive(Syntax)]` adds the constructor, printing, pretty tree, CST
migration, and child walkers; it reads the field types and never adds fields.
`Box`, `Option`, `Vec`, `Punctuated`, `Delimited`, and `Token![..]` keep their
usual meaning. A few field names are storage rather than children:
`delimiter: Token![()]` is the pair printed around the other fields, and a
terminal keeps its own private `span`. Comments are not stored on nodes; they
are found from spans, see [Lexer](lexer.md#comments).

A new type also needs its name in the `traversal!` inventory. A type left out
fails to compile, because its child walker calls a visitor method that is
never declared.

## The shape of expressions

`Expr` is one flat enum with a variant per construction, each holding an
`Expr<Kind>` struct: `Expr::Binary(ExprBinary)`, `Expr::Field(ExprField)`,
`Expr::Assign(ExprAssign)`, `Expr::For(ExprFor)`, and so on. A visitor reaches
the concrete node one step below `Expr`, and overrides it by name:
`visit_expr_field` sees every `x.y`, `visit_expr_assign` every assignment.

```rust,ignore
let field = ExprField { base, op: FieldOperator::Dot(dot), member };
let expression = Expr::from(field);
```

The syntax decides the node, not the meaning. Every assignment form is
`ExprAssign { left, op, right }`: `x = 1`, `x := 1`, `x <- 1`, `x += 1`,
`(a, b) = (1, 2)`, `x#i = v`, and the installations `f ZZ := …`,
`X + Y := …`, and `new T := …` all share it, and the shape of `left` together
with `op` tells them apart. A typical value in `f ZZ := ZZ => x -> x` is the
option expression on the right, exactly as Macaulay2 reads it. `x#i` is an
ordinary `ExprBinary`, while `x.y` is an `ExprField` because Macaulay2 requires
a symbol after the dot.

Field names follow a few fixed rules:

`<keyword>_token`
: A keyword or punctuation token: `if_token`, `semi_token`, `arrow_token`.

`<keyword>_clause`
: An optional or reusable clause introduced by a keyword, such as
  `else_clause: Option<ElseClause>` or `from_clause: Option<FromClause>`. Every
  clause stores its keyword and the expression after it in `expr`.

`op`, `left`, `right`
: The operator and the operands of a binary construction.

`expr`
: The single operand of a prefix, postfix, or keyword construction.

`stmts`, `elems`
: The `;`-terminated statements and the trailing elements of a delimited body,
  as in `{x = 1; x, y}`.

`keyword`
: The keyword of a family that shares one shape, such as `ExprTiming`
  (`time`, `timing`, `elapsedTime`, …) or `ExprQuote` (`symbol`, `local`, …).

The presentation tree uses these same type names and field names.

Leaves retain their text through a source span; `Symbol`, marked by
`#[syntax(leaf = symbol)]`, wraps the `IdentToken` that spells it. A string literal, `#[syntax(literal = string)]`,
prints as one literal token while exposing its fragments.

## Tokens and binding

The `tokens!` table declares operator and keyword spellings together with their
parser roles and binding metadata:

```rust,ignore
crate::tokens! {
    [+] [-] { bin, pref, aug } (50, L)
    [*]     { bin, pref, aug } (58, L)
    [=]     { infix }          (14, R)
    [;]                       (8)
    [,]                       (10)
}
```

Adjacent spellings share the metadata on their right. `bin`, `pref`, and `post`
select operator roles; `infix` supplies syntax-specific infix binding. `L` and
`R` describe binary associativity. A third binding slot can give a prefix
operator a different strength. `aug` derives the augmented assignment spelling
and uses the assignment token's binding rule.

A row `["≠"] = [!=]` gives a declared token another spelling instead of a type
of its own. M2 has thirteen such synonyms, and `threadVariable` for
`threadLocal`. `Token![!=]` matches either spelling and prints the one written,
so `a ≠ b` parses into the same `ExprBinary` as `a != b`.

Use `Token![+]`, `Token![=]`, and other literal spellings in public code. The
concrete generated token struct names are expansion details. Tokens have no
visitor methods of their own: `Visit::visit_terminal` sees every token, leaf,
and literal as a `&dyn Spanned`.

## Delimiters and punctuation

`Delimited<T>` stores a value with its opening and closing boundaries. Its
second parameter can restrict the delimiter family, as in
`Delimited<T, Token![()]>`. The `paren!`, `brackets!`, `braces!`, and
`angle_bars!` macros provide convenient type and construction forms.

```rust
use m2_syn::{Lex, Source, Span};

type Parenthesized = m2_syn::paren!(m2_syn::Token![+]);
let span = Span::unlocated(Source::synthetic(""));
let value: Parenthesized = m2_syn::paren!(
    m2_syn::Token![+](&span),
    &span,
);
assert_eq!(value.to_m2(), "(+)");
```

`Punctuated<T>` is a nonempty comma-separated sequence that retains the comma
tokens. `punct!(T)` names this type; the macro also supplies construction forms.
Use `Option<Punctuated<T>>` when the entire sequence can be absent. An empty
component belongs in the element type itself.

## External CST migration

Migration relates external node kinds and child fields to the typed schema.
A field matches its Rust name, and `#[syntax(cst = ...)]` selects a different
external node name.

These rules work for compatible shapes. A parser adapter must explicitly lower
external structures that do not match the typed graph. The
[parsing chapter](./parsers.md) describes the `Parser` and `ExternalCstNode`
boundaries.
