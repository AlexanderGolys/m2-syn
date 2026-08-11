use std::borrow::Cow;

use m2_syn::{CstChild, CstNode, NodeIdentity, Reconstruct, Span, syntax};

syntax! {
    tokens {
        Marker [marker]
    }

    pub struct Left;
    pub struct Right;

    pub struct Pair(pub Left, pub Right);
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

impl CstNode for Node {
    type Children<'syntax>
        = std::vec::IntoIter<CstChild<Self>>
    where
        Self: 'syntax;

    fn identity(&self) -> NodeIdentity<'_> {
        NodeIdentity::new(self.name, true)
    }

    fn children(&self) -> Self::Children<'_> {
        self.children
            .iter()
            .cloned()
            .map(|node| CstChild { field: None, node })
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn text(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.name)
    }

    fn span(&self) -> Span {
        Span::detached()
    }
}

#[test]
fn tuple_product_fields_default_to_unfielded_children() {
    let pair = Pair::reconstruct(Node {
        name: "pair",
        children: vec![Node::leaf("left"), Node::leaf("right")],
    })
    .unwrap();

    assert_eq!(pair.0.text, "left");
    assert_eq!(pair.1.text, "right");
}
