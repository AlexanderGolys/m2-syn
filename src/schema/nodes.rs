//! The typed Macaulay2 syntax tree.
//!
//! Every type is declared as it is stored. `#[derive(Syntax)]` adds the
//! constructor, printing, pretty tree, CST migration, and child walkers from
//! the fields it sees; a field named `delimiter` is the pair around the node,
//! and a terminal's private `span` is its own source. Comments are not stored:
//! spans find them in their source. Field names follow the grammar, so only
//! the types carry documentation.
#![allow(missing_docs)]
// Payloads are stored inline, as declared, so the tree's layout reads off the
// declarations.
#![allow(clippy::large_enum_variant)]

use crate::traversal::{__walk, fold, visit, visit_mut};
use crate::{Delimited, IdentToken, Punctuated, Span, Spanned, Syntax, qf};

pub use crate::tokens::*;

/// An identifier, possibly package-qualified or indexed: `x`, `Core$x`, `x$1`.
#[derive(Debug, Spanned, Syntax)]
#[syntax(leaf = symbol)]
pub struct Symbol {
    ident: IdentToken,
}

/// The missing element between two commas: the middle of `a, , b`.
#[derive(Debug, Spanned, Syntax)]
#[syntax(leaf = empty)]
pub struct Empty {
    span: Span,
}

/// An escape inside a string literal: `\n`, `\"`.
#[derive(Debug, Spanned, Syntax)]
#[syntax(leaf = text)]
pub struct EscapeSequence {
    span: Span,
}

/// A floating-point literal: `1.5`, `.5e3`, `0x1p3`.
#[derive(Debug, Spanned, Syntax)]
#[syntax(leaf = float)]
pub struct FloatLiteral {
    span: Span,
}

/// An integer literal: `42`, `0x2a`.
#[derive(Debug, Spanned, Syntax)]
#[syntax(leaf = integer)]
pub struct IntegerLiteral {
    span: Span,
}

/// Text inside a raw string literal: `text` in `///text///`.
#[derive(Debug, Spanned, Syntax)]
#[syntax(leaf = text)]
pub struct RawStringContent {
    span: Span,
}

/// Text inside a string literal: `text` in `"text"`.
#[derive(Debug, Spanned, Syntax)]
#[syntax(leaf = text)]
pub struct StringContent {
    span: Span,
}

/// A parsed source file: the cells between its start and end markers.
#[derive(Debug, Spanned, Syntax)]
#[syntax(cell_stream)]
pub struct SourceFile {
    pub bof: Token![BOF],
    pub cells: Vec<Cell>,
    pub eof: Token![EOF],
}

/// A top-level cell ended by a newline: `x = 1`, `a, b;`.
///
/// A trailing `;` suppresses the printed value of the cell.
#[derive(Debug, Spanned, Syntax)]
pub struct Cell {
    pub elems: Elements,
    pub semi_token: Option<Token![;]>,
    pub eoc: Token![EOC],
}

/// One element of a comma-separated pack: `a` or the empty slot in `a, , b`.
#[derive(Debug, Spanned, Syntax)]
pub enum Element {
    Expr(Box<Expr>),
    Empty(Empty),
}

/// A single expression or a comma-separated pack of at least two elements:
/// `x`, `a, b`, `a, , b`.
#[derive(Debug, Spanned, Syntax)]
pub enum Elements {
    Single(Box<Expr>),
    Pack(ElementsPack),
}

/// The payload of [`Elements::Pack`].
#[derive(Debug, Spanned, Syntax)]
pub struct ElementsPack {
    pub elems: Punctuated<Element>,
}

/// Elements whose value is discarded by a following `;`: `x = 1;`.
#[derive(Debug, Spanned, Syntax)]
pub struct Stmt {
    pub elems: Elements,
    pub semi_token: Token![;],
}

/// A fragment of a string literal.
#[derive(Debug, Spanned, Syntax)]
pub enum StringFragment {
    Escape(EscapeSequence),
    Content(StringContent),
}

/// A string literal: `"text\n"`.
#[derive(Debug, Spanned, Syntax)]
#[syntax(literal = string)]
pub struct StringLiteral {
    pub fragments: Vec<StringFragment>,
    span: Span,
}

/// A fragment of a raw string literal.
#[derive(Debug, Spanned, Syntax)]
pub enum RawStringFragment {
    Escape(EscapeSequence),
    Content(RawStringContent),
}

/// A raw string literal: `///text///`.
#[derive(Debug, Spanned, Syntax)]
#[syntax(literal = raw_string)]
pub struct RawStringLiteral {
    pub fragments: Vec<RawStringFragment>,
    span: Span,
}

