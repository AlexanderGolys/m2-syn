//! Raw and typed delimiter support.
//!
//! [`Delimiter`] is stored by raw [`TokenGroup`] token trees. Generated
//! `Delimiter![..]` atoms implement [`DelimiterToken`] and are stored on typed
//! delimited nodes, so flattening can recover the corresponding raw group.

use std::fmt::{Display, Formatter};
use std::ops::Bound;

use crate::{
    Lex, Parse, ParseStream, Span, Spanned, TokenGroup, TokenParseError, TokenStream, TokenTree,
    TrailingTrivia,
};
use these_macros_should_be_illegal::discriminated_str;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The delimiter families recognized by raw syntax containers.
///
/// Every kind is spelled in the source and pairs an opening token with a
/// closing one, so a group is opaque and its extent is read rather than
/// inferred. Cell boundaries are deliberately absent: they are a parse
/// result, not a delimiter.
#[discriminated_str(expression_name)]
pub enum DelimiterKind {
    /// `( ... )`.
    Parenthesis = "parenthesized expression",
    /// `[ ... ]`.
    Bracket = "array expression",
    /// `{ ... }`.
    Brace = "list expression",
    /// `<| ... |>`.
    AngleBar = "angle-bar list expression",
}

/// Delimiter data that can surround typed syntax.
///
/// The type-erased [`struct@Delimiter`] accepts every delimiter family. Generated
/// [`DelimiterToken`] implementations refine it to one statically known family.
pub trait DelimiterValue: Spanned + Lex + Sized {
    /// Converts raw delimiter data into this representation.
    fn from_delimiter(delimiter: Delimiter) -> Option<Self>;

    /// Returns the type-erased delimiter data.
    fn delimiter(&self) -> Delimiter;

    /// Describes the accepted delimiter family in parse diagnostics.
    fn expected() -> &'static str;

    /// Borrows the non-space trivia following the opening delimiter.
    fn opening_trivia(&self) -> &TrailingTrivia;

    /// Mutably borrows the non-space trivia following the opening delimiter.
    fn opening_trivia_mut(&mut self) -> &mut TrailingTrivia;

    /// Borrows the non-space trivia following the closing delimiter.
    fn closing_trivia(&self) -> &TrailingTrivia;

    /// Mutably borrows the non-space trivia following the closing delimiter.
    fn closing_trivia_mut(&mut self) -> &mut TrailingTrivia;

    /// Emits `contents` surrounded by this delimiter.
    fn surround(&self, output: &mut TokenStream, contents: TokenStream) {
        let mut grouped = TokenStream::new();
        self.opening_trivia().to_tokens(&mut grouped);
        contents.to_tokens(&mut grouped);
        output.extend([TokenGroup::new(self.delimiter(), grouped)]);
        self.closing_trivia().to_tokens(output);
    }
}

/// A generated typed refinement of one raw delimiter kind.
///
/// Delimiter atoms are the container analogue of `Token![..]`: the typed CST
/// stores the precise delimiter family, while [`TokenGroup`]
/// carries the erased raw representation.
pub trait DelimiterToken: Parse + DelimiterValue {
    /// The raw delimiter family represented by this type.
    const KIND: DelimiterKind;

    /// Creates a typed delimiter from its two boundary spans.
    fn new(spans: DoubleSpan) -> Self;

    /// Returns the real opening and closing boundaries this atom was built
    /// from, or a collapsed pair when it was constructed from one span.
    fn spans(&self) -> DoubleSpan;
}

/// The source boundaries of a complete or incomplete delimiter pair.
///
/// A delimiter is two spelled tokens, not one span with an inferred width:
/// storing both is what makes [`Delimiter::opening_span`] and
/// [`Delimiter::closing_span`] plain field reads instead of arithmetic that
/// assumes the source layout. The closing boundary is [`Bound::Unbounded`]
/// while the group is incomplete, so a missing token is never confused with
/// a zero-width token at the current end of input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoubleSpan(pub Span, pub Bound<Span>);

impl DoubleSpan {
    /// Creates an opening and closing span pair.
    pub fn new(open: Span, close: Span) -> Self {
        Self(open, Bound::Included(close))
    }

    /// Creates a pair backed by the delimiter's synthetic source spelling.
    pub fn synthetic(kind: DelimiterKind) -> Self {
        let spelling = format!("{}{}", kind.opening(), kind.closing());
        let span = Span::for_text(spelling);
        let boundary = kind.opening().len();
        Self::new(
            span.subspan(0, boundary),
            span.subspan(boundary, span.len().unwrap_or(boundary)),
        )
    }

    /// Creates a delimiter whose closing token is not present yet.
    pub fn unclosed(open: Span) -> Self {
        Self(open, Bound::Unbounded)
    }

    /// Returns the opening delimiter span.
    pub fn open(&self) -> Span {
        self.0.clone()
    }

    /// Returns the closing delimiter span when it is present.
    pub fn close(&self) -> Option<Span> {
        match &self.1 {
            Bound::Included(span) | Bound::Excluded(span) => Some(span.clone()),
            Bound::Unbounded => None,
        }
    }

