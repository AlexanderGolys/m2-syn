//! Infers the Macaulay2 class of every cell in a file with node-specific `Visit` rules.
//!
//! The generated [`Visit`] implementation already walks the whole typed graph:
//! every method defaults to the free walker that descends into that node's
//! children. An analysis therefore overrides only the nodes it actually has a
//! rule for, and each override calls its free walker first when it needs the
//! facts of its operands. Nothing here dispatches on the shape of an expression
//! by hand — `visit_integer_literal` runs because the parse produced an integer
//! literal, exactly like `syn`'s `visit_expr_binary` runs on a binary Rust
//! expression.
//!
//! Inference produces an upper bound rather than an exact answer: a rule records
//! the most specific class it can prove, joining branches walks the class
//! hierarchy up to a common ancestor, and every unmodelled expression stays
//! `Thing`, the ancestor of everything in Macaulay2. Storage is the flat
//! span-keyed table an analysis or LSP layer would own; the syntax crate only
//! supplies the traversal.

use std::collections::HashMap;
use std::fmt;
use std::ops::Range;

use m2_syn::nodes::{
    AngleBarList, Array, Cell, ExprAssignSimpleGlobal, ExprAssignSimpleLocal, ExprIf,
    ExprOpBinRegular, ExprOpPrefix, FloatLiteral, Holder, IntegerLiteral, List, PackedContent,
    RawStringLiteral, Sequence, StringLiteral, Symbol,
};
use m2_syn::visit::{self, Visit};
use m2_syn::{Lex, Offset, ParseError, Spanned, parse_file};

/// A class of the Macaulay2 hierarchy, read as an upper bound: an expression
/// inferred as `class` evaluates to `class` or to one of its descendants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum M2Class {
    /// Integers.
    ZZ,
    /// Rationals.
    QQ,
    /// Real numbers.
    RR,
    /// The inexact numbers, the common ancestor of the floating point classes.
    InexactNumber,
    /// The numbers, the common ancestor of every numeric class.
    Number,
    /// `true` and `false`.
    Boolean,
    /// Strings.
    String,
    /// The nets, the common ancestor of the printable text classes.
    Net,
    /// The class of `null`, the value of a suppressed or missing expression.
    Nothing,
    /// Parenthesized comma packs.
    Sequence,
    /// Brace lists.
    List,
    /// Bracket lists.
    Array,
    /// Angle bar lists.
    AngleBarList,
    /// The visible lists, the common ancestor of the four bracketing classes.
    VisibleList,
    /// The basic lists, the common ancestor of every list-like class.
    BasicList,
    /// The ancestor of everything, inferred where no rule applies.
    Thing,
}

impl M2Class {
    /// The M2 promotion tower `ZZ` ⊂ `QQ` ⊂ `RR`, ordered by widening.
    const PROMOTION_TOWER: [Self; 3] = [Self::ZZ, Self::QQ, Self::RR];

    /// Returns the class this one inherits from, or `None` for `Thing`.
    fn parent(self) -> Option<Self> {
        Some(match self {
            Self::ZZ | Self::QQ | Self::InexactNumber => Self::Number,
            Self::RR => Self::InexactNumber,
            Self::String => Self::Net,
            Self::Sequence | Self::List | Self::Array | Self::AngleBarList => Self::VisibleList,
            Self::VisibleList => Self::BasicList,
            Self::Number | Self::Boolean | Self::Net | Self::Nothing | Self::BasicList => {
                Self::Thing
            }
            Self::Thing => return None,
        })
    }

    /// Returns this class and its ancestors, ending at `Thing`.
    fn ancestry(self) -> impl Iterator<Item = Self> {
        std::iter::successors(Some(self), |class| class.parent())
    }

    /// Reports whether `descendant` is this class or inherits from it.
    fn covers(self, descendant: Self) -> bool {
        descendant.ancestry().any(|class| class == self)
    }

    /// Returns the most specific class covering both, their common ancestor.
    fn join(self, other: Self) -> Self {
        self.ancestry()
            .find(|class| class.covers(other))
            .expect("`Thing` covers every class")
    }

    /// Returns the class an arithmetic operator produces from two operands,
    /// widening along the promotion tower.
    fn promoted_with(self, other: Self) -> Self {
        let position = |class| Self::PROMOTION_TOWER.iter().position(|&rung| rung == class);
        match (position(self), position(other)) {
            (Some(left), Some(right)) if left >= right => self,
            (Some(_), Some(_)) => other,
            _ => Self::Thing,
        }
    }
}

