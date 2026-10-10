//! Integration tests for tree sitter reconstruction.

use m2_syn::nodes::{
    BinaryOperator, Expr, ExprAssign, ExprAssignSimple, ExprCollection, ExprControl, ExprLiteral,
    ExprOp, ExprOpBin, ExprQuote, PackedContent,
};
use m2_syn::{DelimiterToken, Lex, Span, Spanned, parse_file, parse_file_recovering, parse_native};
use m2_syn::{Tokenize, lex_str, parse_tokens};

fn expression(source: &str) -> Expr {
    let mut file = parse_file(source).unwrap();
    assert_eq!(file.cells.len(), 1);
    let PackedContent::Expr(expression) = file.cells.pop().unwrap().content else {
        panic!("expected one expression");
    };
    *expression
}

fn native_expression(source: &str) -> Expr {
    let mut file = parse_native(source).unwrap();
    let PackedContent::Expr(expression) = file.cells.pop().unwrap().content else {
        panic!("expected one native expression");
    };
    *expression
}

#[test]
fn token_parsing_uses_the_supplied_transformed_stream() {
    let mut tokens = lex_str("left\nright").unwrap();
    tokens.pop();

    let parsed = parse_tokens(&tokens).unwrap();

    assert_eq!(parsed.cells.len(), 1);
    assert_eq!(parsed.to_cell_stream().unwrap().to_string(), "left");
    assert_eq!(parsed.span().text(), Some("left"));
}

#[test]
fn tree_sitter_lowers_binary_nodes_directly() {
    let expression = expression("left + right");
    let Expr::Op(ExprOp::Bin(ExprOpBin::Regular(expression))) = expression else {
        panic!("expected a binary expression");
    };

    assert!(
        matches!(expression.left_operand.as_ref(), Expr::Symbol(symbol) if symbol.text() == "left")
    );
    assert!(matches!(expression.operator, BinaryOperator::Add(_)));
    assert!(
        matches!(expression.right_operand.as_ref(), Expr::Symbol(symbol) if symbol.text() == "right")
    );
}

#[test]
fn implicit_application_has_its_own_empty_marker() {
    let expression = expression("f x");
    let Expr::Op(ExprOp::Bin(ExprOpBin::Adj(expression))) = expression else {
        panic!("application must be an adjacency expression");
    };

    assert_eq!(expression.operator.span().bounds(), Some((2, 2)));
    assert_eq!(expression.operator.to_m2(), "");
}

#[test]
fn adjacency_is_syntactic_and_never_migrated_from_trivia() {
    for source in ["f(x)", "f (x)"] {
        let Expr::Op(ExprOp::Bin(ExprOpBin::Adj(application))) = expression(source) else {
            panic!("`{source}` must be an adjacency expression");
        };
        assert!(application.operator.span().is_empty());
        assert_eq!(application.operator.to_m2(), "");
    }

    let Expr::Control(ExprControl::Return(returned)) = expression("return 1") else {
        panic!("return must own its operand directly");
    };
    assert!(matches!(returned.value.as_deref(), Some(Expr::Literal(_))));
}

