//! Parsing boundaries over the shared raw token representation.
//!
//! [`Parser`] consumes a whole [`CellStream`] and is the backend interface.
//! [`Parse`] consumes one value from a [`ParseStream`] and is the generated
//! dual of `ToTokens`. Both native parsing and generated token parsing use the
//! same cursor facts instead of maintaining parallel token arrays.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io::Cursor;
use std::rc::Rc;

use crate::cst::Token;
use crate::{
    CellStream, ExternalCstNode, LexError, Reconstruct, ReconstructError, SourceFile, SourceId,
    Span, Spanned, TextRange, TokenStream, TokenTree, TriviaKind, lex_str,
};

/// A parser backend that produces one typed syntax target.
///
/// Backends may construct `T` directly or adapt their concrete syntax tree
/// through [`CstNode`] and [`reconstruct`].
pub trait Parser<T = SourceFile> {
    type Error: Error;

    fn parse(&mut self, tokens: CellStream) -> Result<T, Self::Error>;
}

/// Parses an existing cell stream with an explicitly selected backend.
pub fn parse_with<T, P>(parser: &mut P, tokens: CellStream) -> Result<T, P::Error>
where
    P: Parser<T>,
{
    parser.parse(tokens)
}

/// A typed syntax value that can consume itself from an M2 token cursor.
pub trait Parse: Sized {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError>;
}

/// Parses one complete token stream as `T`.
pub fn parse2<T: Parse>(tokens: TokenStream) -> Result<T, TokenParseError> {
    let mut input = ParseStream::new(tokens);
    let value = T::parse(&mut input)?;
    input.finish()?;
    Ok(value)
}

/// Lexes one source cell and parses it as a typed syntax fragment.
///
/// This applies the grammar expressed by `T`'s [`Parse`] implementation. It
/// does not validate the fragment against the complete Macaulay2 grammar; use
/// [`crate::parse_native`] or [`crate::parse_file`] when parsing a source file.
/// A delimited fragment containing semicolons is still one cell because cell
/// boundaries are recognized only at the top level.
pub fn parse_fragment_str<T: Parse>(
    source: &str,
    source_id: SourceId,
) -> Result<T, FragmentParseError> {
    let cells = lex_str(source, source_id)?.into_cells();
    let tokens = match cells.len() {
        0 => TokenStream::new(),
        1 => cells.into_iter().next().unwrap().into_stream(),
        count => return Err(FragmentParseError::MultipleCells { count }),
    };
    parse2(tokens).map_err(Into::into)
}

/// An error produced while lexing or parsing one typed syntax fragment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FragmentParseError {
    /// The source could not be converted into token trees.
    Lex(LexError),
    /// The token trees did not match the requested fragment type.
    Parse(TokenParseError),
    /// The source contained top-level cell boundaries and therefore was not a
    /// single fragment.
    MultipleCells {
        /// The number of cells produced by the lexer.
        count: usize,
    },
}

impl Display for FragmentParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lex(error) => error.fmt(formatter),
            Self::Parse(error) => error.fmt(formatter),
            Self::MultipleCells { count } => {
                write!(formatter, "expected one source cell, found {count}")
            }
        }
    }
}

impl Error for FragmentParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Lex(error) => Some(error),
            Self::Parse(error) => Some(error),
            Self::MultipleCells { .. } => None,
        }
    }
}

impl From<LexError> for FragmentParseError {
    fn from(error: LexError) -> Self {
        Self::Lex(error)
    }
}

impl From<TokenParseError> for FragmentParseError {
    fn from(error: TokenParseError) -> Self {
        Self::Parse(error)
    }
}

/// One non-trivia token together with facts about the trivia immediately
/// before it. Parser backends share this view instead of maintaining their own
/// token vectors, cursor positions, and newline scanners.
#[derive(Debug, Clone)]
pub(crate) struct SignificantToken {
    pub(crate) token: TokenTree,
    pub(crate) crossed_newline: bool,
    pub(crate) leading_span: Span,
}

#[derive(Debug, Clone)]
pub(crate) enum Lookahead {
    Token(SignificantToken),
    End(Span),
}

impl Lookahead {
    pub(crate) fn span(&self) -> Span {
        match self {
            Self::Token(token) => token.token.span(),
            Self::End(span) => *span,
        }
    }
}

/// An owned, cheaply forkable cursor over one immutable token stream.
///
/// Forks share the token storage through [`Rc`] and copy only their current
/// position. Advancing a fork never changes the cursor from which it came.
#[derive(Clone)]
pub struct ParseStream {
    cursor: Cursor<Rc<TokenStream>>,
    eof_span: Span,
}

impl ParseStream {
    /// Creates a cursor at the beginning of `tokens`.
    pub fn new(tokens: TokenStream) -> Self {
        let eof_span = tokens
            .last()
            .and_then(|token| {
                let source = token.span().source().ok()?;
                let end = token.span().end_point().ok()?;
                Some(Span::new(source, TextRange::from_point(end)))
            })
            .unwrap_or_else(Span::detached);
        Self {
            cursor: Cursor::new(Rc::new(tokens)),
            eof_span,
        }
    }

    /// Creates an independent cursor over the same immutable token storage.
    pub fn fork(&self) -> Self {
        self.clone()
    }

