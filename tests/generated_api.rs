use m2_syn::{
    AstNode, BinaryExpression, BinaryOperator, Eql, Expr, Mul, PrefixOperator, SourceId, Span,
    Spanned, Symbol, SyntaxKind, SyntaxNode, TextPoint, TextRange, ToTokens, Token, fold::Fold,
    quote_m2, visit::Visit, visit_mut::VisitMut,
};

fn span(start: usize, end: usize) -> Span {
    Span::located(
        SourceId(7),
        TextRange::new(
            TextPoint::new(0, start as u32, start),
            TextPoint::new(0, end as u32, end),
        )
        .unwrap(),
    )
}

fn symbol(name: &str, start: usize) -> Symbol {
    Symbol::new(name, span(start, start + name.len()))
}

fn assignment() -> SyntaxNode {
    BinaryExpression::new(
        symbol("left", 0).into(),
        Eql::new(span(5, 6)).into(),
        symbol("right", 7).into(),
    )
    .into()
}

#[test]
fn constructors_hide_recursive_storage_and_preserve_public_categories() {
    let node = assignment();
    assert_eq!(node.kind(), SyntaxKind::BinaryExpression);
    assert_eq!(node.span().source(), Ok(SourceId(7)));
    assert_eq!(node.span().range().unwrap().start.byte, 0);
    assert_eq!(node.span().range().unwrap().end.byte, 12);
    assert_eq!(<m2_syn::Token![=] as Token>::SPELLING, "=");
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
    let operator: BinaryOperator = Eql::new(span(1, 2)).into();
    let syntax_from_symbol: SyntaxNode = symbol("y", 2).into();

    assert!(matches!(expression, Expr::Symbol(_)));
    assert!(matches!(operator, BinaryOperator::Eql(_)));
    assert!(matches!(
        syntax_from_symbol,
        SyntaxNode::Expr(Expr::Symbol(_))
    ));
}

#[test]
fn one_token_type_can_belong_to_multiple_operator_categories() {
    let binary: BinaryOperator = Mul::new(span(0, 1)).into();
    let prefix: PrefixOperator = Mul::new(span(0, 1)).into();

    assert!(matches!(binary, BinaryOperator::Mul(_)));
    assert!(matches!(prefix, PrefixOperator::Mul(_)));
}

#[test]
fn quote_builds_an_m2_token_stream_with_interpolation() {
    let value = symbol("value", 0);
    let tokens = quote_m2! {
        result = $(value) + 1 EOC
        return result EOF
    };

    assert_eq!(tokens.to_string(), "result=value+1\nreturn result");
    assert_eq!(tokens.to_m2(), "result=value+1\nreturn result");
}
