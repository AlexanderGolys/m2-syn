#![doc = include_str!("../README.md")]

extern crate self as m2_syn;

mod cst;
#[macro_use]
mod nodes;
pub mod fold;
pub mod parse;
mod span;
mod token_stream;
pub mod visit;
pub mod visit_mut;

pub mod treesitter;

pub use treesitter::{ParseError, TreeSitterParser, parse_file, parse_tokens};

pub use parse::{ParseInput, Parser, parse_with, reconstruct};

pub use cst::{
    AstNode, ChildCursor, CstChild, CstNode, NodeIdentity, Reconstruct, ReconstructError, Token,
};
pub use m2_syn_macros::{quote_m2, syntax};
pub use nodes::*;
pub use span::{MissingLocation, SourceId, Span, Spanned, TextPoint, TextRange};
pub use token_stream::delim::{Delimiter, DelimiterKind, DoubleSpan};
pub use token_stream::{ToTokens, TokenStream, TokenTree};
