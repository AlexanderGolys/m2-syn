use std::error::Error;

use crate::{CellStream, CstNode, Reconstruct, ReconstructError, SourceFile};

/// A parser that produces one typed syntax target.
///
/// Implementations may construct `T` directly or adapt their concrete syntax
/// tree through [`CstNode`] and [`reconstruct`]. A backend can implement this
/// trait more than once for different targets.
pub trait Parse<T = SourceFile> {
    type Error: Error;

    fn parse(&mut self, tokens: CellStream) -> Result<T, Self::Error>;
}

/// Parses an existing token stream with an explicitly selected parser.
pub fn parse_with<T, P>(parser: &mut P, tokens: CellStream) -> Result<T, P::Error>
where
    P: Parse<T>,
{
    parser.parse(tokens)
}

/// Reconstructs a typed syntax target from a parser-specific CST root.
pub fn reconstruct<T, N>(root: N) -> Result<T, ReconstructError>
where
    N: CstNode,
    T: Reconstruct<N>,
{
    T::reconstruct(root)
}
