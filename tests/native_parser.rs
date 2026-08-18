use m2_syn::{
    AnyCell, BinaryOperator, Collection, Expr, LiteralKind, NativeParseError, OperatorExpr,
    SourceId, Spanned, ToCells, ToTokens, TokenTree, lex_str, parse_native, parse_with,
};

fn expression(source: &str) -> Expr {
    let mut file = parse_native(source, SourceId(31)).unwrap();
    assert_eq!(file.elements.len(), 1);
    let AnyCell::ExpressionCell(cell) = file.elements.pop().unwrap() else {
        panic!("expected one expression cell");
    };
    *cell.value
}

fn adjacent(expression: &Expr) -> (&Expr, &Expr) {
    let Expr::OperatorExpr(OperatorExpr::BinaryExpression(expression)) = expression else {
        panic!("expected implicit application, found {expression:?}");
    };
    assert!(matches!(expression.operator, BinaryOperator::Space(_)));
    (&expression.left, &expression.right)
}

fn binary(expression: &Expr) -> (&Expr, &BinaryOperator, &Expr) {
    let Expr::OperatorExpr(OperatorExpr::BinaryExpression(expression)) = expression else {
        panic!("expected a binary expression, found {expression:?}");
    };
    (&expression.left, &expression.operator, &expression.right)
}

#[test]
fn binding_strength_relative_to_precedence_controls_associativity() {
    let left = expression("a * b * c");
    let (left_operand, operator, _) = binary(&left);
    assert_eq!(operator.spelling(), "*");
    assert_eq!(binary(left_operand).1.spelling(), "*");

    let right = expression("a @ b @ c");
    let (_, operator, right_operand) = binary(&right);
    assert_eq!(operator.spelling(), "@");
    assert_eq!(binary(right_operand).1.spelling(), "@");
}

#[test]
fn implicit_application_uses_the_consumed_tokens_unary_handler_at_level_61() {
    let application = expression("b c d");
    let (_, right) = adjacent(&application);
    adjacent(right);

    let prefixed = expression("# f x");
    let Expr::OperatorExpr(OperatorExpr::PrefixExpression(prefix)) = prefixed else {
        panic!("expected prefix expression");
    };
    assert_eq!(prefix.operator.spelling(), "#");
    adjacent(&prefix.operand);
}

#[test]
fn bracket_precedence_differs_from_parenthesis_precedence() {
    let indexed_quotient = expression("R / I [x]");
    let (left, right) = adjacent(&indexed_quotient);
    assert_eq!(binary(left).1.spelling(), "/");
    assert!(matches!(right, Expr::Collection(Collection::Array(_))));

    let bracket = expression("a x []");
    let (left, right) = adjacent(&bracket);
    adjacent(left);
    assert!(matches!(right, Expr::Collection(Collection::Array(_))));

    let parenthesis = expression("a x ()");
    let (_, right) = adjacent(&parenthesis);
    let (_, call) = adjacent(right);
    assert!(matches!(
        call,
        Expr::Collection(Collection::ParenthesizedExpression(_))
    ));
}

#[test]
fn chained_delimiters_inherit_the_application_floor() {
    let low_then_high = expression("a [] ()");
    let (_, right) = adjacent(&low_then_high);
    let (array, parenthesis) = adjacent(right);
    assert!(matches!(array, Expr::Collection(Collection::Array(_))));
    assert!(matches!(
        parenthesis,
        Expr::Collection(Collection::ParenthesizedExpression(_))
    ));

    let high_then_low = expression("a () []");
    let (left, array) = adjacent(&high_then_low);
    let (_, parenthesis) = adjacent(left);
    assert!(matches!(
        parenthesis,
        Expr::Collection(Collection::ParenthesizedExpression(_))
    ));
    assert!(matches!(array, Expr::Collection(Collection::Array(_))));
}

