//! Retained native parsing over independently cached source cells.
//!
//! [`NativeRealtimeParser`] treats a confirmed cell boundary as a restart
//! point. Edits invalidate the cell containing the edit and only the suffix
//! whose framing changes. The final EOF boundary remains speculative, so an
//! append reopens that cell while leaving every confirmed predecessor intact.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::ops::Range;
use std::sync::Arc;

use crate::cell_stream::{end_span, ends_with_cell_semicolon, frame_recovering};
use crate::native::{
    NativeCellState, NativeParseError, incomplete_lex_error, incomplete_parse_error, parse_raw_cell,
};
use crate::{
    Cell, CellStream, LexError, Source, Span, Spanned, TextEdit, TokenStream, lex_str_recovering,
};

/// The structural boundary following one retained native cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeCellBoundary {
    /// A newline or semicolon confirmed the end of this cell.
    EOC,
    /// The current end of input provisionally ends otherwise complete syntax.
    EOF,
    /// The cell still requires more input and therefore has no closing boundary.
    Unbounded,
}

/// A source edit that does not fit the current realtime buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeRealtimeEditError;

impl Display for NativeRealtimeEditError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("source edit is outside the current realtime buffer")
    }
}

impl Error for NativeRealtimeEditError {}

/// One independently retained cell in a [`NativeRealtimeParser`].
///
/// Spans inside the token and typed representations are local to the exact
/// source returned by [`Self::source`]. [`Self::document_range`] locates that
/// immutable cell source in the current document. This separation permits an
/// unchanged cell to survive edits above it without rewriting every leaf span.
#[derive(Debug)]
pub struct NativeRealtimeCell {
    document_range: Range<usize>,
    source: Span,
    tokens: TokenStream,
    syntax: Option<Cell>,
    lexer_diagnostics: Vec<LexError>,
    parse_error: Option<NativeParseError>,
    state: NativeCellState,
    boundary: NativeCellBoundary,
}

impl NativeRealtimeCell {
    /// Returns this cell's exact source spelling.
    ///
    /// # Panics
    ///
    /// Panics if the internally retained source span cannot resolve its text.
    pub fn source(&self) -> &str {
        self.source
            .text()
            .expect("realtime cell source spans must resolve to text")
    }

    /// Returns this cell's half-open byte range in the current document.
    pub fn document_range(&self) -> Range<usize> {
        self.document_range.clone()
    }

    /// Returns the tokenized cell, including inferred adjacency punctuation.
    pub fn tokens(&self) -> &TokenStream {
        &self.tokens
    }

    /// Returns recovered typed syntax when the native parser could construct it.
    pub fn syntax(&self) -> Option<&Cell> {
        self.syntax.as_ref()
    }

    /// Returns recoverable lexer diagnostics associated with this cell.
    pub fn lexer_diagnostics(&self) -> &[LexError] {
        &self.lexer_diagnostics
    }

    /// Returns the native parser failure associated with this cell.
    pub fn parse_error(&self) -> Option<&NativeParseError> {
        self.parse_error.as_ref()
    }

    /// Returns whether this cell is complete, incomplete, or invalid.
    pub fn state(&self) -> NativeCellState {
        self.state
    }

    /// Returns the structural boundary currently following this cell.
    pub fn boundary(&self) -> NativeCellBoundary {
        self.boundary
    }
}

impl Spanned for NativeRealtimeCell {
    fn try_span(&self) -> Option<Span> {
        Some(self.source.clone())
    }
}

/// A pseudo-incremental native parser retaining independently framed cells.
///
/// Appending to an EOF-complete cell reparses only that speculative cell.
/// Confirmed cells above it retain their token and typed allocations. An edit
/// inside an earlier cell reparses strictly forwards; after cell framing
/// stabilizes, an unchanged suffix is reused in its entirety. Adjacency and
/// cell boundaries are regenerated together for every reparsed region.
#[derive(Debug)]
pub struct NativeRealtimeParser {
    source: String,
    cells: Vec<NativeRealtimeCell>,
    reparsed_cells: Range<usize>,
}

impl Default for NativeRealtimeParser {
    fn default() -> Self {
        Self::new("")
    }
}

impl NativeRealtimeParser {
    /// Parses and retains an initial source snapshot.
    pub fn new(source: impl Into<String>) -> Self {
        let source = source.into();
        let cells = analyze_region(&source, 0);
        let reparsed_cells = 0..cells.len();
        Self {
            source,
            cells,
            reparsed_cells,
        }
    }

