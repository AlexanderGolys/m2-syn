//! Integration tests for expression specializations.

use m2_syn::nodes::{
    Expr, ExprAssign, ExprOp, ExprOpBin, ExprOpBinMember, PackedContent, SourceFile,
};
use m2_syn::visit::{self, Visit};
use m2_syn::{parse_file, parse_native};

fn expression(file: SourceFile) -> Expr {
    let [cell] = file.cells.try_into().unwrap_or_else(|_| {
        panic!("expected exactly one parsed cell");
    });
    let PackedContent::Expr(expression) = cell.content else {
        panic!("expected exactly one expression");
    };
    *expression
}

fn both_backends(source: &str) -> [Expr; 2] {
    [
        expression(parse_native(source).unwrap()),
        expression(parse_file(source).unwrap()),
    ]
}

#[test]
fn augmented_assignment_has_its_own_assignment_specialization() {
    for expression in both_backends("target += value") {
        let Expr::Assign(ExprAssign::Augmented(assignment)) = expression else {
            panic!("expected an augmented assignment");
        };
        assert_eq!(assignment.operator.spelling(), "+=");
    }
}

#[test]
fn member_access_and_indexing_are_not_generic_binary_expressions() {
    for expression in both_backends("receiver.member") {
        assert!(matches!(
            expression,
            Expr::Op(ExprOp::Bin(ExprOpBin::Member(_)))
        ));
    }

    for expression in both_backends("receiver#index") {
        assert!(matches!(
            expression,
            Expr::Op(ExprOp::Bin(ExprOpBin::Index(_)))
        ));
    }
}

#[derive(Default)]
struct DispatchTrace {
    expressions: usize,
    operators: usize,
    member_accesses: usize,
}

impl<'ast> Visit<'ast> for DispatchTrace {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        self.expressions += 1;
        visit::visit_expr(self, expression);
    }

    fn visit_expr_op(&mut self, expression: &'ast ExprOp) {
        self.operators += 1;
        visit::visit_expr_op(self, expression);
    }

    fn visit_expr_op_bin_member(&mut self, access: &'ast ExprOpBinMember) {
        self.member_accesses += 1;
        visit::visit_expr_op_bin_member(self, access);
    }
}

#[test]
fn visits_dispatch_through_every_declared_specialization() {
    let expression = expression(parse_native("receiver.member").unwrap());
    let mut trace = DispatchTrace::default();

    trace.visit_expr(&expression);

    assert_eq!(trace.expressions, 3);
    assert_eq!(trace.operators, 1);
    assert_eq!(trace.member_accesses, 1);
}