impl fmt::Display for M2Class {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

/// The inferred class of every node a rule could prove something about, keyed by
/// the source range the node covers.
#[derive(Default)]
struct TypeInference {
    /// The proven classes.
    classes: HashMap<Range<Offset>, M2Class>,
}

impl TypeInference {
    /// Stores the class proven for one node.
    fn record(&mut self, node: &impl Spanned, class: M2Class) {
        if let Some(range) = node.try_span().and_then(|span| span.range()) {
            self.classes.insert(range, class);
        }
    }

    /// Returns the class proven for one node, `Thing` when no rule proved one.
    fn inferred(&self, node: &impl Spanned) -> M2Class {
        node.try_span()
            .and_then(|span| span.range())
            .and_then(|range| self.classes.get(&range).copied())
            .unwrap_or(M2Class::Thing)
    }
}

impl<'ast> Visit<'ast> for TypeInference {
    fn visit_integer_literal(&mut self, node: &'ast IntegerLiteral) {
        self.record(node, M2Class::ZZ);
    }

    fn visit_float_literal(&mut self, node: &'ast FloatLiteral) {
        self.record(node, M2Class::RR);
    }

    fn visit_string_literal(&mut self, node: &'ast StringLiteral) {
        self.record(node, M2Class::String);
    }

    fn visit_raw_string_literal(&mut self, node: &'ast RawStringLiteral) {
        self.record(node, M2Class::String);
    }

    fn visit_symbol(&mut self, node: &'ast Symbol) {
        // Only the built-in constants are known without a symbol table; every
        // other name keeps the `Thing` bound the analysis layer would refine.
        match node.text() {
            "true" | "false" => self.record(node, M2Class::Boolean),
            "null" => self.record(node, M2Class::Nothing),
            _ => {}
        }
    }

    fn visit_sequence(&mut self, node: &'ast Sequence) {
        visit::visit_sequence(self, node);
        self.record(node, M2Class::Sequence);
    }

    fn visit_list(&mut self, node: &'ast List) {
        visit::visit_list(self, node);
        self.record(node, M2Class::List);
    }

    fn visit_array(&mut self, node: &'ast Array) {
        visit::visit_array(self, node);
        self.record(node, M2Class::Array);
    }

    fn visit_angle_bar_list(&mut self, node: &'ast AngleBarList) {
        visit::visit_angle_bar_list(self, node);
        self.record(node, M2Class::AngleBarList);
    }

    fn visit_holder(&mut self, node: &'ast Holder) {
        visit::visit_holder(self, node);
        let held = match &node.value {
            Some(value) => self.inferred(value.as_ref()),
            // `()` is the empty sequence, while `(f x;)` discards its value.
            None if node.muted.is_empty() => M2Class::Sequence,
            None => M2Class::Nothing,
        };
        self.record(node, held);
    }

    fn visit_expr_assign_simple_global(&mut self, node: &'ast ExprAssignSimpleGlobal) {
        visit::visit_expr_assign_simple_global(self, node);
        self.record(node, self.inferred(node.value.as_ref()));
    }

    fn visit_expr_assign_simple_local(&mut self, node: &'ast ExprAssignSimpleLocal) {
        visit::visit_expr_assign_simple_local(self, node);
        self.record(node, self.inferred(node.value.as_ref()));
    }

    fn visit_expr_op_prefix(&mut self, node: &'ast ExprOpPrefix) {
        visit::visit_expr_op_prefix(self, node);
        let operand = self.inferred(node.operand.as_ref());
        let result = match node.operator.spelling() {
            "not" => M2Class::Boolean,
            "+" | "-" => operand,
            _ => M2Class::Thing,
        };
        self.record(node, result);
    }

