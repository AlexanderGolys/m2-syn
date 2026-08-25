use m2_syn::{
    DelimiterKind, LexErrorKind, Spanned, TokenStream, TokenTree, TriviaKind, lex_str,
    lex_str_recovering,
};

fn lexed(source: &str) -> TokenStream {
    let stream = lex_str(source).unwrap();
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
fn lexical_atoms_share_one_source_identity() {
    let source = "alpha + beta";
    let stream = lexed(source);
    let tokens = stream.iter().collect::<Vec<_>>();
    let alpha = tokens[0].span();
    let whitespace = tokens[1].span();
    let plus = tokens[2].span();
    let beta = tokens[4].span();

    assert_eq!(alpha.text(), Some("alpha"));
    assert_eq!(whitespace.text(), Some(" "));
    assert_eq!(plus.text(), Some("+"));
    assert_eq!(beta.text(), Some("beta"));
    assert!(alpha.shares_source(&whitespace));
    assert!(alpha.shares_source(&plus));
    assert!(alpha.shares_source(&beta));
    assert_eq!(stream.span().text(), Some(source));
}

#[test]
fn public_token_stream_exposes_nested_groups_and_delimiter_spans() {
    let source = "(1, [2, <|3|>])";
    let mut stream = lexed(source).into_iter();
    let Some(TokenTree::Group(parenthesized)) = stream.next() else {
        panic!("the source should lex to one parenthesized group");
    };
    assert!(matches!(stream.next(), Some(TokenTree::Eoc(_))));
    assert!(stream.next().is_none());
    assert_eq!(parenthesized.delim_kind(), DelimiterKind::Parenthesis);

    let span = parenthesized.span();
    assert_eq!(span.start().unwrap(), 0);
    assert_eq!(span.end().unwrap(), source.len());
    let opening = parenthesized.delimiter().opening_span();
    let closing = parenthesized.delimiter().closing_span();
    assert_eq!(opening.start().unwrap(), 0);
    assert_eq!(opening.end().unwrap(), 1);
    assert_eq!(closing.start().unwrap(), source.len() - 1);
    assert_eq!(closing.end().unwrap(), source.len());
}

#[test]
fn public_lexer_keeps_line_breaks_as_ordinary_trivia() {
    // The physical line break remains trivia, while the lexer also inserts a
    // zero-width boundary after recognizing that the top-level cell is whole.
    let stream = lexed("1\n2");
    let tokens = stream.iter().collect::<Vec<_>>();

    assert!(matches!(tokens[0], TokenTree::Literal(_)));
    assert!(matches!(
        tokens[1],
        TokenTree::Trivia(trivia)
            if trivia.kind() == TriviaKind::Whitespace && trivia.contains_line_break()
    ));
    assert!(matches!(tokens[2], TokenTree::Eoc(marker) if marker.span().bounds() == Some((2, 2))));
    assert!(matches!(tokens[3], TokenTree::Literal(_)));
    assert!(matches!(tokens[4], TokenTree::Eoc(marker) if marker.span().bounds() == Some((3, 3))));
    assert_eq!(tokens.len(), 5);
}

#[test]
fn quoted_punctuation_is_a_token_rather_than_structure() {
    // A quote specifier consumes the token after it, so that token carries no
    // structural meaning: brackets do not open or close a group and `;` does
    // not end a cell. Checked against M2 v1.26.05, where each of these is a
    // symbol rather than a delimiter.
    for source in ["symbol (", "symbol )", "local {", "symbol ;"] {
        assert!(
            !lexed(source)
                .iter()
                .any(|token| matches!(token, TokenTree::Group(_))),
            "`{source}` should not build a group"
        );
    }
}

#[test]
fn quoting_rearms_for_every_specifier() {
    // The second specifier quotes the closing bracket, so the two never pair.
    assert!(
        !lexed("symbol ( symbol )")
            .iter()
            .any(|token| matches!(token, TokenTree::Group(_)))
    );
}

#[test]
fn a_quoted_specifier_does_not_quote_an_additional_token() {
    let tokens = lexed("symbol symbol (1)")
        .iter()
        .cloned()
        .collect::<Vec<_>>();

    assert!(
        matches!(tokens.get(tokens.len() - 2), Some(TokenTree::Group(group))
        if group.delim_kind() == DelimiterKind::Parenthesis)
    );
    assert!(matches!(tokens.last(), Some(TokenTree::Eoc(_))));
}

#[test]
fn a_semicolon_stays_an_ordinary_punctuation_token() {
    // The physical `;` remains punctuation. The boundary following it is an
    // additional zero-width token rather than a replacement for source text.
    let tokens = lexed("1;2").iter().cloned().collect::<Vec<_>>();

    assert_eq!(tokens.len(), 5);
    assert!(matches!(tokens[1], TokenTree::Punct(_)));
    assert_eq!(tokens[1].spelling(), Some(";"));
    assert_eq!(tokens[1].span().start().unwrap(), 1);
    assert!(matches!(&tokens[2], TokenTree::Eoc(marker) if marker.span().bounds() == Some((2, 2))));
    assert!(matches!(&tokens[4], TokenTree::Eoc(marker) if marker.span().bounds() == Some((3, 3))));
}

#[test]
fn public_lexer_ignores_unpaired_carriage_returns_without_losing_them() {
    let stream = lexed("1\r2\r\n3");
    let first = stream.iter().collect::<Vec<_>>();

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
    // `1` `\r` `2`, so the carriage return is skipped but still occupies a byte.
    assert_eq!(two.span().start(), Some(2));
}

#[test]
fn public_string_lexer_reports_malformed_source_as_errors() {
    for (source, kind) in [
        ("\"\\q\"", LexErrorKind::InvalidEscape),
        ("(1", LexErrorKind::UnterminatedGroup),
        (")", LexErrorKind::UnexpectedClosingDelimiter),
        ("(]", LexErrorKind::UnexpectedClosingDelimiter),
        // The `(` is quoted, so the `)` closes nothing. M2 v1.26.05 agrees:
        // `symbol ( 1 )` is `syntax error: unmatched )`.
        ("symbol ( 1 )", LexErrorKind::UnexpectedClosingDelimiter),
        ("///", LexErrorKind::UnterminatedRawString),
        ("-* unterminated", LexErrorKind::UnterminatedBlockComment),
    ] {
        assert_eq!(lex_str(source).unwrap_err().kind(), kind);
    }

    let nested = "(".repeat(300);
    assert_eq!(
        lex_str(&nested).unwrap_err().kind(),
        LexErrorKind::NestingLimitExceeded
    );
}

#[test]
fn recovering_lexer_retains_nested_unclosed_group_interiors() {
    let source = "([x, y";
    let recovered = lex_str_recovering(source).unwrap();

    assert_eq!(recovered.value.to_string(), source);
    assert_eq!(recovered.diagnostics.len(), 2);
    assert_eq!(
        recovered.diagnostics[0].kind(),
        LexErrorKind::UnterminatedGroup
    );
    assert_eq!(
        recovered.diagnostics[0].span().bounds(),
        Some((1, source.len()))
    );
    assert_eq!(
        recovered.diagnostics[1].span().bounds(),
        Some((0, source.len()))
    );

    let Some(TokenTree::Group(parenthesis)) = recovered.value.iter().next() else {
        panic!("the recovered stream must retain the outer group");
    };
    assert!(!parenthesis.delimiter().is_closed());
    assert_eq!(
        parenthesis.delimiter().closing_span().bounds(),
        Some((source.len(), source.len()))
    );

    let Some(TokenTree::Group(bracket)) = parenthesis.stream().iter().next() else {
        panic!("the recovered stream must retain the nested group");
    };
    assert_eq!(bracket.stream().to_string(), "x, y");
    assert!(!bracket.delimiter().is_closed());
    assert_eq!(bracket.span().bounds(), Some((1, source.len())));
}
