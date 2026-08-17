//! Typed syntax traits and the compatibility bridge from external CSTs.
//!
//! [`Token`] and generated node fields are the typed graph.
//! [`CstNode`] and [`Reconstruct`] are an adapter-only seam for parsers such as
//! Tree-sitter that first produce an untyped concrete tree. They are not a
//! traversal interface for the typed graph; use `Visit`, `VisitMut`, or `Fold`
//! for that. A parser that can construct typed nodes directly does not need
//! this bridge.

use std::borrow::Cow;
use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::{Span, Spanned, TokenTree};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Backend-local node identity used only during external CST reconstruction.
pub struct CSTNodeClassLabel<'syntax> {
    pub name: Cow<'syntax, str>,
    pub named: bool,
}

impl<'syntax> CSTNodeClassLabel<'syntax> {
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

pub struct ExternalCstChild<N> {
    pub field: Option<&'static str>,
    pub node: N,
}

/// Minimal untyped view required to reconstruct generated typed nodes.
///
/// This trait is deliberately confined to parser adapters. Its homogeneous
/// child iterator must not be used to model or walk the typed syntax graph.
pub trait ExternalCstNode: Sized {
    type Children<'syntax>: Iterator<Item = ExternalCstChild<Self>>
    where
        Self: 'syntax;

    fn identity(&self) -> CSTNodeClassLabel<'_>;
    fn children(&self) -> Self::Children<'_>;
    fn text(&self) -> Cow<'_, str>;
    fn span(&self) -> Span;

    fn is_extra(&self) -> bool {
        false
    }
}

/// A generated closed token atom backed by one matching raw [`TokenTree`].
///
/// Parsing validates the raw category and spelling, then retains the raw atom
/// so emission preserves distinctions such as implicit whitespace versus the
/// explicit `SPACE` spelling.
pub trait Token: Spanned + Sized {
    /// Canonical spelling used in diagnostics and freshly constructed atoms.
    const SPELLING: &'static str;

    /// Tests whether a raw atom belongs to this closed token type.
    fn matches_token_tree(token: &TokenTree) -> bool;

    /// Validates and retains a raw atom as this typed token.
    fn from_token_tree(token: TokenTree) -> Option<Self>;
}

/// Reconstructs one typed value from a parser backend's untyped CST node.
pub trait Reconstruct<N: ExternalCstNode>: Sized {
    fn matches(node: &N) -> bool;
    fn reconstruct(node: N) -> Result<Self, ReconstructError>;
}

/// Single-pass field selection used only by generated reconstruction code.
pub struct ChildCursor<N> {
    children: Vec<Option<ExternalCstChild<N>>>,
    parent: String,
}

impl<N: ExternalCstNode> ChildCursor<N> {
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

    fn take_first(&mut self, predicate: impl Fn(&ExternalCstChild<N>) -> bool) -> Option<N> {
        self.children
            .iter_mut()
            .find(|child| child.as_ref().is_some_and(&predicate))
            .and_then(Option::take)
            .map(|child| child.node)
    }

    fn take_all(&mut self, predicate: impl Fn(&ExternalCstChild<N>) -> bool) -> Vec<N> {
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
        actual: CSTNodeClassLabel<'_>,
    ) -> Self {
        Self::WrongNode {
            expected,
            expected_named,
            actual: actual.name.into_owned(),
            actual_named: actual.named,
        }
    }

    pub fn wrong_category(category: &'static str, actual: CSTNodeClassLabel<'_>) -> Self {
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
