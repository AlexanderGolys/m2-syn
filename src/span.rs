use std::fmt::{Debug, Display, Formatter};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU16, Ordering},
};

/// A boundary in a source buffer, as a byte offset.
///
/// Boundaries sit between bytes rather than on them, so a span's `end` is the
/// offset past its last byte and `end - start` is its length. One offset is
/// enough because a span's only job is to index a buffer: line and column are
/// facts about a particular buffer's contents, recovered from it when something
/// needs to *display* a location, and so they belong with the buffer rather
/// than with the span. This is what `rustc` does too — its `Span` stores `lo`
/// and `hi` byte positions and nothing else, and `SourceMap` binary-searches a
/// line table on demand.
pub type Offset = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
/// A human-facing position in a source snapshot.
///
/// Lines and columns are one-based. Columns count Unicode scalar values rather
/// than bytes or terminal cells, so the first character on every line is at
/// column one regardless of its UTF-8 width.
pub struct LineColumn {
    /// The one-based line number.
    pub line: usize,
    /// The one-based character column.
    pub column: usize,
}

impl Display for LineColumn {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}", self.line, self.column)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The human-facing opening and closing positions of a source span.
///
/// Like the underlying byte range, this range is half-open: `end` is the
/// position immediately after the final character.
pub struct LineColumnRange {
    /// The position at which the span opens.
    pub start: LineColumn,
    /// The position immediately after the span closes.
    pub end: LineColumn,
}

impl Display for LineColumnRange {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        if self.start == self.end {
            Display::fmt(&self.start, formatter)
        } else {
            write!(formatter, "{}-{}", self.start, self.end)
        }
    }
}

/// One text change: the text a buffer gains at a point, or the stretch it
/// loses.
///
/// An edit acts on *both* halves of the indexing relation — on source text
/// through [`Self::apply`] and on a [`Span`] through [`Span::shifted`] — and
/// those two actions are designed to agree.
///
/// An insertion carries the text it inserts rather than a pair of boundaries,
/// because the boundaries are recoverable (`at` and `at + text.len()`) while the
/// text is not, and without the text an edit cannot act on a buffer at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextEdit {
    /// Inserts `text` at one byte boundary.
    Insertion {
        /// The insertion boundary.
        at: Offset,
        /// The inserted text.
        text: String,
    },
    /// Deletes one half-open byte range.
    Deletion {
        /// The first deleted byte.
        start: Offset,
        /// The boundary after the last deleted byte.
        end: Offset,
    },
}

impl TextEdit {
    /// Creates an insertion of `text` landing at `at`.
    pub fn insertion(at: Offset, text: impl Into<String>) -> Self {
        Self::Insertion {
            at,
            text: text.into(),
        }
    }

    /// Creates a deletion of the text occupying `start..end`.
    ///
    /// # Panics
    ///
    /// Panics when `start` is greater than `end`.
    pub fn deletion(start: Offset, end: Offset) -> Self {
        assert!(start <= end, "deletion start must not exceed its end");
        Self::Deletion { start, end }
    }

    /// Returns `offset` in the buffer this edit produces.
    ///
    /// For an insertion of width `d` at `s` this is `offset + (offset >= s) * d`
    /// — everything at or after the insertion point moves by the width, and
    /// nothing else moves at all. A deletion is the mirror image, with the text
    /// it covered collapsing onto its start.
    ///
    /// The rule is monotone in `offset`, so applying it to both ends of a range
    /// can never invert them, and only the edit's *width* enters the arithmetic.
    /// Reaching the same result by summing two absolute offsets would
    /// manufacture an overflow the buffer never causes.
    fn shift(&self, offset: Offset) -> Offset {
        match self {
            Self::Insertion { at, text } => {
                if offset >= *at {
                    offset + text.len()
                } else {
                    offset
                }
            }
            Self::Deletion { start, end } => {
                if offset <= *start {
                    offset
                } else if offset <= *end {
                    // Everything the deletion covered now sits at its start.
                    *start
                } else {
                    offset - (end - start)
                }
            }
        }
    }

    /// Returns `source` with this edit applied, or `None` when the edit falls
    /// outside the buffer or across a character boundary.
    pub fn apply(&self, source: &str) -> Option<String> {
        let (cut, resumes) = match self {
            Self::Insertion { at, .. } => (*at, *at),
            Self::Deletion { start, end } => (*start, *end),
        };
        let mut edited = String::from(source.get(..cut)?);
        if let Self::Insertion { text, .. } = self {
            edited.push_str(text);
        }
        edited.push_str(source.get(resumes..)?);
        Some(edited)
    }
}

