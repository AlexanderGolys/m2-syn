use std::error::Error;

use crate::{CstNode, Reconstruct, ReconstructError, SourceFile, SourceId};

/// Source text and provenance supplied to a parser implementation.
#[derive(Debug, Clone, Copy)]
pub struct ParseInput<'source> {
    pub source: &'source str,
    pub source_id: SourceId,
}

impl<'source> ParseInput<'source> {
    pub fn new(source: &'source str, source_id: SourceId) -> Self {
        Self { source, source_id }
    }
}

/// A parser that produces one typed syntax target.
///
/// Implementations may construct `T` directly or adapt their concrete syntax
/// tree through [`CstNode`] and [`reconstruct`]. A backend can implement this
/// trait more than once for different targets.
pub trait Parser<T = SourceFile> {
    type Error: Error;

    fn parse(&mut self, input: ParseInput<'_>) -> Result<T, Self::Error>;
}

/// Parses source with an explicitly selected parser implementation.
pub fn parse_with<T, P>(parser: &mut P, source: &str, source_id: SourceId) -> Result<T, P::Error>
where
    P: Parser<T>,
{
    parser.parse(ParseInput::new(source, source_id))
}

/// Reconstructs a typed syntax target from a parser-specific CST root.
pub fn reconstruct<T, N>(root: N) -> Result<T, ReconstructError>
where
    N: CstNode,
    T: Reconstruct<N>,
{
    T::reconstruct(root)
}
