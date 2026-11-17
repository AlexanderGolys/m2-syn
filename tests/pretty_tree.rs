//! Integration tests for pretty tree.

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
fn virtual_adjacency_is_presented_as_an_empty_glyph() {
    let syntax = parse_native("x y").unwrap();
    let rendered = format_pretty_tree(&syntax);

    assert!(rendered.contains("└─ ∅  Token![SPACE]"), "{rendered}");
}

#[test]
fn storage_containers_are_spliced_into_the_owning_syntax_node() {
    let syntax = parse_native("(x, y) -> x").unwrap();
    let tree = syntax.pretty_tree();
    let rendered = format_pretty_tree(&syntax);

    assert!(!contains_named(&tree, "Delimited"), "{rendered}");
    assert!(
        !contains_name_starting_with(&tree, "Punctuated"),
        "{rendered}"
    );

    let function = find_named(&tree, "ExprFunction").expect("function node");
    assert_eq!(
        child_edges(function),
        [
            ("params", "FunctionParams"),
            ("arrow_token", "Token![->]"),
            ("body", "Expr"),
        ]
    );
    let params = find_named(&tree, "ParamList").expect("parameter list node");
    assert_eq!(
        child_edges(params),
        [
            ("opening", "Token![()]"),
            ("contents", "Params"),
            ("closing", "Token![()]"),
        ]
    );
    assert!(params.children().iter().all(|edge| {
        edge.node().name() != "Token![()]" || edge.node().role() == PrettyNodeRole::Delimiter
    }));
    let pack = find_named(&tree, "ParamsPack").expect("two parameters form a pack");
    assert_eq!(
        child_edges(pack),
        [
            ("[0]", "Symbol"),
            ("punctuation", "Token![,]"),
            ("[1]", "Symbol"),
        ]
    );
}

#[test]
fn every_expression_is_one_category_level_above_its_syntax() {
    let syntax = parse_native("ZZ ZZ := ZZ => (x, y) -> {x y;3}").unwrap();
    let tree = syntax.pretty_tree();
    let rendered = format_pretty_tree(&syntax);

    let chains: &[(&[&str], &str)] = &[
        (&["Elements", "Expr"], "ExprAssign"),
        (&["Expr"], "ExprList"),
        (&["Elements", "Expr"], "ExprBinary"),
        (&["Elements", "Expr", "Literal"], "IntegerLiteral"),
    ];
    for &(categories, concrete) in chains {
        assert!(
            contains_specialization_chain(&tree, categories, concrete),
            "missing category path {categories:?} to {concrete}:\n{rendered}"
        );
    }
    let list = find_named(&tree, "ExprList").expect("list node");
    let statement = list
        .children()
        .iter()
        .find(|edge| edge.node().name() == "Stmt")
        .expect("a statement is contained by the list");
    assert_eq!(statement.node().role(), PrettyNodeRole::Syntax);
    assert!(
        statement.node().children().iter().any(|edge| {
            edge.label() == "elems"
                && edge.node().name() == "Elements"
                && edge.node().role() == PrettyNodeRole::Category
        }),
        "a statement's elements are composition, not a specialization:\n{rendered}"
    );
    assert_eq!(list.children().first().unwrap().label(), "opening");
    assert_eq!(list.children().last().unwrap().label(), "closing");
}

#[test]
fn single_cell_expression_has_no_expression_pack_node() {
    let syntax = parse_native("ZZ ZZ := ZZ => (x, y) -> x y").unwrap();
    let tree = syntax.pretty_tree();
    let rendered = format_pretty_tree(&syntax);

    assert!(!contains_named(&tree, "ElementsPack"), "{rendered}");
    let method = find_named(&tree, "ExprBinary").expect("method and signature");
    let operator = method
        .children()
        .iter()
        .find(|edge| edge.label() == "op")
        .expect("a binary operation has an operator");
    assert!(operator.node().children().iter().any(|edge| {
        edge.node().name() == "Token![SPACE]"
            && edge
                .node()
                .detail()
                .is_some_and(|detail| detail.text() == "∅")
    }));
}

#[test]
fn pack_nodes_require_a_comma() {
    let single_params = parse_native("(x) -> x").unwrap().pretty_tree();
    assert!(contains_named(&single_params, "ParamList"));
    assert!(!contains_named(&single_params, "ParamsPack"));

    let packed_params = parse_native("(x, y) -> x").unwrap().pretty_tree();
    assert!(contains_named(&packed_params, "ParamsPack"));

    let single_target = parse_native("(x) = y").unwrap().pretty_tree();
    assert!(contains_named(&single_target, "ExprParen"));
    assert!(!contains_named(&single_target, "ElementsPack"));

    let packed_targets = parse_native("(x, y) = y").unwrap().pretty_tree();
    assert!(contains_named(&packed_targets, "ElementsPack"));
}

fn child_edges(node: &PrettyNode) -> Vec<(&str, &str)> {
    node.children()
        .iter()
        .map(|edge| (edge.label(), edge.node().name()))
        .collect()
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
        && node.children().iter().any(|edge| {
            if remaining.is_empty() {
                edge.node().name() == concrete && edge.node().role() != PrettyNodeRole::Category
            } else {
                contains_specialization_chain(edge.node(), remaining, concrete)
            }
        })
    {
        return true;
    }
    node.children()
        .iter()
        .any(|edge| contains_specialization_chain(edge.node(), categories, concrete))
}