    /// Applies an edit and reparses only the forward cell region it invalidates.
    ///
    /// The operation is transactional when the edit is outside the source or
    /// crosses a UTF-8 character boundary. Invalid Macaulay2 syntax is retained
    /// as [`NativeCellState::Invalid`] rather than failing the edit operation.
    ///
    /// # Errors
    ///
    /// Returns [`NativeRealtimeEditError`] when `edit` cannot be applied to the
    /// current source buffer.
    pub fn apply_edit(
        &mut self,
        edit: &TextEdit,
    ) -> Result<NativeCellState, NativeRealtimeEditError> {
        let source = edit.apply(&self.source).ok_or(NativeRealtimeEditError)?;
        let first_changed = self.first_affected_cell(edit);
        let region_start = self
            .cells
            .get(first_changed)
            .map_or(self.source.len(), |cell| cell.document_range.start);
        let reusable_suffix = self.reusable_suffix(edit, &source, first_changed, region_start);
        let rebuild_end = reusable_suffix
            .map(|(_, start)| start)
            .unwrap_or(source.len());
        let rebuilt = analyze_region(&source[region_start..rebuild_end], region_start);

        let mut previous = std::mem::take(&mut self.cells);
        let previous_suffix = previous.split_off(first_changed);
        let rebuilt_len = rebuilt.len();
        previous.extend(rebuilt);
        let reuse_from = reusable_suffix
            .map(|(index, _)| index.saturating_sub(first_changed))
            .unwrap_or(previous_suffix.len());
        for mut retained in previous_suffix.into_iter().skip(reuse_from) {
            retained.document_range = shifted_range(edit, retained.document_range);
            previous.push(retained);
        }

        self.source = source;
        self.cells = previous;
        self.reparsed_cells = first_changed..first_changed + rebuilt_len;
        Ok(self.state())
    }

    /// Appends source text at the current EOF.
    ///
    /// # Panics
    ///
    /// Panics if the parser's retained source length is not a valid insertion
    /// boundary, which would violate its internal UTF-8 source invariant.
    pub fn append(&mut self, source: impl Into<String>) -> NativeCellState {
        let edit = TextEdit::insertion(self.source.len(), source);
        self.apply_edit(&edit)
            .expect("an insertion at the current EOF is always valid")
    }

    /// Returns the complete current source buffer.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns every retained complete, incomplete, or invalid cell.
    pub fn cells(&self) -> &[NativeRealtimeCell] {
        &self.cells
    }

    /// Returns the range of newly analyzed cells after the most recent edit.
    ///
    /// Cells outside this range were retained from the preceding snapshot.
    pub fn reparsed_cells(&self) -> Range<usize> {
        self.reparsed_cells.clone()
    }

    /// Returns the aggregate state of the current source snapshot.
    pub fn state(&self) -> NativeCellState {
        if self
            .cells
            .iter()
            .any(|cell| cell.state == NativeCellState::Invalid)
        {
            NativeCellState::Invalid
        } else if self
            .cells
            .last()
            .is_some_and(|cell| cell.state == NativeCellState::Incomplete)
        {
            NativeCellState::Incomplete
        } else if self.cells.is_empty() {
            NativeCellState::Empty
        } else {
            NativeCellState::Complete
        }
    }

    /// Projects the retained cells as the parser-facing tokenized stream.
    ///
    /// The projection includes an incomplete or invalid tail so a parser or
    /// macro can inspect the exact current tokenization.
    pub fn cell_stream(&self) -> CellStream {
        CellStream::from_cells(self.cells.iter().map(|cell| cell.tokens.clone()))
    }

    fn first_affected_cell(&self, edit: &TextEdit) -> usize {
        let start = match edit {
            TextEdit::Insertion { at, .. } => *at,
            TextEdit::Deletion { start, .. } => *start,
        };
        self.cells
            .iter()
            .position(|cell| {
                start < cell.document_range.end
                    || (start == cell.document_range.end
                        && cell.boundary != NativeCellBoundary::EOC)
            })
            .unwrap_or(self.cells.len())
    }

    fn reusable_suffix(
        &self,
        edit: &TextEdit,
        source: &str,
        first_changed: usize,
        region_start: usize,
    ) -> Option<(usize, usize)> {
        for index in first_changed.saturating_add(1)..self.cells.len() {
            let old_start = self.cells[index].document_range.start;
            let new_start = shifted_offset(edit, old_start);
            if new_start < region_start
                || self.source.get(old_start..) != source.get(new_start..)
                || !ends_at_confirmed_boundary(&source[region_start..new_start])
            {
                continue;
            }
            return Some((index, new_start));
        }
        None
    }
}

fn shifted_range(edit: &TextEdit, range: Range<usize>) -> Range<usize> {
    shifted_offset(edit, range.start)..shifted_offset(edit, range.end)
}

