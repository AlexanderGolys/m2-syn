//! Parsing boundaries over tokenized cells and their shared token representation.
//!
//! [`Parser`] consumes a whole [`CellStream`] and is the backend interface.
//! [`Parse`] consumes one value from a [`ParseStream`] and is the generated
//! dual of [`crate::Lex`]. Both native parsing and generated token parsing use the
//! same cursor facts instead of maintaining parallel token arrays.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::iter::FusedIterator;
use std::rc::Rc;

use crate::cell_stream::frame_recovering;
use crate::{
    Adj, CellStream, ExternalCstNode, IdentToken, LexError, LiteralToken, Migrate, MigrateError,
    Punct, SourceFile, Span, Spanned, Terminal, Token, TokenGroup, TokenStream, TokenTree,
    TrailingTrivia, lex_str,
};

/// A parser backend that produces one typed syntax target.
///
/// Backends may construct `T` directly or adapt their concrete syntax tree
/// through [`ExternalCstNode`] and [`migrate`].
pub trait Parser<T = SourceFile> {
    /// The backend-specific parse failure.
    type Error: Error;

    /// Parses `cells` into one typed syntax target.
    ///
    /// # Errors
    ///
    /// Returns the backend's [`Self::Error`] when it cannot produce `T`.
    fn parse_cells(&mut self, cells: CellStream) -> Result<T, Self::Error>;
}

/// Parses an existing cell stream with an explicitly selected backend.
///
/// # Errors
///
/// Returns the selected backend's error when parsing fails.
pub fn parse_with<T, P>(parser: &mut P, cells: CellStream) -> Result<T, P::Error>
where
    P: Parser<T>,
{
    parser.parse_cells(cells)
}

/// A typed syntax value that can consume itself from an M2 token cursor.
pub trait Parse: Sized {
    /// Consumes this value from `input`.
    ///
    /// # Errors
    ///
    /// Returns [`TokenParseError`] when the next tokens do not encode `Self`.
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError>;
}

impl<T: Parse> Parse for Box<T> {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        T::parse(input).map(Self::new)
    }
}

impl Parse for TokenTree {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        input
            .next_raw_token()
            .ok_or_else(|| TokenParseError::unexpected_end("a token tree", input.eof_span()))
    }
}

macro_rules! parse_raw_variant {
    ($ty:ty, $variant:ident, $expected:literal) => {
        impl Parse for $ty {
            fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
                let token = TokenTree::parse(input)?;
                match token {
                    TokenTree::$variant(value) => Ok(value),
                    token => Err(TokenParseError::unexpected_token($expected, &token)),
                }
            }
        }
    };
}

parse_raw_variant!(TokenGroup, Group, "a delimited group");
parse_raw_variant!(LiteralToken, Literal, "a literal");
parse_raw_variant!(Punct, Punct, "punctuation");
parse_raw_variant!(IdentToken, Ident, "an identifier");

impl Parse for Adj {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let Some(token) = input.next_significant() else {
            return Err(TokenParseError::unexpected_end(
                "adjacency",
                input.eof_span(),
            ));
        };
        match token.token {
            TokenTree::Punct(punctuation) if punctuation.text().is_empty() => {
                let mut adjacency = Self::new("", punctuation.expect_span());
                *adjacency.trailing_trivia_mut() = token.trailing_trivia;
                Ok(adjacency)
            }
            token => Err(TokenParseError::unexpected_token("adjacency", &token)),
        }
    }
}

impl Parse for TokenStream {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        Ok(input.by_ref().collect())
    }
}