static NEXT_FILE_ID: AtomicU16 = AtomicU16::new(1);

/// The source site containing syntax.
///
/// `FileId` is the ordinary source-file identity. Keeping the identity behind
/// an enum lets the syntax model add distinct embedded or macro hygiene sites
/// without changing every span and token field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum File {
    /// An ordinary source file registered under a compact process-local id.
    FileId(u16),
    /// Syntax constructed without a corresponding on-disk source file.
    Synthetic(u16),
}

impl File {
    /// Allocates a compact process-local file label.
    ///
    /// The counter may recycle after all `u16` values have been used. Source
    /// snapshot identity is carried by the shared [`Source`] allocation, so a
    /// recycled display label cannot make unrelated spans equal or joinable.
    pub fn fresh() -> Self {
        let id = NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed);
        Self::FileId(id)
    }

    /// Creates an identity from an application-defined integer.
    pub const fn from_raw(raw: u16) -> Self {
        Self::FileId(raw)
    }

    /// Returns the application-defined integer stored by this identity.
    pub const fn into_raw(self) -> u16 {
        match self {
            Self::FileId(id) | Self::Synthetic(id) => id,
        }
    }
}

/// Compatibility name for the source identity stored by [`Span`].
pub type SourceId = File;

/// One immutable source snapshot shared by every span into it.
pub struct Source {
    file: File,
    text: Arc<str>,
    line_starts: OnceLock<Arc<[Offset]>>,
}

impl Source {
    /// Creates an ordinary source file with a fresh compact identity.
    pub fn new(text: impl Into<Arc<str>>) -> Arc<Self> {
        Self::with_file(File::fresh(), text)
    }

    /// Creates source text for syntax constructed outside an input file.
    pub fn synthetic(text: impl Into<Arc<str>>) -> Arc<Self> {
        let id = NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed);
        Self::with_file(File::Synthetic(id), text)
    }

    /// Creates a source snapshot with an explicitly selected source kind.
    pub fn with_file(file: File, text: impl Into<Arc<str>>) -> Arc<Self> {
        let text = text.into();
        Arc::new(Self {
            file,
            text,
            line_starts: OnceLock::new(),
        })
    }

    /// Returns this source's semantic identity.
    pub fn file(&self) -> File {
        self.file
    }

    /// Returns the complete immutable source snapshot.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Converts a byte boundary into a one-based line and character column.
    ///
    /// Returns `None` when `offset` lies outside this snapshot or inside a
    /// multi-byte UTF-8 character.
    pub fn line_column(&self, offset: Offset) -> Option<LineColumn> {
        self.text.get(..offset)?;
        let line_starts = self.line_starts.get_or_init(|| {
            std::iter::once(0)
                .chain(
                    self.text
                        .bytes()
                        .enumerate()
                        .filter_map(|(offset, byte)| (byte == b'\n').then_some(offset + 1)),
                )
                .collect::<Vec<_>>()
                .into()
        });
        let line_index = line_starts
            .partition_point(|line_start| *line_start <= offset)
            .checked_sub(1)?;
        let line_start = line_starts[line_index];
        let column = self.text.get(line_start..offset)?.chars().count() + 1;
        Some(LineColumn {
            line: line_index + 1,
            column,
        })
    }
}

impl Debug for Source {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Source")
            .field("file", &self.file)
            .field("bytes", &self.text.len())
            .finish_non_exhaustive()
    }
}

/// A shared source snapshot and optional byte range within it.
#[derive(Clone)]
pub struct Span {
    source: Arc<Source>,
    range: Option<Range<Offset>>,
}

impl Span {
    /// Builds a span in `source` with an optional byte range.
    pub fn new(source: Arc<Source>, range: Option<Range<Offset>>) -> Self {
        Self {
            source,
            range: range.filter(|range| range.start <= range.end),
        }
    }

    /// Builds a span covering `range` in `source`.
    pub fn ranged(source: Arc<Source>, range: Range<Offset>) -> Self {
        Self::new(source, Some(range))
    }

    /// Builds a span whose source is known but whose byte range is not.
    pub fn unlocated(source: Arc<Source>) -> Self {
        Self {
            source,
            range: None,
        }
    }

