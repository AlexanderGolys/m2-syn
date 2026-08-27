use m2_syn::nodes::{BinaryOperator, Collection, Component, ControlFlow, Expr, OperatorExpr};
use m2_syn::{
    DelimiterToken, Lex, LexErrorKind, LiteralKind, NativeParseError, Span, Spanned, Terminal,
    TokenTree, Tokenize, lex_str, parse_native, parse_native_recovering, parse_with,
};

fn expression(source: &str) -> Expr {
    let mut file = parse_native(source).unwrap();
    assert_eq!(file.cells.len(), 1);
    let cell = file.cells.pop().unwrap();
    let mut components = cell.content.pack.into_iter();
    let Some(Component::Expr(value)) = components.next() else {
        panic!("expected one expression component");
    };
    assert!(components.next().is_none());
    *value
}

fn adjacent(expression: &Expr) -> (&Expr, &Expr) {
    let Expr::OperatorExpr(OperatorExpr::AdjacentExpr(expression)) = expression else {
        panic!("expected implicit application, found {expression:?}");
    };
    (&expression.left_operand, &expression.right_operand)
}

fn binary(expression: &Expr) -> (&Expr, &BinaryOperator, &Expr) {
    let Expr::OperatorExpr(OperatorExpr::BinaryExpr(expression)) = expression else {
        panic!("expected a binary expression, found {expression:?}");
    };
    (
        &expression.left_operand,
        &expression.operator,
        &expression.right_operand,
    )
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
    let Expr::OperatorExpr(OperatorExpr::PrefixExpr(prefix)) = prefixed else {
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
    assert!(matches!(call, Expr::Holder(_)));
}

#[test]
fn chained_delimiters_inherit_the_application_floor() {
    let low_then_high = expression("a [] ()");
    let (_, right) = adjacent(&low_then_high);
    let (array, parenthesis) = adjacent(right);
    assert!(matches!(array, Expr::Collection(Collection::Array(_))));
    assert!(matches!(parenthesis, Expr::Holder(_)));

    let high_then_low = expression("a () []");
    let (left, array) = adjacent(&high_then_low);
    let (_, parenthesis) = adjacent(left);
    assert!(matches!(parenthesis, Expr::Holder(_)));
    assert!(matches!(array, Expr::Collection(Collection::Array(_))));
}

#[test]
fn physical_newlines_are_cell_boundaries_but_required_operands_cross_them() {
    assert_eq!(parse_native("1\n1").unwrap().cells.len(), 2);
    assert_eq!(parse_native("1 +\n1").unwrap().cells.len(), 1);
    assert_eq!(parse_native("1 -* comment\n*- 1").unwrap().cells.len(), 2);
    assert!(matches!(
        parse_native("(1\n1)"),
        Err(NativeParseError::NewlineInApplication { .. })
    ));

    for source in ["(1 +\n1)", "(1\n+ 1)"] {
        parse_native(source)
            .unwrap_or_else(|error| panic!("`{source}` should parse, but failed with {error}"));
    }
}

#[test]
fn paired_top_level_semicolons_create_muted_cells() {
    let file = parse_native("2;2;2").unwrap();
    assert_eq!(file.cells.len(), 3);
    assert!(file.cells[0].semicolon.is_some());
    assert!(file.cells[1].semicolon.is_some());
    assert!(file.cells[2].semicolon.is_none());
}

#[test]
fn semicolons_cannot_delimit_empty_statements() {
    for source in [";", "1;;2", "(;)", "(1;;2)"] {
        parse_native(source).expect_err(&format!("`{source}` should reject an empty statement"));
    }
}

#[test]
fn application_is_independent_of_physical_whitespace() {
    adjacent(&expression("f(x)"));
    adjacent(&expression("f (x)"));
}

#[test]
fn physical_eof_retains_trailing_layout_after_the_final_cell() {
    let file = parse_native("x   ").unwrap();

    assert_eq!(file.cells[0].eoc.span().bounds(), Some((1, 1)));
    assert_eq!(file.eof.span().bounds(), Some((4, 4)));
    assert_eq!(file.span().bounds(), Some((0, 4)));
}

#[test]
fn compiler_edge_case_examples_reach_the_precedence_parser() {
    for source in [
        r#""1!!!1_!!!_!^^1!!(*)!(*()(*))1!Core$not$1$\\Core$symbol((*)""#,
        r#""1>>#####<<<<<<><>1>>> > > 1""#,
        r#"1!!!1_!!!_!^^1!!(*)!(()!(*))1!Core$not$1$Core$%symbol(*)"#,
        r#"1.p1e-00.00p1e-1_!!_!...1.e1"#,
    ] {
        parse_native(source).unwrap_or_else(|error| panic!("`{source}` failed with {error:?}"));
    }
}

#[test]
fn precedence_parser_consumes_the_tokenized_cell_stream() {
    let tokens = lex_str("a @ b @ c").unwrap();
    let mut parser = m2_syn::NativeParser;
    let file = parse_with(&mut parser, tokens.to_cell_stream().unwrap()).unwrap();
    assert_eq!(file.to_m2(), "a @ b @ c");
}

#[test]
fn typed_tokens_and_cells_round_trip_through_the_native_parser() {
    let lexed = lex_str("a * b;\nreturn c").unwrap();
    let expected_multiply_span = lexed
        .iter()
        .find_map(|tree| match tree {
            TokenTree::Punct(token) if token.text() == "*" => Some(token.span()),
            _ => None,
        })
        .expect("the lexer should emit the shared raw punctuation atom");

    let mut parser = m2_syn::NativeParser;
    let file = parse_with(&mut parser, lexed.to_cell_stream().unwrap()).unwrap();
    assert!(file.to_token_stream().into_iter().any(|tree| {
        matches!(tree, TokenTree::Punct(token)
            if token.text() == "*" && token.span() == expected_multiply_span)
    }));

    // The typed graph projects back through the same explicit cell boundary.
    assert_eq!(file.cells.len(), 2);
    let emitted = file.to_token_stream();
    assert_eq!(emitted.to_string(), file.to_m2());

    let reparsed = parse_with(&mut parser, emitted.to_cell_stream().unwrap()).unwrap();
    assert_eq!(reparsed.to_m2(), file.to_m2());
}

#[test]
fn typed_strings_emit_literal_tokens_instead_of_delimiter_groups() {
    for (source, expected_kind) in [
        (r#""a\n""#, LiteralKind::String),
        ("///a//b///", LiteralKind::RawString),
    ] {
        let file = parse_native(source).unwrap();
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
    let Expr::ControlFlow(ControlFlow::BreakExpr(statement)) = expression("break") else {
        panic!("expected a break statement");
    };
    assert!(statement.value.is_none());

    let Expr::ControlFlow(ControlFlow::ContinueExpr(statement)) = expression("continue x") else {
        panic!("expected a continue statement");
    };
    assert!(matches!(statement.value.as_deref(), Some(Expr::Symbol(_))));

    let Expr::ControlFlow(ControlFlow::ReturnExpr(statement)) = expression("return 1 + 2") else {
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

    assert_eq!(parse_native("return\n1").unwrap().cells.len(), 2);
    let Expr::Catch(_) = expression("catch\n1") else {
        panic!("catch should require its value across a physical line break");
    };

    for keyword in ["catch", "throw", "trap"] {
        assert!(matches!(
            parse_native(keyword),
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
    let Some(m2_syn::nodes::IterationDomain::IterRange(range)) = &statement.iteration_domain else {
        panic!("expected a range");
    };
    assert!(range.lower_bound.is_some());
    assert!(range.upper_bound.is_some());
    assert!(statement.when_condition.is_some());
    assert!(statement.body.list_step.is_some());
    assert!(statement.body.do_step.is_some());

    let Expr::WhileLoop(statement) = expression("while x < 10 do x = x + 1") else {
        panic!("expected a while loop");
    };
    assert_eq!(binary(&statement.condition).1.spelling(), "<");
    assert!(statement.body.do_step.is_some());

    let Expr::TryStatement(statement) = expression("try x then y except err do err") else {
        panic!("expected a try statement");
    };
    assert!(statement.on_success.is_some());
    assert!(matches!(
        statement.fallback.as_ref(),
        Some(m2_syn::nodes::TryFallback::ExceptDo(_))
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
        let file =
            parse_native(source).unwrap_or_else(|error| panic!("`{source}` failed with {error:?}"));
        let normalized = file.to_m2();
        parse_native(&normalized).unwrap_or_else(|error| {
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
            parse_native(source).is_err(),
            "`{source}` should be rejected"
        );
    }
}

#[test]
fn incomplete_operators_report_expected_and_found_state() {
    let error = parse_native("value +").unwrap_err();

    assert_eq!(
        error.to_string(),
        "expected an operand after `+`, found end of input"
    );
}

#[test]
fn new_statement_parses_its_ordered_optional_clauses() {
    let Expr::NewStatement(statement) = expression("new HashTable of MutableHashTable from pairs")
    else {
        panic!("expected a new statement");
    };
    assert!(matches!(statement.class.as_ref(), Expr::Symbol(_)));
    assert!(matches!(
        statement
            .parent_class
            .as_ref()
            .map(|clause| clause.parent.as_ref()),
        Some(Expr::Symbol(_))
    ));
    assert!(matches!(
        statement
            .instance
            .as_ref()
            .map(|clause| clause.instance.as_ref()),
        Some(Expr::Symbol(_))
    ));

    for source in ["new A", "new A of B", "new A from C"] {
        let file =
            parse_native(source).unwrap_or_else(|error| panic!("`{source}` failed with {error:?}"));
        let normalized = file.to_m2();
        parse_native(&normalized).unwrap_or_else(|error| {
            panic!("normalized `{normalized}` from `{source}` failed with {error:?}")
        });
    }

    for source in ["new", "new A of", "new A from"] {
        assert!(
            matches!(
                parse_native(source),
                Err(NativeParseError::MissingOperand { .. })
            ),
            "`{source}` should require its missing operand"
        );
    }
}

#[test]
fn debug_clauses_distinguish_nullable_and_required_operands() {
    for source in ["step", "finish"] {
        let Expr::DebugExpr(clause) = expression(source) else {
            panic!("`{source}` should be a debug clause");
        };
        assert!(clause.value.is_none());
    }

    for source in ["step x", "finish x", "shield x", "TEST x", "breakpoint x"] {
        let Expr::DebugExpr(clause) = expression(source) else {
            panic!("`{source}` should be a debug clause");
        };
        assert!(clause.value.is_some());
    }

    for source in [
        "time x",
        "timing x",
        "elapsedTime x",
        "elapsedTiming x",
        "profile x",
    ] {
        assert!(matches!(expression(source), Expr::ProfilingExpr(_)));
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
            parse_native(keyword),
            Err(NativeParseError::MissingOperand { operator, .. }) if operator == keyword
        ));

        let source = format!("{keyword}\nx");
        assert_eq!(
            parse_native(&source).unwrap().cells.len(),
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
    let symbol = quote.symbol;
    assert_eq!(symbol.text(), "if");
}

#[test]
fn quote_specifiers_take_brackets_and_semicolons_as_plain_symbols() {
    // A quote specifier consumes whatever token follows it, so punctuation
    // after one is quoted rather than read as structure. Checked against
    // M2 v1.26.05, where `symbol (`, `local {` and `symbol ;` each evaluate to
    // a symbol instead of opening a group or ending a statement.
    for (source, quoted) in [
        ("symbol (", "("),
        ("symbol )", ")"),
        ("local {", "{"),
        ("symbol ;", ";"),
    ] {
        let Expr::QuoteExpression(quote) = expression(source) else {
            panic!("`{source}` should quote its bracket");
        };
        assert_eq!(quote.symbol.text(), quoted);
    }
}

#[test]
fn each_quote_specifier_quotes_only_the_token_after_it() {
    // The second `symbol` re-arms quoting, so neither bracket pairs with the
    // other. M2 accepts this as syntax and only fails when evaluating it.
    let application = expression("symbol ( symbol )");
    let (left, right) = adjacent(&application);
    let Expr::QuoteExpression(left) = left else {
        panic!("expected a quoted left operand");
    };
    let Expr::QuoteExpression(right) = right else {
        panic!("expected a quoted right operand");
    };
    assert_eq!(left.symbol.text(), "(");
    assert_eq!(right.symbol.text(), ")");

    // A specifier that is itself quoted is data, not another instruction to
    // quote. The following parentheses therefore retain their normal grouping
    // role in both lexing and top-level cell splitting.
    let file = parse_native("symbol symbol (1)").unwrap();
    assert_eq!(file.cells.len(), 1);
    assert_eq!(file.to_m2(), "symbol symbol (1)");
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
    assert!(statement.parent_class.is_some());
    assert!(statement.instance.is_some());

    let Expr::QuoteExpression(quote) = expression("Core$symbol if") else {
        panic!("Core$symbol should parse as a quote specifier");
    };
    assert_eq!(quote.symbol.text(), "if");
}

#[test]
fn core_word_aliases_apply_exactly_once() {
    let file = parse_native("Core$if x\nCore$then y").unwrap();
    assert_eq!(file.cells.len(), 1);

    let Expr::OperatorExpr(OperatorExpr::PrefixExpr(qualified_not)) = expression("Core$not x")
    else {
        panic!("one Core qualifier must retain the hardcoded `not` alias");
    };
    assert_eq!(qualified_not.operator.spelling(), "not");

    let conjunction = expression("x Core$and y");
    assert_eq!(binary(&conjunction).1.spelling(), "and");

    let recursive = expression("Core$Core$if x");
    let (left, _) = adjacent(&recursive);
    assert!(matches!(left, Expr::Symbol(symbol) if symbol.text() == "Core$Core$if"));

    let recursive_not = expression("Core$Core$not x");
    let (left, _) = adjacent(&recursive_not);
    assert!(matches!(left, Expr::Symbol(symbol) if symbol.text() == "Core$Core$not"));

    let Expr::QuoteExpression(quoted) = expression("symbol Core$not") else {
        panic!("quotation must suppress the hardcoded alias");
    };
    assert_eq!(quoted.symbol.text(), "Core$not");
    assert_eq!(
        quoted
            .symbol
            .package_qualifier
            .as_ref()
            .and_then(Span::bounds),
        Some((7, 11))
    );
}

#[test]
fn symbols_retain_optional_package_and_index_qualifier_spans() {
    let Expr::Symbol(symbol) = expression("Package$name$12") else {
        panic!("the qualified spelling must remain one symbol");
    };
    assert_eq!(symbol.text(), "Package$name$12");
    assert_eq!(
        symbol.package_qualifier.as_ref().and_then(Span::bounds),
        Some((0, 7))
    );
    assert_eq!(
        symbol.index_qualifier.as_ref().and_then(Span::bounds),
        Some((13, 15))
    );

    let Expr::Symbol(symbol) = expression("Foo$if") else {
        panic!("non-Core qualification must not activate the keyword alias");
    };
    assert_eq!(
        symbol.package_qualifier.as_ref().and_then(Span::bounds),
        Some((0, 3))
    );
    assert_eq!(symbol.index_qualifier, None);
}

#[test]
fn recovering_native_parser_keeps_unclosed_collection_contents() {
    let source = "[x, y";
    let recovered = parse_native_recovering(source).unwrap();
    assert_eq!(recovered.diagnostics.len(), 1);
    assert_eq!(
        recovered.diagnostics[0].kind(),
        LexErrorKind::UnterminatedGroup
    );
    assert_eq!(
        recovered.diagnostics[0].span().bounds(),
        Some((0, source.len()))
    );
    assert_eq!(recovered.value.to_m2(), "[x, y");

    let mut cells = recovered.value.cells;
    let mut components = cells.pop().unwrap().content.pack.into_iter();
    let Some(Component::Expr(expression)) = components.next() else {
        panic!("the recovered cell must contain the array expression");
    };
    let Expr::Collection(Collection::Array(array)) = *expression else {
        panic!("the recovered expression must retain its array node");
    };
    assert_eq!(array.elements.unwrap().pack.len(), 2);
    assert!(array.delimiter.spans().close().is_none());
}

#[test]
fn documentation_line_blocks_belong_to_the_following_cell() {
    let source = concat!(
        "    -- Recording rather than printing keeps the captured transcript free of\n",
        "    -- everything but genuinely printed text. A `;` still suppresses the value:\n",
        "    -- NoPrint deliberately skips `net value`, which is the render the punctuation\n",
        "    -- exists to avoid on large objects. The type is read either way, so the shell\n",
        "    -- can say what a suppressed result is without paying to render it.\n",
        "    recordValue := value -> output.value = \"\" | toString net value;",
    );

    let file = parse_native(source).unwrap();

    assert_eq!(file.cells.len(), 1);
    assert_eq!(file.cells[0].comments.outer().len(), 1);
    assert!(file.cells[0].comments.inner().is_empty());
    assert_eq!(
        file.cells[0].comments.outer()[0].text(),
        source.split_once("\n    recordValue").unwrap().0
    );
    assert_eq!(file.to_m2(), source);
}

#[test]
fn cell_owned_documentation_does_not_leak_onto_its_first_leaf() {
    let file = parse_native("-- documentation\nvalue").unwrap();
    let Some(Component::Expr(expression)) = file.cells[0].content.pack.iter().next() else {
        panic!("the cell must contain its documented expression");
    };
    let Expr::Symbol(symbol) = expression.as_ref() else {
        panic!("the documented cell must contain the symbol expression");
    };

    assert_eq!(file.cells[0].comments.outer().len(), 1);
    assert!(symbol.leading_trivia().is_empty());
    assert!(symbol.trailing_trivia().is_empty());
}

#[test]
fn inline_line_comments_are_inner_comments_of_their_cell() {
    let source = "value := 1; -- suppressed\nnext";
    let file = parse_native(source).unwrap();

    assert_eq!(file.cells.len(), 2);
    assert_eq!(file.cells[0].comments.inner().len(), 1);
    assert_eq!(file.cells[0].comments.inner()[0].text(), "-- suppressed");
    assert!(file.cells[1].comments.is_empty());
    assert_eq!(file.to_m2(), source);
}

#[test]
fn comment_only_source_is_owned_by_the_source_file() {
    let source = "-- first\n-- second";
    let file = parse_native(source).unwrap();

    assert!(file.cells.is_empty());
    assert_eq!(file.comments.inner().len(), 1);
    assert_eq!(file.comments.inner()[0].text(), source);
    assert_eq!(file.to_m2(), source);
}

#[test]
fn block_comments_are_inner_comments_of_the_containing_cell() {
    let source = "-* implementation note *-\nvalue";
    let file = parse_native(source).unwrap();

    assert_eq!(file.cells.len(), 1);
    assert!(file.cells[0].comments.outer().is_empty());
    assert_eq!(file.cells[0].comments.inner().len(), 1);
    assert_eq!(
        file.cells[0].comments.inner()[0].text(),
        "-* implementation note *-"
    );
    assert_eq!(file.to_m2(), source);
}
