use std::collections::HashSet;

use m2_syn::{
    Assignment, Collection, Component, Expr, Installation, Literal, NativeParser, Parse,
    ParseStream, Parser, Spanned, StringElement, TreeSitterParser, lex_str, parse_file,
    parse_fragment_str, parse_native, parse_with, quote_m2,
};

fn expression_with(parser: &mut impl Parser<Error = impl std::fmt::Debug>, source: &str) -> Expr {
    let mut file = parse_with(parser, lex_str(source).unwrap()).unwrap();
    let mut components = file.cells.pop().unwrap().content.pack.into_iter();
    let Some(Component::Expr(expression)) = components.next() else {
        panic!("expected one expression component");
    };
    assert!(components.next().is_none());
    *expression
}

#[test]
fn tree_sitter_reassociates_new_installations_like_the_native_parser() {
    for (source, parent, instance) in [
        ("new x := f", None, None),
        ("new x from y := f", None, Some("y")),
        ("new x of t := f", Some("t"), None),
        ("new x of t from y := f", Some("t"), Some("y")),
    ] {
        for expression in [
            expression_with(&mut NativeParser, source),
            expression_with(&mut TreeSitterParser::new().unwrap(), source),
        ] {
            let Expr::Assignment(Assignment::Installation(Installation::NewInstallation(
                installation,
            ))) = expression
            else {
                panic!("expected a new installation for `{source}`, found {expression:?}");
            };
            let statement = installation.new_statement;
            assert!(matches!(statement.class.as_ref(), Expr::Symbol(class) if class.text() == "x"));
            assert_eq!(
                statement
                    .parent_class
                    .as_ref()
                    .map(|clause| match clause.parent.as_ref() {
                        Expr::Symbol(parent) => parent.text(),
                        expression => panic!("expected a parent symbol, found {expression:?}"),
                    }),
                parent,
            );
            assert_eq!(
                statement
                    .instance
                    .as_ref()
                    .map(|clause| match clause.instance.as_ref() {
                        Expr::Symbol(instance) => instance.text(),
                        expression => panic!("expected an instance symbol, found {expression:?}"),
                    }),
                instance,
            );
            assert!(
                matches!(installation.function.as_ref(), Expr::Symbol(function) if function.text() == "f")
            );
        }
    }

    for expression in [
        expression_with(&mut NativeParser, "new x := T => f"),
        expression_with(&mut TreeSitterParser::new().unwrap(), "new x := T => f"),
    ] {
        let Expr::Assignment(Assignment::Installation(Installation::NewInstallation(installation))) =
            expression
        else {
            panic!("expected a new installation, found {expression:?}");
        };
        assert!(matches!(
            installation.function.as_ref(),
            Expr::OptionExpression(annotation)
                if matches!(annotation.key.as_ref(), Expr::Symbol(symbol) if symbol.text() == "T")
        ));
    }

    assert!(parse_native("new x = f").is_err());
    assert!(parse_file("new x = f").is_err());
}

#[test]
fn quote_keeps_punctuation_fragments_lexically_separate() {
    let plus = parse_fragment_str::<m2_syn::Token![+]>("+").unwrap();
    let tokens = quote_m2!($(plus) + 1);

    assert_eq!(tokens.to_string(), "+ +1");
    parse_with(&mut NativeParser, tokens.clone()).unwrap();
    parse_with(&mut TreeSitterParser::new().unwrap(), tokens).unwrap();

    let plus = parse_fragment_str::<m2_syn::Token![+]>("+").unwrap();
    assert_eq!(quote_m2!($(plus) * 1).to_string(), "+*1");
}

#[test]
fn source_identities_participate_in_span_equality_and_hashing() {
    let first = lex_str("a").unwrap().span();
    let second = lex_str("b").unwrap().span();
    let same_text_but_distinct_source = lex_str("a").unwrap().span();
    let mut spans = HashSet::new();

    assert_ne!(first, second);
    assert_ne!(first, same_text_but_distinct_source);
    spans.insert(first);
    spans.insert(second);
    spans.insert(same_text_but_distinct_source);
    assert_eq!(spans.len(), 3);
}

#[test]
fn native_empty_components_are_zero_width_at_the_missing_value() {
    let Expr::Collection(Collection::Sequence(sequence)) =
        expression_with(&mut NativeParser, "(x,,y)")
    else {
        panic!("expected a sequence");
    };
    let components = sequence
        .elements
        .unwrap()
        .pack
        .into_iter()
        .collect::<Vec<_>>();
    let Component::Empty(empty) = &components[1] else {
        panic!("expected the middle component to be empty");
    };

    assert_eq!(empty.span().bounds(), Some((3, 3)));
    assert_eq!(empty.span().text(), Some(""));
}

#[test]
fn literal_nodes_include_quotes_and_split_escapes_with_exact_spans() {
    for expression in [
        expression_with(&mut NativeParser, r#""a\n""#),
        expression_with(&mut TreeSitterParser::new().unwrap(), r#""a\n""#),
    ] {
        let Expr::Literal(Literal::StringLiteral(literal)) = expression else {
            panic!("expected a string literal");
        };
        assert_eq!(literal.span().bounds(), Some((0, 5)));
        assert_eq!(literal.span().text(), Some(r#""a\n""#));
        assert!(matches!(&literal.elements[..],
            [StringElement::StringContent(content), StringElement::EscapeSequence(escape)]
                if content.text() == "a"
                    && content.span().bounds() == Some((1, 2))
                    && escape.text() == r"\n"
                    && escape.span().bounds() == Some((2, 4))
        ));
    }

    for source in [r#""""#, "//////"] {
        for expression in [
            expression_with(&mut NativeParser, source),
            expression_with(&mut TreeSitterParser::new().unwrap(), source),
        ] {
            match expression {
                Expr::Literal(Literal::StringLiteral(literal)) => {
                    assert!(literal.elements.is_empty());
                    assert_eq!(literal.span().text(), Some(source));
                }
                Expr::Literal(Literal::RawStringLiteral(literal)) => {
                    assert!(literal.elements.is_empty());
                    assert_eq!(literal.span().text(), Some(source));
                }
                expression => panic!("expected an empty literal, found {expression:?}"),
            }
        }
    }
}

#[test]
fn typed_token_fragments_skip_leading_and_trailing_trivia() {
    let token = parse_fragment_str::<m2_syn::Token![+]>(" + ").unwrap();

    assert_eq!(token.span().bounds(), Some((1, 2)));
}

struct RawToken;

impl Parse for RawToken {
    fn parse(input: &mut ParseStream) -> Result<Self, m2_syn::TokenParseError> {
        let _ = input.next_raw_token();
        Ok(Self)
    }
}

#[test]
fn raw_token_consumption_remains_lossless() {
    let mut input = ParseStream::new(lex_str(" +").unwrap());
    let _ = RawToken::parse(&mut input).unwrap();

    assert_eq!(input.next_token().unwrap().spelling(), Some("+"));
}
