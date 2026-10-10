//! `x <- y` written as a parallel assignment target. M2 parses it, so the file
//! keeps its typed syntax and the target keeps its own node.

/// The debug rendering of a one-line source, which must parse.
fn parsed(source: &str) -> String {
    let file = m2_syn::parse_file_recovering(source)
        .expect("a `<-` target is grammatical")
        .value;
    format!("{file:?}")
}

#[test]
fn a_single_left_arrow_target_is_a_parallel_assignment_component() {
    let rendered = parsed("(T <- T) := (x, y, v) -> v\n");
    assert!(rendered.contains("ExprAssignParallelLocal"), "{rendered}");
    assert!(
        rendered.contains("Evaluated(ExprAssignEvaluated"),
        "{rendered}"
    );
}

#[test]
fn a_left_arrow_target_mixes_with_other_targets() {
    let rendered = parsed("(T <- T, d) := ((x, y, v) -> v, 8)\n");
    assert!(rendered.contains("ExprAssignParallelLocal"), "{rendered}");
    assert!(
        rendered.contains("Evaluated(ExprAssignEvaluated"),
        "{rendered}"
    );
}