    /// Commits this cursor to a position reached by one of its forks.
    ///
    /// Panics if `fork` belongs to another token stream.
    pub fn advance_to(&mut self, fork: &Self) {
        assert!(
            Rc::ptr_eq(self.cursor.get_ref(), fork.cursor.get_ref()),
            "cannot advance to a cursor from another token stream"
        );
        self.cursor.set_position(fork.cursor.position());
    }

    /// Returns the next non-trivia token without consuming it.
    pub fn peek(&self) -> Option<&TokenTree> {
        self.tokens().get(self.significant_position())
    }

    pub(crate) fn lookahead(&self) -> Lookahead {
        let mut position = self.position();
        let mut crossed_newline = false;
        let mut leading_span = Span::detached();
        while let Some(TokenTree::Trivia(trivia)) = self.tokens().get(position) {
            crossed_newline |= trivia.contains_line_break();
            leading_span = leading_span.join(trivia.span());
            position += 1;
        }
        let Some(token) = self.tokens().get(position).cloned() else {
            return Lookahead::End(self.eof_span);
        };
        if leading_span == Span::detached() {
            leading_span = token
                .span()
                .source()
                .ok()
                .zip(token.span().start_point().ok())
                .map(|(source, point)| Span::new(source, TextRange::from_point(point)))
                .unwrap_or_else(Span::detached);
        }
        Lookahead::Token(SignificantToken {
            token,
            crossed_newline,
            leading_span,
        })
    }

    pub(crate) fn consume_lookahead(&mut self) -> Lookahead {
        let lookahead = self.lookahead();
        if matches!(lookahead, Lookahead::Token(_)) {
            self.set_position(self.significant_position() + 1);
        }
        lookahead
    }

    pub(crate) fn skip_trivia(&mut self) {
        self.set_position(self.significant_position());
    }

    pub(crate) fn eof_span(&self) -> Span {
        self.eof_span
    }

    /// Consumes and returns the next non-trivia token.
    pub fn next_token(&mut self) -> Option<TokenTree> {
        self.set_position(self.significant_position());
        self.next_raw_token()
    }

    /// Consumes the next token, including trivia.
    pub fn next_raw_token(&mut self) -> Option<TokenTree> {
        let token = self.tokens().get(self.position())?.clone();
        self.cursor.set_position(self.cursor.position() + 1);
        Some(token)
    }

    /// Parses one generated typed token from its matching raw atom.
    pub fn parse_token<T: Token>(&mut self) -> Result<T, TokenParseError> {
        let token = self.next_raw_token();
        let Some(token) = token else {
            return Err(TokenParseError::UnexpectedEnd {
                expected: T::SPELLING,
                span: self.eof_span,
            });
        };
        let span = token.span();
        let found = token_description(&token);
        T::from_token_tree(token).ok_or(TokenParseError::UnexpectedToken {
            expected: T::SPELLING,
            found,
            span,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.peek().is_none()
    }

    fn significant_position(&self) -> usize {
        let mut position = self.position();
        while matches!(self.tokens().get(position), Some(TokenTree::Trivia(_))) {
            position += 1;
        }
        position
    }

    fn tokens(&self) -> &TokenStream {
        self.cursor.get_ref()
    }

    fn position(&self) -> usize {
        self.cursor.position() as usize
    }

    fn set_position(&mut self, position: usize) {
        self.cursor.set_position(position as u64);
    }

    fn finish(&mut self) -> Result<(), TokenParseError> {
        let Some(token) = self.next_token() else {
            return Ok(());
        };
        Err(TokenParseError::TrailingToken {
            found: token_description(&token),
            span: token.span(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenParseError {
    UnexpectedEnd {
        expected: &'static str,
        span: Span,
    },
    UnexpectedToken {
        expected: &'static str,
        found: String,
        span: Span,
    },
    TrailingToken {
        found: String,
        span: Span,
    },
}

impl Display for TokenParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnexpectedEnd { expected, .. } => {
                write!(formatter, "expected `{expected}`, found the end of input")
            }
            Self::UnexpectedToken {
                expected, found, ..
            } => write!(formatter, "expected `{expected}`, found `{found}`"),
            Self::TrailingToken { found, .. } => {
                write!(formatter, "expected the end of input, found `{found}`")
            }
        }
    }
}

impl Error for TokenParseError {}

impl Spanned for TokenParseError {
    fn span(&self) -> Span {
        match self {
            Self::UnexpectedEnd { span, .. }
            | Self::UnexpectedToken { span, .. }
            | Self::TrailingToken { span, .. } => *span,
        }
    }
}

fn token_description(token: &TokenTree) -> String {
    match token {
        TokenTree::Group(group) => group.delim_kind().to_string(),
        TokenTree::Trivia(trivia) => match trivia.kind() {
            TriviaKind::Whitespace => "whitespace".into(),
            TriviaKind::CarriageReturn => "carriage return".into(),
            TriviaKind::LineBreak => "line break".into(),
            TriviaKind::LineComment => "line comment".into(),
            TriviaKind::BlockComment => "block comment".into(),
        },
        token => token.spelling().unwrap_or("").to_owned(),
    }
}

/// Reconstructs a typed syntax target from a parser-specific CST root.
pub fn reconstruct<T, N>(root: N) -> Result<T, ReconstructError>
where
    N: ExternalCstNode,
    T: Reconstruct<N>,
{
    T::reconstruct(root)
}
