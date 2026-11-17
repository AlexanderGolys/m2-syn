//! `x <- y` written as a parallel assignment target. M2 parses it, so the file
//! keeps its typed syntax and the target keeps its own node.

use m2_syn::nodes::{AssignOperator, Element, Elements, Expr, ExprAssign};

/// The assignment forming the only cell of a one-line source, which must parse.
fn assignment(source: &str) -> ExprAssign {
    let mut file = m2_syn::parse_file_recovering(source)
        .expect("a `<-` target is grammatical")
        .value;
    let Elements::Single(expression) = file.cells.pop().unwrap().elems else {
        panic!("expected one expression");
    };
    let Expr::Assign(assignment) = *expression else {
        panic!("expected an assignment");
    };
    assignment
}

fn is_left_arrow_assignment(expression: &Expr) -> bool {
    matches!(expression, Expr::Assign(target) if matches!(target.op, AssignOperator::Evaluated(_)))
}

#[test]
fn a_single_left_arrow_target_is_a_parallel_assignment_component() {
    let assignment = assignment("(T <- T) := (x, y, v) -> v\n");
    assert!(matches!(assignment.op, AssignOperator::Local(_)));
    let Expr::Paren(target) = assignment.left.as_ref() else {
        panic!("expected a parenthesized target");
    };
    assert!(target.expr.as_deref().is_some_and(is_left_arrow_assignment));
}

#[test]
fn a_left_arrow_target_mixes_with_other_targets() {
    let assignment = assignment("(T <- T, d) := ((x, y, v) -> v, 8)\n");
    assert!(matches!(assignment.op, AssignOperator::Local(_)));
    let Expr::Sequence(targets) = assignment.left.as_ref() else {
        panic!("expected a sequence of targets");
    };
    let Some(Elements::Pack(targets)) = &targets.elems else {
        panic!("expected two targets");
    };
    assert!(
        matches!(targets.elems.first(), Element::Expr(target) if is_left_arrow_assignment(target))
    );
}
