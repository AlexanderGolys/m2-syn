//! Structural cell boundaries over raw Macaulay2 token trees.
//!
//! [`TokenStream`] retains lexical atoms, comments, source spans, and nested
//! delimiter groups. [`CellStream`] is the next representation in the parsing
//! pipeline: a sequence of token streams that can each be parsed independently.
//! End-of-cell and end-of-file are properties of that sequence rather than
//! virtual [`TokenTree`] variants.
//!
//! Framing recognizes top-level semicolons and physical newlines at which the
//! current operator, clause, and delimiter state can end. It also inserts the
//! one non-lexical atom required by the native parser: zero-width
//! [`CellTokenTree::Adj`]. Framing does not otherwise parse or validate the
//! Macaulay2 grammar.
//!
//! [`CellStream`] behaves as a borrowed slice of [`CellTokenStream`] values. It
//! can be indexed and iterated through the standard slice APIs, and consuming
//! it yields owned cells. Clones share their backing allocation until mutation.
//!
//! # Examples
//!
//! ```
//! use m2_syn::{Tokenize, lex_str};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let cells = lex_str("x = 1;\ny = 2")?.to_cell_stream()?;
//!
//! assert_eq!(cells.len(), 2);
//! assert_eq!(cells[0].to_string(), "x = 1;");
//! assert_eq!(cells.iter().map(ToString::to_string).collect::<Vec<_>>(), [
//!     "x = 1;",
//!     "y = 2",
//! ]);
//! # Ok(())
//! # }
//! ```

use std::error::Error;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::mem::take;
use std::ops::Deref;
use std::slice::Iter;
use std::sync::Arc;
use std::vec::IntoIter;

use crate::lexer::{canonical_spelling, is_quote_specifier, newline_ends_cell};
use crate::token_stream::{layout_between, write_gap, write_stream};
use crate::{
    Delimiter, DelimiterKind, IdentToken, Lex, LiteralToken, Punct, Source, Span, Spanned,
    TOKEN_PARSE_RULES, TokenGroup, TokenParseRule, TokenStream, TokenTree,
};

#[derive(Debug, Clone, Default)]
/// Internal state for CellTokenStreamData.
struct CellTokenStreamData {
    /// The trees.
    trees: Vec<CellTokenTree>,
    /// The source.
    source: Option<Arc<Source>>,
}

impl PartialEq for CellTokenStreamData {
    fn eq(&self, other: &Self) -> bool {
        self.trees == other.trees
            && match (&self.source, &other.source) {
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                (None, None) => true,
                _ => false,
            }
    }
}

impl Eq for CellTokenStreamData {}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One token tree in a framed Macaulay2 cell.
///
/// Lexical atoms retain their raw token types. Delimiter groups recursively
/// contain [`CellTokenStream`], while [`Self::Adj`] represents the inferred
/// application boundary that does not exist in [`TokenTree`].
pub enum CellTokenTree {
    /// A recursively framed delimiter group.
    Group(CellTokenGroup),
    /// A lexical literal.
    Literal(LiteralToken),
    /// Lexical punctuation.
    Punct(Punct),
    /// A lexical identifier or keyword.
    Ident(IdentToken),
    /// Inferred same-line adjacency at its zero-width source position.
    Adj(Span),
}

impl CellTokenTree {
    /// Returns the lexical spelling of a flat token or the empty adjacency spelling.
    pub fn spelling(&self) -> Option<&str> {
        match self {
            Self::Group(_) => None,
            Self::Literal(token) => Some(token.text()),
            Self::Punct(token) => Some(token.text()),
            Self::Ident(token) => Some(token.text()),
            Self::Adj(_) => Some(""),
        }
    }

    /// Creates the value from augmented.
    fn from_augmented(tree: TokenTree) -> Self {
        match tree {
            TokenTree::Group(group) => Self::Group(CellTokenGroup::from_augmented(group)),
            TokenTree::Literal(token) => Self::Literal(token),
            TokenTree::Punct(token) if token.text().is_empty() => Self::Adj(token.expect_span()),
            TokenTree::Punct(token) => Self::Punct(token),
            TokenTree::Ident(token) => Self::Ident(token),
        }
    }

    /// Converts the value into lexical.
    fn into_lexical(self) -> Option<TokenTree> {
        match self {
            Self::Group(group) => Some(TokenTree::Group(group.into_lexical())),
            Self::Literal(token) => Some(TokenTree::Literal(token)),
            Self::Punct(token) => Some(TokenTree::Punct(token)),
            Self::Ident(token) => Some(TokenTree::Ident(token)),
            Self::Adj(_) => None,
        }
    }

    /// Converts the value into parser token.
    fn into_parser_token(self) -> TokenTree {
        match self {
            Self::Group(group) => TokenTree::Group(group.into_parser_group()),
            Self::Literal(token) => TokenTree::Literal(token),
            Self::Punct(token) => TokenTree::Punct(token),
            Self::Ident(token) => TokenTree::Ident(token),
            Self::Adj(span) => TokenTree::Punct(Punct::from_span(span)),
        }
    }
}

