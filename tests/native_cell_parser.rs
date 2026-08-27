use std::sync::Arc;

use m2_syn::{Lex, LexErrorKind, NativeCellParser, NativeCellState, NativeParseError, Spanned};

#[test]
fn a_long_function_remains_typed_until_its_block_closes() {
    let mut parser = NativeCellParser::new();

    assert_eq!(parser.push_line("f = x -> ("), NativeCellState::Incomplete);
    assert!(parser.active_syntax().is_some());

    assert_eq!(parser.push_line("y := x + 1;"), NativeCellState::Incomplete);
    assert_eq!(parser.push_line("y^2"), NativeCellState::Incomplete);
    assert_eq!(
        parser.active_syntax().unwrap().to_m2(),
        "f = x -> (\ny := x + 1;\ny^2"
    );
    assert_eq!(parser.lexer_diagnostics().len(), 1);
    assert_eq!(
        parser.lexer_diagnostics()[0].kind(),
        LexErrorKind::UnterminatedGroup
    );
    assert_eq!(parser.lexer_diagnostics()[0].expected(), Some(")"));

    assert_eq!(parser.push_line(")"), NativeCellState::Complete);
    assert_eq!(parser.active_source(), "");
    assert!(parser.active_syntax().is_none());

    let completed = parser.take_completed();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].source(), "f = x -> (\ny := x + 1;\ny^2\n)\n");
    assert_eq!(
        completed[0].syntax().to_m2(),
        "f = x -> (\ny := x + 1;\ny^2\n)"
    );
    assert_eq!(parser.state(), NativeCellState::Empty);
}

#[test]
fn an_operator_tail_is_incomplete_but_an_explicit_bad_boundary_is_invalid() {
    let mut parser = NativeCellParser::new();
    assert_eq!(parser.push_line("x +"), NativeCellState::Incomplete);
    assert!(matches!(
        parser.parse_error(),
        Some(NativeParseError::MissingOperand { .. })
    ));

    assert_eq!(parser.push_line("1"), NativeCellState::Complete);
    let completed = parser.take_completed();
    assert_eq!(completed[0].source(), "x +\n1\n");
    assert_eq!(completed[0].syntax().to_m2(), "x +\n1");

    let mut parser = NativeCellParser::new();
    assert_eq!(parser.push_line("x +;"), NativeCellState::Invalid);
    assert!(matches!(
        parser.parse_error(),
        Some(NativeParseError::MissingOperand { .. })
    ));
}

#[test]
fn required_control_clauses_continue_across_submitted_lines() {
    let mut parser = NativeCellParser::new();
    assert_eq!(parser.push_line("if x"), NativeCellState::Incomplete);
    assert_eq!(parser.push_line("then y"), NativeCellState::Complete);

    let completed = parser.take_completed();
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].source(), "if x\nthen y\n");
    assert_eq!(completed[0].syntax().to_m2(), "if x\nthen y");

    let mut parser = NativeCellParser::new();
    assert_eq!(parser.push_line("1 = x"), NativeCellState::Invalid);
    assert!(matches!(
        parser.parse_error(),
        Some(NativeParseError::InvalidAssignmentTarget { .. })
    ));
}

#[test]
fn a_mismatched_closer_is_invalid_and_can_be_abandoned() {
    let mut parser = NativeCellParser::new();
    assert_eq!(parser.push_line("f("), NativeCellState::Incomplete);
    assert_eq!(parser.push_line("]"), NativeCellState::Invalid);
    assert_eq!(parser.lexer_diagnostics().len(), 1);
    assert_eq!(
        parser.lexer_diagnostics()[0].kind(),
        LexErrorKind::UnexpectedClosingDelimiter
    );

    parser.reset_active();
    assert_eq!(parser.state(), NativeCellState::Empty);
    assert_eq!(parser.active_source(), "");
}

#[test]
fn nested_unclosed_groups_report_every_required_closer() {
    let mut parser = NativeCellParser::new();
    assert_eq!(parser.push_line("f([x"), NativeCellState::Incomplete);
    assert!(parser.active_syntax().is_some());
    assert_eq!(parser.lexer_diagnostics().len(), 2);
    assert_eq!(parser.lexer_diagnostics()[0].expected(), Some("]"));
    assert_eq!(parser.lexer_diagnostics()[1].expected(), Some(")"));

    assert_eq!(parser.push_line("])"), NativeCellState::Complete);
    let completed = parser.take_completed();
    assert_eq!(completed[0].syntax().to_m2(), "f([x\n])");
}

#[test]
fn an_unterminated_block_comment_is_incomplete_until_its_closer_arrives() {
    let mut parser = NativeCellParser::new();
    assert_eq!(
        parser.push_line("x -* comment"),
        NativeCellState::Incomplete
    );
    assert_eq!(parser.lexer_diagnostics().len(), 1);
    assert_eq!(
        parser.lexer_diagnostics()[0].kind(),
        LexErrorKind::UnterminatedBlockComment
    );

    assert_eq!(parser.push_line("*-"), NativeCellState::Complete);
    let completed = parser.take_completed();
    assert_eq!(completed[0].source(), "x -* comment\n*-\n");
    assert_eq!(completed[0].syntax().to_m2(), "x -* comment\n*-");
}

#[test]
fn a_completed_prefix_is_not_rebuilt_with_the_unfinished_tail() {
    let mut parser = NativeCellParser::new();
    assert_eq!(parser.push_line("x = 1; f("), NativeCellState::Incomplete);
    assert_eq!(parser.completed().len(), 1);
    assert_eq!(parser.completed()[0].source(), "x = 1;");
    assert_eq!(parser.completed()[0].syntax().to_m2(), "x = 1;");
    assert_eq!(parser.active_source(), " f(\n");

    let source = Arc::clone(parser.completed()[0].syntax().span().source());
    assert_eq!(parser.push_line("x"), NativeCellState::Incomplete);
    assert!(Arc::ptr_eq(
        &source,
        parser.completed()[0].syntax().span().source()
    ));

    assert_eq!(parser.push_line(")"), NativeCellState::Complete);
    let completed = parser.take_completed();
    assert_eq!(completed.len(), 2);
    assert_eq!(completed[0].syntax().to_m2(), "x = 1;");
    assert_eq!(completed[1].syntax().to_m2(), "f(\nx\n)");
}
