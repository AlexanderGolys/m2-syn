use std::hash::{Hash, Hasher};
use std::ops::Index;
use std::sync::Arc;

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

/// Where syntax originated: a stretch of a buffer, or nowhere at all.
///
/// A zero-width `Range` is `start == end`, which is what the virtual marker
/// nodes use — end of input, the `Adj` between adjacent expressions,
/// an omitted component in `(x,,y)`. Because boundaries sit between bytes,
/// `x..x` already names a position exactly, so such a marker needs no
/// representation of its own. An inverted pair can never be constructed:
/// [`Self::new`] collapses it to `Detached` rather than fabricating a range.
///
#[derive(Debug, Clone)]
struct SourceSlice {
    text: Arc<str>,
    origin: Offset,
}

impl PartialEq for SourceSlice {
    fn eq(&self, other: &Self) -> bool {
        self.origin == other.origin && Arc::ptr_eq(&self.text, &other.text)
    }
}

impl Eq for SourceSlice {}

impl Hash for SourceSlice {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.origin.hash(state);
        std::ptr::hash(Arc::as_ptr(&self.text), state);
    }
}

impl SourceSlice {
    fn covers(&self, start: Offset, end: Offset) -> bool {
        self.origin <= start && end <= self.origin.saturating_add(self.text.len())
    }
}

fn common_source(
    left: Option<&SourceSlice>,
    right: Option<&SourceSlice>,
    start: Offset,
    end: Offset,
) -> Option<SourceSlice> {
    match (left, right) {
        (Some(left), Some(right)) if left == right => Some(left.clone()),
        (Some(source), None) | (None, Some(source)) if source.covers(start, end) => {
            Some(source.clone())
        }
        _ => None,
    }
}

/// Where syntax originated and, when available, the source buffer it indexes.
///
/// Source-derived tokens retain only this value. Their spelling is recovered
/// by slicing the shared source buffer; it is never copied into token or syntax
/// node storage. Spans constructed only from offsets remain useful for
/// diagnostics and synthetic trees, but naturally cannot yield source text.
#[derive(Debug, Clone, Default)]
pub struct Span {
    bounds: Option<(Offset, Offset)>,
    source: Option<SourceSlice>,
}

impl PartialEq for Span {
    fn eq(&self, other: &Self) -> bool {
        self.bounds == other.bounds && self.source == other.source
    }
}

impl Eq for Span {}

impl Hash for Span {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bounds.hash(state);
        self.source.hash(state);
    }
}

impl Span {
    /// A detached span without source text.
    #[allow(non_upper_case_globals)]
    pub const Detached: Self = Self {
        bounds: None,
        source: None,
    };

    /// Creates a span with no source location.
    pub fn detached() -> Self {
        Self::Detached
    }

    /// Builds a span covering `start..end`, where `end` is the offset past the
    /// last byte.
    ///
    /// An inverted pair collapses to `Detached` rather than fabricating a range.
    pub fn new(start: Offset, end: Offset) -> Self {
        if start <= end {
            Self {
                bounds: Some((start, end)),
                source: None,
            }
        } else {
            Self::Detached
        }
    }

    /// A zero-width span at one boundary, for syntax that occupies no bytes.
    pub fn from_offset(offset: Offset) -> Self {
        Self {
            bounds: Some((offset, offset)),
            source: None,
        }
    }

    /// Attaches the complete source fragment covered by this span.
    ///
    /// Existing source backing is retained. For an offset-only span, `text`
    /// must have the same byte width as the span; its first byte is associated
    /// with the span's opening offset. Detached syntax may also carry a source
    /// fragment, which is useful for programmatically constructed syntax.
    ///
    /// # Panics
    ///
    /// Panics when an offset-only span and `text` have different byte widths.
    pub fn with_text(self, text: impl Into<Arc<str>>) -> Self {
        if self.source.is_some() {
            return self;
        }
        let text = text.into();
        let origin = match self.bounds {
            Some((start, end)) => {
                assert_eq!(
                    end - start,
                    text.len(),
                    "source fragment width must equal its span width"
                );
                start
            }
            None => 0,
        };
        Self {
            source: Some(SourceSlice { text, origin }),
            ..self
        }
    }

