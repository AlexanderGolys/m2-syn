//! Grammar-aware partitioning of raw token trees into source cells.
//!
//! [`TokenStream`] retains physical atoms, trivia, and delimiter groups.
//! [`CellStream`] is the next representation: an iterable sequence of
//! independently parseable token streams. End-of-cell and end-of-file are
//! structural boundaries, not token variants. The only inferred atom is
//! adjacency, represented as zero-width [`Punct`] with an empty spelling.

use std::error::Error;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::mem::take;
use std::sync::Arc;
use std::vec::IntoIter;

#[cfg(feature = "tree-sitter")]
use crate::CommentTrivia;
use crate::lexer::{canonical_word_spelling, is_quote_specifier, newline_ends_cell};
use crate::{
    Lex, Punct, Source, Span, Spanned, TOKEN_PARSE_RULES, TokenParseRule, TokenStream, TokenTree,
};

#[derive(Debug, Clone, Default)]
/// A sequence of independently parseable Macaulay2 token streams.
///
/// Cell boundaries are expressed by the collection itself. The boundary after
/// its final element is also the end-of-file boundary, so neither fact needs a
/// virtual token in the contained streams.
pub struct CellStream {
    cells: Vec<TokenStream>,
    source: Option<Arc<Source>>,
}

impl PartialEq for CellStream {
    fn eq(&self, other: &Self) -> bool {
        self.cells == other.cells
    }
}

impl Eq for CellStream {}

impl CellStream {
    /// Partitions a complete raw token stream and inserts inferred adjacency.
    ///
    /// # Errors
    ///
    /// Returns [`CellStreamError`] when the final tokens require more input
    /// before they can form an independently parseable cell.
    pub fn new(tokens: TokenStream) -> Result<Self, CellStreamError> {
        let source = tokens.source();
        let FramedCells { cells, incomplete } = frame_recovering(tokens);
        match incomplete {
            Some(tokens) => Err(CellStreamError {
                span: end_span(&tokens),
            }),
            None => Ok(Self { cells, source }),
        }
    }

    /// Creates a stream from token streams already known to be complete cells.
    pub fn from_cells(cells: impl IntoIterator<Item = TokenStream>) -> Self {
        Self::from_cells_with_source(cells, None)
    }

    pub(crate) fn from_cells_with_source(
        cells: impl IntoIterator<Item = TokenStream>,
        source: Option<Arc<Source>>,
    ) -> Self {
        Self {
            cells: cells.into_iter().collect(),
            source,
        }
    }

    /// Borrows the independently parseable cells.
    pub fn cells(&self) -> &[TokenStream] {
        &self.cells
    }

    /// Iterates over the independently parseable cells.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &TokenStream> + ExactSizeIterator {
        self.cells.iter()
    }

    /// Returns the number of cells.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Reports whether the stream contains no cells.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Flattens the cells back into one token stream.
    pub fn into_token_stream(self) -> TokenStream {
        flatten_cells(self.cells)
    }

    /// Flattens borrowed cells back into one token stream.
    pub fn to_token_stream(&self) -> TokenStream {
        flatten_cells(self.cells.iter().cloned())
    }

    pub(crate) fn complete_source(&self) -> Option<Arc<Source>> {
        let source = self.source.as_ref()?;
        let mut cursor = 0;
        for cell in &self.cells {
            if !covers_stream(cell, source, &mut cursor) {
                return None;
            }
        }
        source.text()[cursor..]
            .bytes()
            .all(|byte| byte.is_ascii_whitespace())
            .then(|| Arc::clone(source))
    }

    pub(crate) fn physical_eof(&self) -> Span {
        if let Some(source) = self.complete_source() {
            return Span::ranged(
                Arc::clone(&source),
                source.text().len()..source.text().len(),
            );
        }
        self.cells
            .last()
            .map_or_else(|| Span::for_text(""), end_span)
    }

    #[cfg(feature = "tree-sitter")]
    pub(crate) fn comment_ownership(&self) -> (Vec<CommentTrivia>, CommentTrivia) {
        let mut cells = Vec::new();
        let mut stream = CommentTrivia::default();
        for tokens in &self.cells {
            let mut tokens = tokens.clone();
            if tokens.has_significant_tokens() {
                cells.push(CommentTrivia::take_from_cell(&mut tokens));
            } else {
                stream.append(CommentTrivia::take_inner(&mut tokens));
            }
        }
        (cells, stream)
    }

    pub(crate) fn from_source_file(source_file: &crate::SourceFile) -> Self {
        let mut cells = source_file
            .cells
            .iter()
            .map(Lex::to_token_stream)
            .collect::<Vec<_>>();
        if !source_file.comments.is_empty() {
            if cells.is_empty() {
                cells.push(TokenStream::new());
            }
            if let Some(cell) = cells.last_mut() {
                source_file.comments.__apply_to(cell);
            }
        }
        Self {
            cells,
            source: None,
        }
    }
}

