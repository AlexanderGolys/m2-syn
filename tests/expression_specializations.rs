//! Integration tests for expression specializations.

use m2_syn::nodes::{AssignOperator, Elements, Expr, ExprField, FieldOperator, SourceFile};
use m2_syn::visit::{self, Visit};
use m2_syn::{Lex, PrettyTree, format_pretty_tree, parse_file, parse_native};

fn expression(file: SourceFile) -> Expr {
    let [cell] = file.cells.try_into().unwrap_or_else(|_| {
        panic!("expected exactly one parsed cell");
    });
    let Elements::Single(expression) = cell.elems else {
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
fn every_assignment_form_is_one_assignment_node() {
    for (source, operator) in [
        ("x = 1", "="),
        ("x := 1", ":="),
        ("x <- 1", "<-"),
        ("x += 1", "+="),
        ("(a, b) = (1, 2)", "="),
        ("x#i = 1", "="),
        ("f ZZ := x -> x", ":="),
    ] {
        for expression in both_backends(source) {
            let Expr::Assign(assignment) = expression else {
                panic!("expected an assignment for `{source}`");
            };
            let spelling = match &assignment.op {
                AssignOperator::Augmented(operator) => operator.spelling().to_owned(),
                operator => operator.to_m2(),
            };
            assert_eq!(spelling, operator, "source: {source}");
        }
    }
}

#[test]
fn member_access_is_a_field_and_indexing_is_binary() {
    for expression in both_backends("receiver.?member") {
        let Expr::Field(field) = expression else {
            panic!("expected a member access");
        };
        assert!(matches!(field.op, FieldOperator::OptionalDot(_)));
        assert_eq!(field.member.to_m2(), "member");
    }

    for expression in both_backends("receiver#index") {
        assert!(matches!(expression, Expr::Binary(_)));
    }
}

#[test]
fn member_access_requires_a_symbol() {
    parse_native("x.(y)").expect_err("the member of `.` must be a symbol");
}

#[derive(Default)]
struct DispatchTrace {
    expressions: usize,
    fields: usize,
}

impl<'ast> Visit<'ast> for DispatchTrace {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        self.expressions += 1;
        visit::visit_expr(self, expression);
    }

    fn visit_expr_field(&mut self, field: &'ast ExprField) {
        self.fields += 1;
        visit::visit_expr_field(self, field);
    }
}

#[test]
fn visits_dispatch_on_the_concrete_expression() {
    let expression = expression(parse_native("receiver.member").unwrap());
    let mut trace = DispatchTrace::default();

    trace.visit_expr(&expression);

    assert_eq!(trace.expressions, 2);
    assert_eq!(trace.fields, 1);
}

#[test]
fn member_assignment_exposes_target_and_value_in_both_backends() {
    let source = "SheafOfRings.typeDepth = 2";
    for expression in both_backends(source) {
        let Expr::Assign(assignment) = &expression else {
            panic!("expected an assignment: {expression:#?}");
        };
        let Expr::Field(field) = assignment.left.as_ref() else {
            panic!("expected a member access target: {:#?}", assignment.left);
        };
        assert_eq!(field.base.to_m2(), "SheafOfRings");
        assert_eq!(field.member.to_m2(), "typeDepth");
        assert_eq!(assignment.right.to_m2(), "2");
        assert_eq!(expression.to_m2(), source);

        let tree = assignment.pretty_tree();
        let fields: Vec<_> = tree.children().iter().map(|edge| edge.label()).collect();
        assert_eq!(fields, ["left", "op", "right"]);
        let rendered = format_pretty_tree(&expression);
        assert!(rendered.contains("ExprAssign"), "{rendered}");
        assert!(rendered.contains("ExprField"), "{rendered}");
    }
}