    /// Replaces this span's source fragment while retaining its opening
    /// location and updating its width.
    pub fn replaced_text(&self, text: impl Into<Arc<str>>) -> Self {
        let text = text.into();
        let bounds = self
            .start()
            .map(|start| (start, start.saturating_add(text.len())));
        let origin = bounds.map_or(0, |(start, _)| start);
        Self {
            bounds,
            source: Some(SourceSlice { text, origin }),
        }
    }

    /// Creates a source-backed span covering an entire source buffer.
    pub fn source(text: impl Into<Arc<str>>) -> Self {
        let text = text.into();
        let end = text.len();
        Self {
            bounds: Some((0, end)),
            source: Some(SourceSlice { text, origin: 0 }),
        }
    }

    /// Returns a subspan at byte offsets relative to this span's opening.
    ///
    /// The returned span shares the same source buffer. Invalid or detached
    /// ranges become detached spans.
    pub fn subspan(&self, start: Offset, end: Offset) -> Self {
        let Some((outer_start, outer_end)) = self.bounds else {
            return Self::Detached;
        };
        let Some(start) = outer_start.checked_add(start) else {
            return Self::Detached;
        };
        let Some(end) = outer_start.checked_add(end) else {
            return Self::Detached;
        };
        if start > end || end > outer_end {
            return Self::Detached;
        }
        Self {
            bounds: Some((start, end)),
            source: self.source.clone(),
        }
    }

    /// Returns the exact source text covered by this span when source backing
    /// is available.
    pub fn text(&self) -> Option<&str> {
        let source = self.source.as_ref()?;
        match self.bounds {
            Some((start, end)) => source
                .text
                .get(start.checked_sub(source.origin)?..end.checked_sub(source.origin)?),
            None => Some(&source.text),
        }
    }

    /// Reports whether both spans slice the same shared source buffer.
    pub fn shares_source(&self, other: &Self) -> bool {
        match (&self.source, &other.source) {
            (Some(left), Some(right)) => Arc::ptr_eq(&left.text, &right.text),
            _ => false,
        }
    }

    /// The boundary this span opens at, or `None` when it is detached.
    pub fn start(&self) -> Option<Offset> {
        self.bounds().map(|(start, _)| start)
    }

    /// The boundary this span closes at, past its last byte.
    pub fn end(&self) -> Option<Offset> {
        self.bounds().map(|(_, end)| end)
    }

    /// The boundaries this span lies between.
    pub fn bounds(&self) -> Option<(Offset, Offset)> {
        self.bounds
    }

    /// How many bytes this span covers, or `None` when it is detached.
    pub fn len(&self) -> Option<usize> {
        self.bounds().map(|(start, end)| end - start)
    }

    /// Reports whether this span covers no bytes, as the virtual marker tokens
    /// do. Detached syntax covers no bytes either.
    pub fn is_empty(&self) -> bool {
        self.len().unwrap_or(0) == 0
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
    /// span at the deletion point rather than becoming [`Self::Detached`],
    /// because it still records where the syntax used to be.
    pub fn shifted(&self, edit: &TextEdit) -> Self {
        match self.bounds() {
            None => Self::Detached,
            Some((start, end)) => Self::new(edit.shift(start), edit.shift(end)),
        }
    }

    /// The smallest span covering both, ignoring detached operands.
    pub fn join(&self, other: Self) -> Self {
        match (self.bounds(), other.bounds()) {
            (None, _) => other,
            (_, None) => self.clone(),
            (Some((start, end)), Some((start2, end2))) => {
                let start = start.min(start2);
                let end = end.max(end2);
                let bounds = Some((start, end));
                let source = common_source(self.source.as_ref(), other.source.as_ref(), start, end);
                Self { bounds, source }
            }
        }
    }

    /// The overlap of both, or `Detached` when they do not overlap.
    pub fn meet(&self, other: Self) -> Self {
        match (self.bounds(), other.bounds()) {
            (Some((start, end)), Some((start2, end2))) => {
                let start = start.max(start2);
                let end = end.min(end2);
                if start < end {
                    let source =
                        common_source(self.source.as_ref(), other.source.as_ref(), start, end);
                    Self {
                        bounds: Some((start, end)),
                        source,
                    }
                } else {
                    Self::Detached
                }
            }
            _ => Self::Detached,
        }
    }

    /// Joins every span in an iterator, ignoring detached spans.
    pub fn join_all(spans: impl IntoIterator<Item = Span>) -> Self {
        spans
            .into_iter()
            .reduce(|left, right| left.join(right))
            .unwrap_or_default()
    }
}

/// Indexes the buffer, panicking when the span does not name text in it.
///
/// This is legal despite `Index` and `str` both being foreign because [`Span`]
/// is local, and it cannot overlap `str`'s own
/// `impl<I: SliceIndex<str>> Index<I> for str` because `Span` does not implement
/// `SliceIndex`.
impl Index<Span> for str {
    type Output = str;

