use m2_syn::{
    Body, Delimited, Delimiter, FragmentParseError, Literal, LiteralKind, Parse, ParseStream,
    Punctuated, SourceId, Span, Spanned, ToTokens, TokenParseError, TokenStream, TokenTree,
    parse_fragment_str, parse_native, parse2, punct,
};

fn parse_valid_fragment<T: Parse>(source: &str) -> T {
    parse_native(source, SourceId(1))
        .unwrap_or_else(|error| panic!("`{source}` is not valid M2 syntax: {error}"));
    parse_fragment_str(source, SourceId(1))
        .unwrap_or_else(|error| panic!("`{source}` does not match the test fragment: {error}"))
}

#[test]
fn fragment_string_parser_preserves_top_level_cell_boundaries() {
    assert!(matches!(
        parse_fragment_str::<Body<Punctuated<Value>>>("1;2", SourceId(1)),
        Err(FragmentParseError::MultipleCells { count: 2 })
    ));
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Integer(Literal),
    Empty(punct::Empty),
}

impl From<punct::Empty> for Value {
    fn from(value: punct::Empty) -> Self {
        Self::Empty(value)
    }
}

impl Parse for Value {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let Some(token) = input.next_token() else {
            return Err(TokenParseError::UnexpectedEnd {
                expected: "an integer",
                span: Span::detached(),
            });
        };
        let span = token.span();
        match token {
            TokenTree::Literal(literal) if literal.kind == LiteralKind::Integer => {
                Ok(Self::Integer(literal))
            }
            token => Err(TokenParseError::UnexpectedToken {
                expected: "an integer",
                found: token.spelling().unwrap_or("a group").to_owned(),
                span,
            }),
        }
    }
}

impl Spanned for Value {
    fn span(&self) -> Span {
        match self {
            Self::Integer(value) => value.span(),
            Self::Empty(value) => value.span(),
        }
    }
}

impl ToTokens for Value {
    fn to_tokens(&self, output: &mut TokenStream) {
        match self {
            Self::Integer(value) => value.to_tokens(output),
            Self::Empty(value) => value.to_tokens(output),
        }
    }
}

#[test]
fn comma_sequences_insert_real_empty_components() {
    type Parenthesized = Delimited<Punctuated<Value>, Delimiter![()]>;

    let values = parse_valid_fragment::<Parenthesized>("(,1,,2)").contents;

    assert_eq!(values.len(), 4);
    assert!(matches!(values.iter().next(), Some(Value::Empty(_))));
    assert!(matches!(values.iter().nth(1), Some(Value::Integer(_))));
    assert!(matches!(values.iter().nth(2), Some(Value::Empty(_))));
    assert!(matches!(values.iter().nth(3), Some(Value::Integer(_))));
    assert_eq!(values.to_m2(), ",1,,2");
}

#[test]
fn consuming_value_iteration_discards_commas_but_not_empty_components() {
    type Parenthesized = Delimited<Punctuated<Value>, Delimiter![()]>;

    let values = parse_valid_fragment::<Parenthesized>("(1,)")
        .contents
        .into_iter()
        .collect::<Vec<_>>();

    assert_eq!(values.len(), 2);
    assert!(matches!(values[0], Value::Integer(_)));
    assert!(matches!(values[1], Value::Empty(_)));
}

#[test]
fn semicolons_require_a_nonempty_statement_on_their_left() {
    type Parenthesized = Delimited<Body<Punctuated<Value>>, Delimiter![()]>;

    for source in ["(;)", "(1;;2)"] {
        assert!(
            parse_native(source, SourceId(1)).is_err(),
            "native parser accepted `{source}`"
        );
        assert!(
            parse_fragment_str::<Parenthesized>(source, SourceId(1)).is_err(),
            "fragment parser accepted `{source}`"
        );
    }

    let body = parse_valid_fragment::<Parenthesized>("(1,;2)").contents;
    assert_eq!(body.statements.len(), 1);
    assert_eq!(body.statements[0].statement.len(), 2);
    assert!(matches!(
        body.statements[0].statement.iter().nth(1),
        Some(Value::Empty(_))
    ));
    assert!(body.tail.is_some());
    assert_eq!(body.to_m2(), "1,;2");
}

#[test]
fn delimited_bodies_parse_emit_and_reparse() {
    type Parenthesized = Delimited<Body<Punctuated<Value>>, Delimiter![()]>;

    let parsed: Parenthesized = parse_valid_fragment("(,1;2)");
    let emitted = parsed.to_token_stream();
    let reparsed: Parenthesized = parse2(emitted.clone()).unwrap();

    assert_eq!(emitted.to_string(), "(,1;2)");
    assert_eq!(reparsed, parsed);
}