    /// Creates a span covering one complete synthetic source buffer.
    pub fn for_text(text: impl Into<Arc<str>>) -> Self {
        let source = Source::synthetic(text);
        Self::ranged(Arc::clone(&source), 0..source.text().len())
    }

    /// Returns the source file identity.
    pub fn file(&self) -> File {
        self.source.file()
    }

    /// Borrows the shared source snapshot.
    pub fn source(&self) -> &Arc<Source> {
        &self.source
    }

    /// Returns the source identity.
    pub fn source_id(&self) -> File {
        self.file()
    }

    /// Returns a copy of the indexed byte range.
    pub fn range(&self) -> Option<Range<Offset>> {
        self.range.clone()
    }

    /// Changes the source snapshot while preserving this span's range.
    pub fn with_source(mut self, source: Arc<Source>) -> Self {
        self.source = source;
        self
    }

    /// Ensures that this span resolves to exactly `text`.
    ///
    /// Parsed syntax retains its original source allocation. Programmatically
    /// constructed syntax receives a compact synthetic source allocation
    /// rather than storing a second copy of its spelling in every token.
    pub fn with_text(self, text: impl Into<Arc<str>>) -> Self {
        let text = text.into();
        if self.text() == Some(text.as_ref()) {
            return self;
        }
        let source = Source::with_file(self.file(), text);
        Self::ranged(Arc::clone(&source), 0..source.text().len())
    }

    /// Returns the exact source text covered by this span.
    pub fn text(&self) -> Option<&str> {
        self.source.text().get(self.range.as_ref()?.clone())
    }
    /// Creates a zero-width span at `offset` with this span's source identity.
    pub fn at(&self, offset: Offset) -> Self {
        if self.range.is_none() {
            return Self::unlocated(Arc::clone(&self.source));
        }
        Self {
            source: Arc::clone(&self.source),
            range: Some(offset..offset),
        }
    }

    /// Returns a subspan at byte offsets relative to this span's opening.
    ///
    /// The returned span shares the same source identity. Invalid or unavailable
    /// ranges become unlocated spans in the same file.
    pub fn subspan(&self, start: Offset, end: Offset) -> Self {
        let Some(outer) = &self.range else {
            return Self::unlocated(Arc::clone(&self.source));
        };
        let Some(start) = outer.start.checked_add(start) else {
            return Self::unlocated(Arc::clone(&self.source));
        };
        let Some(end) = outer.start.checked_add(end) else {
            return Self::unlocated(Arc::clone(&self.source));
        };
        if start > end || end > outer.end {
            return Self::unlocated(Arc::clone(&self.source));
        }
        Self {
            source: Arc::clone(&self.source),
            range: Some(start..end),
        }
    }

    /// The boundary this span opens at, or `None` when it is unlocated.
    pub fn start(&self) -> Option<Offset> {
        self.bounds().map(|(start, _)| start)
    }

    /// The boundary this span closes at, past its last byte.
    pub fn end(&self) -> Option<Offset> {
        self.bounds().map(|(_, end)| end)
    }

    /// The boundaries this span lies between.
    pub fn bounds(&self) -> Option<(Offset, Offset)> {
        self.range.as_ref().map(|range| (range.start, range.end))
    }

    /// Returns this span as one-based line and character-column positions.
    ///
    /// Returns `None` for an unlocated span, a range outside its source
    /// snapshot, or a boundary inside a multi-byte UTF-8 character.
    pub fn line_column_range(&self) -> Option<LineColumnRange> {
        let (start, end) = self.bounds()?;
        Some(LineColumnRange {
            start: self.source.line_column(start)?,
            end: self.source.line_column(end)?,
        })
    }

    /// How many bytes this span covers, or `None` when it is unlocated.
    pub fn len(&self) -> Option<usize> {
        self.bounds().map(|(start, end)| end - start)
    }

    /// Reports whether this span covers no bytes, as the virtual marker tokens
    /// do. Unlocated syntax covers no bytes either.
    pub fn is_empty(&self) -> bool {
        self.len().unwrap_or(0) == 0
    }

