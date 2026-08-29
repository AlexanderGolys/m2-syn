//! Integration tests for native lexer.

use std::ops::Bound;

use m2_syn::{
    DelimiterKind, IdentToken, LexErrorKind, Span, Spanned, TokenStream, TokenTree, lex_str,
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
fn complete_line_comments_coalesce_but_inline_comments_stay_inner() {
    let source = "    -- first\n\t-- second\nvalue -- trailing\nnext";
    let tokens = lexed(source);
    let mut tokens = tokens.iter();
    let value = tokens.next().unwrap();
    let next = tokens.next().unwrap();

    assert_eq!(value.leading_trivia().as_slice().len(), 1);
    assert_eq!(
        value.leading_trivia().as_slice()[0].text(),
        "    -- first\n\t-- second"
    );
    assert_eq!(value.trailing_trivia().as_slice().len(), 1);
    assert_eq!(value.trailing_trivia().as_slice()[0].text(), "-- trailing");
    assert!(next.leading_trivia().is_empty());
}

#[test]
fn lexical_atoms_share_one_source_identity() {
    let source = "alpha + beta";
    let stream = lexed(source);
    let tokens = stream.iter().collect::<Vec<_>>();
    let alpha = tokens[0].span();
    let plus = tokens[1].span();
    let beta = tokens[2].span();

    assert_eq!(alpha.text(), Some("alpha"));
    assert_eq!(plus.text(), Some("+"));
    assert_eq!(beta.text(), Some("beta"));
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
    assert!(stream.next().is_none());
    assert_eq!(parenthesized.delim_kind(), DelimiterKind::Parenthesis);

    let span = parenthesized.span();
    assert_eq!(span.start().unwrap(), 0);
    assert_eq!(span.end().unwrap(), source.len());
    let opening = parenthesized.delimiter().opening_span();
    let closing = parenthesized.delimiter().closing_span().unwrap();
    assert_eq!(opening.start().unwrap(), 0);
    assert_eq!(opening.end().unwrap(), 1);
    assert_eq!(closing.start().unwrap(), source.len() - 1);
    assert_eq!(closing.end().unwrap(), source.len());
}

#[test]
fn public_lexer_keeps_line_breaks_in_source_gaps() {
    // Cell boundaries belong to CellStream, while physical layout remains
    // recoverable from the atoms' shared source.
    let stream = lexed("1\n2");
    let tokens = stream.iter().collect::<Vec<_>>();

    assert!(matches!(tokens[0], TokenTree::Literal(_)));
    assert!(matches!(tokens[1], TokenTree::Literal(_)));
    assert_eq!(tokens.len(), 2);
    let left = tokens[0].span();
    let right = tokens[1].span();
    assert_eq!(
        &left.source().text()[left.end().unwrap()..right.start().unwrap()],
        "\n"
    );
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

    assert!(matches!(tokens.last(), Some(TokenTree::Group(group))
        if group.delim_kind() == DelimiterKind::Parenthesis));
}

#[test]
fn a_semicolon_stays_an_ordinary_punctuation_token() {
    // The physical `;` remains punctuation; the boundary is represented by
    // partitioning only after conversion to CellStream.
    let tokens = lexed("1;2").iter().cloned().collect::<Vec<_>>();

    assert_eq!(tokens.len(), 3);
    assert!(matches!(tokens[1], TokenTree::Punct(_)));
    assert_eq!(tokens[1].spelling(), Some(";"));
    assert_eq!(tokens[1].span().start().unwrap(), 1);
    assert!(matches!(tokens[2], TokenTree::Literal(_)));
}

#[test]
fn public_lexer_ignores_unpaired_carriage_returns_without_losing_them() {
    let stream = lexed("1\r2\r\n3");
    let first = stream.iter().collect::<Vec<_>>();

    let TokenTree::Literal(two) = &first[1] else {
        panic!("expected the literal after the ignored carriage return");
    };
    // `1` `\r` `2`, so the carriage return is skipped but still occupies a byte.
    assert_eq!(two.span().start(), Some(2));
    assert_eq!(first.len(), 3);
    assert_eq!(
        &first[0].span().source().text()
            [first[0].span().end().unwrap()..first[1].span().start().unwrap()],
        "\r"
    );
    assert_eq!(
        &first[1].span().source().text()
            [first[1].span().end().unwrap()..first[2].span().start().unwrap()],
        "\r\n"
    );
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
    assert_eq!(parenthesis.delimiter().closing_span(), None);
    assert!(matches!(
        parenthesis.delimiter().spans.close_bound(),
        Bound::Unbounded
    ));

    let Some(TokenTree::Group(bracket)) = parenthesis.stream().iter().next() else {
        panic!("the recovered stream must retain the nested group");
    };
    assert_eq!(bracket.stream().to_string(), "x, y");
    assert!(!bracket.delimiter().is_closed());
    assert_eq!(bracket.span().bounds(), Some((1, 2)));
}

#[test]
fn raw_streams_can_be_extended_without_virtual_boundary_cleanup() {
    let mut stream = lexed("x");
    stream.extend([IdentToken::new("y", Span::for_text("y"))]);
    let tokens = stream.iter().collect::<Vec<_>>();
    assert_eq!(tokens.len(), 2);
    assert!(matches!(tokens[0], TokenTree::Ident(token) if token.text() == "x"));
    assert!(matches!(tokens[1], TokenTree::Ident(token) if token.text() == "y"));
}

#[test]
fn extending_an_incomplete_group_descends_into_its_interior() {
    let mut stream = lex_str_recovering("([x").unwrap().value;
    stream.extend([IdentToken::new("y", Span::for_text("y"))]);

    let Some(TokenTree::Group(parenthesis)) = stream.iter().next() else {
        panic!("expected the incomplete parenthesized group");
    };
    let Some(TokenTree::Group(bracket)) = parenthesis.stream().iter().next() else {
        panic!("expected the nested incomplete bracketed group");
    };
    assert!(
        matches!(bracket.stream().iter().last(), Some(TokenTree::Ident(token)) if token.text() == "y")
    );
}
