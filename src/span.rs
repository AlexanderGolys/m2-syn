use std::cmp::Ordering;
use std::error::Error;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::ops::{BitAnd, BitOr, Bound, RangeBounds};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId(pub u64);

/// Complete describption of position in a text document.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextRange {
    Range {
        start: TextPoint,
        end: TextPoint,
    },
    #[default]
    Empty,
}

impl TextRange {
    pub fn from_point(point: TextPoint) -> Self {
        Self::Range {
            start: point,
            end: point,
        }
    }

    pub fn new(start: TextPoint, end: TextPoint) -> Self {
        match start.cmp(&end) {
            Ordering::Less => Self::Range { start, end },
            Ordering::Equal => Self::from_point(start),
            Ordering::Greater => Self::Empty,
        }
    }

    pub fn join(self, other: Self) -> Self {
        match (self, other) {
            (Self::Empty, _) => other,
            (_, Self::Empty) => self,
            (
                Self::Range { start, end },
                Self::Range {
                    start: start2,
                    end: end2,
                },
            ) => Self::new(start.min(start2), end.max(end2)),
        }
    }

    pub fn meet(self, other: Self) -> Self {
        match (self, other) {
            (Self::Empty, _) => Self::Empty,
            (_, Self::Empty) => Self::Empty,
            (
                Self::Range { start, end },
                Self::Range {
                    start: start2,
                    end: end2,
                },
            ) => Self::new(start.max(start2), end.min(end2)),
        }
    }

    pub fn start(&self) -> Option<TextPoint> {
        match self {
            Self::Empty => None,
            Self::Range { start, .. } => Some(*start),
        }
    }

    pub fn end(&self) -> Option<TextPoint> {
        match self {
            Self::Empty => None,
            Self::Range { end, .. } => Some(*end),
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    pub fn is_point(&self) -> bool {
        matches!(self, Self::Range { start, end } if start == end)
    }

    pub fn size(&self) -> Option<usize> {
        self.end()?.byte.checked_sub(self.start()?.byte)
    }
}

impl BitOr for TextRange {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        self.join(other)
    }
}

impl BitAnd for TextRange {
    type Output = Self;

    fn bitand(self, other: Self) -> Self {
        self.meet(other)
    }
}

/// transforming range a..b of two text points interpretable as
/// text range into the two endpoints a, b  
impl RangeBounds<TextPoint> for TextRange {
    fn start_bound(&self) -> Bound<&TextPoint> {
        match self {
            Self::Empty => Bound::Unbounded,
            Self::Range { start, .. } => Bound::Included(start),
        }
    }

    fn end_bound(&self) -> Bound<&TextPoint> {
        match self {
            Self::Empty => Bound::Unbounded,
            Self::Range { end, .. } => Bound::Included(end),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Span {
    #[default]
    Detached,
    FileLocated {
        source: SourceId,
        range: TextRange,
    },
    LocalRefFrame {
        range: TextRange,
    },
}

impl Span {
    pub fn detached() -> Self {
        Self::default()
    }

    pub fn new(source: SourceId, range: TextRange) -> Self {
        Self::FileLocated { source, range }
    }

    pub fn in_tmp_file(range: TextRange) -> Self {
        Self::LocalRefFrame { range }
    }

    pub fn source(self) -> Result<SourceId, MissingData> {
        match self {
            Self::FileLocated { source, .. } => Ok(source),
            _ => Err(MissingData),
        }
    }

    pub fn range(self) -> Result<TextRange, MissingData> {
        match self {
            Self::Detached => Err(MissingData),
            Self::FileLocated { range, .. } => Ok(range),
            Self::LocalRefFrame { range } => Ok(range),
        }
    }

    pub fn start_point(self) -> Result<TextPoint, MissingData> {
        self.range()?.start().ok_or(MissingData)
    }

    pub fn end_point(self) -> Result<TextPoint, MissingData> {
        self.range()?.end().ok_or(MissingData)
    }

    pub fn byte_size(self) -> Result<usize, MissingData> {
        self.range()?.size().ok_or(MissingData)
    }

    pub fn join(self, other: Self) -> Self {
        match (self, other) {
            (
                Self::FileLocated { source, range },
                Self::FileLocated {
                    source: source2,
                    range: range2,
                },
            ) => {
                if source == source2 {
                    Self::new(source, range | range2)
                } else {
                    Self::Detached
                }
            }

            (Self::LocalRefFrame { range }, Self::LocalRefFrame { range: range2 }) => {
                Self::in_tmp_file(range | range2)
            }
            (_, _) => Self::Detached,
        }
    }

    pub fn join_all(spans: impl IntoIterator<Item = Span>) -> Self {
        spans.into_iter().reduce(Span::join).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MissingData;

impl Display for MissingData {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str("syntax has no source location")
    }
}

impl Error for MissingData {}

pub trait Spanned {
    fn span(&self) -> Span;
}

impl Spanned for Span {
    fn span(&self) -> Span {
        *self
    }
}

impl<T: Spanned + ?Sized> Spanned for &T {
    fn span(&self) -> Span {
        (*self).span()
    }
}

impl<T: Spanned + ?Sized> Spanned for Box<T> {
    fn span(&self) -> Span {
        self.as_ref().span()
    }
}

impl<T: Spanned> Spanned for Option<T> {
    fn span(&self) -> Span {
        self.as_ref().map_or_else(Span::detached, Spanned::span)
    }
}

impl<T: Spanned> Spanned for Vec<T> {
    fn span(&self) -> Span {
        Span::join_all(self.iter().map(Spanned::span))
    }
}

impl<T: Spanned> Spanned for &[T] {
    fn span(&self) -> Span {
        Span::join_all(self.iter().map(Spanned::span))
    }
}
