#![cfg(feature = "pretty")]

use m2_syn::{PrettyDetailRole, PrettyNode, PrettyNodeRole, PrettyTree, parse_native};

fn render_structure(node: &PrettyNode, depth: usize, output: &mut String) {
    output.push_str(&"  ".repeat(depth));
    output.push_str(node.name());
    if let Some(detail) = node.detail() {
        output.push_str(": ");
        output.push_str(detail.text());
    }
    output.push('\n');

    for edge in node.children() {
        output.push_str(&"  ".repeat(depth + 1));
        output.push_str(edge.label());
        output.push_str(" ->\n");
        render_structure(edge.node(), depth + 2, output);
    }
}

#[test]
fn presentation_tree_can_be_rendered_by_a_downstream_crate() {
    let syntax = parse_native("value").unwrap();
    let tree = syntax.pretty_tree();
    let mut rendered = String::new();
    render_structure(&tree, 0, &mut rendered);

    assert_eq!(tree.role(), PrettyNodeRole::Syntax);
    assert_eq!(tree.span().and_then(|span| span.text()), Some("value"));
    assert!(!tree.children().is_empty());
    assert!(rendered.starts_with("SourceFile\n"));
    assert!(rendered.contains("Symbol: value\n"));

    let symbol = tree
        .children()
        .iter()
        .map(|edge| edge.node())
        .find_map(find_symbol)
        .expect("the parsed symbol should be represented by a token node");
    assert_eq!(symbol.role(), PrettyNodeRole::Token);
    assert_eq!(symbol.detail().unwrap().role(), PrettyDetailRole::Text);
}

#[test]
fn presentation_tree_exposes_structural_comment_ownership() {
    let syntax = parse_native("-- documentation\nvalue").unwrap();
    let tree = syntax.pretty_tree();
    let cell = tree
        .children()
        .iter()
        .find(|edge| edge.label() == "cells")
        .and_then(|edge| edge.node().children().first())
        .map(|edge| edge.node())
        .expect("the source presentation must contain its cell");
    let comments = cell
        .children()
        .iter()
        .find(|edge| edge.label() == "comments")
        .map(|edge| edge.node())
        .expect("the cell presentation must expose comment ownership");

    assert!(comments.children().iter().any(|edge| {
        edge.label() == "outer[0]"
            && edge.node().name() == "LineCommentBlock"
            && edge.node().detail().unwrap().text() == "-- documentation"
    }));
}

fn find_symbol(node: &PrettyNode) -> Option<&PrettyNode> {
    if node.detail().is_some_and(|detail| detail.text() == "value") {
        return Some(node);
    }
    node.children()
        .iter()
        .find_map(|edge| find_symbol(edge.node()))
}
