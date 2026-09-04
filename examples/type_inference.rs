//! Demonstrates expression type inference with the generated visitor.

use m2_syn::nodes::{
    Cell, Expr, ExprAssign, ExprAssignSimple, ExprCollection, ExprLiteral, ExprOp, ExprOpBin,
    PackedContent,
};
use m2_syn::visit::Visit;
use m2_syn::{ParseError, parse_file};

#[derive(Default)]
struct TypeVisitor {
    inferred: Option<String>,
}

impl TypeVisitor {
    fn type_of(&mut self, expression: &Expr) -> String {
        assert!(self.inferred.is_none());
        self.visit_expr(expression);
        self.inferred
            .take()
            .expect("visiting an expression must infer one type")
    }

    fn type_of_cell(&mut self, cell: &Cell) -> String {
        if cell.semicolon.is_some() {
            return "Nothing".into();
        }

        match &cell.content {
            PackedContent::Expr(expression) => self.type_of(expression),
            PackedContent::Pack(_) => "Sequence".into(),
        }
    }
}

impl<'ast> Visit<'ast> for TypeVisitor {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        let inferred = match expression {
            Expr::Literal(ExprLiteral::Integer(_)) => "Int".into(),
            Expr::Literal(ExprLiteral::Float(_)) => "Float".into(),
            Expr::Literal(ExprLiteral::String(_) | ExprLiteral::RawString(_)) => "String".into(),
            Expr::Symbol(symbol) => match symbol.text() {
                "null" => "Nothing".into(),
                "true" | "false" => "Bool".into(),
                _ => "Unknown".into(),
            },
            Expr::Assign(ExprAssign::Simple(assignment)) => match assignment {
                ExprAssignSimple::Global(assignment) => self.type_of(&assignment.value),
                ExprAssignSimple::Local(assignment) => self.type_of(&assignment.value),
            },
            Expr::Op(ExprOp::Bin(ExprOpBin::Regular(binary)))
                if binary.operator.spelling() == "==" =>
            {
                "Bool".into()
            }
            Expr::If(expression) => {
                let then_type = self.type_of(&expression.then_clause.expr);
                let else_type = expression
                    .else_clause
                    .as_ref()
                    .map_or_else(|| "Nothing".into(), |clause| self.type_of(&clause.expr));
                union(then_type, else_type)
            }
            Expr::Holder(holder) => match &holder.value {
                Some(expression) => self.type_of(expression),
                None if !holder.muted.is_empty() => "Nothing".into(),
                None => "Sequence".into(),
            },
            Expr::Collection(ExprCollection::Sequence(_)) => "Sequence".into(),
            Expr::Collection(ExprCollection::List(_)) => "List".into(),
            Expr::Collection(ExprCollection::Array(_)) => "Array".into(),
            Expr::Collection(ExprCollection::AngleBar(_)) => "List".into(),
            _ => "Unknown".into(),
        };
        self.inferred = Some(inferred);
    }
}

fn union(left: String, right: String) -> String {
    if left == right {
        left
    } else {
        format!("{left}|{right}")
    }
}

fn infer(source: &str) -> Result<String, ParseError> {
    let file = parse_file(source)?;
    let [cell] = file.cells.as_slice() else {
        panic!("expected one expression");
    };
    Ok(TypeVisitor::default().type_of_cell(cell))
}

fn main() -> Result<(), ParseError> {
    for source in [
        "1",
        "1.5",
        "answer = 1",
        "a == b",
        "if condition then 1 else 1.5",
        "if condition then 1 else 2",
        "if condition then 1",
        "null",
        "(1)",
        "1;",
        "(1, 2)",
        "1 + 2",
    ] {
        println!("{source} => {}", infer(source)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::infer;

    #[test]
    fn infers_the_demonstrated_expression_types() {
        for (source, expected) in [
            ("1", "Int"),
            ("1.5", "Float"),
            ("answer = 1", "Int"),
            ("a == b", "Bool"),
            ("if condition then 1 else 1.5", "Int|Float"),
            ("if condition then 1 else 2", "Int"),
            ("if condition then 1", "Int|Nothing"),
            ("null", "Nothing"),
            ("(1)", "Int"),
            ("1;", "Nothing"),
            ("(1, 2)", "Sequence"),
            ("1 + 2", "Unknown"),
        ] {
            assert_eq!(infer(source).unwrap(), expected, "{source}");
        }
    }
}