    fn index(&self, span: Span) -> &str {
        span.bounds()
            .and_then(|(start, end)| self.get(start..end))
            .unwrap_or_else(|| panic!("{span:?} does not name text in this buffer"))
    }
}

impl Index<&Span> for str {
    type Output = str;

    fn index(&self, span: &Span) -> &str {
        span.bounds()
            .and_then(|(start, end)| self.get(start..end))
            .unwrap_or_else(|| panic!("{span:?} does not name text in this buffer"))
    }
}

/// `String` needs its own implementation rather than reaching `str`'s through
/// deref: it already has an `Index` impl of its own, so operator resolution stops
/// there instead of falling through.
impl Index<Span> for String {
    type Output = str;

    fn index(&self, span: Span) -> &str {
        self.as_str().index(span)
    }
}

impl Index<&Span> for String {
    type Output = str;

    fn index(&self, span: &Span) -> &str {
        self.as_str().index(span)
    }
}

/// A syntax value that reports its source extent.
pub trait Spanned {
    /// Returns the smallest span covering this value.
    fn span(&self) -> Span;
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

impl<A: Spanned, B: Spanned> Spanned for (A, B) {
    fn span(&self) -> Span {
        self.0.span().join(self.1.span())
    }
}

impl<T: Spanned> Spanned for &[T] {
    fn span(&self) -> Span {
        Span::join_all(self.iter().map(Spanned::span))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The span of `text` starting at `start`, as the lexer would emit it:
    /// boundaries around the text, so `end` is one past its last byte.
    fn span_of(start: Offset, text: &str) -> Span {
        Span::new(start, start + text.len())
    }

    #[test]
    fn boundaries_are_gaps_so_adjacent_spans_never_own_the_same_position() {
        // `a+b`: the gap at byte 1 closes `a` and opens `+`.
        let a = span_of(0, "a");
        let plus = span_of(1, "+");

        assert!(!a.contains(1));
        assert!(plus.contains(1));
        // Both touch it, which is what a cursor query wants.
        assert!(a.touches(1));
        assert!(plus.touches(1));
    }

    #[test]
    fn a_marker_token_contains_nothing_but_touches_its_own_position() {
        let eof = Span::from_offset(3);

        assert!(eof.is_empty());
        assert!(!eof.contains(3));
        assert!(eof.touches(3));
    }

    /// An insertion of width `d` at `s` sends `x` to `x + (x >= s) * d`, and
    /// nothing else about it varies — there is no side to choose.
    #[test]
    fn an_insertion_moves_exactly_the_offsets_at_or_after_it() {
        let insert = TextEdit::insertion(3, "xy");

        assert_eq!(insert.shift(2), 2);
        // At the insertion point, so it travels — which is what a marker token
        // at end of input wants, staying past everything typed at it.
        assert_eq!(insert.shift(3), 5);
        assert_eq!(insert.shift(4), 6);
    }

    /// Shifting is monotone, so the two ends of a span can never cross and a
    /// zero-width span stays zero-width without needing a special case.
    #[test]
    fn shifting_never_inverts_a_span() {
        let edits = [TextEdit::insertion(3, "xy"), TextEdit::deletion(2, 5)];

        for edit in &edits {
            for start in 0..8 {
                for end in start..8 {
                    let shifted = Span::new(start, end).shifted(edit);
                    assert_ne!(shifted, Span::Detached, "{start}..{end} under {edit:?}");
                }
            }
            assert!(Span::from_offset(3).shifted(edit).is_empty());
        }
    }

    #[test]
    fn insertion_at_a_span_boundary_moves_it_or_extends_the_span() {
        let span = span_of(2, "xy");

        // At byte 2, the span's own start: both ends clear the threshold, so the
        // span moves whole and the new text lands before it.
        assert_eq!(
            span.shifted(&TextEdit::insertion(2, "AB")),
            span_of(4, "xy")
        );
        // At byte 4, the span's own closing boundary: only the end clears it, so
        // the span extends to keep reaching what now follows.
        assert_eq!(span.shifted(&TextEdit::insertion(4, "AB")), Span::new(2, 6));
    }

    #[test]
    fn insertion_strictly_inside_a_span_extends_it() {
        let span = span_of(0, "abcd");

        assert_eq!(
            span.shifted(&TextEdit::insertion(2, "efg")),
            span_of(0, "abcdefg")
        );
    }

    #[test]
    fn text_after_a_deletion_moves_back_and_covered_text_collapses() {
        let deleted = TextEdit::deletion(1, 3);

        // Wholly after: shifts back by the deleted width.
        assert_eq!(span_of(4, "z").shifted(&deleted), span_of(2, "z"));
        // Wholly before: untouched.
        assert_eq!(span_of(0, "a").shifted(&deleted), span_of(0, "a"));
        // Wholly covered: collapses to where it used to be, not detached.
        let covered = span_of(1, "xy").shifted(&deleted);
        assert_eq!(covered, Span::from_offset(1));
        assert!(covered.is_empty());
    }

    #[test]
    fn a_detached_span_is_unaffected_by_edits() {
        assert_eq!(
            Span::detached().shifted(&TextEdit::insertion(0, "abc")),
            Span::detached()
        );
    }

    /// Offsets near the top of the range are ordinary buffer positions, so
    /// shifting must stay within the edit's own width rather than summing two
    /// absolute offsets.
    #[test]
    fn shifting_late_offsets_does_not_overflow() {
        let span = Span::new(usize::MAX - 4, usize::MAX - 3);
        // Two bytes inserted far below the span.
        let insert = TextEdit::insertion(1, "ab");

        // The displacement is two bytes. Summing the absolute offsets instead
        // would compute `(usize::MAX - 4) + 3`, overflowing.
        assert_eq!(
            span.shifted(&insert),
            Span::new(usize::MAX - 2, usize::MAX - 1)
        );
    }

    /// `shift` moves its argument whenever the edit has any width, so leaving
    /// earlier text alone rests entirely on its guards.
    #[test]
    fn text_before_an_edit_never_moves() {
        let edits = [TextEdit::insertion(8, "AB"), TextEdit::deletion(8, 10)];
        let earlier = [
            span_of(5, "ab"),
            Span::new(0, 2),
            // Ends just short of the edit. A span ending *exactly* at an
            // insertion extends instead, which
            // `insertion_at_a_span_boundary_moves_it_or_extends_the_span`
            // covers.
            Span::new(6, 7),
            // A zero-width marker before the edit.
            Span::from_offset(1),
        ];

        for edit in &edits {
            for span in &earlier {
                assert_eq!(span.shifted(edit), *span, "{span:?} moved under {edit:?}");
            }
        }
    }

    /// Inserting at a span's own start carries the whole span forward: both
    /// endpoints move by the edit's width, so the span keeps its length and the
    /// new text lands outside it.
    #[test]
    fn inserting_at_a_span_start_moves_both_endpoints() {
        let span = Span::new(2, 6);
        let shifted = span.shifted(&TextEdit::insertion(2, "xyz"));

        assert_eq!(shifted, Span::new(5, 9));
        assert_eq!(shifted.len(), span.len());
    }

    #[test]
    fn an_inverted_pair_collapses_rather_than_becoming_a_range() {
        assert_eq!(Span::new(5, 2), Span::Detached);
    }

    #[test]
    #[should_panic(expected = "deletion start must not exceed its end")]
    fn an_inverted_deletion_is_rejected_at_construction() {
        TextEdit::deletion(3, 1);
    }

    #[test]
    fn adjacent_spans_do_not_have_an_overlap() {
        assert_eq!(Span::new(0, 1).meet(Span::new(1, 2)), Span::Detached);
    }

    #[test]
    fn a_span_indexes_the_buffer_it_came_from() {
        let source = "quotient ring";

        assert_eq!(&source[span_of(9, "ring")], "ring");
        // A marker token names the empty string at its own position.
        assert_eq!(&source[Span::from_offset(9)], "");
        assert_eq!(Span::new(0, source.len()), Span::new(0, 13));

        // Spans that name no text in this buffer are refused rather than guessed.
    }

    #[test]
    #[should_panic(expected = "does not name text in this buffer")]
    fn indexing_with_a_detached_span_is_a_bug_rather_than_an_empty_slice() {
        let _ = &"ring"[Span::detached()];
    }

    /// The law tying the two halves together: an edit acts on the buffer and on
    /// the span, and a span disjoint from the edit still names the same text
    /// afterwards. This is what `shifted` is *for*, so it is checked against
    /// every edit shape rather than a single hand-picked case.
    #[test]
    fn an_edit_acts_on_buffer_and_span_alike() {
        let source = "quotient ring R";
        let ring = span_of(9, "ring");

        let disjoint = [
            // Before the span.
            TextEdit::insertion(0, "the "),
            TextEdit::deletion(0, 9),
            // Exactly at its opening boundary: the span moves whole, so it
            // still names the same text. An insertion at the *closing*
            // boundary extends the span instead and is not disjoint.
            TextEdit::insertion(9, "graded "),
            // After the span.
            TextEdit::insertion(15, " = QQ"),
            TextEdit::deletion(13, 15),
            // A multi-byte character, to catch byte/char confusion.
            TextEdit::insertion(0, "ℤ "),
        ];

        for edit in &disjoint {
            let edited = edit.apply(source).expect("edit lands inside the buffer");
            assert_eq!(
                &edited[ring.shifted(edit)],
                &source[&ring],
                "{edit:?} broke the span"
            );
        }
    }

    /// The general form of the same law: for an edit landing *inside* a span,
    /// the shifted span names the edited text, so the edit acts on the slice too.
    #[test]
    fn an_edit_inside_a_span_acts_on_the_text_the_span_names() {
        let source = "quotient ring";
        let ring = span_of(9, "ring");
        let edit = TextEdit::insertion(11, "AB");

        let edited = edit.apply(source).expect("edit lands inside the buffer");
        // The span still names its own text, with the insertion carried into it.
        assert_eq!(&edited[ring.shifted(&edit)], "riABng");
        // Which is exactly the edit acting on the original slice, rebased to it.
        let inside = TextEdit::insertion(11 - ring.start().unwrap(), "AB");
        assert_eq!(inside.apply(&source[ring]).unwrap(), "riABng");
    }

    /// Text typed at a span's closing boundary is absorbed by it. This is what
    /// an unclosed group wants: its span keeps reaching the end of whatever has
    /// been typed since, without anyone re-deriving it.
    #[test]
    fn a_span_absorbs_text_appended_at_its_closing_boundary() {
        let source = "f(x";
        // An unclosed group, running from its opening delimiter to end of input.
        let group = Span::new(1, source.len());
        assert_eq!(&source[&group], "(x");

        let typed = TextEdit::insertion(3, ", y");
        let edited = typed.apply(source).expect("edit lands inside the buffer");

        assert_eq!(&edited[group.shifted(&typed)], "(x, y");
        // Still opens at the delimiter and still reaches end of input.
        assert_eq!(group.shifted(&typed), Span::new(1, edited.len()));
    }
}