/// A literal: `1`, `1.5`, `"text"`, `///text///`.
#[derive(Debug, Spanned, Syntax)]
pub enum Literal {
    Integer(IntegerLiteral),
    Float(FloatLiteral),
    String(StringLiteral),
    RawString(RawStringLiteral),
}

/// A Macaulay2 expression.
#[derive(Debug, Spanned, Syntax)]
pub enum Expr {
    /// An angle-bar list: `<|a, b|>`.
    AngleBar(ExprAngleBar),
    /// An array: `[a, b]`.
    Array(ExprArray),
    /// An assignment or installation: `x = 1`, `x := 1`, `f ZZ := x -> x`.
    Assign(ExprAssign),
    /// A binary operation: `a + b`, `x#i`.
    Binary(ExprBinary),
    /// A `break`: `break x`.
    Break(ExprBreak),
    /// A `catch`: `catch f x`.
    Catch(ExprCatch),
    /// A `continue`: `continue x`.
    Continue(ExprContinue),
    /// A debugging keyword: `step 2`, `breakpoint`.
    Debug(ExprDebug),
    /// A member access: `x.y`, `x.?y`.
    Field(ExprField),
    /// A `for` loop: `for i to 3 list i`.
    For(ExprFor),
    /// A function: `(x, y) -> x + y`.
    Function(ExprFunction),
    /// An `if` expression: `if c then a else b`.
    If(ExprIf),
    /// A list: `{a, b}`.
    List(ExprList),
    /// A literal: `1`, `"text"`.
    Literal(Literal),
    /// A `new` expression: `new HashTable from {}`.
    New(ExprNew),
    /// An option: `Strategy => Linear`.
    Option(ExprOption),
    /// A parenthesized expression: `(a; b)`.
    Paren(ExprParen),
    /// A postfix operation: `n!`.
    Postfix(ExprPostfix),
    /// A prefix operation: `-x`.
    Prefix(ExprPrefix),
    /// A scoped symbol reference: `local x`, `symbol x`.
    Quote(ExprQuote),
    /// A `return`: `return x`.
    Return(ExprReturn),
    /// A parenthesized sequence: `(a, b)`, `()`.
    Sequence(ExprSequence),
    /// A symbol: `x`, `Core$x`.
    Symbol(Symbol),
    /// A `throw`: `throw e`.
    Throw(ExprThrow),
    /// A timing keyword: `time f x`.
    Timing(ExprTiming),
    /// A `trap`: `trap f x`.
    Trap(ExprTrap),
    /// A `try` expression: `try f x else 0`.
    Try(ExprTry),
    /// A `while` loop: `while c do f()`.
    While(ExprWhile),
}

/// An angle-bar list: `<|a, b|>`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprAngleBar {
    pub delimiter: Token![<||>],
    pub stmts: Vec<Stmt>,
    pub elems: Option<Elements>,
}

/// An array: `[a, b]`, `[x = 1; x]`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprArray {
    pub delimiter: Token![[]],
    pub stmts: Vec<Stmt>,
    pub elems: Option<Elements>,
}

/// A list: `{a, b}`, `{x = 1; x}`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprList {
    pub delimiter: Token![{}],
    pub stmts: Vec<Stmt>,
    pub elems: Option<Elements>,
}

/// A parenthesized sequence: `(a, b)`, `(x = 1; a, b)`, `()`.
///
/// Parentheses around a single expression form an [`ExprParen`] instead.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprSequence {
    pub delimiter: Token![()],
    pub stmts: Vec<Stmt>,
    pub elems: Option<Elements>,
}

/// A parenthesized expression: `(x)`, `(x = 1; x)`, `(x;)`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprParen {
    pub delimiter: Token![()],
    pub stmts: Vec<Stmt>,
    pub expr: Option<Box<Expr>>,
}

/// A binary operation: `a + b`, `x#i`, `x_i`, and adjacency `f x`, which is
/// the operator `SPACE` with nothing written, as in `f SPACE x`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprBinary {
    pub left: Box<Expr>,
    pub op: BinaryOperator,
    pub right: Box<Expr>,
}

/// A member access: `x.y`, `x.?y`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprField {
    pub base: Box<Expr>,
    pub op: FieldOperator,
    pub member: Symbol,
}

/// The operator of an [`ExprField`].
#[derive(Debug, Spanned, Syntax)]
pub enum FieldOperator {
    /// `.`: the value stored under the member symbol.
    Dot(Token![.]),
    /// `.?`: whether a value is stored under the member symbol.
    OptionalDot(Token![.?]),
}

/// A prefix operation: `-x`, `#L`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprPrefix {
    pub op: PrefixOperator,
    pub expr: Box<Expr>,
}

