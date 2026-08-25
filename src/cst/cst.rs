//! Typed syntax traits and the reconstruction bridge from external CSTs.
//!
//! [`Token`] and generated node fields are the typed graph.
//! [`ExternalCstNode`] and [`Reconstruct`] are an adapter-only seam for parsers such as
//! Tree-sitter that first produce an untyped concrete tree. They are not a
//! traversal interface for the typed graph; use `Visit`, `VisitMut`, or `Fold`
//! for that. A parser that can construct typed nodes directly does not need
//! this bridge.

use std::borrow::Cow;
use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::{Delimiter, DelimiterKind, DoubleSpan, Spanned, TokenTree};

type PunctuatedNodes<N> = (N, Vec<(N, N)>);

#[derive(Debug, Clone, PartialEq, Eq)]
/// Backend-local node identity used only during external CST reconstruction.
pub struct CSTNodeClassLabel<'syntax> {
    /// The backend-specific node kind.
    pub name: Cow<'syntax, str>,
    /// Whether the backend exposes the node as named syntax.
    pub named: bool,
}

impl<'syntax> CSTNodeClassLabel<'syntax> {
    /// Creates a backend node identity.
    pub fn new(name: impl Into<Cow<'syntax, str>>, named: bool) -> Self {
        Self {
            name: name.into(),
            named,
        }
    }

    /// Reports whether both the kind and namedness match.
    pub fn matches(&self, name: &str, named: bool) -> bool {
        self.named == named && self.name == name
    }
}

/// One child supplied by an external CST backend.
pub struct ExternalCstChild<N> {
    /// The grammar field name, when the child occupies a named field.
    pub field: Option<&'static str>,
    /// The backend-local child node.
    pub node: N,
}

/// Minimal untyped view required to reconstruct generated typed nodes.
///
/// This trait is deliberately confined to parser adapters. Its homogeneous
/// child iterator must not be used to model or walk the typed syntax graph.
pub trait ExternalCstNode: Sized + Spanned {
    /// The iterator returned by [`Self::children`].
    type Children<'syntax>: Iterator<Item = ExternalCstChild<Self>>
    where
        Self: 'syntax;

    /// Returns the node kind used by reconstruction matching.
    fn identity(&self) -> CSTNodeClassLabel<'_>;
    /// Returns the node's immediate children in source order.
    fn children(&self) -> Self::Children<'_>;
    /// Returns the exact source text covered by the node.
    fn text(&self) -> Cow<'_, str>;
    /// Reports whether this is parser-added extra syntax such as a comment.
    fn is_extra(&self) -> bool {
        false
    }
}

/// A generated closed token atom backed by one matching raw [`TokenTree`].
///
/// Parsing validates the raw category and spelling, then retains the raw atom
/// so emission preserves its source text and span.
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
    /// Reports whether `node` can reconstruct this typed value.
    fn matches(node: &N) -> bool;
    /// Converts a matching external node into typed syntax.
    ///
    /// # Errors
    ///
    /// Returns a [`ReconstructError`] when the node or its children do not
    /// match the requested typed syntax.
    fn reconstruct(node: N) -> Result<Self, ReconstructError>;
}

/// Single-pass field selection used only by generated reconstruction code.
pub struct ChildCursor<N> {
    children: Vec<Option<ExternalCstChild<N>>>,
    parent: String,
}

impl<N: ExternalCstNode> ChildCursor<N> {
    /// Collects the children of `parent` for single-pass field selection.
    pub fn new(parent: &N) -> Self {
        Self {
            children: parent.children().map(Some).collect(),
            parent: parent.identity().name.into_owned(),
        }
    }

    /// Takes the child in required named `field`.
    ///
    /// # Errors
    ///
    /// Returns [`ReconstructError::MissingField`] when no child occupies it.
    pub fn required_field(&mut self, field: &'static str) -> Result<N, ReconstructError> {
        self.optional_field(field)
            .ok_or_else(|| ReconstructError::MissingField {
                node: self.parent.clone(),
                field,
            })
    }

    /// Takes the child in optional named `field`.
    pub fn optional_field(&mut self, field: &'static str) -> Option<N> {
        self.take_first(|child| !child.node.is_extra() && child.field == Some(field))
    }

    /// Takes every child in repeated named `field`.
    pub fn repeated_field(&mut self, field: &'static str) -> Vec<N> {
        self.take_all(|child| !child.node.is_extra() && child.field == Some(field))
    }

    /// Takes a required positional child reconstructible as `T`.
    ///
    /// # Errors
    ///
    /// Returns [`ReconstructError::MissingChild`] when no remaining positional
    /// child can reconstruct `T`.
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

    /// Takes an optional positional child reconstructible as `T`.
    pub fn optional_matching<T>(&mut self) -> Option<N>
    where
        T: Reconstruct<N>,
    {
        self.take_first(|child| {
            !child.node.is_extra() && child.field.is_none() && T::matches(&child.node)
        })
    }

    /// Takes every positional child reconstructible as `T`.
    pub fn repeated_matching<T>(&mut self) -> Vec<N>
    where
        T: Reconstruct<N>,
    {
        self.take_all(|child| {
            !child.node.is_extra() && child.field.is_none() && T::matches(&child.node)
        })
    }

    /// Takes a required anonymous delimiter or punctuation child.
    ///
    /// # Errors
    ///
    /// Returns [`ReconstructError::MissingChild`] when the child is absent.
    pub fn required_anonymous(&mut self, spelling: &'static str) -> Result<N, ReconstructError> {
        self.take_first(|child| {
            child.field.is_none() && child.node.identity().matches(spelling, false)
        })
        .ok_or_else(|| ReconstructError::MissingChild {
            node: self.parent.clone(),
            expected: spelling,
        })
    }

