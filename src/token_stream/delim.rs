//! Raw and typed delimiter support.
//!
//! [`Delimiter`] is stored by raw [`Group`] token trees. Generated
//! `Delimiter![..]` atoms implement [`DelimiterToken`] and are stored on typed
//! delimited nodes, so flattening can recover the corresponding raw group.

use std::fmt::{Display, Formatter};

use crate::{
    Group, Parse, ParseStream, Punct, Span, Spanned, TextPoint, TextRange, ToTokens,
    TokenParseError, TokenStream, TokenTree,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The six delimiter families recognized by raw syntax containers.
///
/// Paired delimiters surround ordinary token-tree groups. [`Empty`](Self::Empty)
/// and [`Semicolon`](Self::Semicolon) delimit source cells: both have an
/// implicit opening boundary, while only the semicolon has visible closing
/// text.
pub enum DelimiterKind {
    Empty,
    Semicolon,
    Parenthesis,
    Bracket,
    Brace,
    AngleBar,
}

/// A generated typed refinement of one raw delimiter kind.
///
/// Delimiter atoms are the container analogue of `Token![..]`: the typed CST
/// stores the precise delimiter family, while [`Group`] and
/// [`crate::CellBlock`] carry the erased raw representation.
pub trait DelimiterToken: Parse + Spanned + ToTokens + Sized {
    const KIND: DelimiterKind;

    fn new(span: Span) -> Self;

    fn surround(&self, output: &mut TokenStream, contents: TokenStream) {
        Delimiter::new(Self::KIND, self.span()).surround(output, contents);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Spanned)]
/// Type-erased delimiter data stored by a raw [`Group`].
pub struct Delimiter {
    pub kind: DelimiterKind,
    /// The complete span of the delimited container.
    pub span: Span,
}

impl Delimiter {
    pub fn new(kind: DelimiterKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Returns the opening delimiter span derived from the container span.
    pub fn opening_span(self) -> Span {
        boundary_span(self.span, Boundary::Opening, self.kind.opening().len())
    }

    /// Returns the closing delimiter span derived from the container span.
    pub fn closing_span(self) -> Span {
        boundary_span(self.span, Boundary::Closing, self.kind.closing().len())
    }

    /// Emits `contents` with this delimiter's concrete token-tree shape.
    pub fn surround(&self, output: &mut TokenStream, contents: TokenStream) {
        match self.kind {
            DelimiterKind::Empty => output.extend([contents]),
            DelimiterKind::Semicolon => {
                output.extend([contents]);
                output.push_punct(Punct::new(";", self.closing_span()));
            }
            DelimiterKind::Parenthesis
            | DelimiterKind::Bracket
            | DelimiterKind::Brace
            | DelimiterKind::AngleBar => output.push_group(Group::new(*self, contents)),
        }
    }
}

/// Parses one delimiter atom without any contained syntax.
#[doc(hidden)]
pub fn parse_delimiter<D: DelimiterToken>(input: &mut ParseStream) -> Result<D, TokenParseError> {
    match D::KIND {
        DelimiterKind::Empty => {
            let span = input
                .peek()
                .map(|token| boundary_before(token.span()))
                .unwrap_or_else(|| input.eof_span());
            Ok(D::new(span))
        }
        DelimiterKind::Semicolon => {
            let semicolon = <Token![;]>::parse(input)?;
            Ok(D::new(semicolon.span()))
        }
        kind => {
            let Some(token) = input.next_token() else {
                return Err(TokenParseError::UnexpectedEnd {
                    expected: kind.opening(),
                    span: input.eof_span(),
                });
            };
            let span = token.span();
            let TokenTree::Group(group) = token else {
                return Err(TokenParseError::UnexpectedToken {
                    expected: kind.opening(),
                    found: token.spelling().unwrap_or("trivia").to_owned(),
                    span,
                });
            };
            if group.delim_kind() != kind || !group.stream().is_empty() {
                return Err(TokenParseError::UnexpectedToken {
                    expected: kind.opening(),
                    found: group.delim_kind().opening().to_owned(),
                    span,
                });
            }
            Ok(D::new(group.span()))
        }
    }
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
    pub fn opening(self) -> &'static str {
        match self {
            Self::Empty | Self::Semicolon => "",
            Self::Parenthesis => "(",
            Self::Bracket => "[",
            Self::Brace => "{",
            Self::AngleBar => "<|",
        }
    }

    pub fn closing(self) -> &'static str {
        match self {
            Self::Empty => "",
            Self::Semicolon => ";",
            Self::Parenthesis => ")",
            Self::Bracket => "]",
            Self::Brace => "}",
            Self::AngleBar => "|>",
        }
    }
}

#[derive(Clone, Copy)]
enum Boundary {
    Opening,
    Closing,
}

fn boundary_span(span: Span, boundary: Boundary, width: usize) -> Span {
    let Ok(range) = span.range() else {
        return Span::detached();
    };
    let (Some(start), Some(end)) = (range.start(), range.end()) else {
        return Span::detached();
    };
    if end.byte.saturating_sub(start.byte) < width {
        return span;
    }

    let range = match boundary {
        Boundary::Opening => TextRange::new(start, shifted(start, width)),
        Boundary::Closing => TextRange::new(shifted_back(end, width), end),
    };
    match span {
        Span::FileLocated { source, .. } => Span::new(source, range),
        Span::LocalRefFrame { .. } => Span::in_tmp_file(range),
        Span::Detached => Span::detached(),
    }
}

fn shifted(mut point: TextPoint, width: usize) -> TextPoint {
    point.byte = point.byte.saturating_add(width);
    point.column = point.column.saturating_add(width as u32);
    point
}

fn shifted_back(mut point: TextPoint, width: usize) -> TextPoint {
    point.byte = point.byte.saturating_sub(width);
    point.column = point.column.saturating_sub(width as u32);
    point
}

fn boundary_before(span: Span) -> Span {
    span.source()
        .ok()
        .zip(span.start_point().ok())
        .map(|(source, point)| Span::new(source, TextRange::from_point(point)))
        .unwrap_or_else(Span::detached)
}