impl Spanned for CellTokenTree {
    fn try_span(&self) -> Option<Span> {
        match self {
            Self::Group(token) => token.try_span(),
            Self::Literal(token) => token.try_span(),
            Self::Punct(token) => token.try_span(),
            Self::Ident(token) => token.try_span(),
            Self::Adj(span) => Some(span.clone()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A delimiter group whose contents have passed through cell framing.
pub struct CellTokenGroup {
    /// The delimiter.
    delimiter: Delimiter,
    /// The stream.
    stream: CellTokenStream,
}

impl CellTokenGroup {
    /// Returns the group's delimiter data.
    pub fn delimiter(&self) -> &Delimiter {
        &self.delimiter
    }

    /// Returns the group's delimiter family.
    pub fn delim_kind(&self) -> DelimiterKind {
        self.delimiter.kind
    }

    /// Returns the recursively framed contents.
    pub fn stream(&self) -> &CellTokenStream {
        &self.stream
    }

    /// Consumes the group and returns its recursively framed contents.
    pub fn into_stream(self) -> CellTokenStream {
        self.stream
    }

    /// Creates the value from augmented.
    fn from_augmented(group: TokenGroup) -> Self {
        let (delimiter, stream) = group.into_parts();
        Self {
            delimiter,
            stream: CellTokenStream::from_augmented(stream),
        }
    }

    /// Converts the value into lexical.
    fn into_lexical(self) -> TokenGroup {
        TokenGroup::new(self.delimiter, self.stream.into_token_stream())
    }

    /// Converts the value into parser group.
    fn into_parser_group(self) -> TokenGroup {
        TokenGroup::new(self.delimiter, self.stream.into_parser_token_stream())
    }
}

impl Spanned for CellTokenGroup {
    fn try_span(&self) -> Option<Span> {
        self.delimiter.try_span()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// The framed tokens of one independently parseable Macaulay2 cell.
///
/// Unlike a lexical [`TokenStream`], this representation has passed through
/// cell framing. It may contain inferred zero-width adjacency punctuation at
/// every delimiter depth, and its surrounding [`CellStream`] records the
/// structural boundary after it.
///
/// The stream dereferences to `[CellTokenTree]` for parser access. Converting
/// it back into a [`TokenStream`] removes inferred adjacency recursively,
/// because adjacency is not a lexical token.
pub struct CellTokenStream(Arc<CellTokenStreamData>);

impl CellTokenStream {
    /// Creates a constant-time fork over the same framed token storage.
    ///
    /// Mutating either fork internally detaches only the modified path.
    pub fn fork(&self) -> Self {
        self.clone()
    }

    /// Consumes this framed cell and returns its lexical token representation.
    ///
    /// Inferred adjacency is removed recursively. Delimiter groups, source
    /// spans, and physical spellings are preserved.
    pub fn into_token_stream(self) -> TokenStream {
        let CellTokenStreamData { trees, source } = self.into_data();
        let trees = trees.into_iter().filter_map(CellTokenTree::into_lexical);
        TokenStream::from_parts(trees.collect(), source)
    }

    /// Returns this framed cell's lexical token representation.
    ///
    /// This is the borrowing counterpart of [`Self::into_token_stream`].
    pub fn to_token_stream(&self) -> TokenStream {
        self.clone().into_token_stream()
    }

    /// Borrows the shared backing data.
    fn data(&self) -> &CellTokenStreamData {
        &self.0
    }

    /// Converts the value into data.
    fn into_data(self) -> CellTokenStreamData {
        Arc::try_unwrap(self.0).unwrap_or_else(|data| (*data).clone())
    }

    /// Creates the value from augmented.
    pub(crate) fn from_augmented(tokens: TokenStream) -> Self {
        let (trees, source) = tokens.into_parts();
        Self(Arc::new(CellTokenStreamData {
            trees: trees
                .into_iter()
                .map(CellTokenTree::from_augmented)
                .collect(),
            source,
        }))
    }

    /// Returns the associated source snapshot.
    pub(crate) fn source(&self) -> Option<Arc<Source>> {
        self.data().source.clone().or_else(|| {
            self.iter()
                .find_map(Spanned::try_span)
                .map(|span| Arc::clone(span.source()))
        })
    }

    #[cfg(test)]
    pub(crate) fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Converts the value into parser token stream.
    pub(crate) fn into_parser_token_stream(self) -> TokenStream {
        let CellTokenStreamData { trees, source } = self.into_data();
        let trees = trees.into_iter().map(CellTokenTree::into_parser_token);
        TokenStream::from_parts(trees.collect(), source)
    }

    /// Creates the value from framed cells.
    pub(crate) fn from_framed_cells(cells: impl IntoIterator<Item = CellTokenStream>) -> Self {
        let mut tokens = TokenStream::new();
        for cell in cells {
            tokens.extend([cell.into_parser_token_stream()]);
        }
        Self::from_augmented(tokens)
    }
}

impl Deref for CellTokenStream {
    type Target = [CellTokenTree];

    fn deref(&self) -> &Self::Target {
        &self.data().trees
    }
}

impl AsRef<[CellTokenTree]> for CellTokenStream {
    fn as_ref(&self) -> &[CellTokenTree] {
        self
    }
}

impl Spanned for CellTokenStream {
    fn try_span(&self) -> Option<Span> {
        Span::join_all(self.iter().filter_map(Spanned::try_span))
    }
}

impl Display for CellTokenStream {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        self.to_token_stream().fmt(formatter)
    }
}

impl IntoIterator for CellTokenStream {
    type Item = CellTokenTree;
    type IntoIter = IntoIter<CellTokenTree>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_data().trees.into_iter()
    }
}

impl<'syntax> IntoIterator for &'syntax CellTokenStream {
    type Item = &'syntax CellTokenTree;
    type IntoIter = Iter<'syntax, CellTokenTree>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

#[derive(Debug, Clone, Default)]
struct CellStreamData {
    cells: Vec<CellTokenStream>,
    source: Option<Arc<Source>>,
    /// The physical start of the source.
    bof: Option<Span>,
    /// The physical end of the source.
    eof: Option<Span>,
}

#[derive(Debug, Clone, Default)]
/// A sequence of independently parseable Macaulay2 token streams.
///
/// Cell boundaries are expressed by the collection itself. The boundary after
/// its final element is also the end-of-file boundary, so neither fact needs a
/// virtual token in the contained streams. Clones and [`Self::fork`] share
/// immutable backing storage, with mutations detaching through copy-on-write.
///
/// The stream dereferences to `[CellTokenStream]`, so slice operations such as
/// indexing, [`slice::iter`], [`slice::len`], and [`slice::is_empty`] are
/// available directly. Iterating `&CellStream` borrows cells; iterating an
/// owned value consumes it and yields owned cells.
pub struct CellStream(Arc<CellStreamData>);

impl PartialEq for CellStream {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for CellStream {}

impl Deref for CellStream {
    type Target = [CellTokenStream];

    fn deref(&self) -> &Self::Target {
        &self.data().cells
    }
}

impl AsRef<[CellTokenStream]> for CellStream {
    fn as_ref(&self) -> &[CellTokenStream] {
        self
    }
}

impl CellStream {
    /// Partitions a complete raw token stream into independently parseable cells.
    ///
    /// Top-level cell boundaries are retained structurally, and inferred
    /// adjacency punctuation is inserted recursively. This operation checks
    /// whether the final cell requires more input, but it does not validate the
    /// complete Macaulay2 grammar.
    ///
    /// # Errors
    ///
    /// Returns [`CellStreamError`] when the final tokens require more input
    /// before they can form an independently parseable cell, such as after an
    /// operator without its right operand or inside an unclosed delimiter.
    /// Use [`frame_recovering`] to retain completed cells alongside that tail.
    pub fn new(tokens: TokenStream) -> Result<Self, CellStreamError> {
        let source = tokens.source();
        let bof = source.as_ref().map(source_start);
        let eof = source.as_ref().map(source_end);
        let FramedCells { cells, incomplete } = frame_recovering(tokens);
        match incomplete {
            Some(tokens) => Err(CellStreamError {
                span: end_span(&tokens),
            }),
            None => Ok(Self(Arc::new(CellStreamData {
                cells,
                source,
                bof,
                eof,
            }))),
        }
    }

    /// Creates a stream from token streams already known to be complete cells.
    ///
    /// This constructor does not inspect, repartition, or insert adjacency into
    /// its input. Use [`Self::new`] or [`Tokenize::to_cell_stream`] when starting
    /// from an unframed raw stream.
    pub fn from_cells(cells: impl IntoIterator<Item = CellTokenStream>) -> Self {
        Self::from_cells_with_source(cells, None)
    }

    /// Creates the value from cells with source.
    pub(crate) fn from_cells_with_source(
        cells: impl IntoIterator<Item = CellTokenStream>,
        source: Option<Arc<Source>>,
    ) -> Self {
        Self(Arc::new(CellStreamData {
            cells: cells.into_iter().collect(),
            bof: source.as_ref().map(source_start),
            eof: source.as_ref().map(source_end),
            source,
        }))
    }

    /// Creates a constant-time fork over the same cell storage.
    ///
    /// The fork initially shares both the cell collection and every contained
    /// [`TokenStream`]. Mutating either value preserves the other and copies
    /// only the backing structures along the modified path. This is equivalent
    /// to [`Clone::clone`], with a name that emphasizes speculative use.
    pub fn fork(&self) -> Self {
        self.clone()
    }

    /// Borrows the shared backing data.
    fn data(&self) -> &CellStreamData {
        &self.0
    }

    /// Borrows the shared backing data mutably.
    fn data_mut(&mut self) -> &mut CellStreamData {
        Arc::make_mut(&mut self.0)
    }

    /// Converts the value into data.
    fn into_data(self) -> CellStreamData {
        Arc::try_unwrap(self.0).unwrap_or_else(|data| (*data).clone())
    }

    /// Consumes the cells and flattens them into one raw token stream.
    ///
    /// Cell boundaries are not emitted as tokens and therefore cannot be
    /// recovered structurally from the result alone. Original whitespace
    /// between cells remains renderable when their spans share one source.
    pub fn into_token_stream(self) -> TokenStream {
        let source = self.complete_source();
        let mut tokens = flatten_cells(self.into_data().cells);
        if let Some(source) = source {
            tokens.set_source(source);
        }
        tokens
    }

    /// Flattens borrowed cells into one raw token stream.
    ///
    /// This has the same boundary semantics as [`Self::into_token_stream`] but
    /// leaves `self` available for reuse.
    pub fn to_token_stream(&self) -> TokenStream {
        let mut tokens = flatten_cells(self.iter().cloned());
        if let Some(source) = self.complete_source() {
            tokens.set_source(source);
        }
        tokens
    }

    /// Handles complete source.
    pub(crate) fn complete_source(&self) -> Option<Arc<Source>> {
        let source = self.data().source.as_ref()?;
        let mut cursor = 0;
        for cell in self {
            if !covers_stream(cell, source, &mut cursor) {
                return None;
            }
        }
        source
            .is_layout(cursor..source.text().len())
            .then(|| Arc::clone(source))
    }

    /// Where the source starts, or, for cells that do not cover one shared
    /// source snapshot, where the first cell starts.
    pub(crate) fn physical_bof(&self) -> Span {
        if let Some(source) = self.complete_source() {
            return source_start(&source);
        }
        self.first()
            .and_then(cell_first_span)
            .and_then(|span| span.start().map(|start| span.at(start)))
            .unwrap_or_else(|| Span::for_text(""))
    }

    /// Where the source ends, or, for cells that do not cover one shared
    /// source snapshot, where the last cell ends.
    pub(crate) fn physical_eof(&self) -> Span {
        if let Some(source) = self.complete_source() {
            return source_end(&source);
        }
        self.last().map_or_else(|| Span::for_text(""), end_span)
    }

    /// Creates the value from source file.
    pub(crate) fn from_source_file(source_file: &crate::SourceFile) -> Self {
        Self(Arc::new(CellStreamData {
            cells: source_file
                .cells
                .iter()
                .map(|cell| CellTokenStream::from_augmented(cell.to_token_stream()))
                .collect(),
            source: None,
            bof: source_file.bof.try_span(),
            eof: source_file.eof.try_span(),
        }))
    }
}

/// Returns whether the value covers stream.
fn covers_stream(stream: &CellTokenStream, source: &Arc<Source>, cursor: &mut usize) -> bool {
    for token in stream.iter() {
        let covered = match token {
            CellTokenTree::Group(group) => {
                covers_span(group.delimiter().opening_span(), source, cursor)
                    && covers_stream(group.stream(), source, cursor)
                    && group
                        .delimiter()
                        .closing_span()
                        .is_none_or(|span| covers_span(span, source, cursor))
            }
            _ => covers_span(token.span(), source, cursor),
        };
        if !covered {
            return false;
        }
    }
    true
}

/// Returns whether the value covers span.
fn covers_span(span: Span, source: &Arc<Source>, cursor: &mut usize) -> bool {
    if !Arc::ptr_eq(span.source(), source) {
        return false;
    }
    let (Some(start), Some(end)) = (span.start(), span.end()) else {
        return false;
    };
    if start == end {
        return start >= *cursor;
    }
    if start < *cursor || !source.is_layout(*cursor..start) {
        return false;
    }
    *cursor = end;
    true
}

impl Display for CellStream {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        let (bof, eof) = (self.data().bof.as_ref(), self.data().eof.as_ref());
        let first = self.first().and_then(cell_first_span);
        if let (Some(bof), Some(first)) = (bof, &first)
            && let Some(layout) = layout_between(bof, first)
        {
            formatter.write_str(layout)?;
        }
        let mut previous: Option<Span> = None;
        for cell in self.iter() {
            let next = cell_first_span(cell);
            if let Some(left) = &previous {
                write_cell_boundary(formatter, left, next.as_ref())?;
            }
            write_stream(formatter, &cell.to_token_stream(), None)?;
            previous = cell_last_span(cell);
        }
        let Some(eof) = eof else {
            return Ok(());
        };
        let resume = previous.as_ref().or(bof).unwrap_or(eof);
        match layout_between(resume, eof) {
            Some(layout) => formatter.write_str(layout),
            None => write_gap(formatter, Some(resume), None).map(drop),
        }
    }
}

/// Writes the layout between two cells: the source between them when it is
/// only layout, otherwise their comments with a line break between.
fn write_cell_boundary(
    formatter: &mut Formatter<'_>,
    left: &Span,
    right: Option<&Span>,
) -> FmtResult {
    // Synthetic text is assembled without layout, so cells touching there
    // were never written on one line.
    let touching_synthetic = left.source().file().is_synthetic()
        && right.is_some_and(|right| left.end() == right.start());
    if let Some(layout) = right.and_then(|right| layout_between(left, right))
        && !touching_synthetic
    {
        return formatter.write_str(layout);
    }
    write_gap(formatter, Some(left), None)?;
    formatter.write_str("\n")?;
    write_gap(formatter, None, right).map(drop)
}

fn source_start(source: &Arc<Source>) -> Span {
    Span::ranged(Arc::clone(source), 0..0)
}

fn source_end(source: &Arc<Source>) -> Span {
    let end = source.text().len();
    Span::ranged(Arc::clone(source), end..end)
}

/// Handles cell first span.
fn cell_first_span(tokens: &CellTokenStream) -> Option<Span> {
    tokens.iter().next().map(Spanned::span)
}

/// Handles cell last span.
fn cell_last_span(tokens: &CellTokenStream) -> Option<Span> {
    tokens.iter().next_back().map(token_end_span)
}

impl IntoIterator for CellStream {
    type Item = CellTokenStream;
    type IntoIter = IntoIter<CellTokenStream>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_data().cells.into_iter()
    }
}

impl<'syntax> IntoIterator for &'syntax CellStream {
    type Item = &'syntax CellTokenStream;
    type IntoIter = Iter<'syntax, CellTokenStream>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl FromIterator<CellTokenStream> for CellStream {
    fn from_iter<T: IntoIterator<Item = CellTokenStream>>(iter: T) -> Self {
        Self::from_cells(iter)
    }
}

/// Handles flatten cells.
fn flatten_cells(cells: impl IntoIterator<Item = CellTokenStream>) -> TokenStream {
    let mut output = TokenStream::new();
    for cell in cells {
        output.extend([cell.into_token_stream()]);
    }
    output
}

#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
/// An incomplete final cell encountered while framing a raw token stream.
///
/// The error's [`Spanned`] implementation points at the position where more
/// input is required. It describes incompleteness only; malformed syntax that
/// already has a structural boundary is diagnosed by a parser backend.
pub struct CellStreamError {
    /// The span.
    span: Span,
}

impl Display for CellStreamError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        formatter.write_str("input ends inside an incomplete cell")
    }
}

impl Error for CellStreamError {}

/// Converts syntax or raw tokens into structural cells.
///
/// This boundary is intentionally distinct from [`Lex`]: most macros should
/// preserve cell partitioning, while function-like expansion may explicitly
/// choose a raw [`TokenStream`] when syntax is not valid before expansion.
/// Implementations append to the supplied output rather than replacing its
/// existing cells.
///
/// # Examples
///
/// ```
/// use m2_syn::{CellStream, Tokenize, lex_str};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let first = lex_str("x = 1;")?;
/// let second = lex_str("y = 2")?;
/// let mut cells = CellStream::default();
///
/// first.tokenize(&mut cells)?;
/// second.tokenize(&mut cells)?;
///
/// assert_eq!(cells.len(), 2);
/// # Ok(())
/// # }
/// ```
pub trait Tokenize {
    /// Appends this value's independently parseable cells to `output`.
    ///
    /// # Errors
    ///
    /// Returns [`CellStreamError`] when a raw token prefix does not end at a
    /// complete cell boundary.
    fn tokenize(&self, output: &mut CellStream) -> Result<(), CellStreamError>;

    /// Tokenizes this value into a fresh cell stream.
    ///
    /// This is the allocating convenience form of [`Self::tokenize`].
    ///
    /// # Errors
    ///
    /// Returns [`CellStreamError`] when a raw token prefix does not end at a
    /// complete cell boundary.
    fn to_cell_stream(&self) -> Result<CellStream, CellStreamError> {
        let mut output = CellStream::default();
        self.tokenize(&mut output)?;
        Ok(output)
    }
}

impl Tokenize for TokenStream {
    fn tokenize(&self, output: &mut CellStream) -> Result<(), CellStreamError> {
        let tokenized = CellStream::new(self.clone())?.into_data();
        let output = output.data_mut();
        if output.cells.is_empty() {
            output.source = tokenized.source;
            output.bof = tokenized.bof;
            output.eof = tokenized.eof;
        } else {
            output.source = None;
            output.eof = None;
        }
        output.cells.extend(tokenized.cells);
        Ok(())
    }
}

impl Tokenize for crate::SourceFile {
    fn tokenize(&self, output: &mut CellStream) -> Result<(), CellStreamError> {
        let tokenized = CellStream::from_source_file(self).into_data();
        let output = output.data_mut();
        if output.cells.is_empty() {
            output.bof = tokenized.bof;
        }
        output.eof = output.cells.is_empty().then_some(tokenized.eof).flatten();
        output.cells.extend(tokenized.cells);
        output.source = None;
        Ok(())
    }
}

impl Tokenize for crate::ParsedFile {
    fn tokenize(&self, output: &mut CellStream) -> Result<(), CellStreamError> {
        self.cst().tokenize(output)
    }
}

impl<T: Tokenize + ?Sized> Tokenize for &T {
    fn tokenize(&self, output: &mut CellStream) -> Result<(), CellStreamError> {
        (*self).tokenize(output)
    }
}

/// The recovering result of partitioning one raw stream into source cells.
///
/// Only the final source prefix can be incomplete: every confirmed boundary
/// before it is retained in [`Self::cells`]. This lets realtime and recovering
/// parsers reuse completed work while continuing to accumulate the tail.
/// Both parts retain their original source spans, accessible through
/// [`Spanned`] and [`CellTokenStream::to_token_stream`]. A cell's span covers
/// its tokens; its comments can be collected with [`crate::Lex::comments`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FramedCells {
    /// Complete cells in their original source order.
    pub cells: Vec<CellTokenStream>,
    /// The final token prefix that still requires more input, when present.
    pub incomplete: Option<CellTokenStream>,
}

/// Partitions raw tokens at confirmed top-level cell boundaries.
///
/// An unquoted top-level semicolon always confirms a boundary. A physical
/// newline confirms one only when the current
/// operator/clause state can end there. At end of input, a nonempty tail is
/// complete under the same rule unless its final delimiter group is unclosed;
/// otherwise the tail is returned through [`FramedCells::incomplete`]. Empty
/// or comment-only input never becomes an incomplete cell.
///
/// The pass does not parse expressions or reject malformed grammar. It retains
/// comments and physical source spans, observes quote specifiers so their next
/// token cannot act as syntax, and inserts the zero-width adjacency punctuation
/// required by the native parser into both complete and incomplete results.
/// It retains no state between calls; callers decide which results to reuse.
/// Lexical recovery is separate: use [`crate::lex_str_recovering`] to obtain
/// tokens for source with unclosed delimiters or string literals, and retain
/// its diagnostics independently.
///
/// # Examples
///
/// ```
/// use m2_syn::{Spanned, frame_recovering, lex_str_recovering};
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let tokens = lex_str_recovering("a = 1\nf = x -> (")?;
/// let framed = frame_recovering(tokens.value);
/// assert_eq!(framed.cells.len(), 1);
/// assert_eq!(framed.cells[0].span().text(), Some("a = 1"));
/// let tail = framed.incomplete.unwrap();
/// assert_eq!(tail.span().text(), Some("f = x -> ("));
/// # Ok(())
/// # }
/// ```
pub fn frame_recovering(tokens: TokenStream) -> FramedCells {
    let mut cells = Vec::new();
    let mut current = TokenStream::new();
    let mut pending_boundary = false;
    let mut quote_next = false;

    for token in tokens {
        if pending_boundary {
            cells.push(insert_adjacency(take(&mut current)));
            pending_boundary = false;
            quote_next = false;
        } else if current
            .last()
            .is_some_and(|left| tokens_cross_line_break(left, &token))
            && newline_ends_cell(&current)
        {
            cells.push(insert_adjacency(take(&mut current)));
            quote_next = false;
        }

        let quoted = take(&mut quote_next);
        let quotes_next = !quoted
            && matches!(&token, TokenTree::Ident(identifier)
                if is_quote_specifier(canonical_spelling(identifier.text())));
        let semicolon = !quoted && token.spelling() == Some(";");

        current.extend([token]);
        if semicolon {
            pending_boundary = true;
        }
        if quotes_next {
            quote_next = true;
        }
    }

    // A final semicolon confirms the current cell independently of its other
    // operator or delimiter state.
    if pending_boundary {
        cells.push(insert_adjacency(current));
        return FramedCells {
            cells,
            incomplete: None,
        };
    }

    // Input without syntax, such as only comments, needs no further line.
    if current.is_empty() {
        return FramedCells {
            cells,
            incomplete: None,
        };
    }

    // EOF behaves like a possible line boundary, except that an unclosed group
    // always keeps its containing cell recoverable.
    if !ends_in_incomplete_group(&current) && newline_ends_cell(&current) {
        cells.push(insert_adjacency(current));
        FramedCells {
            cells,
            incomplete: None,
        }
    } else {
        FramedCells {
            cells,
            incomplete: Some(insert_adjacency(current)),
        }
    }
}

/// Handles end span.
pub(crate) fn end_span(tokens: &CellTokenStream) -> Span {
    let ends_with_semicolon = ends_with_cell_semicolon(tokens);
    let Some(span) = tokens.iter().next_back().map(token_end_span) else {
        return Span::for_text("");
    };
    // A line comment after the last token runs to the line break that ends
    // the cell.
    let span = span.comments_after().last().map_or(span, Spanned::span);
    let Some(end) = span.end() else {
        return Span::unlocated(Arc::clone(span.source()));
    };
    if !ends_with_semicolon && let Some(newline) = span.source().text()[end..].find('\n') {
        return span.at(end + newline + 1);
    }
    span.at(end)
}

/// Handles token end span.
fn token_end_span(token: &CellTokenTree) -> Span {
    if let CellTokenTree::Group(group) = token
        && !group.delimiter().is_closed()
        && let Some(inner) = group.stream().iter().next_back()
    {
        return token_end_span(inner);
    }
    token.span()
}

/// Returns whether the value ends with cell semicolon.
pub(crate) fn ends_with_cell_semicolon(tokens: &CellTokenStream) -> bool {
    let mut quote_next = false;
    let mut final_semicolon = false;
    for token in tokens.iter() {
        let quoted = take(&mut quote_next);
        final_semicolon = !quoted && token.spelling() == Some(";");
        quote_next = !quoted
            && matches!(token, CellTokenTree::Ident(identifier)
                if is_quote_specifier(canonical_spelling(identifier.text())));
    }
    final_semicolon
}

/// Returns whether the value ends in incomplete group.
fn ends_in_incomplete_group(stream: &TokenStream) -> bool {
    matches!(stream.iter().next_back(), Some(TokenTree::Group(group)) if !group.delimiter().is_closed())
}

/// Inserts adjacency.
fn insert_adjacency(tokens: TokenStream) -> CellTokenStream {
    let mut output = TokenStream::new();
    let mut left_can_end = false;
    let mut quote_next = false;

    for mut token in tokens {
        if let TokenTree::Group(group) = &mut token {
            let contents = take(group.stream_mut());
            *group.stream_mut() = insert_adjacency(contents).into_parser_token_stream();
        }

        let quoted = take(&mut quote_next);
        let crossed_newline = output
            .last()
            .is_some_and(|left| tokens_cross_line_break(left, &token));
        if left_can_end && !quoted && !crossed_newline && can_begin_implicit_operand(&token) {
            let span = token.expect_span().subspan(0, 0);
            output.extend([TokenTree::Punct(Punct::from_span(span))]);
        }

        let quotes_next = !quoted
            && matches!(&token, TokenTree::Ident(identifier)
                if is_quote_specifier(canonical_spelling(identifier.text())));
        left_can_end = quoted || can_end_operand(&token);
        quote_next = quotes_next;
        output.extend([token]);
    }
    CellTokenStream::from_augmented(output)
}

/// Handles tokens cross line break.
fn tokens_cross_line_break(left: &TokenTree, right: &TokenTree) -> bool {
    let left = left.expect_span();
    let right = right.expect_span();
    if !left.shares_source(&right) {
        return false;
    }
    let (Some(end), Some(start)) = (left.end(), right.start()) else {
        return false;
    };
    end <= start && left.source().text()[end..start].contains('\n')
}

/// Handles token parse rule.
fn token_parse_rule(token: &TokenTree) -> Option<TokenParseRule> {
    let spelling = token.spelling().map(canonical_spelling)?;
    TOKEN_PARSE_RULES
        .binary_search_by_key(&spelling, |(declared, _)| *declared)
        .ok()
        .map(|index| TOKEN_PARSE_RULES[index].1)
}

/// Returns whether the value can begin implicit operand.
fn can_begin_implicit_operand(token: &TokenTree) -> bool {
    match token_parse_rule(token) {
        Some(TokenParseRule::Prefix(_, _) | TokenParseRule::Unary(_, _)) => true,
        Some(_) => false,
        None => matches!(
            token,
            TokenTree::Ident(_) | TokenTree::Literal(_) | TokenTree::Group(_)
        ),
    }
}

/// Returns whether the value can end operand.
fn can_end_operand(token: &TokenTree) -> bool {
    match token_parse_rule(token) {
        Some(TokenParseRule::Postfix(_)) => true,
        Some(_) => false,
        None => matches!(
            token,
            TokenTree::Ident(_) | TokenTree::Literal(_) | TokenTree::Group(_)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{Arc, CellStream, frame_recovering};
    use crate::{
        CellTokenTree, CommentKind, Lex, Spanned, TokenTree, Tokenize, lex_str, lex_str_recovering,
    };

    #[test]
    fn recovering_framing_retains_completed_prefix_and_incomplete_tail() {
        let source = "a = 1\nf = x -> (";
        let recovered = lex_str_recovering(source).unwrap();
        let framed = frame_recovering(recovered.value);

        assert_eq!(framed.cells.len(), 1);
        let completed = framed.cells[0].span();
        assert_eq!(completed.text(), Some("a = 1"));
        assert_eq!(completed.range(), Some(0..5));
        let tail = framed.incomplete.unwrap();
        assert_eq!(tail.span().text(), Some("f = x -> ("));
        assert_eq!(tail.span().range(), Some(6..source.len()));
        assert!(completed.shares_source(&tail.span()));
    }

    #[test]
    fn recovering_framing_preserves_comments_in_both_parts() {
        let source = "-- complete\na = 1 -- retained\n-- incomplete\nf = x -> ( -- inside\n";
        let framed = frame_recovering(lex_str_recovering(source).unwrap().value);
        assert_eq!(framed.cells.len(), 1);
        let completed = &framed.cells[0];
        let tail = framed.incomplete.unwrap();

        let comments = |cell: &super::CellTokenStream| {
            cell.to_token_stream()
                .comments()
                .into_iter()
                .map(|comment| {
                    assert_eq!(comment.span().text(), Some(comment.text()));
                    assert!(comment.span().shares_source(&cell.span()));
                    comment.text().to_owned()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(comments(completed), ["-- complete", "-- retained"]);
        assert_eq!(comments(&tail), ["-- incomplete", "-- inside"]);
        assert_eq!(completed.span().text(), Some("a = 1"));
        assert_eq!(tail.span().text(), Some("f = x -> ("));
    }

    #[test]
    fn recovering_framing_handles_complete_empty_and_comment_only_input() {
        for (source, expected) in [
            ("a = 1; b = 2", vec!["a = 1;", "b = 2"]),
            ("", vec![]),
            (" \n ", vec![]),
            ("-- comment", vec![]),
        ] {
            let framed = frame_recovering(lex_str(source).unwrap());
            assert!(framed.incomplete.is_none(), "{source:?}");
            assert_eq!(
                framed
                    .cells
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>(),
                expected,
                "{source:?}",
            );
        }
    }

    fn count_cell_adjacency(tokens: &[CellTokenTree]) -> usize {
        tokens
            .iter()
            .map(|token| match token {
                CellTokenTree::Group(group) => count_cell_adjacency(group.stream()),
                CellTokenTree::Adj(_) => 1,
                _ => 0,
            })
            .sum()
    }

    fn count_raw_adjacency(tokens: &[TokenTree]) -> usize {
        tokens
            .iter()
            .map(|token| match token {
                TokenTree::Group(group) => count_raw_adjacency(group.stream()),
                TokenTree::Punct(punctuation) if punctuation.text().is_empty() => 1,
                _ => 0,
            })
            .sum()
    }

    #[test]
    fn cells_are_structural_partitions() {
        let cells = CellStream::new(lex_str("f \t-- note\n x").unwrap()).unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells.to_string(), "f \t-- note\n x");
    }

    #[test]
    fn same_line_application_gets_one_empty_punctuation() {
        let cells = lex_str("f x").unwrap().to_cell_stream().unwrap();
        assert_eq!(
            cells
                .iter()
                .flat_map(|cell| cell.iter())
                .filter(|token| matches!(token, CellTokenTree::Adj(_)))
                .count(),
            1,
        );
    }

    #[test]
    fn flattening_a_framed_cell_restores_lexical_tokens() {
        let raw = lex_str("f (g x)").unwrap();
        assert_eq!(count_raw_adjacency(&raw), 0);

        let cells = raw.to_cell_stream().unwrap();
        assert_eq!(count_cell_adjacency(&cells[0]), 2);

        let raw = cells.into_token_stream();
        assert_eq!(count_raw_adjacency(&raw), 0);
        assert_eq!(raw.to_string(), "f (g x)");
    }

    #[test]
    fn line_comment_blocks_begin_the_cell_on_their_right() {
        let cells = CellStream::new(lex_str("x\n-- first\n-- second\ny").unwrap()).unwrap();
        assert_eq!(cells.len(), 2);
        let comments = cells[1].to_token_stream().comments();
        let outer = comments.outer();
        assert_eq!(outer.len(), 1);
        assert_eq!(outer[0].kind(), CommentKind::Line);
        assert_eq!(outer[0].text(), "-- first\n-- second");
    }

    #[test]
    fn a_blank_line_separates_outer_comment_blocks() {
        let cells = CellStream::new(lex_str("-- first\n\n-- second\ny").unwrap()).unwrap();
        let comments = cells[0].to_token_stream().comments();
        let outer = comments.outer();

        assert_eq!(outer.len(), 2);
        assert_eq!(outer[0].text(), "-- first");
        assert_eq!(outer[1].text(), "-- second");
    }

    #[test]
    fn line_comments_do_not_split_an_incomplete_cell() {
        let cells = CellStream::new(lex_str("x + -- reason\ny").unwrap()).unwrap();
        assert_eq!(cells.len(), 1);
        assert!(cells[0].iter().any(|token| {
            token
                .span()
                .comments_after()
                .iter()
                .any(|comment| comment.kind() == CommentKind::Line)
        }));
    }

    #[test]
    fn fork_copies_only_the_modified_cell_path() {
        let original = CellStream::new(lex_str("left; right").unwrap()).unwrap();
        let mut fork = original.fork();

        assert!(Arc::ptr_eq(&original.0, &fork.0));

        fork.data_mut().cells.pop();

        assert!(!Arc::ptr_eq(&original.0, &fork.0));
        assert_eq!(original.to_string(), "left; right");
        assert_eq!(fork.to_string(), "left;");
        assert!(original[0].shares_storage(&fork[0]));
    }
}