    fn visit_expr_op_bin_regular(&mut self, node: &'ast ExprOpBinRegular) {
        visit::visit_expr_op_bin_regular(self, node);
        let left = self.inferred(node.left_operand.as_ref());
        let right = self.inferred(node.right_operand.as_ref());
        let result = match node.operator.spelling() {
            "==" | "!=" | "===" | "=!=" | "<" | "<=" | ">" | ">=" | "and" | "or" | "xor" => {
                M2Class::Boolean
            }
            "+" | "-" | "*" | "%" | "//" => left.promoted_with(right),
            // Exact division leaves the integers even when both operands are `ZZ`.
            "/" => left.promoted_with(right).promoted_with(M2Class::QQ),
            ".." | "..<" => M2Class::Sequence,
            // Concatenation of two texts or two lists, bitwise `or` on integers.
            "|" => left.join(right),
            _ => M2Class::Thing,
        };
        self.record(node, result);
    }

    fn visit_expr_if(&mut self, node: &'ast ExprIf) {
        visit::visit_expr_if(self, node);
        let taken = self.inferred(node.then_clause.expr.as_ref());
        let untaken = node
            .else_clause
            .as_ref()
            .map_or(M2Class::Nothing, |clause| {
                self.inferred(clause.expr.as_ref())
            });
        self.record(node, taken.join(untaken));
    }

    fn visit_cell(&mut self, node: &'ast Cell) {
        visit::visit_cell(self, node);
        let value = if node.semicolon.is_some() {
            // A trailing `;` suppresses the cell's value.
            M2Class::Nothing
        } else {
            match &node.content {
                PackedContent::Expr(expression) => self.inferred(expression.as_ref()),
                PackedContent::Pack(_) => M2Class::Sequence,
            }
        };
        self.record(node, value);
    }
}

/// Source exercising one cell per inference rule.
const DEMONSTRATED_CELLS: &str = r#"6
1.5
6 / 4
6 + 1.5
"an ideal " | "of a ring"
answer := 6 * 7
{1, 2, 3}
(1, 2)
[0, 1]
<|0, 1|>
0 .. 5
not (6 < 7)
(6; 7)
6;
if isPrime n then 6 else 1.5
if isPrime n then "prime"
resolution someModule
"#;

fn main() -> Result<(), ParseError> {
    let file = parse_file(DEMONSTRATED_CELLS)?;
    let mut inference = TypeInference::default();
    inference.visit_source_file(&file);

    for cell in &file.cells {
        println!(
            "{:<34} => {}",
            cell.to_m2().trim_end(),
            inference.inferred(cell)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{M2Class, TypeInference};
    use m2_syn::parse_file;
    use m2_syn::visit::Visit;

    fn inferred_class(source: &str) -> M2Class {
        let file = parse_file(source).expect("the demonstrated sources parse");
        let [cell] = file.cells.as_slice() else {
            panic!("expected exactly one cell");
        };
        let mut inference = TypeInference::default();
        inference.visit_source_file(&file);
        inference.inferred(cell)
    }

    #[test]
    fn infers_the_class_macaulay2_would_report() {
        for (source, expected) in [
            ("6", M2Class::ZZ),
            ("1.5", M2Class::RR),
            ("6 / 4", M2Class::QQ),
            ("6 // 4", M2Class::ZZ),
            ("6 + 1.5", M2Class::RR),
            (r#""an ideal " | "of a ring""#, M2Class::String),
            ("answer := 6 * 7", M2Class::ZZ),
            ("answer = 6 * 7", M2Class::ZZ),
            ("{1, 2, 3}", M2Class::List),
            ("(1, 2)", M2Class::Sequence),
            ("[0, 1]", M2Class::Array),
            ("<|0, 1|>", M2Class::AngleBarList),
            ("0 .. 5", M2Class::Sequence),
            ("not (6 < 7)", M2Class::Boolean),
            ("(6)", M2Class::ZZ),
            ("()", M2Class::Sequence),
            ("(6; 7)", M2Class::ZZ),
            ("6;", M2Class::Nothing),
            ("null", M2Class::Nothing),
        ] {
            assert_eq!(inferred_class(source), expected, "{source}");
        }
    }

    #[test]
    fn branches_and_unmodelled_expressions_infer_an_ancestor_bound() {
        for (source, expected) in [
            ("if isPrime n then 6 else 7", M2Class::ZZ),
            ("if isPrime n then 6 else 1.5", M2Class::Number),
            ("if isPrime n then (1, 2) else {3}", M2Class::VisibleList),
            ("if isPrime n then 6", M2Class::Thing),
            ("resolution someModule", M2Class::Thing),
            ("6 + someRing", M2Class::Thing),
        ] {
            assert_eq!(inferred_class(source), expected, "{source}");
        }
    }
}
