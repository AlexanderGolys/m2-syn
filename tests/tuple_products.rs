use std::borrow::Cow;

use m2_syn::{
    CSTNodeClassLabel, ExternalCstChild, ExternalCstNode, Reconstruct, Span, Spanned, syntax,
};

syntax! {
    precedence: {}
    augmented: (14, 13)
    tokens {}
    keywords: {}
    markers: {}
    punct: {}

    struct Left;
    struct Right;
    struct Pair {
        left: (_) Left,
        right: (_) Right,
    }
}

#[derive(Clone)]
struct Node {
    name: &'static str,
    children: Vec<Node>,
}

impl Node {
    fn leaf(name: &'static str) -> Self {
        Self {
            name,
            children: Vec::new(),
        }
    }
}

impl ExternalCstNode for Node {
    type Children<'syntax>
        = std::vec::IntoIter<ExternalCstChild<Self>>
    where
        Self: 'syntax;

    fn identity(&self) -> CSTNodeClassLabel<'_> {
        CSTNodeClassLabel::new(self.name, true)
    }

    fn children(&self) -> Self::Children<'_> {
        self.children
            .iter()
            .cloned()
            .map(|node| ExternalCstChild { field: None, node })
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn text(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.name)
    }
}

impl Spanned for Node {
    fn span(&self) -> Span {
        Span::detached()
    }
}

#[test]
fn positional_fields_reconstruct_from_unnamed_cst_children() {
    let pair = Pair::reconstruct(Node {
        name: "pair",
        children: vec![Node::leaf("left"), Node::leaf("right")],
    })
    .unwrap();

    assert_eq!(pair.left.text, "left");
    assert_eq!(pair.right.text, "right");
}
