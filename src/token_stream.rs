use std::{
    fmt::{Display, Formatter, Result},
    vec::IntoIter,
};

use crate::{SourceId, Span, Spanned};
use delim::{Delimiter, DelimiterKind, DoubleSpan};

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

    pub fn double_span(&self) -> DoubleSpan {
        self.delimiter.span2
    }

    pub fn stream(&self) -> &TokenStream {
        &self.stream
    }

    pub fn into_stream(self) -> TokenStream {
        self.stream
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

    /// Returns the cell's implicit opening and explicit or implicit closing spans.
    pub fn double_span(&self) -> DoubleSpan {
        self.delimiter.span2
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

impl Spanned for CellBlock {
    fn span(&self) -> Span {
        self.delimiter.span()
    }
}

impl Display for Group {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        formatter.write_str(self.delim_kind().opening())?;
        self.stream.fmt(formatter)?;
        formatter.write_str(self.delim_kind().closing())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
pub enum TokenTree {
    Group(Group),
    Literal(Literal),
    Punct(Punct),
    Ident(IdentToken),
    Trivia(Trivia),
}

impl TokenTree {
    pub fn spelling(&self) -> Option<&str> {
        match self {
            Self::Literal(token) => Some(token.text()),
            Self::Punct(token) => Some(token.text()),
            Self::Ident(token) => Some(token.text()),
            Self::Group(_) | Self::Trivia(_) => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
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

    pub(crate) fn get(&self, position: usize) -> Option<&TokenTree> {
        self.0.get(position)
    }

    pub(crate) fn last(&self) -> Option<&TokenTree> {
        self.0.last()
    }

    pub fn starts_with_whitespace(&self) -> bool {
        matches!(self.0.first(), Some(TokenTree::Trivia(_)))
    }

    pub fn ends_with_whitespace(&self) -> bool {
        matches!(self.0.last(), Some(TokenTree::Trivia(_)))
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

    /// Removes and returns the final token tree, if any.
    pub fn pop(&mut self) -> Option<TokenTree> {
        self.0.pop()
    }
}

impl Spanned for TokenStream {
    fn span(&self) -> Span {
        Span::join_all(self.iter().map(Spanned::span))
    }
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

impl ToTokens for TokenStream {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.0.extend(self.0.iter().cloned());
    }
}

impl ToTokens for TokenTree {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push(self.clone());
    }
}

impl ToTokens for IdentToken {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_ident(self.clone());
    }
}

impl ToTokens for Literal {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_literal(self.clone());
    }
}

impl ToTokens for Punct {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_punct(self.clone());
    }
}

impl ToTokens for Trivia {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_trivia(self.clone());
    }
}

impl ToTokens for Group {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_group(self.clone());
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

pub trait ToCellStream {
    fn to_cell_stream(&self, source_id: SourceId) -> CellStream;
}
