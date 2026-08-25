//! Raw and typed delimiter support.
//!
//! [`Delimiter`] is stored by raw [`TokenGroup`] token trees. Generated
//! `Delimiter![..]` atoms implement [`DelimiterToken`] and are stored on typed
//! delimited nodes, so flattening can recover the corresponding raw group.

use std::fmt::{Display, Formatter};

use crate::{
    Lex, Offset, Parse, ParseStream, Span, Spanned, TokenGroup, TokenParseError, TokenStream,
    TokenTree,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The delimiter families recognized by raw syntax containers.
///
/// Every kind is spelled in the source and pairs an opening token with a
/// closing one, so a group is opaque and its extent is read rather than
/// inferred. Cell boundaries are deliberately absent: they are a parse
/// result, not a delimiter.
pub enum DelimiterKind {
    /// `( ... )`.
    Parenthesis,
    /// `[ ... ]`.
    Bracket,
    /// `{ ... }`.
    Brace,
    /// `<| ... |>`.
    AngleBar,
}

impl DelimiterKind {
    /// Returns the expression family introduced by this delimiter.
    pub const fn expression_name(self) -> &'static str {
        match self {
            Self::Parenthesis => "parenthesized expression",
            Self::Bracket => "array expression",
            Self::Brace => "list expression",
            Self::AngleBar => "angle-bar list expression",
        }
    }
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

    /// Emits `contents` surrounded by this delimiter.
    fn surround(&self, output: &mut TokenStream, contents: TokenStream) {
        output.extend([TokenGroup::new(self.delimiter(), contents)]);
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

/// The real opening and closing boundaries of a paired delimiter.
///
/// A delimiter is two spelled tokens, not one span with an inferred width:
/// storing both is what makes [`Delimiter::opening_span`] and
/// [`Delimiter::closing_span`] plain field reads instead of arithmetic that
/// assumes the source layout. `DoubleSpan(open, close)` mirrors
/// `proc_macro2`'s open/close pair for a `Group`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DoubleSpan(pub Span, pub Span);

impl DoubleSpan {
    /// Creates an opening and closing span pair.
    pub fn new(open: Span, close: Span) -> Self {
        Self(open, close)
    }

    /// A pair with no real boundaries, for synthetic or quoted delimiters
    /// that were never lexed from source.
    pub fn detached() -> Self {
        Self(Span::detached(), Span::detached())
    }

    /// Creates a delimiter whose closing token is missing at `eof`.
    pub fn unclosed(open: Span, eof: Offset) -> Self {
        Self(open, Span::from_offset(eof))
    }

    /// Returns the opening delimiter span.
    pub fn open(&self) -> Span {
        self.0.clone()
    }

    /// Returns the closing delimiter span.
    pub fn close(&self) -> Span {
        self.1.clone()
    }

    /// Reports whether a closing delimiter was present or synthetically quoted.
    pub fn is_closed(&self) -> bool {
        self.1.bounds().is_none_or(|(start, end)| start != end)
    }
}

impl Spanned for DoubleSpan {
    fn span(&self) -> Span {
        self.0.join(self.1.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Type-erased delimiter data stored by a raw [`TokenGroup`].
pub struct Delimiter {
    /// The delimiter family.
    pub kind: DelimiterKind,
    /// The opening and closing source spans.
    pub spans: DoubleSpan,
}

impl Delimiter {
    /// Creates type-erased delimiter data.
    pub fn new(kind: DelimiterKind, spans: DoubleSpan) -> Self {
        Self { kind, spans }
    }

    /// Returns the real span of the opening bracket.
    pub fn opening_span(&self) -> Span {
        self.spans.open()
    }

    /// Returns the real span of the closing bracket.
    pub fn closing_span(&self) -> Span {
        self.spans.close()
    }

    /// Reports whether the closing delimiter was present in source.
    pub fn is_closed(&self) -> bool {
        self.spans.is_closed()
    }
}

impl Spanned for Delimiter {
    fn span(&self) -> Span {
        self.spans.span()
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
}

impl Parse for Delimiter {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let group = parse_any_group(input, Self::expected())?;
        if !group.stream().is_empty() {
            return Err(TokenParseError::unexpected(
                "an empty delimited group",
                group.delim_kind().opening(),
                group.span(),
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
            group.span(),
        ));
    }
    Ok(D::new(group.delimiter().spans.clone()))
}

pub(crate) fn parse_group(
    input: &mut ParseStream,
    kind: DelimiterKind,
) -> Result<TokenGroup, TokenParseError> {
    let group = parse_any_group(input, kind.opening())?;
    if group.delim_kind() != kind {
        return Err(TokenParseError::unexpected(
            kind.opening(),
            group.delim_kind().opening(),
            group.span(),
        ));
    }
    Ok(group)
}

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