/// A postfix operation: `n!`, `M^*`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprPostfix {
    pub expr: Box<Expr>,
    pub op: PostfixOperator,
}

/// An assignment: `x = 1`, `x := 1`, `x <- 1`, `x += 1`.
///
/// The left side selects the meaning: a symbol is assigned, a parenthesized
/// sequence is assigned element by element, `x#i = v` assigns through an
/// installed method, and `f ZZ := …`, `X + Y := …`, or `new T := …` with
/// `:=` installs a method.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprAssign {
    pub left: Box<Expr>,
    pub op: AssignOperator,
    pub right: Box<Expr>,
}

/// The operator of an [`ExprAssign`].
#[derive(Debug, Spanned, Syntax)]
pub enum AssignOperator {
    /// `=`: global assignment.
    Global(Token![=]),
    /// `:=`: local assignment or installation.
    Local(Token![:=]),
    /// `<-`: assignment to the symbol that the left side evaluates to.
    Evaluated(Token![<-]),
    /// `+=`, `|=`, …: augmented assignment.
    Augmented(AugmentedOperator),
}

/// An option: `Strategy => Linear`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprOption {
    pub key: Box<Expr>,
    pub fat_arrow_token: Token![=>],
    pub value: Box<Expr>,
}

/// A function: `x -> x + 1`, `(x, y) -> x * y`, `() -> 0`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprFunction {
    pub params: FunctionParams,
    pub arrow_token: Token![->],
    pub body: Box<Expr>,
}

/// The parameters of an [`ExprFunction`].
#[derive(Debug, Spanned, Syntax)]
pub enum FunctionParams {
    /// `x -> …` binds the whole argument sequence to `x`.
    Variadic(Symbol),
    /// `(x, y) -> …` binds the elements of the argument sequence one by one.
    List(ParamList),
}

/// A delimited parameter list: `(x, y)`, `(x)`, `()`, `{x, y}`.
#[derive(Debug, Spanned, Syntax)]
pub struct ParamList {
    pub params: Delimited<Option<Params>>,
}

/// The symbols of a [`ParamList`].
#[derive(Debug, Spanned, Syntax)]
pub enum Params {
    Single(Symbol),
    Pack(ParamsPack),
}

/// The payload of [`Params::Pack`].
#[derive(Debug, Spanned, Syntax)]
pub struct ParamsPack {
    pub params: Punctuated<Symbol>,
}

/// An `if` expression: `if c then a`, `if c then a else b`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprIf {
    pub if_token: Token![if],
    pub cond: Box<Expr>,
    pub then_clause: ThenClause,
    pub else_clause: Option<ElseClause>,
}

/// A `for` loop: `for x in L do f x`, `for i from 1 to n when odd i list i`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprFor {
    pub for_token: Token![for],
    pub var: Symbol,
    pub iter: Option<ForIter>,
    pub when_clause: Option<WhenClause>,
    pub body: LoopBody,
}

/// What a [`ExprFor`] iterates over.
///
/// A loop without one, `for i do …`, counts from zero without end.
#[derive(Debug, Spanned, Syntax)]
pub enum ForIter {
    /// `in L`: the elements of a collection.
    In(InClause),
    /// `from a to b`, with either bound optional: a range of integers.
    Range(ForRange),
}

/// The bounds of a counting [`ExprFor`]: `from 1 to n`, `to n`, `from 1`.
#[derive(Debug, Spanned, Syntax)]
pub struct ForRange {
    pub from_clause: Option<FromClause>,
    pub to_clause: Option<ToClause>,
}

/// A `while` loop: `while c do f()`, `while c list x`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprWhile {
    pub while_token: Token![while],
    pub cond: Box<Expr>,
    pub body: LoopBody,
}

/// The body of a loop: `list a`, `do b`, or `list a do b`.
#[derive(Debug, Spanned, Syntax)]
pub struct LoopBody {
    pub list_clause: Option<ListClause>,
    pub do_clause: Option<DoClause>,
}

/// A `try` expression: `try f x`, `try f x then y else z`,
/// `try f x except e do g e`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprTry {
    pub try_token: Token![try],
    pub expr: Box<Expr>,
    pub then_clause: Option<ThenClause>,
    pub fallback: Option<TryFallback>,
}

/// What an [`ExprTry`] evaluates when its expression fails.
#[derive(Debug, Spanned, Syntax)]
pub enum TryFallback {
    Else(ElseClause),
    Except(ExceptClause),
}

/// The error handler of an [`ExprTry`]: `except e do g e`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExceptClause {
    pub except_token: Token![except],
    pub var: Symbol,
    pub do_token: Token![do],
    pub expr: Box<Expr>,
}

