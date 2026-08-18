use std::{
    fmt::{Display, Formatter, Result},
    vec::IntoIter,
};

use crate::{SourceId, Span, Spanned};
use delim::{Delimiter, DelimiterKind};

pub mod delim;
mod punct;
pub use punct::Punct;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LiteralKind {
    String,
    RawString,
    Integer,
    Float,
}

#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
pub struct Literal {
    pub kind: LiteralKind,
    text: String,
    span: Span,
}

impl Display for Literal {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        formatter.write_str(&self.text)
    }
}

impl Literal {
    pub fn new(kind: LiteralKind, text: String, span: Span) -> Self {
        Self { kind, text, span }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}
impl ToTokens for Literal {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_literal(self.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
pub struct IdentToken {
    text: String,
    span: Span,
}

impl IdentToken {
    pub fn new(text: impl Into<String>, span: Span) -> Self {
        Self {
            text: text.into(),
            span,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

impl Display for IdentToken {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        formatter.write_str(&self.text)
    }
}
impl ToTokens for IdentToken {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_ident(self.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
pub struct Group {
    delimiter: Delimiter,
    stream: TokenStream,
}

impl Group {
    pub fn new(delimiter: Delimiter, stream: TokenStream) -> Self {
        Self { delimiter, stream }
    }

    pub fn delimiter(&self) -> &Delimiter {
        &self.delimiter
    }

    pub fn delim_kind(&self) -> DelimiterKind {
        self.delimiter.kind
    }

    pub fn stream(&self) -> &TokenStream {
        &self.stream
    }

    pub fn into_stream(self) -> TokenStream {
        self.stream
    }
}

impl Display for Group {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        formatter.write_str(self.delim_kind().opening())?;
        self.stream.fmt(formatter)?;
        formatter.write_str(self.delim_kind().closing())
    }
}
impl ToTokens for Group {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_group(self.clone());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TriviaKind {
    Whitespace,
    CarriageReturn,
    LineBreak,
    LineComment,
    BlockComment,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Spanned)]
pub struct Trivia {
    kind: TriviaKind,
    text: String,
    span: Span,
}

impl Trivia {
    pub fn new(kind: TriviaKind, text: impl Into<String>, span: Span) -> Self {
        Self {
            kind,
            text: text.into(),
            span,
        }
    }

    pub fn kind(&self) -> TriviaKind {
        self.kind
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn contains_line_break(&self) -> bool {
        self.text.contains('\n')
    }
}

impl Display for Trivia {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        formatter.write_str(&self.text)
    }
}
impl ToTokens for Trivia {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_trivia(self.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
/// One top-level token sequence together with its terminating delimiter.
///
/// An ordinary cell has [`DelimiterKind::Empty`]. A muted cell has
/// [`DelimiterKind::Semicolon`], and its semicolon is held by the delimiter
/// rather than repeated in `stream`.
pub struct CellBlock {
    delimiter: Delimiter,
    stream: TokenStream,
}

impl CellBlock {
    /// Creates a cell from its delimiter and interior token stream.
    pub fn new(delimiter: Delimiter, stream: TokenStream) -> Self {
        Self { delimiter, stream }
    }

    /// Returns the type-erased cell delimiter and its boundary spans.
    pub fn delimiter(&self) -> &Delimiter {
        &self.delimiter
    }

    /// Returns whether this is an ordinary or semicolon-terminated cell.
    pub fn delim_kind(&self) -> DelimiterKind {
        self.delimiter.kind
    }

    /// Borrows the tokens inside the cell delimiter.
    pub fn stream(&self) -> &TokenStream {
        &self.stream
    }

    /// Consumes the cell and returns the tokens inside its delimiter.
    pub fn into_stream(self) -> TokenStream {
        self.stream
    }
}

impl Display for CellBlock {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        formatter.write_str(self.delim_kind().opening())?;
        self.stream.fmt(formatter)?;
        formatter.write_str(self.delim_kind().closing())
    }
}

impl ToTokens for CellBlock {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.delimiter.surround(output, self.stream.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
pub enum TokenTree {
    Group(Group),
    Literal(Literal),
    Punct(Punct),
    Ident(IdentToken),
    Trivia(Trivia),
    #[doc(hidden)]
    Eof(Span),
}

impl TokenTree {
    pub fn spelling(&self) -> Option<&str> {
        match self {
            Self::Literal(token) => Some(token.text()),
            Self::Punct(token) => Some(token.text()),
            Self::Ident(token) => Some(token.text()),
            Self::Group(_) | Self::Trivia(_) | Self::Eof(_) => None,
        }
    }
}
impl ToTokens for TokenTree {
    fn to_tokens(&self, output: &mut TokenStream) {
        if !matches!(self, Self::Eof(_)) {
            output.push(self.clone());
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Spanned)]
pub struct TokenStream(Vec<TokenTree>);

impl TokenStream {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn iter(&self) -> impl Iterator<Item = &TokenTree> {
        self.0.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn get(&self, position: usize) -> Option<&TokenTree> {
        self.0.get(position)
    }

    pub(crate) fn last(&self) -> Option<&TokenTree> {
        self.0.last()
    }

    pub(crate) fn push_eof(&mut self, span: Span) {
        self.0.push(TokenTree::Eof(span));
    }

    pub fn starts_with_whitespace(&self) -> bool {
        matches!(self.0.first(), Some(TokenTree::Trivia(_)))
    }

    pub fn ends_with_whitespace(&self) -> bool {
        matches!(self.0.last(), Some(TokenTree::Trivia(_)))
    }

    fn ends_with_line_break(&self) -> bool {
        matches!(self.0.last(), Some(TokenTree::Trivia(trivia)) if trivia.contains_line_break())
    }

    fn starts_with_line_break(&self) -> bool {
        matches!(self.0.first(), Some(TokenTree::Trivia(trivia)) if trivia.contains_line_break())
    }

    fn remove_leading_line_break(&mut self) {
        if self.starts_with_line_break() {
            self.0.remove(0);
        }
    }

    pub fn push_ident(&mut self, ident: IdentToken) {
        self.0.push(TokenTree::Ident(ident));
    }

    pub fn push_literal(&mut self, literal: Literal) {
        self.0.push(TokenTree::Literal(literal));
    }

    pub fn push_punct(&mut self, punct: Punct) {
        self.0.push(TokenTree::Punct(punct));
    }

    pub fn push_trivia(&mut self, trivia: Trivia) {
        self.0.push(TokenTree::Trivia(trivia));
    }

    pub fn push_group(&mut self, group: Group) {
        self.0.push(TokenTree::Group(group));
    }

    pub fn push(&mut self, tree: TokenTree) {
        self.0.push(tree);
    }

    /// Appends one emitted syntax fragment with any separator required to keep
    /// adjacent word-like fragments lexically distinct.
    ///
    /// This is the composition operation used by [`crate::quote_m2!`]. The
    /// fragment retains all of its internal token structure and trivia; a
    /// detached space is inserted only when the last existing token and first
    /// appended token would otherwise be rendered as adjacent words.
    #[doc(hidden)]
    pub fn append_fragment<T: ToTokens + ?Sized>(&mut self, fragment: &T) {
        let fragment = fragment.to_token_stream();
        if self.last().is_some_and(is_wordlike) && fragment.iter().next().is_some_and(is_wordlike) {
            self.push_trivia(Trivia::new(TriviaKind::Whitespace, " ", Span::detached()));
        }
        self.extend([fragment]);
    }

    /// Removes and returns the final token tree, if any.
    pub fn pop(&mut self) -> Option<TokenTree> {
        self.0.pop()
    }
}

fn is_wordlike(tree: &TokenTree) -> bool {
    matches!(
        tree,
        TokenTree::Ident(_) | TokenTree::Literal(_) | TokenTree::Group(_)
    )
}

impl Display for TokenStream {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        for tree in &self.0 {
            match tree {
                TokenTree::Group(group) => group.fmt(formatter)?,
                TokenTree::Literal(literal) => literal.fmt(formatter)?,
                TokenTree::Punct(punct) => punct.fmt(formatter)?,
                TokenTree::Ident(ident) => ident.fmt(formatter)?,
                TokenTree::Trivia(trivia) => trivia.fmt(formatter)?,
                TokenTree::Eof(_) => {}
            }
        }
        Ok(())
    }
}

impl Extend<TokenStream> for TokenStream {
    fn extend<T: IntoIterator<Item = TokenStream>>(&mut self, iter: T) {
        self.0.extend(iter.into_iter().flat_map(|stream| stream.0));
    }
}
impl Extend<TokenTree> for TokenStream {
    fn extend<T: IntoIterator<Item = TokenTree>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl FromIterator<TokenTree> for TokenStream {
    fn from_iter<T: IntoIterator<Item = TokenTree>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl IntoIterator for TokenStream {
    type Item = TokenTree;
    type IntoIter = IntoIter<TokenTree>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl ToTokens for TokenStream {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.0.extend(self.0.iter().cloned());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellStream {
    cells: Vec<CellBlock>,
    source_id: SourceId,
}

impl CellStream {
    pub fn new(cells: Vec<CellBlock>, source_id: SourceId) -> Self {
        Self { cells, source_id }
    }

    pub fn iter(&self) -> impl Iterator<Item = &CellBlock> {
        self.cells.iter()
    }

    pub fn cells(&self) -> &[CellBlock] {
        &self.cells
    }

    pub fn into_cells(self) -> Vec<CellBlock> {
        self.cells
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn source_id(&self) -> SourceId {
        self.source_id
    }

    /// Appends one global cell.
    pub fn push(&mut self, mut cell: CellBlock) {
        if self.cells.last().is_some_and(|previous| {
            !previous.stream().ends_with_line_break() && !cell.stream().starts_with_line_break()
        }) {
            let mut separated = TokenStream::new();
            separated.push_trivia(Trivia::new(TriviaKind::LineBreak, "\n", Span::detached()));
            separated.extend([cell.stream]);
            cell.stream = separated;
        }
        self.cells.push(cell);
    }

    fn push_unseparated(&mut self, cell: CellBlock) {
        self.cells.push(cell);
    }
}

impl Display for CellStream {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        for cell in &self.cells {
            cell.fmt(formatter)?;
        }
        Ok(())
    }
}

impl IntoIterator for CellStream {
    type Item = CellBlock;
    type IntoIter = IntoIter<CellBlock>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.into_iter()
    }
}

impl Extend<CellBlock> for CellStream {
    fn extend<T: IntoIterator<Item = CellBlock>>(&mut self, iter: T) {
        for cell in iter {
            self.push(cell);
        }
    }
}

impl ToTokens for CellStream {
    fn to_tokens(&self, output: &mut TokenStream) {
        for (index, cell) in self.cells.iter().enumerate() {
            let mut stream = cell.stream.clone();
            if index != 0 && self.cells[index - 1].delim_kind() == DelimiterKind::Semicolon {
                stream.remove_leading_line_break();
            }
            cell.delimiter.surround(output, stream);
        }
    }
}

pub trait ToTokens {
    fn to_tokens(&self, output: &mut TokenStream);

    fn to_token_stream(&self) -> TokenStream {
        let mut output = TokenStream::new();
        self.to_tokens(&mut output);
        output
    }

    fn to_m2(&self) -> String {
        self.to_token_stream().to_string()
    }

    fn to_code(&self) -> String {
        self.to_m2()
    }
}

impl<T: ToTokens + ?Sized> ToTokens for &T {
    fn to_tokens(&self, output: &mut TokenStream) {
        (*self).to_tokens(output);
    }
}

impl<T: ToTokens + ?Sized> ToTokens for Box<T> {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.as_ref().to_tokens(output);
    }
}

impl<T: ToTokens> ToTokens for Option<T> {
    fn to_tokens(&self, output: &mut TokenStream) {
        if let Some(value) = self {
            value.to_tokens(output);
        }
    }
}

impl<T: ToTokens> ToTokens for [T] {
    fn to_tokens(&self, output: &mut TokenStream) {
        for token in self {
            token.to_tokens(output);
        }
    }
}

impl<T: ToTokens> ToTokens for Vec<T> {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.as_slice().to_tokens(output);
    }
}

/// Emits syntax as the linear sequence of cells evaluated at global scope.
///
/// This is the global-scope counterpart of [`ToTokens`]. Implementations append
/// to an existing stream so independently produced cell fragments compose
/// without intermediate allocation.
pub trait ToCells {
    /// Appends this value's global cells to `output`.
    fn to_cells(&self, output: &mut CellStream);

    /// Emits this value into a fresh cell stream with the requested identity.
    fn to_cell_stream(&self, source_id: SourceId) -> CellStream {
        let mut output = CellStream::new(Vec::new(), source_id);
        self.to_cells(&mut output);
        output
    }
}

impl<T: ToCells + ?Sized> ToCells for &T {
    fn to_cells(&self, output: &mut CellStream) {
        (*self).to_cells(output);
    }
}

impl<T: ToCells + ?Sized> ToCells for Box<T> {
    fn to_cells(&self, output: &mut CellStream) {
        self.as_ref().to_cells(output);
    }
}

impl<T: ToCells> ToCells for Option<T> {
    fn to_cells(&self, output: &mut CellStream) {
        if let Some(value) = self {
            value.to_cells(output);
        }
    }
}

impl<T: ToCells> ToCells for [T] {
    fn to_cells(&self, output: &mut CellStream) {
        for value in self {
            value.to_cells(output);
        }
    }
}

impl<T: ToCells> ToCells for Vec<T> {
    fn to_cells(&self, output: &mut CellStream) {
        self.as_slice().to_cells(output);
    }
}

impl ToCells for CellBlock {
    fn to_cells(&self, output: &mut CellStream) {
        output.push(self.clone());
    }
}

impl ToCells for CellStream {
    fn to_cells(&self, output: &mut CellStream) {
        output.extend(self.cells.iter().cloned());
    }
}

impl ToCells for TokenStream {
    fn to_cells(&self, output: &mut CellStream) {
        append_promoted_cells(self, output);
    }
}

impl ToCells for Group {
    fn to_cells(&self, output: &mut CellStream) {
        self.stream.to_cells(output);
    }
}

fn append_promoted_cells(tokens: &TokenStream, output: &mut CellStream) {
    let mut current = TokenStream::new();
    let mut first = true;
    for token in tokens.iter() {
        if matches!(token, TokenTree::Eof(_)) {
            continue;
        }
        if matches!(token, TokenTree::Punct(punct) if punct.text() == ";") {
            let span = current.span().join(token.span());
            push_promoted_cell(
                output,
                &mut first,
                CellBlock::new(
                    Delimiter::new(DelimiterKind::Semicolon, span),
                    std::mem::take(&mut current),
                ),
            );
            continue;
        }

        current.push(token.clone());
        if matches!(token, TokenTree::Trivia(trivia) if trivia.kind() == TriviaKind::LineBreak)
            && crate::lexer::newline_ends_cell(&current)
        {
            let span = current.span();
            push_promoted_cell(
                output,
                &mut first,
                CellBlock::new(
                    Delimiter::new(DelimiterKind::Empty, span),
                    std::mem::take(&mut current),
                ),
            );
        }
    }

    if !current.is_empty() {
        let span = current.span();
        push_promoted_cell(
            output,
            &mut first,
            CellBlock::new(Delimiter::new(DelimiterKind::Empty, span), current),
        );
    }
}

fn push_promoted_cell(output: &mut CellStream, first: &mut bool, cell: CellBlock) {
    if std::mem::take(first) {
        output.push(cell);
    } else {
        output.push_unseparated(cell);
    }
}