/// Parses one complete token stream as `T`.
///
/// # Errors
///
/// Returns [`TokenParseError`] when `T` cannot be parsed or leaves trailing
/// tokens.
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
/// [`crate::parse_native`], or `parse_file` with the `tree-sitter` feature,
/// when parsing a source file.
/// A delimited fragment containing semicolons is still one cell because cell
/// boundaries are recognized only at the top level.
///
/// # Errors
///
/// Returns [`FragmentParseError`] when lexing fails, the source contains
/// multiple top-level cells, or the cell does not parse as `T`.
pub fn parse_fragment_str<T: Parse>(source: &str) -> Result<T, FragmentParseError> {
    let framed = frame_recovering(lex_str(source)?);
    let mut cells = framed.cells;
    cells.extend(framed.incomplete);
    if cells.len() > 1 {
        return Err(FragmentParseError::MultipleCells { count: cells.len() });
    }
    let tokens = cells.pop().unwrap_or_default();
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
    pub(crate) trailing_trivia: TrailingTrivia,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum Lookahead {
    Token(SignificantToken),
    End(Span),
}

impl Spanned for Lookahead {
    fn span(&self) -> Span {
        match self {
            Self::Token(token) => token.token.span(),
            Self::End(span) => span.clone(),
        }
    }
}

/// An owned, cheaply forkable cursor over one immutable token stream.
///
/// Forks share the token storage through [`Rc`] and copy only their current
/// position. Advancing a fork never changes the cursor from which it came.
#[derive(Clone)]
pub struct ParseStream {
    tokens: Rc<TokenStream>,
    position: usize,
    /// The zero-width position just past the last token.
    ///
    /// End of input is the absence of a token, which the cursor already
    /// reports. Only the position needs carrying, for diagnostics that point
    /// at where input ran out.
    eof_span: Span,
}

impl ParseStream {
    /// Creates a cursor at the beginning of `tokens`.
    pub fn new(tokens: TokenStream) -> Self {
        let eof_span = tokens
            .last()
            .and_then(|token| {
                let span = token.expect_span();
                Some(span.at(span.end()?))
            })
            .unwrap_or_else(|| Span::for_text(""));
        Self {
            tokens: Rc::new(tokens),
            position: 0,
            eof_span,
        }
    }

    /// Creates an independent cursor over the same immutable token storage.
    pub fn fork(&self) -> Self {
        self.clone()
    }

    /// Commits this cursor to a position reached by one of its forks.
    ///
    /// # Panics
    ///
    /// Panics if `fork` belongs to another token stream.
    pub fn advance_to(&mut self, fork: &Self) {
        assert!(
            Rc::ptr_eq(&self.tokens, &fork.tokens),
            "cannot advance to a cursor from another token stream"
        );
        self.position = fork.position;
    }

    /// Returns the next token without consuming it.
    pub fn peek(&self) -> Option<&TokenTree> {
        self.tokens().get(self.position)
    }

    pub(crate) fn lookahead(&self) -> Lookahead {
        let Some(mut token) = self.tokens().get(self.position).cloned() else {
            return Lookahead::End(self.eof_span.clone());
        };
        let crossed_newline = self.position.checked_sub(1).is_some_and(|left| {
            self.tokens()
                .get(left)
                .is_some_and(|left| tokens_cross_line_break(left, &token))
        });
        let trailing_trivia = std::mem::take(token.trailing_trivia_mut());
        Lookahead::Token(SignificantToken {
            token,
            crossed_newline,
            trailing_trivia,
        })
    }

    pub(crate) fn consume_lookahead(&mut self) -> Lookahead {
        let lookahead = self.lookahead();
        if matches!(lookahead, Lookahead::Token(_)) {
            self.position += 1;
        }
        lookahead
    }

    pub(crate) fn next_significant(&mut self) -> Option<SignificantToken> {
        match self.consume_lookahead() {
            Lookahead::Token(token) => Some(token),
            Lookahead::End(_) => None,
        }
    }

    pub(crate) fn skip_trivia(&mut self) {
        // Comments are owned by tokens, so every stream element is significant.
    }

    pub(crate) fn eof_span(&self) -> Span {
        self.eof_span.clone()
    }

    /// Consumes and returns the next non-trivia token.
    pub fn next_token(&mut self) -> Option<TokenTree> {
        self.next_raw_token()
    }

    /// Consumes the next token, including trivia.
    pub fn next_raw_token(&mut self) -> Option<TokenTree> {
        self.next()
    }

    /// Parses one generated typed token from its matching raw atom.
    ///
    /// # Errors
    ///
    /// Returns [`TokenParseError`] when input ends or the next significant token does
    /// not match `T`.
    pub fn parse_token<T: Token>(&mut self) -> Result<T, TokenParseError> {
        let Some(token) = self.next_significant() else {
            return Err(TokenParseError::unexpected_end(
                T::SPELLING,
                self.eof_span(),
            ));
        };
        let error = TokenParseError::unexpected_token(T::SPELLING, &token.token);
        T::from_token_tree_with_trivia(token.token, token.trailing_trivia).ok_or(error)
    }

    /// Returns whether no significant token remains.
    pub fn is_eof(&self) -> bool {
        self.peek().is_none()
    }

    fn tokens(&self) -> &TokenStream {
        &self.tokens
    }

    fn finish(&mut self) -> Result<(), TokenParseError> {
        if let Some(token) = self.next() {
            return Err(TokenParseError::TrailingToken {
                found: token.description(),
                span: token.expect_span(),
            });
        }
        Ok(())
    }
}

fn tokens_cross_line_break(left: &TokenTree, right: &TokenTree) -> bool {
    let left = left.expect_span();
    let right = right.expect_span();
    if !left.shares_source(&right) {
        return false;
    }
    let (Some(end), Some(start)) = (left.end(), right.start()) else {
        return false;
    };
    end <= start && left.source().text()[end..start].contains('\n')
}

impl Iterator for ParseStream {
    type Item = TokenTree;

    fn next(&mut self) -> Option<Self::Item> {
        let token = self.tokens().get(self.position)?.clone();
        self.position += 1;
        Some(token)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.tokens().len().saturating_sub(self.position);
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for ParseStream {}
impl FusedIterator for ParseStream {}

#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
/// A failure while consuming typed syntax from a [`ParseStream`].
pub enum TokenParseError {
    /// Input ended before the required syntax appeared.
    UnexpectedEnd {
        /// The expected syntax description or spelling.
        expected: &'static str,
        /// The end-of-input position.
        span: Span,
    },
    /// The next token did not match the required syntax.
    UnexpectedToken {
        /// The expected syntax description or spelling.
        expected: &'static str,
        /// A readable description of the received token.
        found: String,
        /// The received token span.
        span: Span,
    },
    /// A complete parse left a significant token unconsumed.
    TrailingToken {
        /// A readable description of the remaining token.
        found: String,
        /// The remaining token span.
        span: Span,
    },
    /// A parser backend reported a higher-level syntax failure.
    Syntax {
        /// The backend's diagnostic message.
        message: String,
        /// The syntax associated with the failure.
        span: Span,
    },
}

impl TokenParseError {
    pub(crate) fn unexpected_end(expected: &'static str, span: Span) -> Self {
        Self::UnexpectedEnd { expected, span }
    }

    pub(crate) fn unexpected(expected: &'static str, found: impl Into<String>, span: Span) -> Self {
        Self::UnexpectedToken {
            expected,
            found: found.into(),
            span,
        }
    }

    pub(crate) fn unexpected_token(expected: &'static str, token: &TokenTree) -> Self {
        Self::unexpected(expected, token.description(), token.expect_span())
    }
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
            Self::Syntax { message, .. } => formatter.write_str(message),
        }
    }
}

impl Error for TokenParseError {}

/// Migrates a typed syntax target from a parser-specific CST root.
///
/// # Errors
///
/// Returns [`MigrateError`] when the root cannot migrate `T`.
pub fn migrate<T, N>(root: N) -> Result<T, MigrateError>
where
    N: ExternalCstNode,
    T: Migrate<N>,
{
    T::migrate(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LiteralKind, quote_m2};

    #[test]
    fn parse_stream_iteration_excludes_its_eof_token() {
        let mut input = ParseStream::new(quote_m2!(name + 1));
        assert_eq!(input.len(), 3);
        let emitted = input.by_ref().collect::<TokenStream>();

        assert_eq!(emitted.to_string(), "name+1");
        assert!(input.is_eof());
        assert_eq!(input.next(), None);
        assert_eq!(input.next(), None);
    }

    #[test]
    fn exact_size_counts_every_remaining_token() {
        let mut input = ParseStream::new(quote_m2!(name));

        assert_eq!(input.len(), 1);
        assert!(input.next().is_some());
        assert_eq!(input.len(), 0);
    }

    #[test]
    fn raw_token_categories_implement_parse() {
        let identifier = parse2::<IdentToken>(quote_m2!(name)).unwrap();
        let literal = parse2::<LiteralToken>(quote_m2!(1)).unwrap();
        let punctuation = parse2::<Punct>(quote_m2!(+)).unwrap();
        let group = parse2::<TokenGroup>(quote_m2!((name))).unwrap();

        assert_eq!(identifier.text(), "name");
        assert_eq!(literal.kind, LiteralKind::Integer);
        assert_eq!(punctuation.text(), "+");
        assert_eq!(crate::Lex::to_token_stream(&group).to_string(), "(name)");
    }

    #[test]
    fn lookahead_reports_the_stored_eof_span_through_spanned() {
        let tokens = lex_str("name").unwrap();
        let mut input = ParseStream::new(tokens);
        input.next_token();

        let span = input.lookahead().expect_span();
        assert_eq!(span.start().unwrap(), "name".len());
        assert_eq!(span, input.eof_span().span());
    }
}