fn covers_stream(stream: &TokenStream, source: &Arc<Source>, cursor: &mut usize) -> bool {
    for trivia in stream.prefix_trivia() {
        if !covers_span(trivia.span(), source, cursor) {
            return false;
        }
    }
    for token in stream.iter() {
        for trivia in token.leading_trivia().as_slice() {
            if !covers_span(trivia.span(), source, cursor) {
                return false;
            }
        }
        let covered = match token {
            TokenTree::Group(group) => {
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
        for trivia in token.trailing_trivia().as_slice() {
            if !covers_span(trivia.span(), source, cursor) {
                return false;
            }
        }
    }
    stream
        .suffix_trivia()
        .iter()
        .all(|trivia| covers_span(trivia.span(), source, cursor))
}

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
        for (index, cell) in self.cells.iter().enumerate() {
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
        Ok(())
    }
}

fn cell_first_span(tokens: &TokenStream) -> Option<Span> {
    tokens
        .prefix_trivia()
        .first()
        .map(Spanned::span)
        .or_else(|| {
            tokens.iter().next().map(|token| {
                token
                    .leading_trivia()
                    .as_slice()
                    .first()
                    .map_or_else(|| token.span(), Spanned::span)
            })
        })
        .or_else(|| tokens.suffix_trivia().first().map(Spanned::span))
}

fn cell_last_span(tokens: &TokenStream) -> Option<Span> {
    tokens
        .suffix_trivia()
        .last()
        .map(Spanned::span)
        .or_else(|| tokens.iter().next_back().map(token_end_span))
        .or_else(|| tokens.prefix_trivia().last().map(Spanned::span))
}

impl IntoIterator for CellStream {
    type Item = TokenStream;
    type IntoIter = IntoIter<TokenStream>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.into_iter()
    }
}

impl<'syntax> IntoIterator for &'syntax CellStream {
    type Item = &'syntax TokenStream;
    type IntoIter = std::slice::Iter<'syntax, TokenStream>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.iter()
    }
}

impl FromIterator<TokenStream> for CellStream {
    fn from_iter<T: IntoIterator<Item = TokenStream>>(iter: T) -> Self {
        Self::from_cells(iter)
    }
}

fn flatten_cells(cells: impl IntoIterator<Item = TokenStream>) -> TokenStream {
    let mut output = TokenStream::new();
    for cell in cells {
        output.extend([cell]);
    }
    output
}

#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
/// A failure to partition a final token prefix into complete cells.
pub struct CellStreamError {
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
        let tokenized = CellStream::new(self.clone())?;
        if output.cells.is_empty() {
            output.source = tokenized.source;
        } else {
            output.source = None;
        }
        output.cells.extend(tokenized.cells);
        Ok(())
    }
}

