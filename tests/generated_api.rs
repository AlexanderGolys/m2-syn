use m2_syn::{
    Array, BinaryExpression, BinaryOperator, DelimiterKind, DelimiterToken, Expr, OperatorExpr,
    PrefixOperator, Punctuated, SourceId, Span, Spanned, Symbol, SyntaxNode, TextPoint, TextRange,
    ToTokens, TokenParseError, TokenTree, TriviaKind, fold::Fold, parse_quote_m2, parse_tokens,
    parse1, quote_m2, visit::Visit, visit_mut::VisitMut,
};

fn span(start: usize, end: usize) -> Span {
    Span::new(
        SourceId(7),
        TextRange::new(
            TextPoint::new(0, start as u32, start),
            TextPoint::new(0, end as u32, end),
        ),
    )
}

fn symbol(name: &str, start: usize) -> Symbol {
    Symbol::new(name, span(start, start + name.len()))
}

fn multiply(start: usize) -> m2_syn::Token![*] {
    m2_syn::Token![*](span(start, start + 1))
}

fn assignment() -> SyntaxNode {
    BinaryExpression::new(
        symbol("left", 0).into(),
        multiply(5).into(),
        symbol("right", 7).into(),
    )
    .into()
}

#[test]
fn constructors_hide_recursive_storage_and_preserve_public_categories() {
    let node = assignment();
    assert!(matches!(
        &node,
        SyntaxNode::Expr(Expr::OperatorExpr(OperatorExpr::BinaryExpression(_)))
    ));
    assert_eq!(node.span().source(), Ok(SourceId(7)));
    assert_eq!(node.span().start_point().unwrap().byte, 0);
    assert_eq!(node.span().end_point().unwrap().byte, 12);
}

struct SymbolCollector(Vec<String>);

impl<'ast> Visit<'ast> for SymbolCollector {
    fn visit_symbol(&mut self, node: &'ast Symbol) {
        self.0.push(node.text.clone());
    }
}

#[test]
fn visit_uses_generated_default_walkers() {
    let node = assignment();
    let mut collector = SymbolCollector(Vec::new());

    collector.visit_syntax_node(&node);

    assert_eq!(collector.0, ["left", "right"]);
}

struct Rename;

impl VisitMut for Rename {
    fn visit_symbol_mut(&mut self, node: &mut Symbol) {
        node.text.push_str("_mut");
    }
}

#[test]
fn visit_mut_changes_nodes_in_place() {
    let mut node = assignment();
    Rename.visit_syntax_node_mut(&mut node);
    let mut collector = SymbolCollector(Vec::new());

    collector.visit_syntax_node(&node);

    assert_eq!(collector.0, ["left_mut", "right_mut"]);
}

struct Uppercase;

impl Fold for Uppercase {
    fn fold_symbol(&mut self, node: Symbol) -> Symbol {
        Symbol::new(node.text.to_uppercase(), node.span())
    }
}

#[test]
fn fold_reconstructs_the_owned_tree() {
    let node = Uppercase.fold_syntax_node(assignment());
    let mut collector = SymbolCollector(Vec::new());

    collector.visit_syntax_node(&node);

    assert_eq!(collector.0, ["LEFT", "RIGHT"]);
}

#[test]
fn direct_and_transitive_embeddings_are_generated() {
    let expression: Expr = symbol("x", 0).into();
    let operator: BinaryOperator = multiply(1).into();
    let syntax_from_symbol: SyntaxNode = symbol("y", 2).into();

    assert!(matches!(expression, Expr::Symbol(_)));
    assert!(matches!(operator, BinaryOperator::Mul(_)));
    assert!(matches!(
        syntax_from_symbol,
        SyntaxNode::Expr(Expr::Symbol(_))
    ));
}

#[test]
fn one_token_type_can_belong_to_multiple_operator_categories() {
    let binary: BinaryOperator = multiply(0).into();
    let prefix: PrefixOperator = multiply(0).into();

    assert!(matches!(binary, BinaryOperator::Mul(_)));
    assert!(matches!(prefix, PrefixOperator::Mul(_)));
    assert!(matches!(
        multiply(0).to_token_stream().into_iter().next(),
        Some(TokenTree::Punct(token))
            if token.text() == "*" && token.span() == multiply(0).span()
    ));
}

#[test]
fn typed_tokens_parse_back_from_their_raw_emission() {
    let original = multiply(0);
    let emitted = original.to_token_stream();
    let parsed: m2_syn::Token![*] = parse1(emitted.clone()).unwrap();

    assert_eq!(parsed, original);
    assert_eq!(parsed.to_token_stream(), emitted);
}

#[test]
fn token_constructor_accepts_any_supported_span_source() {
    let original = multiply(0);
    let from_span = m2_syn::Token![*](original.span());
    let from_spanned = m2_syn::Token![*](&original);

    assert_eq!(from_span, original);
    assert_eq!(from_spanned, original);
}

#[test]
fn delimiter_macro_names_generated_typed_delimiter_atoms() {
    let group_span = span(0, 3);
    let delimiter: m2_syn::Delimiter![()] = m2_syn::Delimiter![()](group_span);

    assert_eq!(
        <m2_syn::Delimiter![()] as DelimiterToken>::KIND,
        DelimiterKind::Parenthesis
    );
    assert_eq!(delimiter.span(), group_span);
    assert_eq!(
        <m2_syn::Delimiter![] as DelimiterToken>::KIND,
        DelimiterKind::Empty
    );
    assert_eq!(
        <m2_syn::Delimiter![;] as DelimiterToken>::KIND,
        DelimiterKind::Semicolon
    );
}