#[test]
fn physical_newlines_are_cell_boundaries_but_required_operands_cross_them() {
    assert_eq!(
        parse_native("1\n1", SourceId(32)).unwrap().elements.len(),
        2
    );
    assert_eq!(
        parse_native("1 +\n1", SourceId(32)).unwrap().elements.len(),
        1
    );
    assert_eq!(
        parse_native("1 -* comment\n*- 1", SourceId(32))
            .unwrap()
            .elements
            .len(),
        2
    );
    assert!(matches!(
        parse_native("(1\n1)", SourceId(32)),
        Err(NativeParseError::NewlineInApplication { .. })
    ));

    for source in ["(1 SPACE\n1)", "(1\nSPACE 1)", "(1 +\n1)", "(1\n+ 1)"] {
        parse_native(source, SourceId(32))
            .unwrap_or_else(|error| panic!("`{source}` should parse, but failed with {error}"));
    }
}

#[test]
fn paired_top_level_semicolons_create_muted_cells() {
    let file = parse_native("2;2;2", SourceId(33)).unwrap();
    assert_eq!(file.elements.len(), 3);
    assert!(matches!(file.elements[0], AnyCell::MutedCell(_)));
    assert!(matches!(file.elements[1], AnyCell::MutedCell(_)));
    assert!(matches!(file.elements[2], AnyCell::ExpressionCell(_)));
}

#[test]
fn semicolons_cannot_delimit_empty_statements() {
    for source in [";", "1;;2", "(;)", "(1;;2)"] {
        parse_native(source, SourceId(33))
            .expect_err(&format!("`{source}` should reject an empty statement"));
    }
}

#[test]
fn implicit_and_explicit_space_remain_distinct_typed_forms() {
    let implicit = expression("f x");
    adjacent(&implicit);
    assert_eq!(implicit.to_code(), "f x");

    let explicit = expression("f SPACE x");
    assert!(matches!(binary(&explicit).1, BinaryOperator::Space(_)));
    assert_eq!(explicit.to_code(), "f SPACE x");
}

#[test]
fn compiler_edge_case_examples_reach_the_precedence_parser() {
    for source in [
        r#""1!!!1_!!!_!^^1!!(*)!(*()(*))1!Core$not$1$\\Core$symbol((*)""#,
        r#""1>>#####<<<<<<><>1>>> > > 1""#,
        r#"1!!!1_!!!_!^^1!!(*)!(()!(*))1!Core$not$1$Core$%symbol(*)"#,
        r#"1.p1e-00.00p1e-1_!!_!...1.e1"#,
    ] {
        parse_native(source, SourceId(34))
            .unwrap_or_else(|error| panic!("`{source}` failed with {error:?}"));
    }
}

#[test]
fn precedence_parser_consumes_the_shared_token_stream_directly() {
    let tokens = lex_str("a @ b @ c", SourceId(35)).unwrap();
    let mut parser = m2_syn::NativeParser::new();
    let file = parse_with(&mut parser, tokens).unwrap();
    assert_eq!(file.to_code(), "a @ b @ c");
}

#[test]
fn typed_tokens_and_cells_round_trip_through_the_native_parser() {
    let source_id = SourceId(351);
    let lexed = lex_str("a * b;\nreturn c", source_id).unwrap();
    let expected_multiply_span = lexed
        .iter()
        .flat_map(|cell| cell.stream().iter())
        .find_map(|tree| match tree {
            TokenTree::Punct(token) if token.text() == "*" => Some(token.span()),
            _ => None,
        })
        .expect("the lexer should emit the shared raw punctuation atom");

    let mut parser = m2_syn::NativeParser::new();
    let file = parse_with(&mut parser, lexed).unwrap();
    assert!(file.to_token_stream().into_iter().any(|tree| {
        matches!(tree, TokenTree::Punct(token)
            if token.text() == "*" && token.span() == expected_multiply_span)
    }));

    let cells = file.to_cell_stream(source_id);
    assert_eq!(cells.cells().len(), 2);
    assert_eq!(cells.to_string(), file.to_code());

    let reparsed = parse_with(&mut parser, cells).unwrap();
    assert_eq!(reparsed.to_code(), file.to_code());
}