fn shifted_offset(edit: &TextEdit, offset: usize) -> usize {
    match edit {
        TextEdit::Insertion { at, text } if offset >= *at => offset + text.len(),
        TextEdit::Insertion { .. } => offset,
        TextEdit::Deletion { start, .. } if offset <= *start => offset,
        TextEdit::Deletion { start, end } if offset <= *end => *start,
        TextEdit::Deletion { start, end } => offset - (end - start),
    }
}

fn ends_at_confirmed_boundary(source: &str) -> bool {
    if source.trim().is_empty() {
        return true;
    }
    let Ok(recovered) = lex_str_recovering(source) else {
        return false;
    };
    let framed = frame_recovering(recovered.value);
    if framed.incomplete.is_some() {
        return false;
    }
    let Some(last) = framed.cells.last() else {
        return true;
    };
    ends_with_cell_semicolon(last) || source.ends_with('\n')
}

fn analyze_region(source: &str, document_start: usize) -> Vec<NativeRealtimeCell> {
    let recovered = match lex_str_recovering(source) {
        Ok(recovered) => recovered,
        Err(error) => {
            if source.is_empty() {
                return Vec::new();
            }
            return vec![cell_from_lex_error(
                source,
                document_start..document_start + source.len(),
                error,
            )];
        }
    };
    let framed = frame_recovering(recovered.value);
    let complete_count = framed.cells.len();
    let has_incomplete = framed.incomplete.is_some();
    let mut start = 0;
    let mut cells = Vec::with_capacity(complete_count + usize::from(has_incomplete));

    for (index, tokens) in framed.cells.into_iter().enumerate() {
        let has_following = index + 1 < complete_count || has_incomplete;
        let mut end = end_span(&tokens)
            .end()
            .unwrap_or(source.len())
            .min(source.len());
        if !has_following && end < source.len() {
            end = source.len();
        }
        let boundary = if has_following
            || ends_with_cell_semicolon(&tokens)
            || source[start..end].ends_with('\n')
        {
            NativeCellBoundary::EOC
        } else {
            NativeCellBoundary::EOF
        };
        cells.push(analyze_cell(
            &source[start..end],
            document_start + start..document_start + end,
            boundary,
        ));
        start = end;
    }

    if framed.incomplete.is_some() {
        cells.push(analyze_cell(
            &source[start..],
            document_start + start..document_start + source.len(),
            NativeCellBoundary::Unbounded,
        ));
    }
    cells
}

fn analyze_cell(
    source: &str,
    document_range: Range<usize>,
    boundary: NativeCellBoundary,
) -> NativeRealtimeCell {
    let recovered = match lex_str_recovering(source) {
        Ok(recovered) => recovered,
        Err(error) => return cell_from_lex_error(source, document_range, error),
    };
    let diagnostics = recovered.diagnostics;
    let framed = frame_recovering(recovered.value);
    let mut streams = framed.cells;
    streams.extend(framed.incomplete);
    let tokens = match streams.len() {
        0 => TokenStream::new(),
        1 => streams.pop().expect("one framed stream is present"),
        _ => CellStream::from_cells(streams).into_token_stream(),
    };
    let source_owner = tokens.source().unwrap_or_else(|| Source::new(source));
    let source_span = Span::ranged(source_owner, 0..source.len());

    let (syntax, parse_error, state) = match parse_raw_cell(tokens.clone()) {
        Ok(syntax) => {
            let state = if boundary == NativeCellBoundary::Unbounded || !diagnostics.is_empty() {
                NativeCellState::Incomplete
            } else {
                NativeCellState::Complete
            };
            (syntax, None, state)
        }
        Err(error) => {
            let state = if boundary != NativeCellBoundary::EOC && incomplete_parse_error(&error) {
                NativeCellState::Incomplete
            } else {
                NativeCellState::Invalid
            };
            (None, Some(error), state)
        }
    };

    NativeRealtimeCell {
        document_range,
        source: source_span,
        tokens,
        syntax,
        lexer_diagnostics: diagnostics,
        parse_error,
        state,
        boundary,
    }
}

fn cell_from_lex_error(
    source: &str,
    document_range: Range<usize>,
    error: LexError,
) -> NativeRealtimeCell {
    let state = if incomplete_lex_error(error.kind()) {
        NativeCellState::Incomplete
    } else {
        NativeCellState::Invalid
    };
    let source_owner = Arc::clone(error.span().source());
    NativeRealtimeCell {
        document_range,
        source: Span::ranged(source_owner, 0..source.len()),
        tokens: TokenStream::new(),
        syntax: None,
        lexer_diagnostics: vec![error],
        parse_error: None,
        state,
        boundary: NativeCellBoundary::Unbounded,
    }
}