    /// Reports whether both spans identify the same source site.
    pub fn shares_source(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source)
    }

    /// Returns whether `offset` lies inside this span, excluding its closing
    /// boundary.
    ///
    /// Boundaries sit between bytes, so the closing one belongs to whatever
    /// follows: in `a+b` the boundary after `a` is contained by `+`, not by `a`.
    /// This is the predicate to use when exactly one span must own a position. A
    /// zero-width span contains nothing, having no interior.
    pub fn contains(&self, offset: Offset) -> bool {
        self.bounds()
            .is_some_and(|(start, end)| start <= offset && offset < end)
    }

    /// Returns whether `offset` lies inside this span or on either boundary.
    ///
    /// Adjacent spans both touch the boundary between them, so this answers
    /// "which syntax is the cursor at" rather than "which syntax owns this
    /// position" — a cursor just past an identifier still touches it, and a
    /// zero-width marker is touched at its own position.
    pub fn touches(&self, offset: Offset) -> bool {
        self.bounds()
            .is_some_and(|(start, end)| start <= offset && offset <= end)
    }

    /// Returns this span's position in the buffer produced by `edit`.
    ///
    /// Both boundaries move under the same rule, so nothing here decides which
    /// side of an insertion a boundary belongs to. Because that rule is monotone
    /// the pair can never invert, and a zero-width span needs no special case:
    /// its two equal boundaries stay equal.
    ///
    /// Text inserted at a span's *closing* boundary therefore extends the span,
    /// which is what an unclosed group wants — it keeps reaching whatever now
    /// follows it. A span wholly removed by a deletion collapses to a zero-width
    /// span at the deletion point rather than losing its location,
    /// because it still records where the syntax used to be.
    pub fn shifted(&self, edit: &TextEdit) -> Self {
        match self.bounds() {
            None => Self::unlocated(Arc::clone(&self.source)),
            Some((start, end)) => Self {
                source: Arc::clone(&self.source),
                range: Some(edit.shift(start)..edit.shift(end)),
            },
        }
    }

    /// Returns this span with the same opening boundary and source identity but
    /// a new byte length.
    pub fn resized(&self, len: usize) -> Self {
        let Some(start) = self.start() else {
            return Self::unlocated(Arc::clone(&self.source));
        };
        Self {
            source: Arc::clone(&self.source),
            range: Some(start..start.saturating_add(len)),
        }
    }
}

impl Span {
    /// The smallest span covering both, or `None` when they belong to
    /// different files.
    pub fn join(&self, other: Self) -> Option<Self> {
        if !self.shares_source(&other) {
            return None;
        }
        match (self.bounds(), other.bounds()) {
            (None, _) => Some(other),
            (_, None) => Some(self.clone()),
            (Some((start, end)), Some((start2, end2))) => {
                let start = start.min(start2);
                let end = end.max(end2);
                Some(Self {
                    source: Arc::clone(&self.source),
                    range: Some(start..end),
                })
            }
        }
    }

    /// The overlap of both, or `None` when the files differ or the ranges do
    /// not overlap.
    pub fn meet(&self, other: Self) -> Option<Self> {
        if !self.shares_source(&other) {
            return None;
        }
        match (self.bounds(), other.bounds()) {
            (Some((start, end)), Some((start2, end2))) => {
                let start = start.max(start2);
                let end = end.min(end2);
                if start < end {
                    Some(Self {
                        source: Arc::clone(&self.source),
                        range: Some(start..end),
                    })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Joins every span in an iterator, returning `None` for an empty iterator
    /// or when any two spans belong to different files.
    pub fn join_all<I, S>(spans: I) -> Option<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<Option<Self>>,
    {
        let mut spans = spans.into_iter().filter_map(Into::into);
        let first = spans.next()?;
        spans.try_fold(first, |left, right| left.join(right))
    }
}

impl PartialEq for Span {
    fn eq(&self, other: &Self) -> bool {
        self.shares_source(other) && self.range == other.range
    }
}

impl Eq for Span {}

impl Hash for Span {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.source).hash(state);
        self.range.hash(state);
    }
}

impl Debug for Span {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let mut span = formatter.debug_struct("Span");
        span.field("file", &self.file())
            .field("range", &self.line_column_range())
            .field("bytes", &self.range);
        if let Some(text) = self.text() {
            span.field("text", &text);
        }
        span.finish()
    }
}

impl Display for Span {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self.line_column_range() {
            Some(range) => Display::fmt(&range, formatter),
            None => formatter.write_str("unlocated"),
        }
    }
}

/// A syntax value that reports its source extent.
pub trait Spanned {
    /// Returns the smallest span covering this value.
    ///
    /// # Panics
    ///
    /// Panics when the value is absent or assembled from different source
    /// snapshots. Use [`Self::try_span`] when either case is expected.
    fn span(&self) -> Span {
        self.try_span()
            .expect("syntax must have one shared source snapshot")
    }

