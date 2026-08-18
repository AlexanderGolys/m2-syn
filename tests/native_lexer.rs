use m2_syn::{
    CellStream, DelimiterKind, LexErrorKind, SourceId, Spanned, TokenTree, TriviaKind, lex_str,
};

fn lexed(source: &str) -> CellStream {
    let stream = lex_str(source, SourceId(11)).unwrap();
    assert_eq!(format!("{stream}"), source);
    stream
}

#[test]
fn public_string_lexer_returns_a_displayable_token_stream() {
    for source in [
        "<<|-1||>2",
        "|--1",
        "-- comment\n-* block *-",
        "***1",
        "Core$foo'12 1not not1",
        "α⊠β",
        "0b010201",
        "++x +\n+",
        r#""a\n\u0101""#,
        "///left////right///",
        "(1, [2, <|3|>])",
        "(*)",
    ] {
        lexed(source);
    }
}

#[test]
fn public_token_stream_exposes_nested_groups_and_delimiter_spans() {
    let source = "(1, [2, <|3|>])";
    let stream = lexed(source);
    let mut cells = stream.into_iter();
    let cell = cells.next().expect("the source should form one cell");
    assert!(cells.next().is_none());
    let mut cell = cell.into_stream().into_iter();
    let Some(TokenTree::Group(parenthesized)) = cell.next() else {
        panic!("the cell should contain one parenthesized group");
    };
    assert!(cell.next().is_none());
    assert_eq!(parenthesized.delim_kind(), DelimiterKind::Parenthesis);

    let span = parenthesized.span();
    assert_eq!(span.start_point().unwrap().byte, 0);
    assert_eq!(span.end_point().unwrap().byte, source.len());
    let opening = parenthesized.delimiter().opening_span();
    let closing = parenthesized.delimiter().closing_span();
    assert_eq!(opening.start_point().unwrap().byte, 0);
    assert_eq!(opening.end_point().unwrap().byte, 1);
    assert_eq!(closing.start_point().unwrap().byte, source.len() - 1);
    assert_eq!(closing.end_point().unwrap().byte, source.len());
}

#[test]
fn public_lexer_returns_complete_top_level_cells() {
    let stream = lexed("1\n2");
    let cells = stream.into_cells();

    let first = &cells[0];
    let first = first.stream().iter().collect::<Vec<_>>();
    assert!(matches!(first[0], TokenTree::Literal(_)));
    assert!(matches!(
        first[1],
        TokenTree::Trivia(trivia)
            if trivia.kind() == TriviaKind::Whitespace && trivia.contains_line_break()
    ));
    let second = &cells[1];
    assert!(matches!(
        second.stream().iter().next(),
        Some(TokenTree::Literal(_))
    ));
    assert_eq!(cells.len(), 2);
}

#[test]
fn top_level_cells_use_empty_and_semicolon_delimiters() {
    let cells = lexed("1;2").into_cells();

    assert_eq!(cells[0].delim_kind(), DelimiterKind::Semicolon);
    assert_eq!(
        cells[0]
            .delimiter()
            .closing_span()
            .start_point()
            .unwrap()
            .byte,
        1
    );
    assert!(
        cells[0]
            .stream()
            .iter()
            .all(|token| token.spelling() != Some(";"))
    );
    assert_eq!(cells[1].delim_kind(), DelimiterKind::Empty);
}

#[test]
fn public_lexer_ignores_unpaired_carriage_returns_without_losing_them() {
    let stream = lexed("1\r2\r\n3");
    let cells = stream.into_cells();
    let first = &cells[0];
    let first = first.stream().iter().collect::<Vec<_>>();

    assert!(matches!(
        first[1],
        TokenTree::Trivia(trivia)
            if trivia.kind() == TriviaKind::Whitespace
                && trivia.text() == "\r"
                && !trivia.contains_line_break()
    ));
    assert!(matches!(
        first[3],
        TokenTree::Trivia(trivia)
            if trivia.kind() == TriviaKind::Whitespace && trivia.contains_line_break()
    ));

    let TokenTree::Literal(two) = &first[2] else {
        panic!("expected the literal after the ignored carriage return");
    };
    let two = two.span().start_point().unwrap();
    assert_eq!(two.line, 0);
    assert_eq!(two.column, 2);
}

#[test]
fn public_string_lexer_reports_malformed_source_as_errors() {
    for (source, kind) in [
        ("\"\\q\"", LexErrorKind::InvalidEscape),
        ("(1", LexErrorKind::UnterminatedGroup),
        (")", LexErrorKind::UnexpectedClosingDelimiter),
        ("(]", LexErrorKind::UnexpectedClosingDelimiter),
        ("///", LexErrorKind::UnterminatedRawString),
        ("-* unterminated", LexErrorKind::UnterminatedBlockComment),
    ] {
        assert_eq!(lex_str(source, SourceId(11)).unwrap_err().kind(), kind);
    }

    let nested = "(".repeat(300);
    assert_eq!(
        lex_str(&nested, SourceId(11)).unwrap_err().kind(),
        LexErrorKind::NestingLimitExceeded
    );
}
