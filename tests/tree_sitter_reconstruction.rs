#![cfg(feature = "tree-sitter")]

use m2_syn::treesitter::TreeSitterNode;
use m2_syn::{
    AstNode, BinaryExpression, BinaryOperator, Expr, Reconstruct, SourceFile, SourceId, Spanned,
    SyntaxKind, ToTokens, parse_file,
};

#[test]
fn reconstructs_typed_nodes_from_tree_sitter() {
    let source = b"left = right";
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

    assert_eq!(expression.kind(), SyntaxKind::BinaryExpression);
    assert_eq!(expression.span().source(), Ok(SourceId(41)));
    assert!(matches!(expression.left.as_ref(), Expr::Symbol(symbol) if symbol.text == "left"));
    assert!(matches!(expression.operator, BinaryOperator::Eql(_)));
    assert!(matches!(expression.right.as_ref(), Expr::Symbol(symbol) if symbol.text == "right"));
}

#[test]
fn reconstructs_a_complete_source_file() {
    let source = b"x = 1\ny := x + 2\nif y then return y else 0";
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
    assert_eq!(source_file.span().range().unwrap().end.byte, source.len());
}

#[test]
fn emitted_source_reparses_to_the_same_normal_form() {
    let source = "x = 1\ny := x + 2\nif y then return y else 0";
    let syntax = parse_file(source, SourceId(43)).unwrap();
    let emitted = syntax.to_m2();
    let reparsed = parse_file(&emitted, SourceId(44)).unwrap();

    assert_eq!(reparsed.to_m2(), emitted);
}
