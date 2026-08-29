//! Integration tests for pretty tree.

#![cfg(feature = "pretty")]

use m2_syn::{
    PrettyDetailRole, PrettyNode, PrettyNodeRole, PrettyTree, format_pretty_tree, parse_native,
};

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
        .find(|edge| edge.node().name() == "Cell")
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

#[test]
fn virtual_adjacency_is_presented_as_an_empty_glyph() {
    let syntax = parse_native("x y").unwrap();
    let rendered = format_pretty_tree(&syntax);

    assert!(rendered.contains("∅  operator → Adj"), "{rendered}");
    assert!(!rendered.contains("Adj  operator → Adj"), "{rendered}");
}

#[test]
fn storage_containers_are_spliced_into_the_owning_syntax_node() {
    let syntax = parse_native("(x, y) -> x").unwrap();
    let tree = syntax.pretty_tree();
    let rendered = format_pretty_tree(&syntax);

    assert!(find_named(&tree, "ExprLambdaList").is_some(), "{rendered}");
    assert!(!contains_named(&tree, "Delimited"), "{rendered}");
    assert!(
        !contains_name_starting_with(&tree, "Punctuated"),
        "{rendered}"
    );

    let arguments = find_named(&tree, "ExprLambdaList").unwrap();
    assert_eq!(
        arguments
            .children()
            .iter()
            .map(|edge| (edge.label(), edge.node().name()))
            .collect::<Vec<_>>(),
        [
            ("opening", "Delimiter![()]"),
            ("contents", "ArgumentListContent"),
            ("closing", "Delimiter![()]"),
            ("operator", "Token![->]"),
            ("body", "Expr"),
        ]
    );
    assert!(arguments.children().iter().all(|edge| {
        edge.node().name() != "Delimiter![()]" || edge.node().role() == PrettyNodeRole::Delimiter
    }));
    let pack =
        find_named(&tree, "ArgumentListContentPack").expect("two arguments form an argument pack");
    assert_eq!(
        pack.children()
            .iter()
            .map(|edge| (edge.label(), edge.node().name()))
            .collect::<Vec<_>>(),
        [
            ("[0]", "Symbol"),
            ("punctuation", "Token![,]"),
            ("[1]", "Symbol"),
        ]
    );
}

#[test]
fn enum_specializations_and_struct_fields_are_distinct() {
    let syntax = parse_native("ZZ ZZ := ZZ => (x, y) -> {x y;3}").unwrap();
    let tree = syntax.pretty_tree();
    let rendered = format_pretty_tree(&syntax);

    let chains: &[(&[&str], &str)] = &[
        (
            &["PackedContent", "Expr", "ExprAssign", "ExprAssignInstall"],
            "ExprAssignInstallMethod",
        ),
        (&["Expr", "ExprCollection"], "List"),
        (
            &["PackedContent", "Expr", "ExprOp", "ExprOpBin"],
            "ExprOpBinAdj",
        ),
        (&["PackedContent", "Expr", "ExprLiteral"], "IntegerLiteral"),
    ];
    for &(categories, concrete) in chains {
        assert!(
            contains_specialization_chain(&tree, categories, concrete),
            "missing category path {categories:?} to {concrete}:\n{rendered}"
        );
    }
    assert!(
        find_named(&tree, "ExprCollection")
            .is_some_and(|node| node.role() == PrettyNodeRole::Category),
        "{rendered}"
    );
    let list = find_named(&tree, "List").expect("list node");
    let statement = list
        .children()
        .iter()
        .find(|edge| edge.node().name() == "Statement")
        .expect("a statement is contained by the list");
    assert_eq!(statement.node().role(), PrettyNodeRole::Syntax);
    assert!(
        statement.node().children().iter().any(|edge| {
            edge.label() == "content"
                && edge.node().name() == "PackedContent"
                && edge.node().role() == PrettyNodeRole::Category
        }),
        "a statement's content is composition, not a specialization:\n{rendered}"
    );
    assert_eq!(list.children().first().unwrap().label(), "opening");
    assert_eq!(list.children().last().unwrap().label(), "closing");
}

#[test]
fn single_cell_expression_has_no_expression_pack_node() {
    let syntax = parse_native("ZZ ZZ := ZZ => (x, y) -> x y").unwrap();
    let tree = syntax.pretty_tree();
    let rendered = format_pretty_tree(&syntax);

    assert!(!contains_named(&tree, "ExprPack"), "{rendered}");
    let installation = find_named(&tree, "ExprAssignInstallMethod").expect("method installation");
    assert!(installation.children().iter().any(|edge| {
        edge.label() == "operator"
            && edge.node().name() == "Adj"
            && edge
                .node()
                .detail()
                .is_some_and(|detail| detail.text() == "∅")
    }));
}

#[test]
fn pack_nodes_require_a_comma() {
    let single_arguments = parse_native("(x) -> x").unwrap().pretty_tree();
    assert!(contains_named(&single_arguments, "ExprLambdaList"));
    assert!(!contains_named(
        &single_arguments,
        "ArgumentListContentPack"
    ));

    let packed_arguments = parse_native("(x, y) -> x").unwrap().pretty_tree();
    assert!(contains_named(&packed_arguments, "ArgumentListContentPack"));

    let single_assignment = parse_native("(x) = y").unwrap().pretty_tree();
    assert!(contains_named(&single_assignment, "AssignmentList"));
    assert!(!contains_named(
        &single_assignment,
        "AssignmentListContentPack"
    ));

    let packed_assignment = parse_native("(x, y) = y").unwrap().pretty_tree();
    assert!(contains_named(
        &packed_assignment,
        "AssignmentListContentPack"
    ));
}

fn find_symbol(node: &PrettyNode) -> Option<&PrettyNode> {
    if node.detail().is_some_and(|detail| detail.text() == "value") {
        return Some(node);
    }
    node.children()
        .iter()
        .find_map(|edge| find_symbol(edge.node()))
}

fn find_named<'tree>(node: &'tree PrettyNode, name: &str) -> Option<&'tree PrettyNode> {
    (node.name() == name).then_some(node).or_else(|| {
        node.children()
            .iter()
            .find_map(|edge| find_named(edge.node(), name))
    })
}

fn contains_named(node: &PrettyNode, name: &str) -> bool {
    find_named(node, name).is_some()
}

fn contains_name_starting_with(node: &PrettyNode, prefix: &str) -> bool {
    node.name().starts_with(prefix)
        || node
            .children()
            .iter()
            .any(|edge| contains_name_starting_with(edge.node(), prefix))
}

fn contains_specialization_chain(node: &PrettyNode, categories: &[&str], concrete: &str) -> bool {
    if let Some((category, remaining)) = categories.split_first()
        && node.name() == *category
        && node.role() == PrettyNodeRole::Category
    {
        return node.children().iter().any(|edge| {
            if remaining.is_empty() {
                edge.node().name() == concrete && edge.node().role() != PrettyNodeRole::Category
            } else {
                contains_specialization_chain(edge.node(), remaining, concrete)
            }
        });
    }
    node.children()
        .iter()
        .any(|edge| contains_specialization_chain(edge.node(), categories, concrete))
}
