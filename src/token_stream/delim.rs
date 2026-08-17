//! Raw and typed delimiter support.
//!
//! [`Delimiter`] is stored by raw [`Group`] token trees. Generated
//! `Delimiter![..]` atoms implement [`DelimiterToken`] and are stored on typed
//! delimited nodes, so flattening can recover the corresponding raw group.

use std::fmt::{Display, Formatter};

use crate::{Group, Span, Spanned, TokenStream};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Spanned)]
/// Independent source spans for a syntax container's opening and closing boundaries.
pub struct DoubleSpan {
    pub span_open: Span,
    pub span_close: Span,
}

impl DoubleSpan {
    pub fn new(span_open: Span, span_close: Span) -> Self {
        Self {
            span_open,
            span_close,
        }
    }
    pub fn join(&self) -> Span {
        self.span_open.join(self.span_close)
    }

    pub fn detached() -> Self {
        Self::new(Span::detached(), Span::detached())
    }
}

/// A generated typed refinement of one raw delimiter kind.
///
/// Delimiter atoms are the container analogue of `Token![..]`: the typed CST
/// stores the precise delimiter family, while [`Group`] and
/// [`crate::CellBlock`] carry the erased raw representation.
pub trait DelimiterToken: Spanned + Sized {
    const KIND: DelimiterKind;

    fn new(span: DoubleSpan) -> Self;
    fn span2(&self) -> DoubleSpan;

    fn surround(&self, output: &mut TokenStream, contents: TokenStream) {
        output.push_group(Group::new(
            Delimiter::new(Self::KIND, self.span2()),
            contents,
        ));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Type-erased delimiter data stored by a raw [`Group`].
pub struct Delimiter {
    pub kind: DelimiterKind,
    pub span2: DoubleSpan,
}

impl Delimiter {
    pub fn new(kind: DelimiterKind, span2: DoubleSpan) -> Self {
        Self { kind, span2 }
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

impl Spanned for Delimiter {
    fn span(&self) -> Span {
        self.span2.join()
    }
}
