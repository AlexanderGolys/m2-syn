use std::borrow::Cow;

use m2_syn::{CSTNodeClassLabel, ExternalCstChild, ExternalCstNode, Reconstruct, Span, syntax};

syntax! {
    tokens {}
    keywords: {}
    markers: {}
    punct: {}

    Left ::= leaf
    Right ::= leaf
    Pair ::= (Left, Right)
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

    assert_eq!(pair._pair_0.text, "left");
    assert_eq!(pair._pair_1.text, "right");
}
