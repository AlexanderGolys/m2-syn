//! Integration tests for review regressions.

use std::hash::{DefaultHasher, Hash, Hasher};

use m2_syn::nodes::{
    Component, Expr, ExprAssign, ExprAssignInstall, ExprCollection, ExprLiteral, PackedContent,
    StringElement,
};
use m2_syn::{
    Lex, NativeParser, Parse, ParseStream, Parser, Spanned, Tokenize, TreeSitterParser, lex_str,
    parse_file, parse_fragment_str, parse_native, parse_with, quote_m2,
};

fn expression_with(parser: &mut impl Parser<Error = impl std::fmt::Debug>, source: &str) -> Expr {
    let cells = lex_str(source).unwrap().to_cell_stream().unwrap();
    let mut file = parse_with(parser, cells).unwrap();
    let PackedContent::Expr(expression) = file.cells.pop().unwrap().content else {
        panic!("expected one expression");
    };
    *expression
}

#[test]
fn packed_content_and_method_adjacency_preserve_only_present_syntax() {
    let source = "ZZ ZZ := ZZ => (x, y) -> x y";
    for file in [parse_native(source).unwrap(), parse_file(source).unwrap()] {
        assert_eq!(file.to_m2(), source);
        let PackedContent::Expr(expression) = &file.cells[0].content else {
            panic!("a cell without a comma must contain its expression directly");
        };
        let Expr::Assign(ExprAssign::Install(ExprAssignInstall::Method(installation))) =
            expression.as_ref()
        else {
            panic!("expected a method installation, found {expression:?}");
        };
        assert_eq!(installation.operator.span().text(), Some(""));
        assert_eq!(installation.operator.span().bounds(), Some((3, 3)));
    }

    for file in [parse_native("x,y").unwrap(), parse_file("x,y").unwrap()] {
        let PackedContent::Pack(pack) = &file.cells[0].content else {
            panic!("comma syntax must retain an expression pack");
        };
        assert_eq!(pack.pack.len(), 2);
    }
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
            let Expr::Assign(ExprAssign::Install(ExprAssignInstall::New(installation))) =
                expression
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
        let Expr::Assign(ExprAssign::Install(ExprAssignInstall::New(installation))) = expression
        else {
            panic!("expected a new installation, found {expression:?}");
        };
        assert!(matches!(
            installation.function.as_ref(),
            Expr::Option(annotation)
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
    let cells = tokens.to_cell_stream().unwrap();
    parse_with(&mut NativeParser, cells.clone()).unwrap();
    parse_with(&mut TreeSitterParser::new().unwrap(), cells).unwrap();

    let plus = parse_fragment_str::<m2_syn::Token![+]>("+").unwrap();
    assert_eq!(quote_m2!($(plus) * 1).to_string(), "+*1");
}

#[test]
fn source_identities_participate_in_span_equality_and_hashing() {
    let first = lex_str("a").unwrap().span();
    let second = lex_str("b").unwrap().span();
    let same_text_but_distinct_source = lex_str("a").unwrap().span();
    assert_ne!(first, second);
    assert_ne!(first, same_text_but_distinct_source);
    let mut hashes = [first, second, same_text_but_distinct_source]
        .into_iter()
        .map(|span| {
            let mut hasher = DefaultHasher::new();
            span.hash(&mut hasher);
            hasher.finish()
        })
        .collect::<Vec<_>>();
    hashes.sort_unstable();
    hashes.dedup();
    assert_eq!(hashes.len(), 3);
}

#[test]
fn native_empty_components_are_zero_width_at_the_missing_value() {
    let Expr::Collection(ExprCollection::Sequence(sequence)) =
        expression_with(&mut NativeParser, "(x,,y)")
    else {
        panic!("expected a sequence");
    };
    let PackedContent::Pack(elements) = sequence.elements.unwrap() else {
        panic!("expected a comma pack");
    };
    let components = elements.pack.into_iter().collect::<Vec<_>>();
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
        let Expr::Literal(ExprLiteral::String(literal)) = expression else {
            panic!("expected a string literal");
        };
        assert_eq!(literal.span().bounds(), Some((0, 5)));
        assert_eq!(literal.span().text(), Some(r#""a\n""#));
        assert!(matches!(&literal.elements[..],
            [StringElement::Content(content), StringElement::Escape(escape)]
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
                Expr::Literal(ExprLiteral::String(literal)) => {
                    assert!(literal.elements.is_empty());
                    assert_eq!(literal.span().text(), Some(source));
                }
                Expr::Literal(ExprLiteral::RawString(literal)) => {
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

struct RawToken(m2_syn::TokenTree);

impl Parse for RawToken {
    fn parse(input: &mut ParseStream) -> Result<Self, m2_syn::TokenParseError> {
        Ok(Self(input.next_raw_token().expect("one lexical atom")))
    }
}

#[test]
fn raw_token_consumption_visits_only_lexical_atoms() {
    let mut input = ParseStream::new(lex_str(" +").unwrap());
    let token = RawToken::parse(&mut input).unwrap();

    assert_eq!(token.0.spelling(), Some("+"));
    assert!(input.next_token().is_none());
}
