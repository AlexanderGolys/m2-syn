//! Integration tests for native realtime.

use std::sync::Arc;

use m2_syn::{
    CellTokenTree, Lex, LexErrorKind, NativeCellBoundary, NativeCellState, NativeParseError,
    NativeRealtimeParser, Spanned, TextEdit,
};

#[test]
fn append_reopens_only_the_speculative_eof_cell() {
    let mut parser = NativeRealtimeParser::new("f x\nvalue");
    assert_eq!(parser.cells().len(), 2);
    assert_eq!(parser.cells()[0].boundary(), NativeCellBoundary::EOC);
    assert_eq!(parser.cells()[1].boundary(), NativeCellBoundary::EOF);
    let confirmed = Arc::clone(parser.cells()[0].syntax().unwrap().span().source());

    assert_eq!(parser.append(" other"), NativeCellState::Complete);
    assert_eq!(parser.reparsed_cells(), 1..2);
    assert!(Arc::ptr_eq(
        &confirmed,
        parser.cells()[0].syntax().unwrap().span().source()
    ));
    assert_eq!(parser.cells()[1].syntax().unwrap().to_m2(), "value other");

    assert_eq!(parser.append("\n"), NativeCellState::Complete);
    assert_eq!(parser.cells()[1].boundary(), NativeCellBoundary::EOC);
    let newly_confirmed = Arc::clone(parser.cells()[1].syntax().unwrap().span().source());
    assert_eq!(parser.append("next"), NativeCellState::Complete);
    assert_eq!(parser.reparsed_cells(), 2..3);
    assert!(Arc::ptr_eq(
        &newly_confirmed,
        parser.cells()[1].syntax().unwrap().span().source()
    ));
}

#[test]
fn an_open_group_collapses_cells_and_regenerates_every_local_adj() {
    let source = "a b\nc d\ne f\ng h\ni j\n";
    let mut parser = NativeRealtimeParser::new(source);
    assert_eq!(parser.cells().len(), 5);

    parser.apply_edit(&TextEdit::insertion(0, "(")).unwrap();
    assert_eq!(parser.cells().len(), 1);
    assert_eq!(parser.state(), NativeCellState::Invalid);
    assert_eq!(parser.cells()[0].boundary(), NativeCellBoundary::Unbounded);
    assert_eq!(count_adj(parser.cells()[0].tokens()), 5);
    assert!(matches!(
        parser.cells()[0].parse_error(),
        Some(NativeParseError::NewlineInApplication { .. })
    ));

    parser.apply_edit(&TextEdit::deletion(0, 1)).unwrap();
    assert_eq!(parser.cells().len(), 5);
    assert_eq!(parser.state(), NativeCellState::Complete);
}

#[test]
fn native_errors_distinguish_an_unfinished_eof_from_illegal_syntax() {
    let incomplete = NativeRealtimeParser::new("1 + 1 =");
    assert_eq!(incomplete.state(), NativeCellState::Incomplete);
    assert_eq!(
        incomplete.cells()[0].boundary(),
        NativeCellBoundary::Unbounded
    );
    assert!(matches!(
        incomplete.cells()[0].parse_error(),
        Some(NativeParseError::MissingOperand { operator, .. }) if operator == "="
    ));

    let invalid = NativeRealtimeParser::new("2 =");
    assert_eq!(invalid.state(), NativeCellState::Invalid);
    assert!(matches!(
        invalid.cells()[0].parse_error(),
        Some(NativeParseError::InvalidAssignmentTarget { .. })
    ));
}

#[test]
fn framing_stabilization_reuses_the_unchanged_suffix() {
    let source = "a b\nc d\ne f\ng h\n";
    let mut parser = NativeRealtimeParser::new(source);
    let suffix = Arc::clone(parser.cells()[3].syntax().unwrap().span().source());
    let insertion = source.find('d').unwrap() + 1;

    parser
        .apply_edit(&TextEdit::insertion(insertion, "x"))
        .unwrap();

    assert_eq!(parser.reparsed_cells(), 1..2);
    assert_eq!(parser.cells()[1].source(), "c dx\n");
    assert_eq!(parser.cells()[2].document_range(), 9..13);
    assert!(Arc::ptr_eq(
        &suffix,
        parser.cells()[3].syntax().unwrap().span().source()
    ));
}

#[test]
fn an_unterminated_raw_string_invalidates_only_its_forward_suffix() {
    let source = "a b\n///raw///\nc d\n";
    let mut parser = NativeRealtimeParser::new(source);
    let prefix = Arc::clone(parser.cells()[0].syntax().unwrap().span().source());
    let closer = source.find("raw").unwrap() + "raw".len();

    parser
        .apply_edit(&TextEdit::deletion(closer, closer + 3))
        .unwrap();

    assert_eq!(parser.cells().len(), 2);
    assert_eq!(parser.reparsed_cells(), 1..2);
    assert_eq!(parser.state(), NativeCellState::Incomplete);
    assert_eq!(
        parser.cells()[1].lexer_diagnostics()[0].kind(),
        LexErrorKind::UnterminatedRawString
    );
    assert!(Arc::ptr_eq(
        &prefix,
        parser.cells()[0].syntax().unwrap().span().source()
    ));

    parser
        .apply_edit(&TextEdit::insertion(closer, "///"))
        .unwrap();
    assert_eq!(parser.cells().len(), 3);
    assert_eq!(parser.state(), NativeCellState::Complete);
}

#[test]
fn an_invalid_edit_is_transactional() {
    let mut parser = NativeRealtimeParser::new("a b\n");
    let source = parser.source().to_owned();
    assert!(
        parser
            .apply_edit(&TextEdit::insertion(usize::MAX, "x"))
            .is_err()
    );
    assert_eq!(parser.source(), source);
    assert_eq!(parser.cells()[0].syntax().unwrap().to_m2(), "a b");
}

fn count_adj(tokens: &[CellTokenTree]) -> usize {
    tokens
        .iter()
        .map(|token| match token {
            CellTokenTree::Group(group) => count_adj(group.stream()),
            CellTokenTree::Adj(_) => 1,
            _ => 0,
        })
        .sum()
}
