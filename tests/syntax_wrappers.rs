use m2_syn::{
    Body, Delimited, Delimiter, FragmentParseError, Lex, LiteralKind, LiteralToken, Parse,
    ParseStream, Punctuated, Span, Spanned, Terminated, TokenParseError, TokenStream, TokenTree,
    lex_str, parse_fragment_str, parse_native, parse_quote_m2, parse2,
};

fn assert_syntax<T: Parse + Lex>() {}

fn integer(text: &str) -> Value {
    Value::Integer(LiteralToken::new(
        LiteralKind::Integer,
        text.to_owned(),
        Span::detached(),
    ))
}

fn round_trip<T>(value: T, expected: &str)
where
    T: std::fmt::Debug + Eq + Parse + Lex,
{
    let tokens = value.to_token_stream();
    assert_eq!(tokens.to_string(), expected);
    assert_eq!(parse2::<T>(tokens).unwrap(), value);
}

fn parse_valid_fragment<T: Parse>(source: &str) -> T {
    parse_native(source)
        .unwrap_or_else(|error| panic!("`{source}` is not valid M2 syntax: {error}"));
    parse_fragment_str(source)
        .unwrap_or_else(|error| panic!("`{source}` does not match the test fragment: {error}"))
}

#[test]
fn fragment_string_parser_preserves_top_level_cell_boundaries() {
    assert!(matches!(
        parse_fragment_str::<Body<Punctuated<Value>>>("1;2"),
        Err(FragmentParseError::MultipleCells { count: 2 })
    ));
}

#[derive(Debug, Clone, PartialEq, Eq, m2_syn::Spanned)]
enum Value {
    Integer(LiteralToken),
}

#[test]
fn all_delimiter_macros_name_construct_parse_and_emit_their_syntax() {
    type Parenthesized = m2_syn::paren!(Value);
    type Bracketed = m2_syn::brackets!(Value);
    type Braced = m2_syn::braces!(Value);
    type AngleBars = m2_syn::angle_bars!(Value);

    assert_syntax::<Parenthesized>();
    assert_syntax::<Bracketed>();
    assert_syntax::<Braced>();
    assert_syntax::<AngleBars>();

    let span = Span::detached();
    round_trip::<Parenthesized>(m2_syn::paren!(integer("1"), &span), "(1)");
    round_trip::<Bracketed>(m2_syn::brackets!(integer("1"), &span), "[1]");
    round_trip::<Braced>(m2_syn::braces!(integer("1"), &span), "{1}");
    round_trip::<AngleBars>(m2_syn::angle_bars!(integer("1"), &span), "<|1|>");
}

#[test]
fn delimiters_lower_cells_to_local_groups_and_unpack_groups_to_global_cells() {
    let cells = lex_str("left;right").unwrap();
    let local = m2_syn::paren!(cells, Span::detached()).to_token_stream();
    assert_eq!(local.to_string(), "(left;right)");

    let group = match local.into_iter().next().unwrap() {
        TokenTree::Group(group) => group,
        token => panic!("expected a group, found {token:?}"),
    };
    // The group keeps its `;` as an ordinary token: nothing is lifted out of
    // the contents, so embedding the group changed none of them.
    assert_eq!(group.stream().to_string(), "left;right");
    assert!(
        group
            .stream()
            .iter()
            .any(|token| token.spelling() == Some(";"))
    );
}

#[test]
fn independently_emitted_expressions_compose_as_distinct_global_cells() {
    let expressions: [m2_syn::Expr; 2] = [parse_quote_m2!(left), parse_quote_m2!(right)];
    let file = m2_syn::parse_native("left\nright").unwrap();

    assert_eq!(expressions.len(), 2);
    assert_eq!(file.cells.len(), 2);
    assert_eq!(file.to_m2(), "left\nright");
}

#[test]
fn delimiter_atoms_themselves_parse_and_emit() {
    assert_syntax::<m2_syn::Delimiter![()]>();
    assert_syntax::<m2_syn::Delimiter![[]]>();
    assert_syntax::<m2_syn::Delimiter![{}]>();
    assert_syntax::<m2_syn::Delimiter![<| |>]>();

    let span = Span::detached();
    round_trip(m2_syn::Delimiter![()](&span), "()");
    round_trip(m2_syn::Delimiter![[]](&span), "[]");
    round_trip(m2_syn::Delimiter![{}](&span), "{}");
    round_trip(m2_syn::Delimiter![<| |>](&span), "<||>");
}

#[test]
fn punct_macro_names_and_constructs_parseable_sequences() {
    type Values = m2_syn::punc!(Value);
    type ValueComma = m2_syn::PunctPair<Value, m2_syn::Token![,]>;

    assert_syntax::<Values>();
    assert_syntax::<ValueComma>();
    assert_syntax::<Terminated<Values>>();
    assert_syntax::<Body<Values>>();

    let one: Values = m2_syn::punc!(value integer("1"));
    round_trip(one, "1");

    let comma = m2_syn::Token![,](Span::detached());
    let two: Values = m2_syn::punc!(pairs integer("1"); comma => integer("2"));
    round_trip(two, "1,2");

    let pair = ValueComma::new(
        integer("1"),
        m2_syn::Token![,](Span::detached()),
        integer("2"),
    );
    round_trip(pair, "1,2");
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

impl Lex for Value {
    fn to_tokens(&self, output: &mut TokenStream) {
        match self {
            Self::Integer(value) => value.to_tokens(output),
        }
    }
}

#[test]
fn generic_comma_sequences_do_not_invent_empty_elements() {
    type Parenthesized = Delimited<Punctuated<Value>, Delimiter![()]>;

    assert!(parse_fragment_str::<Parenthesized>("(,1)").is_err());
    assert!(parse_fragment_str::<Parenthesized>("(1,)").is_err());
}

#[test]
fn consuming_value_iteration_discards_commas() {
    type Parenthesized = Delimited<Punctuated<Value>, Delimiter![()]>;

    let values = parse_valid_fragment::<Parenthesized>("(1,2)")
        .contents
        .into_iter()
        .collect::<Vec<_>>();

    assert_eq!(values.len(), 2);
    assert!(matches!(values[0], Value::Integer(_)));
    assert!(matches!(values[1], Value::Integer(_)));
}

#[test]
fn semicolons_require_a_nonempty_statement_on_their_left() {
    type Parenthesized = Delimited<Body<Punctuated<Value>>, Delimiter![()]>;

    for source in ["(;)", "(1;;2)"] {
        assert!(
            parse_native(source).is_err(),
            "native parser accepted `{source}`"
        );
        assert!(
            parse_fragment_str::<Parenthesized>(source).is_err(),
            "fragment parser accepted `{source}`"
        );
    }

    let body = parse_valid_fragment::<Parenthesized>("(1,2;3)").contents;
    assert_eq!(body.statements.len(), 1);
    assert_eq!(body.statements[0].statement.len(), 2);
    assert!(body.tail.is_some());
    assert_eq!(body.to_m2(), "1,2;3");
}

#[test]
fn delimited_bodies_parse_emit_and_reparse() {
    type Parenthesized = Delimited<Body<Punctuated<Value>>, Delimiter![()]>;

    let parsed: Parenthesized = parse_valid_fragment("(1,2;3)");
    let emitted = parsed.to_token_stream();
    let reparsed: Parenthesized = parse2(emitted.clone()).unwrap();

    assert_eq!(emitted.to_string(), "(1,2;3)");
    assert_eq!(reparsed.to_m2(), parsed.to_m2());
}
