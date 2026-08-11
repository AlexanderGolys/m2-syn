#![doc = include_str!("../README.md")]

extern crate self as m2_syn;

mod cst;
#[macro_use]
mod nodes;
pub mod fold;
mod print;
mod span;
mod token_stream;
pub mod visit;
pub mod visit_mut;

#[cfg(feature = "tree-sitter")]
pub mod treesitter;

#[cfg(feature = "tree-sitter")]
pub use treesitter::{ParseError, parse_file};

pub use cst::{
    AstNode, ChildCursor, ConcreteNode, CstChild, CstNode, NodeIdentity, Reconstruct,
    ReconstructError, Token, expect_concrete, matches_concrete,
};
pub use m2_syn_macros::{quote_m2, syntax};
pub use nodes::*;
pub use span::{MissingLocation, SourceId, Span, Spanned, TextPoint, TextRange};
pub use token_stream::{Delimiter, ToTokens, TokenStream, TokenTree};