#[test]
fn tree_sitter_applies_exact_core_word_aliases_before_grammar_parsing() {
    let Expr::Op(ExprOp::Prefix(qualified_not)) = expression("Core$not x") else {
        panic!("Core$not must use the hardcoded `not` alias");
    };
    assert_eq!(qualified_not.operator.spelling(), "not");
    assert_eq!(qualified_not.operator.span().bounds(), Some((0, 8)));

    let conjunction = expression("x Core$and y");
    let Expr::Op(ExprOp::Bin(ExprOpBin::Regular(conjunction))) = conjunction else {
        panic!("Core$and must use the hardcoded binary alias");
    };
    assert_eq!(conjunction.operator.spelling(), "and");
    assert_eq!(conjunction.operator.span().bounds(), Some((2, 10)));

    let recursive = expression("Core$Core$not x");
    let Expr::Op(ExprOp::Bin(ExprOpBin::Adj(recursive))) = recursive else {
        panic!("a recursively qualified spelling must remain an ordinary symbol");
    };
    assert!(
        matches!(recursive.left_operand.as_ref(), Expr::Symbol(symbol)
        if symbol.text() == "Core$Core$not"
            && symbol.package_qualifier.is_none()
            && symbol.index_qualifier.is_none())
    );

    let Expr::Quote(ExprQuote::Symbol(quoted)) = expression("symbol Core$not") else {
        panic!("a quote specifier must suppress the alias behavior");
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
fn tree_sitter_symbols_derive_qualifier_metadata_from_one_identifier_node() {
    let Expr::Symbol(symbol) = expression("Package$name$12") else {
        panic!("the spelling must remain one symbol");
    };
    assert_eq!(symbol.text(), "Package$name$12");
    assert_eq!(symbol.span().bounds(), Some((0, 15)));
    assert_eq!(
        symbol.package_qualifier.as_ref().and_then(Span::bounds),
        Some((0, 7))
    );
    assert_eq!(
        symbol.index_qualifier.as_ref().and_then(Span::bounds),
        Some((13, 15))
    );

    let Expr::Symbol(non_core) = expression("Foo$if") else {
        panic!("only Core owns the hardcoded keyword aliases");
    };
    assert_eq!(
        non_core.package_qualifier.as_ref().and_then(Span::bounds),
        Some((0, 3))
    );
}

#[test]
fn both_backends_keep_precision_and_exponent_in_one_float_literal() {
    for source in ["1p53", "1.25p53", "1.25p53e3", ".5p20"] {
        for parsed in [expression(source), native_expression(source)] {
            let Expr::Literal(ExprLiteral::Float(literal)) = parsed else {
                panic!("`{source}` must be represented as a float literal");
            };
            assert_eq!(literal.text(), source);
            assert_eq!(literal.span().bounds(), Some((0, source.len())));
        }
    }

    for parsed in [expression("1e3p20"), native_expression("1e3p20")] {
        let Expr::Op(ExprOp::Bin(ExprOpBin::Adj(application))) = parsed else {
            panic!("precision after an exponent must begin a following symbol");
        };
        assert!(matches!(
            application.left_operand.as_ref(),
            Expr::Literal(_)
        ));
        assert!(
            matches!(application.right_operand.as_ref(), Expr::Symbol(symbol) if symbol.text() == "p20")
        );
    }
}

#[test]
fn tree_sitter_recovery_keeps_unclosed_collection_contents_and_source_spans() {
    let source = "[x, y";
    let recovered = parse_file_recovering(source).unwrap();
    assert_eq!(recovered.diagnostics.len(), 1);
    assert_eq!(
        recovered.diagnostics[0].span().bounds(),
        Some((0, source.len()))
    );
    assert_eq!(recovered.value.to_m2(), "[x, y");

    let mut cells = recovered.value.cells;
    let PackedContent::Expr(expression) = cells.pop().unwrap().content else {
        panic!("the recovered cell must contain the array expression");
    };
    let Expr::Collection(ExprCollection::Array(array)) = *expression else {
        panic!("the recovered expression must retain its array node");
    };
    let Some(PackedContent::Pack(elements)) = array.elements.as_ref() else {
        panic!("the recovered array must retain its comma pack");
    };
    assert_eq!(elements.pack.len(), 2);
    assert_eq!(array.span().bounds(), Some((0, source.len())));
    assert!(array.delimiter.spans().close().is_none());
}

#[test]
fn delimited_nodes_retain_both_boundary_spans() {
    let Expr::Holder(group) = expression("(x)") else {
        panic!("the expression must retain its parenthesis node");
    };

    let spans = group.delimiter.spans();
    assert_eq!(spans.open().bounds(), Some((0, 1)));
    assert_eq!(spans.close().unwrap().bounds(), Some((2, 3)));
}

#[test]
fn complete_source_files_retain_cells_and_boundary_nodes() {
    let source = "x + 1\ny + x * 2\nif y then return y else 0";
    let source_file = parse_file(source).unwrap();

    assert_eq!(source_file.cells.len(), 3);
    assert_eq!(
        source_file.eof.span().bounds(),
        Some((source.len(), source.len()))
    );
    assert_eq!(source_file.span().end(), Some(source.len()));
}

#[test]
fn trivia_only_source_files_place_eof_at_the_physical_end() {
    let source = "-- comment";
    let source_file = parse_file(source).unwrap();

    assert!(source_file.cells.is_empty());
    assert_eq!(source_file.comments().inner().len(), 1);
    assert_eq!(source_file.comments().inner()[0].text(), source);
    assert_eq!(
        source_file.eof.span().bounds(),
        Some((source.len(), source.len()))
    );
    assert_eq!(source_file.span().bounds(), Some((0, source.len())));
    assert_eq!(source_file.to_m2(), source);
}

#[test]
fn tree_sitter_preserves_cell_owned_documentation_blocks() {
    let source = "-- first line\n-- second line\nrecordValue := value;";
    let source_file = parse_file(source).unwrap();

    assert_eq!(source_file.cells.len(), 1);
    assert_eq!(source_file.cells[0].comments().outer().len(), 1);
    assert_eq!(
        source_file.cells[0].comments().outer()[0].text(),
        "-- first line\n-- second line"
    );
    assert_eq!(source_file.to_m2(), source);
}

#[test]
fn both_backends_locate_structural_boundaries_between_cell_streams() {
    let source = "x\n  y; z";
    let tree_sitter = parse_file(source).unwrap();
    let native = parse_native(source).unwrap();

    let expected = [
        Some((4, 4)),
        Some((7, 7)),
        Some((source.len(), source.len())),
    ];
    assert_eq!(
        tree_sitter
            .cells
            .iter()
            .map(|cell| cell.eoc.span().bounds())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(
        native
            .cells
            .iter()
            .map(|cell| cell.eoc.span().bounds())
            .collect::<Vec<_>>(),
        expected
    );
}

#[test]
fn nested_and_global_semicolons_have_distinct_owners() {
    let source_file = parse_file("{x; y}\nx;").unwrap();
    let PackedContent::Expr(expression) = &source_file.cells[0].content else {
        panic!("expected a list expression");
    };
    let Expr::Collection(ExprCollection::List(list)) = expression.as_ref() else {
        panic!("the first cell must contain a list");
    };

    assert_eq!(list.statements.len(), 1);
    assert!(source_file.cells[0].semicolon.is_none());
    assert!(source_file.cells[1].semicolon.is_some());
}

#[test]
fn assignments_use_the_specialized_cleaned_tree() {
    let Expr::Assign(ExprAssign::Simple(ExprAssignSimple::Global(_))) = expression("x = 1") else {
        panic!("expected a global binding");
    };

    let Expr::Option(_) = expression("key => value") else {
        panic!("expected an option expression");
    };

    let Expr::For(_) = expression("for x in y list x") else {
        panic!("expected a for loop");
    };
}

#[test]
fn emitted_source_reparses_to_the_same_normal_form() {
    let source = "x + 1\ny + x * 2\nif y then return y else 0";
    let syntax = parse_file(source).unwrap();
    let emitted = syntax.to_m2();
    let reparsed = parse_file(&emitted).unwrap();

    assert_eq!(reparsed.to_m2(), emitted);
}

#[test]
fn every_tree_sitter_expression_family_lowers_without_native_reparsing() {
    let examples = [
        "x",
        "1",
        "1.5",
        "\"text\\nmore\"",
        "///raw text///",
        "[]",
        "[x,,y]",
        "{x; y,z}",
        "<|x|>",
        "()",
        "(x)",
        "(x,y)",
        "(x; y)",
        "x + y",
        "x += y",
        "f(x)",
        "f (x)",
        "f x",
        "-x",
        "x!",
        "x -> x",
        "(x,y) -> x",
        "x = y",
        "x := y",
        "x + y = z",
        "f x := y => z",
        "-x := y",
        "x! := y",
        "(x,y) = z",
        "{x} := z",
        "x <- y",
        "x => y",
        "if x then y",
        "if x then y else z",
        "for x list x",
        "for x in xs do x",
        "for x from a to b when p list x do y",
        "while p do x",
        "new C",
        "new C of P from x",
        "try x",
        "try x then y else z",
        "try x then y except e do z",
        "symbol x",
        "local if",
        "break",
        "continue x",
        "return x",
        "catch x",
        "throw x",
        "trap x",
        "TEST x",
        "step x",
        "finish",
        "shield x",
        "breakpoint x",
        "time x",
        "timing x",
        "elapsedTime x",
        "elapsedTiming x",
        "profile x",
    ];

    for source in examples {
        let syntax = parse_file(source)
            .unwrap_or_else(|error| panic!("failed to lower `{source}`: {error}"));
        let emitted = syntax.to_m2();
        parse_file(&emitted).unwrap_or_else(|error| {
            panic!("failed to reparse `{source}` emitted as `{emitted}`: {error}")
        });
    }
}