#[test]
fn typed_strings_emit_literal_tokens_instead_of_delimiter_groups() {
    for (source, expected_kind) in [
        (r#""a\n""#, LiteralKind::String),
        ("///a//b///", LiteralKind::RawString),
    ] {
        let file = parse_native(source, SourceId(36)).unwrap();
        let tokens = file.to_token_stream().into_iter().collect::<Vec<_>>();
        assert_eq!(tokens.len(), 1);
        let TokenTree::Literal(literal) = &tokens[0] else {
            panic!("`{source}` emitted a non-literal token: {:?}", tokens[0]);
        };
        assert_eq!(literal.kind, expected_kind);
        assert_eq!(literal.text(), source);
    }
}

#[test]
fn prefix_control_statements_respect_optional_values_and_cell_boundaries() {
    let Expr::BreakStatement(statement) = expression("break") else {
        panic!("expected a break statement");
    };
    assert!(statement.value.is_none());

    let Expr::ContinueStatement(statement) = expression("continue x") else {
        panic!("expected a continue statement");
    };
    assert!(matches!(statement.value.as_deref(), Some(Expr::Symbol(_))));

    let Expr::ReturnStatement(statement) = expression("return 1 + 2") else {
        panic!("expected a return statement");
    };
    assert_eq!(
        binary(
            statement
                .value
                .as_deref()
                .expect("return should have a value")
        )
        .1
        .spelling(),
        "+"
    );

    assert_eq!(
        parse_native("return\n1", SourceId(37))
            .unwrap()
            .elements
            .len(),
        2
    );
    let Expr::CatchStatement(_) = expression("catch\n1") else {
        panic!("catch should require its value across a physical line break");
    };

    for keyword in ["catch", "throw", "trap"] {
        assert!(matches!(
            parse_native(keyword, SourceId(38)),
            Err(NativeParseError::MissingOperand { operator, .. }) if operator == keyword
        ));
    }
}

#[test]
fn clause_control_forms_build_the_existing_typed_nodes() {
    let Expr::IfStatement(statement) = expression("if a > 0 then b else c") else {
        panic!("expected an if statement");
    };
    assert_eq!(binary(&statement.condition).1.spelling(), ">");
    assert!(statement.else_clause.is_some());

    let Expr::ForLoop(statement) = expression("for i from 0 to 4 when i > 1 list i do print i")
    else {
        panic!("expected a for loop");
    };
    let range = statement.range.as_deref().expect("expected a range");
    assert!(range.range_start.is_some());
    assert!(range.range_end.is_some());
    assert!(statement.filter.is_some());
    assert!(statement.body.listed_value.is_some());
    assert!(statement.body.ignored_value.is_some());

    let Expr::WhileLoop(statement) = expression("while x < 10 do x = x + 1") else {
        panic!("expected a while loop");
    };
    assert_eq!(binary(&statement.condition).1.spelling(), "<");
    assert!(statement.body.ignored_value.is_some());

    let Expr::TryStatement(statement) = expression("try x then y except err do err") else {
        panic!("expected a try statement");
    };
    assert!(statement.then_clause.is_some());
    assert!(matches!(
        statement.fallback.as_deref(),
        Some(m2_syn::TryFallback::ExceptClause(_))
    ));
}

#[test]
fn nested_control_clauses_follow_their_12_and_16_stopper_levels() {
    for source in [
        "if a then if b then x else y",
        "if a then x else if b then y else z",
        "if try x then y else z then 1",
        "while for i in L list 1 do 2 do 3",
        "for i in L list for j in L list j do print 2",
        "p = x -> toString if instance(x, Package) then x else if package x =!= null then package x else Core",
    ] {
        let file = parse_native(source, SourceId(39))
            .unwrap_or_else(|error| panic!("`{source}` failed with {error:?}"));
        let normalized = file.to_code();
        parse_native(&normalized, SourceId(40)).unwrap_or_else(|error| {
            panic!("normalized `{normalized}` from `{source}` failed with {error:?}")
        });
    }
}

#[test]
fn incomplete_control_clauses_report_the_missing_component() {
    for source in [
        "if x",
        "if x then",
        "for i in list x",
        "while x",
        "try x except do x",
    ] {
        assert!(
            parse_native(source, SourceId(41)).is_err(),
            "`{source}` should be rejected"
        );
    }
}

#[test]
fn new_statement_parses_its_ordered_optional_clauses() {
    let Expr::NewStatement(statement) = expression("new HashTable of MutableHashTable from pairs")
    else {
        panic!("expected a new statement");
    };
    assert!(matches!(statement.class.as_ref(), Expr::Symbol(_)));
    assert!(matches!(statement.parent.as_deref(), Some(Expr::Symbol(_))));
    assert!(matches!(
        statement.instance.as_deref(),
        Some(Expr::Symbol(_))
    ));

    for source in ["new A", "new A of B", "new A from C"] {
        let file = parse_native(source, SourceId(42))
            .unwrap_or_else(|error| panic!("`{source}` failed with {error:?}"));
        let normalized = file.to_code();
        parse_native(&normalized, SourceId(43)).unwrap_or_else(|error| {
            panic!("normalized `{normalized}` from `{source}` failed with {error:?}")
        });
    }

    for source in ["new", "new A of", "new A from"] {
        assert!(
            matches!(
                parse_native(source, SourceId(44)),
                Err(NativeParseError::MissingOperand { .. })
            ),
            "`{source}` should require its missing operand"
        );
    }
}

#[test]
fn debug_clauses_distinguish_nullable_and_required_operands() {
    for source in ["step", "finish"] {
        let Expr::DebugClause(clause) = expression(source) else {
            panic!("`{source}` should be a debug clause");
        };
        assert!(clause.value.is_none());
    }

    for source in [
        "step x",
        "finish x",
        "shield x",
        "TEST x",
        "time x",
        "timing x",
        "breakpoint x",
        "elapsedTime x",
        "elapsedTiming x",
        "profile x",
    ] {
        let Expr::DebugClause(clause) = expression(source) else {
            panic!("`{source}` should be a debug clause");
        };
        assert!(clause.value.is_some());
    }

    for keyword in [
        "shield",
        "TEST",
        "time",
        "timing",
        "breakpoint",
        "elapsedTime",
        "elapsedTiming",
        "profile",
    ] {
        assert!(matches!(
            parse_native(keyword, SourceId(45)),
            Err(NativeParseError::MissingOperand { operator, .. }) if operator == keyword
        ));

        let source = format!("{keyword}\nx");
        assert_eq!(
            parse_native(&source, SourceId(46)).unwrap().elements.len(),
            1,
            "`{keyword}` should take its required value across a newline"
        );
    }
}

#[test]
fn quote_specifiers_turn_keyword_spellings_into_symbols() {
    let Expr::QuoteExpression(quote) = expression("symbol if") else {
        panic!("expected a quote expression");
    };
    let symbol = quote.token;
    assert_eq!(symbol.text, "if");
}

#[test]
fn core_keyword_aliases_share_their_unqualified_parse_behavior() {
    let Expr::IfStatement(statement) = expression("Core$if x Core$then y Core$else z") else {
        panic!("Core$if should parse as an if statement");
    };
    assert!(statement.else_clause.is_some());

    let Expr::ForLoop(_) = expression("Core$for i Core$in L Core$list i") else {
        panic!("qualified loop keywords should retain their parser behavior");
    };

    let Expr::NewStatement(statement) = expression("Core$new A Core$of B Core$from C") else {
        panic!("Core$new should parse as a new statement");
    };
    assert!(statement.parent.is_some());
    assert!(statement.instance.is_some());

    let Expr::QuoteExpression(quote) = expression("Core$symbol if") else {
        panic!("Core$symbol should parse as a quote specifier");
    };
    assert_eq!(quote.token.text, "if");
}

#[test]
fn core_aliases_affect_cells_without_aliasing_operators_or_recursive_names() {
    let file = parse_native("Core$if x\nCore$then y", SourceId(47)).unwrap();
    assert_eq!(file.elements.len(), 1);
    assert!(matches!(file.elements[0], AnyCell::ExpressionCell(_)));

    let ordinary = expression("Core$not x");
    let (left, _) = adjacent(&ordinary);
    assert!(matches!(left, Expr::Symbol(symbol) if symbol.text == "Core$not"));

    let recursive = expression("Core$Core$if x");
    let (left, _) = adjacent(&recursive);
    assert!(matches!(left, Expr::Symbol(symbol) if symbol.text == "Core$Core$if"));
}
