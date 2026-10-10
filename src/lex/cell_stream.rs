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

use crate::lexer::{canonical_word_spelling, is_quote_specifier, newline_ends_cell};
use crate::token_stream::TokenStreamParts;
use crate::{
    Delimiter, DelimiterKind, IdentToken, LeadingTrivia, Lex, LiteralToken, Punct, Source, Span,
    Spanned, TOKEN_PARSE_RULES, Terminal, TokenGroup, TokenParseRule, TokenStream, TokenTree,
    TrailingTrivia, Trivia,
};

#[derive(Debug, Clone, Default)]
/// Internal state for CellTokenStreamData.
struct CellTokenStreamData {
    /// The trees.
    trees: Vec<CellTokenTree>,
    /// The prefix.
    prefix: TrailingTrivia,
    /// The suffix.
    suffix: LeadingTrivia,
    /// The source.
    source: Option<Arc<Source>>,
}

impl PartialEq for CellTokenStreamData {
    fn eq(&self, other: &Self) -> bool {
        self.trees == other.trees
            && self.prefix == other.prefix
            && self.suffix == other.suffix
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

    /// Returns comments attached before this cell token.
    pub fn leading_trivia(&self) -> &[Trivia] {
        match self {
            Self::Group(token) => token.leading_trivia().as_slice(),
            Self::Literal(token) => token.leading_trivia().as_slice(),
            Self::Punct(token) => token.leading_trivia().as_slice(),
            Self::Ident(token) => token.leading_trivia().as_slice(),
            Self::Adj(_) => &[],
        }
    }

    /// Returns comments attached after this cell token.
    pub fn trailing_trivia(&self) -> &[Trivia] {
        match self {
            Self::Group(token) => token.trailing_trivia().as_slice(),
            Self::Literal(token) => token.trailing_trivia().as_slice(),
            Self::Punct(token) => token.trailing_trivia().as_slice(),
            Self::Ident(token) => token.trailing_trivia().as_slice(),
            Self::Adj(_) => &[],
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
    /// The leading trivia.
    leading_trivia: LeadingTrivia,
    /// The trailing trivia.
    trailing_trivia: TrailingTrivia,
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
        let (delimiter, stream, leading_trivia, trailing_trivia) = group.into_parts();
        Self {
            delimiter,
            stream: CellTokenStream::from_augmented(stream),
            leading_trivia,
            trailing_trivia,
        }
    }

    /// Converts the value into lexical.
    fn into_lexical(self) -> TokenGroup {
        TokenGroup::from_parts(
            self.delimiter,
            self.stream.into_token_stream(),
            self.leading_trivia,
            self.trailing_trivia,
        )
    }

    /// Converts the value into parser group.
    fn into_parser_group(self) -> TokenGroup {
        TokenGroup::from_parts(
            self.delimiter,
            self.stream.into_parser_token_stream(),
            self.leading_trivia,
            self.trailing_trivia,
        )
    }
}

impl Spanned for CellTokenGroup {
    fn try_span(&self) -> Option<Span> {
        self.delimiter.try_span()
    }
}

impl Terminal for CellTokenGroup {
    fn leading_trivia(&self) -> &LeadingTrivia {
        &self.leading_trivia
    }

    fn leading_trivia_mut(&mut self) -> &mut LeadingTrivia {
        &mut self.leading_trivia
    }

    fn trailing_trivia(&self) -> &TrailingTrivia {
        &self.trailing_trivia
    }

    fn trailing_trivia_mut(&mut self) -> &mut TrailingTrivia {
        &mut self.trailing_trivia
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
    /// Inferred adjacency is removed recursively. Comments, delimiter groups,
    /// source spans, and physical spellings are preserved.
    pub fn into_token_stream(self) -> TokenStream {
        let CellTokenStreamData {
            trees,
            prefix,
            suffix,
            source,
        } = self.into_data();
        TokenStream::from_parts(TokenStreamParts {
            trees: trees
                .into_iter()
                .filter_map(CellTokenTree::into_lexical)
                .collect(),
            prefix,
            suffix,
            source,
        })
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
        let TokenStreamParts {
            trees,
            prefix,
            suffix,
            source,
        } = tokens.into_parts();
        Self(Arc::new(CellTokenStreamData {
            trees: trees
                .into_iter()
                .map(CellTokenTree::from_augmented)
                .collect(),
            prefix,
            suffix,
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
    ///
    /// Comments are settled onto the tokens they follow, so a parser finds
    /// them in trailing trivia and in the stream prefix only.
    pub(crate) fn into_parser_token_stream(self) -> TokenStream {
        let CellTokenStreamData {
            trees,
            prefix,
            suffix,
            source,
        } = self.into_data();
        let mut tokens = TokenStream::from_parts(TokenStreamParts {
            trees: trees
                .into_iter()
                .map(CellTokenTree::into_parser_token)
                .collect(),
            prefix,
            suffix,
            source,
        });
        tokens.settle_trivia();
        tokens
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
        Span::join_all(
            self.data()
                .prefix
                .try_span()
                .into_iter()
                .chain(self.iter().filter_map(Spanned::try_span))
                .chain(self.data().suffix.try_span()),
        )
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
    /// The span of the layout that follows the final cell.
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
    pub fn new(tokens: TokenStream) -> Result<Self, CellStreamError> {
        let source = tokens.source();
        let eof = trailing_layout_span(source.as_ref());
        let FramedCells { cells, incomplete } = frame_recovering(tokens);
        match incomplete {
            Some(tokens) => Err(CellStreamError {
                span: end_span(&tokens),
            }),
            None => Ok(Self(Arc::new(CellStreamData { cells, source, eof }))),
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
            eof: trailing_layout_span(source.as_ref()),
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
        source.text()[cursor..]
            .bytes()
            .all(|byte| byte.is_ascii_whitespace())
            .then(|| Arc::clone(source))
    }

    /// Handles physical end-of-file.
    ///
    /// The span covers the layout between the last significant byte and the
    /// physical end, so the end-of-file marker built from it retains input that
    /// no cell owns. It stays empty for a stream whose cells do not cover one
    /// shared source snapshot.
    pub(crate) fn physical_eof(&self) -> Span {
        if let Some(source) = self.complete_source() {
            let trailing = source.trailing_whitespace();
            return Span::ranged(Arc::clone(&source), trailing);
        }
        self.last().map_or_else(|| Span::for_text(""), end_span)
    }

    /// Creates the value from source file.
    pub(crate) fn from_source_file(source_file: &crate::SourceFile) -> Self {
        let mut cells = source_file
            .cells
            .iter()
            .map(Lex::to_token_stream)
            .collect::<Vec<_>>();
        for (index, comments) in [
            (0, &source_file.leading_trivia),
            (cells.len(), source_file.eof.leading_trivia()),
        ]
        .into_iter()
        .rev()
        {
            if !comments.is_empty() {
                let mut comment_cell = TokenStream::new();
                comment_cell.extend(comments.as_slice().iter().cloned());
                cells.insert(index, comment_cell);
            }
        }
        Self(Arc::new(CellStreamData {
            cells: cells
                .into_iter()
                .map(CellTokenStream::from_augmented)
                .collect(),
            source: None,
            eof: source_file.eof.try_span(),
        }))
    }
}

/// Returns whether the value covers stream.
fn covers_stream(stream: &CellTokenStream, source: &Arc<Source>, cursor: &mut usize) -> bool {
    for trivia in stream.data().prefix.as_slice() {
        if !covers_span(trivia.span(), source, cursor) {
            return false;
        }
    }
    for token in stream.iter() {
        for trivia in token.leading_trivia() {
            if !covers_span(trivia.span(), source, cursor) {
                return false;
            }
        }
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
        for trivia in token.trailing_trivia() {
            if !covers_span(trivia.span(), source, cursor) {
                return false;
            }
        }
    }
    stream
        .data()
        .suffix
        .as_slice()
        .iter()
        .all(|trivia| covers_span(trivia.span(), source, cursor))
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
    if start < *cursor
        || !source.text()[*cursor..start]
            .bytes()
            .all(|byte| byte.is_ascii_whitespace())
    {
        return false;
    }
    *cursor = end;
    true
}

impl Display for CellStream {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        let mut previous: Option<Span> = None;
        for (index, cell) in self.iter().enumerate() {
            if index != 0 {
                let next = cell_first_span(cell);
                let mut recovered = false;
                if let Some((left, right)) = previous.as_ref().zip(next.as_ref())
                    && left.shares_source(right)
                    && let (Some(end), Some(start)) = (left.end(), right.start())
                    && end <= start
                {
                    let gap = &left.source().text()[end..start];
                    if !gap.is_empty() && gap.bytes().all(|byte| byte.is_ascii_whitespace()) {
                        formatter.write_str(gap)?;
                        recovered = true;
                    }
                }
                if !recovered {
                    formatter.write_str("\n")?;
                }
            }
            cell.fmt(formatter)?;
            previous = cell_last_span(cell);
        }
        write_trailing_layout(formatter, self.data().eof.as_ref(), previous.as_ref())
    }
}

/// Spans the layout closing a source snapshot the cells were framed from.
fn trailing_layout_span(source: Option<&Arc<Source>>) -> Option<Span> {
    source.map(|source| Span::ranged(Arc::clone(source), source.trailing_whitespace()))
}

/// Writes the layout the end-of-file span retains after the final cell.
///
/// The suffix is emitted only when it is the whitespace physically following
/// the last cell, so a marker left behind by an edited or reassembled stream
/// contributes nothing rather than stale text.
fn write_trailing_layout(
    formatter: &mut Formatter<'_>,
    eof: Option<&Span>,
    last_cell: Option<&Span>,
) -> FmtResult {
    let Some(eof) = eof else {
        return Ok(());
    };
    let resume = match last_cell {
        Some(last) if last.shares_source(eof) => last.end(),
        Some(_) => return Ok(()),
        None => eof.start(),
    };
    let Some((resume, stop)) = resume.zip(eof.end()) else {
        return Ok(());
    };
    match eof.source().text().get(resume..stop) {
        Some(suffix) if suffix.bytes().all(|byte| byte.is_ascii_whitespace()) => {
            formatter.write_str(suffix)
        }
        _ => Ok(()),
    }
}

/// Handles cell first span.
fn cell_first_span(tokens: &CellTokenStream) -> Option<Span> {
    tokens
        .data()
        .prefix
        .as_slice()
        .first()
        .map(Spanned::span)
        .or_else(|| {
            tokens.iter().next().map(|token| {
                token
                    .leading_trivia()
                    .first()
                    .map_or_else(|| token.span(), Spanned::span)
            })
        })
        .or_else(|| tokens.data().suffix.as_slice().first().map(Spanned::span))
}

/// Handles cell last span.
fn cell_last_span(tokens: &CellTokenStream) -> Option<Span> {
    tokens
        .data()
        .suffix
        .as_slice()
        .last()
        .map(Spanned::span)
        .or_else(|| tokens.iter().next_back().map(token_end_span))
        .or_else(|| tokens.data().prefix.as_slice().last().map(Spanned::span))
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
            output.eof = tokenized.eof;
        } else {
            output.source = None;
            output.eof = None;
        }
        output.cells.extend(tokenized.cells);
        Ok(())
    }
}

impl crate::SourceFile {
    /// Places comments that follow every cell: on the end-of-file marker, or
    /// on the file itself when it has no cells for them to follow.
    pub(crate) fn set_closing_comments(&mut self, comments: Vec<Trivia>) {
        if self.cells.is_empty() {
            self.leading_trivia = LeadingTrivia::new(comments);
        } else {
            self.eof.set_leading_trivia(comments);
        }
    }
}

impl Tokenize for crate::SourceFile {
    fn tokenize(&self, output: &mut CellStream) -> Result<(), CellStreamError> {
        let tokenized = CellStream::from_source_file(self).into_data();
        let output = output.data_mut();
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
pub(crate) struct FramedCells {
    /// Complete cells in their original source order.
    pub(crate) cells: Vec<CellTokenStream>,
    /// The final token prefix that still requires more input, when present.
    pub(crate) incomplete: Option<CellTokenStream>,
}

/// Partitions raw tokens at confirmed top-level cell boundaries.
///
/// An unquoted top-level semicolon always confirms a boundary. A physical
/// newline confirms one only when [`newline_ends_cell`] says that the current
/// operator/clause state can end there. At end of input, a nonempty tail is
/// complete under the same rule unless its final delimiter group is unclosed;
/// otherwise the tail is returned through [`FramedCells::incomplete`]. Empty
/// or trivia-only input never becomes an incomplete cell.
///
/// The pass does not parse expressions or reject malformed grammar. It retains
/// comments and physical source spans, observes quote specifiers so their next
/// token cannot act as syntax, and inserts the zero-width adjacency punctuation
/// required by the native parser into both complete and incomplete results.
pub(crate) fn frame_recovering(tokens: TokenStream) -> FramedCells {
    let mut tokens = tokens;
    // Stream-level trivia must follow the first/last framed cell instead of
    // being lost when the significant tokens are distributed between cells.
    let prefix_trivia = tokens.take_prefix_trivia();
    let suffix_trivia = tokens.take_suffix_trivia();
    let mut cells = Vec::new();
    let mut current = TokenStream::new();
    current.extend(prefix_trivia.into_vec());
    let mut pending_boundary = false;
    let mut quote_next = false;

    for token in tokens {
        // Delay a semicolon boundary until the next token so any stream-level
        // suffix trivia discovered at EOF remains owned by that final cell.
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
                if is_quote_specifier(canonical_word_spelling(identifier.text())));
        let semicolon = !quoted && token.spelling() == Some(";");

        current.extend([token]);
        if semicolon {
            pending_boundary = true;
        }
        if quotes_next {
            quote_next = true;
        }
    }

    current.extend(suffix_trivia.into_vec());

    // A final semicolon confirms the current cell independently of its other
    // operator or delimiter state.
    if pending_boundary {
        cells.push(insert_adjacency(current));
        return FramedCells {
            cells,
            incomplete: None,
        };
    }

    // Comments without syntax are complete retained input, not a request for
    // another line of source.
    if !has_syntax(&current) {
        if current.has_trivia() {
            cells.push(CellTokenStream::from_augmented(current));
        }
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
    let span = tokens
        .iter()
        .next_back()
        .map(token_end_span)
        .or_else(|| tokens.data().suffix.as_slice().last().map(Spanned::span))
        .or_else(|| tokens.data().prefix.as_slice().last().map(Spanned::span));
    let Some(span) = span else {
        return Span::for_text("");
    };
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
    if let Some(trivia) = token.trailing_trivia().last() {
        return trivia.span();
    }
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
                if is_quote_specifier(canonical_word_spelling(identifier.text())));
    }
    final_semicolon
}

/// Returns whether the value has syntax.
fn has_syntax(stream: &TokenStream) -> bool {
    !stream.is_empty()
}

/// Returns whether the value ends in incomplete group.
fn ends_in_incomplete_group(stream: &TokenStream) -> bool {
    matches!(stream.iter().next_back(), Some(TokenTree::Group(group)) if !group.delimiter().is_closed())
}

/// Inserts adjacency.
fn insert_adjacency(mut tokens: TokenStream) -> CellTokenStream {
    let prefix = tokens.take_prefix_trivia();
    let suffix = tokens.take_suffix_trivia();
    let mut output = TokenStream::new();
    output.extend(prefix.into_vec());
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
                if is_quote_specifier(canonical_word_spelling(identifier.text())));
        left_can_end = quoted || can_end_operand(&token);
        quote_next = quotes_next;
        output.extend([token]);
    }
    for trivia in suffix.into_vec() {
        output.push_leading_trivia(trivia);
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
    let spelling = token.spelling().map(canonical_word_spelling)?;
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
    use super::{Arc, CellStream};
    use crate::{CellTokenTree, TokenTree, Tokenize, TriviaKind, lex_str};

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
        let comments = cells[1].iter().next().unwrap().leading_trivia();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].kind(), TriviaKind::LineComment);
        assert_eq!(comments[0].text(), "-- first\n-- second");
    }

    #[test]
    fn a_blank_line_separates_outer_comment_blocks() {
        let cells = CellStream::new(lex_str("-- first\n\n-- second\ny").unwrap()).unwrap();
        let comments = cells[0].iter().next().unwrap().leading_trivia();

        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].text(), "-- first");
        assert_eq!(comments[1].text(), "-- second");
    }

    #[test]
    fn line_comments_do_not_split_an_incomplete_cell() {
        let cells = CellStream::new(lex_str("x + -- reason\ny").unwrap()).unwrap();
        assert_eq!(cells.len(), 1);
        assert!(cells[0].iter().any(|token| {
            token
                .trailing_trivia()
                .iter()
                .any(|trivia| trivia.kind() == TriviaKind::LineComment)
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
