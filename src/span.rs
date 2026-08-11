use std::error::Error;
use std::fmt::{Display, Formatter, Result as FmtResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextPoint {
    pub line: u32,
    pub column: u32,
    pub byte: usize,
}

impl TextPoint {
    pub fn new(line: u32, column: u32, byte: usize) -> Self {
        Self { line, column, byte }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TextRange {
    pub start: TextPoint,
    pub end: TextPoint,
}

impl TextRange {
    pub fn new(start: TextPoint, end: TextPoint) -> Option<Self> {
        (start <= end).then_some(Self { start, end })
    }

    pub fn covering(self, other: Self) -> Self {
        Self {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span(Option<SourceSpan>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SourceSpan {
    source: SourceId,
    range: TextRange,
}

impl Span {
    pub fn detached() -> Self {
        Self(None)
    }

    pub fn located(source: SourceId, range: TextRange) -> Self {
        Self(Some(SourceSpan { source, range }))
    }

    pub fn source(self) -> Result<SourceId, MissingLocation> {
        self.0.map(|span| span.source).ok_or(MissingLocation)
    }

    pub fn range(self) -> Result<TextRange, MissingLocation> {
        self.0.map(|span| span.range).ok_or(MissingLocation)
    }

    pub fn join(self, other: Self) -> Self {
        match (self.0, other.0) {
            (None, other) => Self(other),
            (this, None) => Self(this),
            (Some(this), Some(other)) if this.source == other.source => {
                Self::located(this.source, this.range.covering(other.range))
            }
            (Some(_), Some(_)) => Self::detached(),
        }
    }

    pub fn join_all(spans: impl IntoIterator<Item = Span>) -> Self {
        spans.into_iter().fold(Self::detached(), Self::join)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingLocation;

impl Display for MissingLocation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str("syntax has no source location")
    }
}

impl Error for MissingLocation {}

pub trait Spanned {
    fn span(&self) -> Span;
}

impl<T> Spanned for Box<T>
where
    T: Spanned + ?Sized,
{
    fn span(&self) -> Span {
        self.as_ref().span()
    }
}

impl<T> Spanned for Option<T>
where
    T: Spanned,
{
    fn span(&self) -> Span {
        self.as_ref().map_or_else(Span::detached, Spanned::span)
    }
}

impl<T> Spanned for Vec<T>
where
    T: Spanned,
{
    fn span(&self) -> Span {
        Span::join_all(self.iter().map(Spanned::span))
    }
}