impl Tokenize for crate::SourceFile {
    fn tokenize(&self, output: &mut CellStream) -> Result<(), CellStreamError> {
        output
            .cells
            .extend(CellStream::from_source_file(self).cells);
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

pub(crate) struct FramedCells {
    pub(crate) cells: Vec<TokenStream>,
    pub(crate) incomplete: Option<TokenStream>,
}

pub(crate) fn frame_recovering(tokens: TokenStream) -> FramedCells {
    let mut tokens = tokens;
    let prefix_trivia = tokens.take_prefix_trivia();
    let suffix_trivia = tokens.take_suffix_trivia();
    let mut cells = Vec::new();
    let mut current = TokenStream::new();
    current.extend(prefix_trivia.into_vec());
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

    if pending_boundary {
        cells.push(insert_adjacency(current));
        return FramedCells {
            cells,
            incomplete: None,
        };
    }

    if !has_syntax(&current) {
        if current.has_trivia() {
            cells.push(current);
        }
        return FramedCells {
            cells,
            incomplete: None,
        };
    }

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

pub(crate) fn end_span(tokens: &TokenStream) -> Span {
    let span = tokens
        .iter()
        .next_back()
        .map(token_end_span)
        .or_else(|| tokens.suffix_trivia().last().map(Spanned::span))
        .or_else(|| tokens.prefix_trivia().last().map(Spanned::span));
    let Some(span) = span else {
        return Span::for_text("");
    };
    let Some(end) = span.end() else {
        return Span::unlocated(Arc::clone(span.source()));
    };
    if !ends_with_cell_semicolon(tokens)
        && let Some(newline) = span.source().text()[end..].find('\n')
    {
        return span.at(end + newline + 1);
    }
    span.at(end)
}

fn token_end_span(token: &TokenTree) -> Span {
    if let Some(trivia) = token.trailing_trivia().as_slice().last() {
        return trivia.span();
    }
    if let TokenTree::Group(group) = token
        && !group.delimiter().is_closed()
        && let Some(inner) = group.stream().iter().next_back()
    {
        return token_end_span(inner);
    }
    token.span()
}

pub(crate) fn ends_with_cell_semicolon(tokens: &TokenStream) -> bool {
    let mut quote_next = false;
    let mut final_semicolon = false;
    for token in tokens.iter() {
        let quoted = take(&mut quote_next);
        final_semicolon = !quoted && token.spelling() == Some(";");
        quote_next = !quoted
            && matches!(token, TokenTree::Ident(identifier)
                if is_quote_specifier(canonical_word_spelling(identifier.text())));
    }
    final_semicolon
}

fn has_syntax(stream: &TokenStream) -> bool {
    !stream.is_empty()
}

fn ends_in_incomplete_group(stream: &TokenStream) -> bool {
    matches!(stream.iter().next_back(), Some(TokenTree::Group(group)) if !group.delimiter().is_closed())
}

fn insert_adjacency(mut tokens: TokenStream) -> TokenStream {
    let prefix = tokens.take_prefix_trivia();
    let suffix = tokens.take_suffix_trivia();
    let physical_eof = tokens.take_physical_eof();
    let mut output = TokenStream::new();
    output.extend(prefix.into_vec());
    let mut left_can_end = false;
    let mut quote_next = false;

    for mut token in tokens {
        if let TokenTree::Group(group) = &mut token {
            let contents = take(group.stream_mut());
            *group.stream_mut() = insert_adjacency(contents);
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
    if let Some(physical_eof) = physical_eof {
        output.set_physical_eof(physical_eof);
    }
    output
}

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

fn token_parse_rule(token: &TokenTree) -> Option<TokenParseRule> {
    let spelling = token.spelling().map(canonical_word_spelling)?;
    TOKEN_PARSE_RULES
        .binary_search_by_key(&spelling, |(declared, _)| *declared)
        .ok()
        .map(|index| TOKEN_PARSE_RULES[index].1)
}

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
    use super::CellStream;
    use crate::{TokenStream, TokenTree, Tokenize, TriviaKind, lex_str};

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
                .flat_map(TokenStream::iter)
                .filter(|token| matches!(token, TokenTree::Punct(punct) if punct.text().is_empty()))
                .count(),
            1,
        );
    }

    #[test]
    fn line_comment_blocks_begin_the_cell_on_their_right() {
        let cells = CellStream::new(lex_str("x\n-- first\n-- second\ny").unwrap()).unwrap();
        assert_eq!(cells.len(), 2);
        let comments = cells.cells()[1]
            .iter()
            .next()
            .unwrap()
            .leading_trivia()
            .as_slice();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0].kind(), TriviaKind::LineComment);
        assert_eq!(comments[0].text(), "-- first\n-- second");
    }

    #[test]
    fn a_blank_line_separates_outer_comment_blocks() {
        let cells = CellStream::new(lex_str("-- first\n\n-- second\ny").unwrap()).unwrap();
        let comments = cells.cells()[0]
            .iter()
            .next()
            .unwrap()
            .leading_trivia()
            .as_slice();

        assert_eq!(comments.len(), 2);
        assert_eq!(comments[0].text(), "-- first");
        assert_eq!(comments[1].text(), "-- second");
    }

    #[test]
    fn line_comments_do_not_split_an_incomplete_cell() {
        let cells = CellStream::new(lex_str("x + -- reason\ny").unwrap()).unwrap();
        assert_eq!(cells.len(), 1);
        assert!(cells.cells()[0].iter().any(|token| {
            token
                .trailing_trivia()
                .as_slice()
                .iter()
                .any(|trivia| trivia.kind() == TriviaKind::LineComment)
        }));
    }
}
