use std::fmt::{Display, Formatter};

use crate::{Span, Spanned};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DelimiterKind {
    Parenthesis,
    Bracket,
    Brace,
    AngleBar,
    String,
    RawString,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            Self::Parenthesis => "(",
            Self::Bracket => "[",
            Self::Brace => "{",
            Self::AngleBar => "<|",
            Self::String => "\"",
            Self::RawString => "///",
        }
    }

    pub fn closing(self) -> &'static str {
        match self {
            Self::Parenthesis => ")",
            Self::Bracket => "]",
            Self::Brace => "}",
            Self::AngleBar => "|>",
            Self::String => "\"",
            Self::RawString => "///",
        }
    }
}

impl Spanned for Delimiter {
    fn span(&self) -> Span {
        self.span2.join()
    }
}
