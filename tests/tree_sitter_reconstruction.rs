use m2_syn::treesitter::TreeSitterNode;
use m2_syn::{
    AnyCell, AssignmentExpr, BinaryExpression, BinaryOperator, Collection, Expr, Reconstruct,
    SourceFile, SourceId, Spanned, ToTokens, parse_file,
};

#[test]
fn reconstructs_typed_nodes_from_tree_sitter() {
    let source = b"left + right";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_macaulay2::language())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();
    let cell = tree.root_node().named_child(0).unwrap();
    let raw_expression = cell.named_child(0).unwrap();

    let expression =
        BinaryExpression::reconstruct(TreeSitterNode::new(raw_expression, source, SourceId(41)))
            .unwrap();

    assert_eq!(expression.span().source(), Ok(SourceId(41)));
    assert!(matches!(expression.left.as_ref(), Expr::Symbol(symbol) if symbol.text == "left"));
    assert!(matches!(expression.operator, BinaryOperator::Add(_)));
    assert!(matches!(expression.right.as_ref(), Expr::Symbol(symbol) if symbol.text == "right"));
}

#[test]
fn reconstructs_implicit_application_with_the_space_operator() {
    let source_file = parse_file("f x", SourceId(50)).unwrap();
    let AnyCell::ExpressionCell(cell) = &source_file.elements[0] else {
        panic!("application must be an expression cell");
    };
    let Expr::OperatorExpr(m2_syn::OperatorExpr::BinaryExpression(expression)) =
        cell.value.as_ref()
    else {
        panic!("application must be a binary operator expression");
    };
    assert!(matches!(expression.operator, BinaryOperator::Space(_)));
    assert_eq!(source_file.to_code(), "f x");
}

#[test]
fn reconstructs_a_complete_source_file() {
    let source = b"x + 1\ny + x * 2\nif y then return y else 0";
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_macaulay2::language())
        .unwrap();
    let tree = parser.parse(source, None).unwrap();

    assert!(!tree.root_node().has_error());

    let source_file =
        SourceFile::reconstruct(TreeSitterNode::new(tree.root_node(), source, SourceId(42)))
            .unwrap();

    assert_eq!(source_file.elements.len(), 3);
    assert_eq!(source_file.span().source(), Ok(SourceId(42)));
    assert_eq!(source_file.span().end_point().unwrap().byte, source.len());
}

#[test]
fn specializes_global_cells_without_treating_nested_muted_groups_as_cells() {
    let source_file = parse_file("{x; y}\nx;", SourceId(45)).unwrap();

    let AnyCell::ExpressionCell(expression_cell) = &source_file.elements[0] else {
        panic!("a global expression must reconstruct as ExpressionCell");
    };

    let Expr::Collection(Collection::List(list)) = expression_cell.value.as_ref() else {
        panic!("the first cell must contain a list");
    };
    assert_eq!(list.muted.len(), 1);

    let AnyCell::MutedCell(_) = &source_file.elements[1] else {
        panic!("a global semicolon-terminated expression must reconstruct as MutedCell");
    };
}

#[test]
fn bundled_parser_matches_current_assignment_and_control_nodes() {
    let assignment = parse_file("x = 1", SourceId(46)).unwrap();
    let AnyCell::ExpressionCell(cell) = &assignment.elements[0] else {
        panic!("assignment must be an expression cell");
    };
    assert!(matches!(
        cell.value.as_ref(),
        Expr::AssignmentExpr(AssignmentExpr::Assignment(_))
    ));

    let option = parse_file("key => value", SourceId(51)).unwrap();
    let AnyCell::ExpressionCell(cell) = &option.elements[0] else {
        panic!("option must be an expression cell");
    };
    assert!(matches!(cell.value.as_ref(), Expr::OptionExpression(_)));

    let for_loop = parse_file("for x in y list x", SourceId(47)).unwrap();
    assert_eq!(for_loop.elements.len(), 1);
    let AnyCell::ExpressionCell(cell) = &for_loop.elements[0] else {
        panic!("for loop must be an expression cell");
    };
    assert!(matches!(cell.value.as_ref(), Expr::ForLoop(_)));
}

#[test]
fn assignment_kind_is_specialized_by_its_left_child() {
    for (source_id, source) in [
        (SourceId(48), "(x, (y, z)) = values"),
        (SourceId(49), "((x)) = y"),
    ] {
        let file = parse_file(source, source_id).unwrap();
        let AnyCell::ExpressionCell(cell) = &file.elements[0] else {
            panic!("structured assignment must be an expression cell");
        };
        assert!(matches!(
            cell.value.as_ref(),
            Expr::AssignmentExpr(AssignmentExpr::StructuredBinding(_))
        ));
    }
}

#[test]
fn emitted_source_reparses_to_the_same_normal_form() {
    let source = "x + 1\ny + x * 2\nif y then return y else 0";
    let syntax = parse_file(source, SourceId(43)).unwrap();
    let emitted = syntax.to_code();
    let reparsed = parse_file(&emitted, SourceId(44)).unwrap();

    assert_eq!(reparsed.to_code(), emitted);
}