    /// Returns the closing boundary, preserving an incomplete delimiter as
    /// [`Bound::Unbounded`].
    pub fn close_bound(&self) -> Bound<Span> {
        self.1.clone()
    }

    /// Records the closing token of an incomplete delimiter pair.
    pub(crate) fn complete(&mut self, close: Span) {
        debug_assert!(matches!(self.1, Bound::Unbounded));
        self.1 = Bound::Included(close);
    }

    /// Reports whether a closing delimiter was present or synthetically quoted.
    pub fn is_closed(&self) -> bool {
        !matches!(self.1, Bound::Unbounded)
    }
}

impl Spanned for DoubleSpan {
    fn try_span(&self) -> Option<Span> {
        self.close()
            .and_then(|close| self.0.join(close))
            .or_else(|| Some(self.0.clone()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Type-erased delimiter data stored by a raw [`TokenGroup`].
pub struct Delimiter {
    /// The delimiter family.
    pub kind: DelimiterKind,
    /// The opening and closing source spans.
    pub spans: DoubleSpan,
    opening_trivia: TrailingTrivia,
    closing_trivia: TrailingTrivia,
}

impl Delimiter {
    /// Creates type-erased delimiter data.
    pub fn new(kind: DelimiterKind, spans: DoubleSpan) -> Self {
        Self {
            kind,
            spans,
            opening_trivia: TrailingTrivia::default(),
            closing_trivia: TrailingTrivia::default(),
        }
    }

    /// Returns the real span of the opening bracket.
    pub fn opening_span(&self) -> Span {
        self.spans.open()
    }

    /// Returns the real span of the closing bracket.
    pub fn closing_span(&self) -> Option<Span> {
        self.spans.close()
    }

    /// Reports whether the closing delimiter was present in source.
    pub fn is_closed(&self) -> bool {
        self.spans.is_closed()
    }
}

impl Spanned for Delimiter {
    fn try_span(&self) -> Option<Span> {
        self.spans.try_span()
    }
}

impl DelimiterValue for Delimiter {
    fn from_delimiter(delimiter: Delimiter) -> Option<Self> {
        Some(delimiter)
    }

    fn delimiter(&self) -> Delimiter {
        self.clone()
    }

    fn expected() -> &'static str {
        "a delimited group"
    }

    fn opening_trivia(&self) -> &TrailingTrivia {
        &self.opening_trivia
    }

    fn opening_trivia_mut(&mut self) -> &mut TrailingTrivia {
        &mut self.opening_trivia
    }

    fn closing_trivia(&self) -> &TrailingTrivia {
        &self.closing_trivia
    }

    fn closing_trivia_mut(&mut self) -> &mut TrailingTrivia {
        &mut self.closing_trivia
    }
}

impl Parse for Delimiter {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let group = parse_any_group(input, Self::expected())?;
        if !group.stream().is_empty() {
            return Err(TokenParseError::unexpected(
                "an empty delimited group",
                group.delim_kind().opening(),
                group.expect_span(),
            ));
        }
        Ok(group.delimiter().clone())
    }
}

impl Lex for Delimiter {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.surround(output, TokenStream::new());
    }
}

/// Parses one delimiter atom without any contained syntax.
#[doc(hidden)]
pub fn parse_delimiter<D: DelimiterToken>(input: &mut ParseStream) -> Result<D, TokenParseError> {
    let kind = D::KIND;
    let group = parse_group(input, kind)?;
    if !group.stream().is_empty() {
        return Err(TokenParseError::unexpected(
            kind.opening(),
            kind.opening(),
            group.expect_span(),
        ));
    }
    Ok(D::new(group.delimiter().spans.clone()))
}

/// Parses group.
pub(crate) fn parse_group(
    input: &mut ParseStream,
    kind: DelimiterKind,
) -> Result<TokenGroup, TokenParseError> {
    let group = parse_any_group(input, kind.opening())?;
    if group.delim_kind() != kind {
        return Err(TokenParseError::unexpected(
            kind.opening(),
            group.delim_kind().opening(),
            group.expect_span(),
        ));
    }
    Ok(group)
}

/// Parses any group.
pub(crate) fn parse_any_group(
    input: &mut ParseStream,
    expected: &'static str,
) -> Result<TokenGroup, TokenParseError> {
    let Some(token) = input.next_token() else {
        return Err(TokenParseError::unexpected_end(expected, input.eof_span()));
    };
    let TokenTree::Group(group) = token else {
        return Err(TokenParseError::unexpected_token(expected, &token));
    };
    Ok(group)
}

impl Display for Delimiter {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.kind.opening())
    }
}
impl Display for DelimiterKind {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.opening())
    }
}

impl DelimiterKind {
    /// Returns the opening source spelling.
    pub fn opening(self) -> &'static str {
        match self {
            Self::Parenthesis => "(",
            Self::Bracket => "[",
            Self::Brace => "{",
            Self::AngleBar => "<|",
        }
    }

    /// Returns the closing source spelling.
    pub fn closing(self) -> &'static str {
        match self {
            Self::Parenthesis => ")",
            Self::Bracket => "]",
            Self::Brace => "}",
            Self::AngleBar => "|>",
        }
    }
}