    /// Returns the smallest span covering this value, when its constituents
    /// share one source snapshot.
    fn try_span(&self) -> Option<Span> {
        Some(self.span())
    }

    /// Returns this value's span when one source snapshot covers it.
    ///
    /// # Panics
    ///
    /// Panics for absent syntax or syntax assembled from different sources.
    fn expect_span(&self) -> Span {
        self.span()
    }
}

impl Spanned for Span {
    fn span(&self) -> Span {
        self.clone()
    }
}

impl<T: Spanned + ?Sized> Spanned for &T {
    fn span(&self) -> Span {
        (*self).span()
    }

    fn try_span(&self) -> Option<Span> {
        (*self).try_span()
    }
}

impl<T: Spanned + ?Sized> Spanned for Box<T> {
    fn span(&self) -> Span {
        self.as_ref().span()
    }

    fn try_span(&self) -> Option<Span> {
        self.as_ref().try_span()
    }
}

impl<T: Spanned> Spanned for Option<T> {
    fn try_span(&self) -> Option<Span> {
        self.as_ref().and_then(Spanned::try_span)
    }
}

impl<T: Spanned> Spanned for Vec<T> {
    fn try_span(&self) -> Option<Span> {
        Span::join_all(self.iter().map(Spanned::try_span))
    }
}

impl<A: Spanned, B: Spanned> Spanned for (A, B) {
    fn try_span(&self) -> Option<Span> {
        self.0.try_span()?.join(self.1.try_span()?)
    }
}

impl<T: Spanned> Spanned for &[T] {
    fn try_span(&self) -> Option<Span> {
        Span::join_all(self.iter().map(Spanned::try_span))
    }
}

#[cfg(test)]
mod source_tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn spans_resolve_text_from_one_shared_source() {
        let source = Source::new("quotient ring");
        let quotient = Span::ranged(Arc::clone(&source), 0..8);
        let ring = Span::ranged(Arc::clone(&source), 9..13);

        assert_eq!(quotient.text(), Some("quotient"));
        assert_eq!(ring.text(), Some("ring"));
        assert!(Arc::ptr_eq(quotient.source(), ring.source()));
    }

    #[test]
    fn joining_different_sources_returns_none() {
        let left = Span::for_text("x");
        let right = Span::for_text("y");

        assert_eq!(left.join(right), None);
    }

    #[test]
    fn an_unlocated_span_still_knows_its_source() {
        let source = Source::new("x + y");
        let span = Span::unlocated(Arc::clone(&source));

        assert_eq!(span.file(), source.file());
        assert_eq!(span.range(), None);
        assert_eq!(span.text(), None);
    }

    #[test]
    fn source_snapshots_with_the_same_file_do_not_alias() {
        let file = File::from_raw(7);
        let before = Span::ranged(Source::with_file(file, "x"), 0..1);
        let after = Span::ranged(Source::with_file(file, "y"), 0..1);

        assert_ne!(before, after);
        assert_eq!(before.join(after), None);
    }

    #[test]
    fn shifting_preserves_source_identity() {
        let source = Source::new("ring");
        let span = Span::ranged(Arc::clone(&source), 0..4);
        let shifted = span.shifted(&TextEdit::insertion(0, "graded "));

        assert!(Arc::ptr_eq(&source, shifted.source()));
        assert_eq!(shifted.range(), Some(7..11));
    }

    #[test]
    fn line_columns_are_one_based_and_count_characters() {
        let source = Source::new("aα\nβc");

        assert_eq!(
            source.line_column(0),
            Some(LineColumn { line: 1, column: 1 }),
        );
        assert_eq!(
            source.line_column("aα\n".len()),
            Some(LineColumn { line: 2, column: 1 }),
        );
        assert_eq!(
            source.line_column("aα\nβ".len()),
            Some(LineColumn { line: 2, column: 2 }),
        );
        assert_eq!(source.line_column(2), None);
    }

    #[test]
    fn spans_format_as_half_open_line_column_ranges() {
        let source = Source::new("one\ntwo");
        let range = Span::ranged(Arc::clone(&source), 2..6);
        let point = range.at(4);

        assert_eq!(range.to_string(), "1:3-2:3");
        assert_eq!(point.to_string(), "2:1");
    }
}
