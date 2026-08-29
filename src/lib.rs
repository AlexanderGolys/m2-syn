#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![warn(clippy::missing_errors_doc, clippy::missing_panics_doc)]

extern crate self as m2_syn;

// Files are grouped by the stage they belong to — `lex/`, `parser/`,
// `migration/`, `schema/` — while the module paths stay flat, so `#[path]` carries the
// grouping without renaming anything the rest of the crate refers to.

// Declared first: these macros are textually scoped, so every module below
// must come after their definitions.
#[macro_use]
#[path = "schema/macros.rs"]
mod macros;

#[path = "parser/body.rs"]
mod body;
#[path = "lex/cell_stream.rs"]
mod cell_stream;
#[path = "migration/cst.rs"]
mod cst;
#[path = "parser/delimited.rs"]
mod delimited;
#[path = "lex/lexer.rs"]
pub mod lexer;
// `expand!` reads this file during macro expansion; keep Cargo's dependency
// tracking aware of the out-of-line source as well.
const _: &str = include_str!("schema/nodes.rs");

these_macros_should_be_illegal::expand!(
    crate::__m2_syn_syntax;
    /// Generated typed Macaulay2 syntax nodes and literal-token types.
    #[macro_use]
    #[path = "schema/nodes.rs"]
    pub mod nodes;
);
mod expansion;
#[path = "parser/native.rs"]
pub mod native;
#[path = "parser/parse.rs"]
pub mod parse;
mod parsed_file;
#[cfg(feature = "pretty")]
pub mod pretty;
#[path = "parser/punctuated.rs"]
pub mod punctuated;
#[path = "parser/realtime.rs"]
mod realtime;
mod recovery;
/// Internal support for span.
mod span;
#[path = "lex/token_stream.rs"]
/// Internal support for token stream.
mod token_stream;

#[cfg(feature = "tree-sitter")]
#[path = "migration/treesitter.rs"]
pub mod treesitter;

#[cfg(feature = "tree-sitter")]
pub use treesitter::{
    ParseError, RealtimeParser, TreeSitterParser, parse_file, parse_file_recovering, parse_tokens,
};

pub use parse::{
    FragmentParseError, Parse, ParseStream, Parser, TokenParseError, migrate, parse_fragment_str,
    parse_with, parse2,
};

pub use body::{Body, Terminated};
pub use cell_stream::{
    CellStream, CellStreamError, CellTokenGroup, CellTokenStream, CellTokenTree, Tokenize,
};
pub use cst::{
    CSTNodeClassLabel, ChildCursor, ExternalCstChild, ExternalCstNode, Migrate, MigrateError, Token,
};
pub use delimited::Delimited;
pub use expansion::ProcMacro;
#[cfg(feature = "walk")]
pub use expansion::{LiteralReturnTypes, LiteralReturnTypesError};
#[doc(hidden)]
pub use lexer::{__qualified_identifier_spans, __quote_fragment};
pub use lexer::{LexError, LexErrorKind, lex, lex_recovering, lex_str, lex_str_recovering};
#[cfg(not(any(feature = "pretty", feature = "walk")))]
#[doc(hidden)]
pub use m2_syn_macros::syntax as __m2_syn_syntax;
#[cfg(all(feature = "pretty", not(feature = "walk")))]
#[doc(hidden)]
pub use m2_syn_macros::syntax_pretty as __m2_syn_syntax;
#[cfg(all(feature = "pretty", feature = "walk"))]
#[doc(hidden)]
pub use m2_syn_macros::syntax_pretty_walk as __m2_syn_syntax;
#[cfg(all(not(feature = "pretty"), feature = "walk"))]
#[doc(hidden)]
pub use m2_syn_macros::syntax_walk as __m2_syn_syntax;
#[doc(hidden)]
pub use m2_syn_macros::token as __m2_syn_token;
pub use m2_syn_macros::{Spanned, parse_quote_m2, quote_m2};
pub use native::{
    CompletedNativeCell, NativeCellParser, NativeCellState, NativeParseError, NativeParser,
    parse_native, parse_native_recovering,
};
pub(crate) use nodes::*;
#[cfg(feature = "walk")]
pub use nodes::{fold, visit, visit_mut};
pub use parsed_file::ParsedFile;
#[cfg(feature = "pretty")]
pub use pretty::{
    PrettyDetail, PrettyDetailRole, PrettyEdge, PrettyNode, PrettyNodeRole, PrettyOptions,
    PrettyReport, PrettyTree, format_pretty_cell_tokens, format_pretty_tokens, format_pretty_tree,
};
pub use punctuated::{Punctuated, SeparatedPair};
pub use realtime::{
    NativeCellBoundary, NativeRealtimeCell, NativeRealtimeEditError, NativeRealtimeParser,
};
pub use recovery::Recovered;
pub use span::{
    File, LineColumn, LineColumnRange, Offset, Source, SourceId, Span, Spanned, TextEdit,
};
pub use these_macros_should_be_illegal::qf;
#[doc(hidden)]
pub use token_stream::delim::parse_delimiter;
pub use token_stream::delim::{
    Delimiter, DelimiterKind, DelimiterToken, DelimiterValue, DoubleSpan,
};
pub use token_stream::{
    CommentTrivia, IdentToken, LeadingTrivia, Lex, LiteralKind, LiteralToken, Punct, Terminal,
    TokenGroup, TokenStream, TokenTree, TrailingTrivia, Trivia, TriviaKind,
};