#[test]
fn delimited_nodes_store_typed_delimiters_and_flatten_to_raw_groups() {
    let array = Array::new(Vec::new(), Punctuated::new());
    assert_eq!(
        <m2_syn::Delimiter![[]] as DelimiterToken>::KIND,
        DelimiterKind::Bracket
    );
    assert_eq!(array.delimiter.span(), Span::detached());
    assert!(matches!(
        array.to_token_stream().into_iter().next(),
        Some(TokenTree::Group(group)) if group.delim_kind() == DelimiterKind::Bracket
    ));
}

#[test]
fn parse_quote_infers_and_parses_a_typed_token() {
    let plus: m2_syn::Token![+] = parse_quote_m2!(+);

    assert!(matches!(
        plus.to_token_stream().into_iter().next(),
        Some(TokenTree::Punct(token)) if token.text() == "+"
    ));
}

#[test]
fn parse1_rejects_wrong_and_trailing_tokens() {
    assert!(matches!(
        parse1::<m2_syn::Token![+]>(quote_m2!(*)),
        Err(TokenParseError::UnexpectedToken { .. })
    ));
    assert!(matches!(
        parse1::<m2_syn::Token![+]>(quote_m2!(+ *)),
        Err(TokenParseError::TrailingToken { .. })
    ));
}

#[test]
fn quote_builds_an_m2_token_stream_with_interpolation() {
    let value = symbol("value", 0);
    let tokens = quote_m2! {
        result = $(value) + 1;
        return result
    };

    assert_eq!(tokens.to_string(), "result=value+1;return result");
    assert_eq!(tokens.to_code(), "result=value+1;return result");
    let tokens = tokens.into_iter().collect::<Vec<_>>();
    assert!(matches!(tokens[0], TokenTree::Ident(_)));
    assert!(matches!(&tokens[1], TokenTree::Punct(token) if token.text() == "="));
    assert!(matches!(tokens[2], TokenTree::Ident(_)));
    assert!(matches!(&tokens[3], TokenTree::Punct(token) if token.text() == "+"));
    assert!(matches!(tokens[4], TokenTree::Literal(_)));
    assert!(matches!(&tokens[5], TokenTree::Punct(token) if token.text() == ";"));
    assert!(matches!(tokens[6], TokenTree::Ident(_)));
    assert!(matches!(
        tokens[7],
        TokenTree::Trivia(ref trivia) if trivia.kind() == TriviaKind::Whitespace
    ));
    assert!(matches!(tokens[8], TokenTree::Ident(_)));
}

#[test]
fn quote_composes_interpolated_fragments_by_their_actual_boundaries() {
    let plus = m2_syn::Token![+](Span::detached());
    let value = symbol("value", 0);
    let tokens = quote_m2! { left $(plus) $(value) };

    assert_eq!(tokens.to_code(), "left+value");
    assert!(matches!(
        tokens.iter().nth(1),
        Some(TokenTree::Punct(token)) if token.text() == "+"
    ));
}

#[test]
fn quote_repetition_is_an_explicit_rust_loop_with_recursive_interpolation() {
    let values = [symbol("a", 0), symbol("b", 1)];
    let plus = m2_syn::Token![+](Span::detached());
    let constant = symbol("z", 2);
    let tokens = quote_m2! {
        $[value in &values] {
            $(value) $(plus) $(constant);
        }
    };

    assert_eq!(tokens.to_code(), "a+z;b+z;");
}

#[test]
fn quote_recurses_through_groups_around_interpolated_values() {
    let value = symbol("value", 0);
    let tokens = quote_m2! { apply ($(value)) };

    assert_eq!(tokens.to_code(), "apply (value)");
    assert!(matches!(
        tokens.iter().nth(2),
        Some(TokenTree::Group(group))
            if group.delim_kind() == DelimiterKind::Parenthesis
                && matches!(group.stream().iter().next(), Some(TokenTree::Ident(_)))
    ));
}

#[test]
fn quote_builds_balanced_angle_bar_groups_recursively() {
    let value = symbol("value", 0);
    let tokens = quote_m2! { <|<|$(value)|>|> };

    assert_eq!(tokens.to_code(), "<|<|value|>|>");
    assert!(matches!(
        tokens.iter().next(),
        Some(TokenTree::Group(outer))
            if outer.delim_kind() == DelimiterKind::AngleBar
                && matches!(
                    outer.stream().iter().next(),
                    Some(TokenTree::Group(inner))
                        if inner.delim_kind() == DelimiterKind::AngleBar
                )
    ));
}

#[test]
fn quote_supports_nested_explicit_repetitions() {
    let rows = [
        [symbol("a", 0), symbol("b", 1)],
        [symbol("c", 2), symbol("d", 3)],
    ];
    let tokens = quote_m2! {
        $[row in &rows] {
            ($[value in row] { $(value), });
        }
    };

    assert_eq!(tokens.to_code(), "(a,b,);(c,d,);");
}

#[test]
fn quoted_m2_loads_as_a_typed_source_file() {
    let tokens = quote_m2! {
        left + (right);
        return left
    };
    let source_file = parse_tokens(&tokens, SourceId(8)).unwrap();

    assert_eq!(source_file.elements.len(), 2);
    assert_eq!(source_file.to_code(), "left + (right);\nreturn left");
}
