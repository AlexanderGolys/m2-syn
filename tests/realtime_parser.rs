use m2_syn::{Lex, LexErrorKind, ParseError, RealtimeParser, Spanned, TextEdit};

#[test]
fn realtime_parser_retains_and_then_completes_an_unclosed_group() {
    let mut parser = RealtimeParser::new("[x").unwrap();
    let source_id = parser.syntax().span().source_id();
    assert_eq!(parser.source(), "[x");
    assert_eq!(parser.syntax().to_m2(), "[x");
    assert_eq!(parser.diagnostics().len(), 1);
    assert_eq!(
        parser.diagnostics()[0].kind(),
        LexErrorKind::UnterminatedGroup
    );
    assert_eq!(parser.diagnostics()[0].span().bounds(), Some((0, 2)));
    assert_eq!(
        parser.diagnostics()[0].unclosed_expression(),
        Some("array expression")
    );
    assert_eq!(parser.diagnostics()[0].expected(), Some("]"));
    assert_eq!(parser.diagnostics()[0].found(), Some("end of input"));
    assert_eq!(
        parser.diagnostics()[0].to_string(),
        "unclosed array expression: expected `]`, found end of input"
    );

    parser.apply_edit(&TextEdit::insertion(2, ", y")).unwrap();
    assert_eq!(parser.syntax().span().source_id(), source_id);
    assert_eq!(parser.source(), "[x, y");
    assert_eq!(parser.syntax().to_m2(), "[x,y");
    assert_eq!(parser.diagnostics()[0].span().bounds(), Some((0, 5)));
    assert!(!parser.changed_ranges().is_empty());

    parser.apply_edit(&TextEdit::insertion(5, "]")).unwrap();
    assert_eq!(parser.source(), "[x, y]");
    assert_eq!(parser.syntax().to_m2(), "[x,y]");
    assert!(parser.diagnostics().is_empty());
    assert_eq!(parser.syntax().span().text(), Some("[x, y]"));
}

#[test]
fn realtime_parser_handles_multibyte_offsets_and_rejects_invalid_edits_transactionally() {
    let mut parser = RealtimeParser::new("é + x").unwrap();
    parser
        .apply_edit(&TextEdit::insertion("é + x".len(), " + β"))
        .unwrap();
    assert_eq!(parser.source(), "é + x + β");
    assert_eq!(parser.syntax().to_m2(), "é + x + β");

    let previous = parser.source().to_owned();
    assert!(matches!(
        parser.apply_edit(&TextEdit::insertion(usize::MAX, "x")),
        Err(ParseError::InvalidEdit)
    ));
    assert_eq!(parser.source(), previous);
    assert_eq!(parser.syntax().to_m2(), "é + x + β");
}