    /// Takes an optional anonymous delimiter or punctuation child.
    pub fn optional_anonymous(&mut self, spelling: &'static str) -> Option<N> {
        self.take_first(|child| {
            child.field.is_none() && child.node.identity().matches(spelling, false)
        })
    }

    /// Reconstructs and consumes one of the supported delimiter pairs.
    ///
    /// # Errors
    ///
    /// Returns [`ReconstructError::MissingChild`] when no opening delimiter or
    /// its matching closing delimiter remains.
    pub fn required_delimiter(&mut self) -> Result<Delimiter, ReconstructError> {
        for kind in [
            DelimiterKind::Parenthesis,
            DelimiterKind::Bracket,
            DelimiterKind::Brace,
            DelimiterKind::AngleBar,
        ] {
            let Some(opening) = self.optional_anonymous(kind.opening()) else {
                continue;
            };
            let closing = self.required_anonymous(kind.closing())?;
            return Ok(Delimiter::new(
                kind,
                DoubleSpan::new(opening.span(), closing.span()),
            ));
        }
        Err(ReconstructError::MissingChild {
            node: self.parent.clone(),
            expected: "a delimiter",
        })
    }

    /// Pairs a nonempty list of reconstructed value nodes with the anonymous
    /// punctuation nodes separating them.
    ///
    /// # Errors
    ///
    /// Returns [`ReconstructError::InvalidPunctuated`] when there is no first
    /// value or the parent does not contain exactly one separator between each
    /// adjacent value.
    pub fn punctuated(
        &mut self,
        values: Vec<N>,
        spelling: &'static str,
    ) -> Result<PunctuatedNodes<N>, ReconstructError> {
        let punctuation = self.take_all(|child| {
            child.field.is_none() && child.node.identity().matches(spelling, false)
        });
        let value_count = values.len();
        let punctuation_count = punctuation.len();
        let mut values = values.into_iter();
        let Some(first) = values.next() else {
            return Err(ReconstructError::InvalidPunctuated {
                node: self.parent.clone(),
                values: value_count,
                punctuation: punctuation_count,
            });
        };
        if punctuation_count + 1 != value_count {
            return Err(ReconstructError::InvalidPunctuated {
                node: self.parent.clone(),
                values: value_count,
                punctuation: punctuation_count,
            });
        }
        Ok((first, punctuation.into_iter().zip(values).collect()))
    }

    /// Pairs an optional punctuated field without inventing a value or a
    /// separator when the field is absent.
    ///
    /// # Errors
    ///
    /// Returns [`ReconstructError::InvalidPunctuated`] when separators remain
    /// without values or their count does not match the present values.
    pub fn optional_punctuated(
        &mut self,
        values: Vec<N>,
        spelling: &'static str,
    ) -> Result<Option<PunctuatedNodes<N>>, ReconstructError> {
        if values.is_empty() {
            let punctuation = self.take_all(|child| {
                child.field.is_none() && child.node.identity().matches(spelling, false)
            });
            return if punctuation.is_empty() {
                Ok(None)
            } else {
                Err(ReconstructError::InvalidPunctuated {
                    node: self.parent.clone(),
                    values: 0,
                    punctuation: punctuation.len(),
                })
            };
        }
        self.punctuated(values, spelling).map(Some)
    }

    fn take_first(&mut self, predicate: impl Fn(&ExternalCstChild<N>) -> bool) -> Option<N> {
        self.children
            .iter_mut()
            .find_map(|child| child.take_if(|child| predicate(child)))
            .map(|child| child.node)
    }

    fn take_all(&mut self, predicate: impl Fn(&ExternalCstChild<N>) -> bool) -> Vec<N> {
        self.children
            .iter_mut()
            .filter_map(|child| {
                child
                    .take_if(|child| predicate(child))
                    .map(|child| child.node)
            })
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A typed reconstruction failure.
pub enum ReconstructError {
    /// A concrete typed node received the wrong backend node kind.
    WrongNode {
        /// The required node kind.
        expected: &'static str,
        /// Whether the required kind is named.
        expected_named: bool,
        /// The received node kind.
        actual: String,
        /// Whether the received kind is named.
        actual_named: bool,
    },
    /// No alternative in a typed category matched the backend node.
    WrongCategory {
        /// The typed category name.
        category: &'static str,
        /// The received node kind.
        actual: String,
        /// Whether the received kind is named.
        actual_named: bool,
    },
    /// A required named field was absent.
    MissingField {
        /// The parent node kind.
        node: String,
        /// The absent grammar field.
        field: &'static str,
    },
    /// A required positional or anonymous child was absent.
    MissingChild {
        /// The parent node kind.
        node: String,
        /// The expected child type or spelling.
        expected: &'static str,
    },
    /// A punctuated field was empty or did not contain one separator between
    /// every adjacent pair of values.
    InvalidPunctuated {
        /// The parent node kind.
        node: String,
        /// The number of value nodes found.
        values: usize,
        /// The number of punctuation nodes found.
        punctuation: usize,
    },
}

impl ReconstructError {
    /// Creates a concrete node-kind mismatch.
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

    /// Creates a category mismatch.
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
            Self::InvalidPunctuated {
                node,
                values,
                punctuation,
            } => write!(
                formatter,
                "node {node:?} has {values} punctuated values but {punctuation} separators",
            ),
        }
    }
}

impl Error for ReconstructError {}

fn visibility(named: bool) -> &'static str {
    if named { "named" } else { "anonymous" }
}
