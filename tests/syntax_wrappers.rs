use m2_syn::{
    Body, Delimited, Delimiter, FragmentParseError, Literal, LiteralKind, Parse, ParseStream,
    Punctuated, SourceId, Span, Spanned, Terminated, ToCells, ToTokens, TokenParseError,
    TokenStream, TokenTree, lex_str, parse_fragment_str, parse_native, parse_quote_m2, parse1,
    punct,
};

fn assert_syntax<T: Parse + ToTokens>() {}

fn integer(text: &str) -> Value {
    Value::Integer(Literal::new(
        LiteralKind::Integer,
        text.to_owned(),
        Span::detached(),
    ))
}

fn round_trip<T>(value: T, expected: &str)
where
    T: std::fmt::Debug + Eq + Parse + ToTokens,
{
    let tokens = value.to_token_stream();
    assert_eq!(tokens.to_string(), expected);
    assert_eq!(parse1::<T>(tokens).unwrap(), value);
}

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

#[derive(Debug, Clone, PartialEq, Eq, m2_syn::Spanned)]
enum Value {
    Integer(Literal),
    Empty(punct::Empty),
}

#[test]
fn all_delimiter_macros_name_construct_parse_and_emit_their_syntax() {
    type Naked = m2_syn::naked!(Value);
    type Semicolon = m2_syn::semicolon!(Value);
    type Parenthesized = m2_syn::paren!(Value);
    type Bracketed = m2_syn::brackets!(Value);
    type Braced = m2_syn::braces!(Value);
    type AngleBars = m2_syn::angle_bars!(Value);

    assert_syntax::<Naked>();
    assert_syntax::<Semicolon>();
    assert_syntax::<Parenthesized>();
    assert_syntax::<Bracketed>();
    assert_syntax::<Braced>();
    assert_syntax::<AngleBars>();

    let span = Span::detached();
    round_trip::<Naked>(m2_syn::naked!(integer("1"), span), "1");
    round_trip::<Semicolon>(m2_syn::semicolon!(integer("1"), span), "1;");
    round_trip::<Parenthesized>(m2_syn::paren!(integer("1"), span), "(1)");
    round_trip::<Bracketed>(m2_syn::brackets!(integer("1"), span), "[1]");
    round_trip::<Braced>(m2_syn::braces!(integer("1"), span), "{1}");
    round_trip::<AngleBars>(m2_syn::angle_bars!(integer("1"), span), "<|1|>");

    let quoted: Semicolon = parse_quote_m2!(1;);
    assert_eq!(quoted.to_m2(), "1;");
}

#[test]
fn delimiters_lower_cells_to_local_groups_and_unpack_groups_to_global_cells() {
    let cells = lex_str("left;right", SourceId(81)).unwrap();
    let local = m2_syn::paren!(cells, Span::detached()).to_token_stream();
    assert_eq!(local.to_string(), "(left;right)");

    let group = match local.into_iter().next().unwrap() {
        TokenTree::Group(group) => group,
        token => panic!("expected a group, found {token:?}"),
    };
    let promoted = group.to_cell_stream(SourceId(82));

    assert_eq!(promoted.cells().len(), 2);
    assert_eq!(
        promoted.cells()[0].delim_kind(),
        m2_syn::DelimiterKind::Semicolon
    );
    assert_eq!(
        promoted.cells()[1].delim_kind(),
        m2_syn::DelimiterKind::Empty
    );
    assert_eq!(promoted.to_string(), "left;right");
}

#[test]
fn independently_emitted_expressions_compose_as_distinct_global_cells() {
    let expressions: [m2_syn::Expr; 2] = [parse_quote_m2!(left), parse_quote_m2!(right)];
    let cells = expressions.to_cell_stream(SourceId(83));

    assert_eq!(cells.cells().len(), 2);
    assert_eq!(cells.to_string(), "left\nright");
}

#[test]
fn delimiter_atoms_themselves_parse_and_emit() {
    assert_syntax::<m2_syn::Delimiter![]>();
    assert_syntax::<m2_syn::Delimiter![;]>();
    assert_syntax::<m2_syn::Delimiter![()]>();
    assert_syntax::<m2_syn::Delimiter![[]]>();
    assert_syntax::<m2_syn::Delimiter![{}]>();
    assert_syntax::<m2_syn::Delimiter![<| |>]>();

    let span = Span::detached();
    round_trip(m2_syn::Delimiter![](span), "");
    round_trip(m2_syn::Delimiter![;](span), ";");
    round_trip(m2_syn::Delimiter![()](span), "()");
    round_trip(m2_syn::Delimiter![[]](span), "[]");
    round_trip(m2_syn::Delimiter![{}](span), "{}");
    round_trip(m2_syn::Delimiter![<| |>](span), "<||>");
}

#[test]
fn punct_macro_names_and_constructs_parseable_sequences() {
    type Values = m2_syn::punct!(Value);
    type ValueComma = punct::Pair<Value, m2_syn::Token![,]>;

    assert_syntax::<Values>();
    assert_syntax::<ValueComma>();
    assert_syntax::<punct::Empty>();
    assert_syntax::<Terminated<Values>>();
    assert_syntax::<Body<Values>>();

    let empty: Values = m2_syn::punct!();
    round_trip(empty, "");

    let one: Values = m2_syn::punct!(value integer("1"));
    round_trip(one, "1");

    let comma = m2_syn::Token![,](Span::detached());
    let two: Values = m2_syn::punct!(pairs integer("1"); comma => integer("2"));
    round_trip(two, "1,2");

    let pair = ValueComma::Punctuated(integer("1"), m2_syn::Token![,](Span::detached()));
    round_trip(pair, "1,");
    round_trip(punct::Empty::new(Span::detached()), "");
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
    let reparsed: Parenthesized = parse1(emitted.clone()).unwrap();

    assert_eq!(emitted.to_string(), "(,1;2)");
    assert_eq!(reparsed, parsed);
}
