#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![warn(clippy::missing_errors_doc, clippy::missing_panics_doc)]

extern crate self as m2_syn;

// Files are grouped by the stage they belong to — `lex/`, `parser/`, `cst/`,
// `schema/` — while the module paths stay flat, so `#[path]` carries the
// grouping without renaming anything the rest of the crate refers to.

// Declared first: these macros are textually scoped, so every module below
// must come after their definitions.
#[macro_use]
#[path = "schema/macros.rs"]
mod macros;

#[path = "parser/body.rs"]
mod body;
#[path = "cst/cst.rs"]
mod cst;
#[path = "parser/delimited.rs"]
mod delimited;
#[path = "lex/lexer.rs"]
pub mod lexer;
#[path = "parser/native.rs"]
pub mod native;
#[path = "schema/nodes.rs"]
mod nodes;
#[path = "parser/parse.rs"]
pub mod parse;
mod parsed_file;
pub mod pretty;
#[path = "parser/punctuated.rs"]
pub mod punctuated;
mod recovery;
mod span;
#[path = "lex/token_stream.rs"]
mod token_stream;

#[path = "cst/treesitter.rs"]
pub mod treesitter;

pub use treesitter::{
    ParseError, RealtimeParser, TreeSitterParser, parse_file, parse_file_recovering, parse_tokens,
};

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
#[doc(hidden)]
pub use lexer::{__qualified_identifier_spans, __quote_fragment};
pub use lexer::{LexError, LexErrorKind, lex, lex_recovering, lex_str, lex_str_recovering};
#[doc(hidden)]
pub use m2_syn_macros::token as __m2_syn_token;
pub use m2_syn_macros::{Spanned, parse_quote_m2, quote_m2, syntax};
pub use native::{
    CompletedNativeCell, NativeCellParser, NativeCellState, NativeParseError, NativeParser,
    parse_native, parse_native_recovering,
};
pub use nodes::*;
pub use nodes::{fold, visit, visit_mut};
pub use parsed_file::ParsedFile;
pub use pretty::{
    PrettyNode, PrettyOptions, PrettyReport, PrettyTree, format_pretty_tokens, format_pretty_tree,
};
pub use punctuated::{PunctPair, Punctuated};
pub use recovery::Recovered;
pub use span::{File, Offset, Source, SourceId, Span, Spanned, TextEdit};
#[doc(hidden)]
pub use token_stream::delim::parse_delimiter;
pub use token_stream::delim::{
    Delimiter, DelimiterKind, DelimiterToken, DelimiterValue, DoubleSpan,
};
pub use token_stream::{
    EocToken, IdentToken, Lex, LiteralKind, LiteralToken, Punct, TokenGroup, TokenStream,
    TokenTree, Trivia, TriviaKind,
};