/// `then x` in an [`ExprIf`] or [`ExprTry`].
#[derive(Debug, Spanned, Syntax)]
pub struct ThenClause {
    pub then_token: Token![then],
    pub expr: Box<Expr>,
}

/// `else x` in an [`ExprIf`] or [`ExprTry`].
#[derive(Debug, Spanned, Syntax)]
pub struct ElseClause {
    pub else_token: Token![else],
    pub expr: Box<Expr>,
}

/// `when c` in an [`ExprFor`].
#[derive(Debug, Spanned, Syntax)]
pub struct WhenClause {
    pub when_token: Token![when],
    pub expr: Box<Expr>,
}

/// `in L` in an [`ExprFor`].
#[derive(Debug, Spanned, Syntax)]
pub struct InClause {
    pub in_token: Token![in],
    pub expr: Box<Expr>,
}

/// `from x` in an [`ExprFor`] or [`ExprNew`].
#[derive(Debug, Spanned, Syntax)]
pub struct FromClause {
    pub from_token: Token![from],
    pub expr: Box<Expr>,
}

/// `to x` in an [`ExprFor`].
#[derive(Debug, Spanned, Syntax)]
pub struct ToClause {
    pub to_token: Token![to],
    pub expr: Box<Expr>,
}

/// `list x` in a [`LoopBody`].
#[derive(Debug, Spanned, Syntax)]
pub struct ListClause {
    pub list_token: Token![list],
    pub expr: Box<Expr>,
}

/// `do x` in a [`LoopBody`].
#[derive(Debug, Spanned, Syntax)]
pub struct DoClause {
    pub do_token: Token![do],
    pub expr: Box<Expr>,
}

/// `of P` in an [`ExprNew`].
#[derive(Debug, Spanned, Syntax)]
pub struct OfClause {
    pub of_token: Token![of],
    pub expr: Box<Expr>,
}

/// A `new` expression: `new HashTable`, `new Type of Parent from x`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprNew {
    pub new_token: Token![new],
    pub class: Box<Expr>,
    pub of_clause: Option<OfClause>,
    pub from_clause: Option<FromClause>,
}

/// A scoped symbol reference: `symbol x`, `local x`, `global x`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprQuote {
    pub keyword: QuoteKeyword,
    pub symbol: Symbol,
}

/// The keyword of an [`ExprQuote`].
#[derive(Debug, Spanned, Syntax)]
pub enum QuoteKeyword {
    Symbol(Token![symbol]),
    Local(Token![local]),
    Global(Token![global]),
    ThreadLocal(Token![threadLocal]),
}

/// A `break`: `break`, `break x`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprBreak {
    pub break_token: Token![break],
    pub expr: Option<Box<Expr>>,
}

/// A `continue`: `continue`, `continue x`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprContinue {
    pub continue_token: Token![continue],
    pub expr: Option<Box<Expr>>,
}

/// A `return`: `return`, `return x`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprReturn {
    pub return_token: Token![return],
    pub expr: Option<Box<Expr>>,
}

/// A `catch`: `catch f x`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprCatch {
    pub catch_token: Token![catch],
    pub expr: Box<Expr>,
}

/// A `throw`: `throw e`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprThrow {
    pub throw_token: Token![throw],
    pub expr: Box<Expr>,
}

/// A `trap`: `trap f x`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprTrap {
    pub trap_token: Token![trap],
    pub expr: Box<Expr>,
}

/// A debugging keyword with an optional operand: `step 2`, `breakpoint`,
/// `TEST ///…///`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprDebug {
    pub keyword: DebugKeyword,
    pub expr: Option<Box<Expr>>,
}

/// The keyword of an [`ExprDebug`].
#[derive(Debug, Spanned, Syntax)]
pub enum DebugKeyword {
    Test(Token![TEST]),
    Step(Token![step]),
    Finish(Token![finish]),
    Shield(Token![shield]),
    Breakpoint(Token![breakpoint]),
}

/// A measured evaluation: `time f x`, `elapsedTiming f x`, `profile f x`.
#[derive(Debug, Spanned, Syntax)]
pub struct ExprTiming {
    pub keyword: TimingKeyword,
    pub expr: Box<Expr>,
}

/// The keyword of an [`ExprTiming`].
#[derive(Debug, Spanned, Syntax)]
pub enum TimingKeyword {
    ElapsedTime(Token![elapsedTime]),
    ElapsedTiming(Token![elapsedTiming]),
    Profile(Token![profile]),
    Time(Token![time]),
    Timing(Token![timing]),
}
