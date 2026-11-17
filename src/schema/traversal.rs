//! The inventory of syntax types the `visit`, `visit_mut`, and `fold`
//! traversals cover.
//!
//! Every type declared in [`crate::nodes`], and every operator
//! enum the token table defines, is listed here once. A type missing from the
//! list fails to compile: its children walker calls a visitor method that the
//! traversal then never declares.

use crate::nodes::*;

crate::traversal! {
    // Leaves
    Symbol, Empty,
    EscapeSequence, FloatLiteral, IntegerLiteral, RawStringContent, StringContent,

    // Operators
    PrefixOperator, BinaryOperator, PostfixOperator, AugmentedOperator,

    // Files and cells
    SourceFile, Cell, Element, Elements, ElementsPack, Stmt,

    // Literals
    Literal, StringLiteral, StringFragment, RawStringLiteral, RawStringFragment,

    // Expressions
    Expr,
    ExprAngleBar, ExprArray, ExprList, ExprSequence, ExprParen,
    ExprBinary, ExprField, FieldOperator, ExprPrefix, ExprPostfix,
    ExprAssign, AssignOperator, ExprOption,
    ExprFunction, FunctionParams, ParamList, Params, ParamsPack,
    ExprIf, ExprFor, ForIter, ForRange, ExprWhile, LoopBody,
    ExprTry, TryFallback, ExceptClause,
    ExprNew,
    ExprQuote, QuoteKeyword,
    ExprBreak, ExprContinue, ExprReturn,
    ExprCatch, ExprThrow, ExprTrap,
    ExprDebug, DebugKeyword, ExprTiming, TimingKeyword,

    // Clauses
    ThenClause, ElseClause, WhenClause, InClause, FromClause, ToClause,
    ListClause, DoClause, OfClause,
}
