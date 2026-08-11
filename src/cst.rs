use std::borrow::Cow;
use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::{Span, Spanned};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeIdentity<'syntax> {
    pub name: Cow<'syntax, str>,
    pub named: bool,
}

impl<'syntax> NodeIdentity<'syntax> {
    pub fn new(name: impl Into<Cow<'syntax, str>>, named: bool) -> Self {
        Self {
            name: name.into(),
            named,
        }
    }

    pub fn matches(&self, name: &str, named: bool) -> bool {
        self.named == named && self.name == name
    }
}

pub struct CstChild<N> {
    pub field: Option<&'static str>,
    pub node: N,
}

pub trait CstNode: Sized {
    type Children<'syntax>: Iterator<Item = CstChild<Self>>
    where
        Self: 'syntax;

    fn identity(&self) -> NodeIdentity<'_>;
    fn children(&self) -> Self::Children<'_>;
    fn text(&self) -> Cow<'_, str>;
    fn span(&self) -> Span;

    fn is_extra(&self) -> bool {
        false
    }
}

pub trait ConcreteNode: AstNode {
    const NAME: &'static str;
    const NAMED: bool;
}

pub trait AstNode: Spanned {
    type Kind: Copy + Eq;

    fn kind(&self) -> Self::Kind;
}

pub trait Token: ConcreteNode {
    const SPELLING: &'static str;
}

pub trait Reconstruct<N>: Sized
where
    N: CstNode,
{
    fn matches(node: &N) -> bool;
    fn reconstruct(node: N) -> Result<Self, ReconstructError>;
}

pub fn matches_concrete<T, N>(node: &N) -> bool
where
    T: ConcreteNode,
    N: CstNode,
{
    node.identity().matches(T::NAME, T::NAMED)
}

pub fn expect_concrete<T, N>(node: &N) -> Result<(), ReconstructError>
where
    T: ConcreteNode,
    N: CstNode,
{
    if matches_concrete::<T, N>(node) {
        Ok(())
    } else {
        Err(ReconstructError::wrong_node(
            T::NAME,
            T::NAMED,
            node.identity(),
        ))
    }
}

pub struct ChildCursor<N> {
    children: Vec<Option<CstChild<N>>>,
    parent: String,
}

impl<N> ChildCursor<N>
where
    N: CstNode,
{
    pub fn new(parent: &N) -> Self {
        Self {
            children: parent
                .children()
                .filter(|child| !child.node.is_extra())
                .map(Some)
                .collect(),
            parent: parent.identity().name.into_owned(),
        }
    }

    pub fn required_field(&mut self, field: &'static str) -> Result<N, ReconstructError> {
        self.optional_field(field)
            .ok_or_else(|| ReconstructError::MissingField {
                node: self.parent.clone(),
                field,
            })
    }

    pub fn optional_field(&mut self, field: &'static str) -> Option<N> {
        self.take_first(|child| child.field == Some(field))
    }

    pub fn repeated_field(&mut self, field: &'static str) -> Vec<N> {
        self.take_all(|child| child.field == Some(field))
    }

    pub fn required_matching<T>(&mut self) -> Result<N, ReconstructError>
    where
        T: Reconstruct<N>,
    {
        self.optional_matching::<T>()
            .ok_or_else(|| ReconstructError::MissingChild {
                node: self.parent.clone(),
                expected: std::any::type_name::<T>(),
            })
    }

    pub fn optional_matching<T>(&mut self) -> Option<N>
    where
        T: Reconstruct<N>,
    {
        self.take_first(|child| child.field.is_none() && T::matches(&child.node))
    }

    pub fn repeated_matching<T>(&mut self) -> Vec<N>
    where
        T: Reconstruct<N>,
    {
        self.take_all(|child| child.field.is_none() && T::matches(&child.node))
    }

    fn take_first(&mut self, predicate: impl Fn(&CstChild<N>) -> bool) -> Option<N> {
        self.children
            .iter_mut()
            .find(|child| child.as_ref().is_some_and(&predicate))
            .and_then(Option::take)
            .map(|child| child.node)
    }

    fn take_all(&mut self, predicate: impl Fn(&CstChild<N>) -> bool) -> Vec<N> {
        self.children
            .iter_mut()
            .filter_map(|child| {
                child
                    .as_ref()
                    .is_some_and(&predicate)
                    .then(|| child.take().expect("matched child").node)
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconstructError {
    WrongNode {
        expected: &'static str,
        expected_named: bool,
        actual: String,
        actual_named: bool,
    },
    WrongCategory {
        category: &'static str,
        actual: String,
        actual_named: bool,
    },
    MissingField {
        node: String,
        field: &'static str,
    },
    MissingChild {
        node: String,
        expected: &'static str,
    },
}

impl ReconstructError {
    pub fn wrong_node(
        expected: &'static str,
        expected_named: bool,
        actual: NodeIdentity<'_>,
    ) -> Self {
        Self::WrongNode {
            expected,
            expected_named,
            actual: actual.name.into_owned(),
            actual_named: actual.named,
        }
    }

    pub fn wrong_category(category: &'static str, actual: NodeIdentity<'_>) -> Self {
        Self::WrongCategory {
            category,
            actual: actual.name.into_owned(),
            actual_named: actual.named,
        }
    }
}

impl Display for ReconstructError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongNode {
                expected,
                expected_named,
                actual,
                actual_named,
            } => write!(
                formatter,
                "expected {} node {expected:?}, found {} node {actual:?}",
                visibility(*expected_named),
                visibility(*actual_named),
            ),
            Self::WrongCategory {
                category,
                actual,
                actual_named,
            } => write!(
                formatter,
                "node {} {actual:?} does not belong to {category}",
                visibility(*actual_named),
            ),
            Self::MissingField { node, field } => {
                write!(formatter, "node {node:?} has no {field:?} field")
            }
            Self::MissingChild { node, expected } => {
                write!(formatter, "node {node:?} has no child matching {expected}")
            }
        }
    }
}

impl Error for ReconstructError {}

fn visibility(named: bool) -> &'static str {
    if named { "named" } else { "anonymous" }
}
