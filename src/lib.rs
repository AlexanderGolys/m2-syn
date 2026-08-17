#![doc = include_str!("../README.md")]

extern crate self as m2_syn;

mod cst;
#[macro_use]
mod nodes;
mod body;
mod delimited;
pub mod fold;
pub mod lexer;
pub mod native;
pub mod parse;
mod parsed_file;
pub mod punct;
mod span;
mod token_stream;
pub mod visit;
pub mod visit_mut;

pub mod treesitter;

pub use treesitter::{ParseError, TreeSitterParser, parse_file, parse_tokens};

pub use parse::{
    FragmentParseError, Parse, ParseStream, Parser, TokenParseError, parse_fragment_str,
    parse_with, parse2, reconstruct,
};

pub use body::{Body, Terminated};
pub use cst::{
    CSTNodeClassLabel, ChildCursor, ExternalCstChild, ExternalCstNode, Reconstruct,
    ReconstructError, Token,
};
pub use delimited::Delimited;
pub use lexer::{LexError, LexErrorKind, lex, lex_str};
pub use m2_syn_macros::{Spanned, parse_quote_m2, quote_m2, syntax};
pub use native::{NativeParseError, NativeParser, parse_native};
pub use nodes::*;
pub use parsed_file::ParsedFile;
pub use punct::Punctuated;
pub use span::{MissingData, SourceId, Span, Spanned, TextPoint, TextRange};
pub use token_stream::delim::{Delimiter, DelimiterKind, DelimiterToken, DoubleSpan};
pub use token_stream::{
    CellBlock, CellStream, Group, IdentToken, Literal, LiteralKind, Punct, ToCellStream, ToTokens,
    TokenStream, TokenTree, Trivia, TriviaKind,
};
