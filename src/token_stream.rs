use std::fmt::{Display, Formatter, Result};

use crate::{Span, Spanned};
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Literal {
    pub kind: LiteralKind,
    text: String,
    span: Span,
}

impl Spanned for Literal {
    fn span(&self) -> Span {
        self.span
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

impl Spanned for IdentToken {
    fn span(&self) -> Span {
        self.span
    }
}

impl Display for IdentToken {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        formatter.write_str(&self.text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    delimiter: Delimiter,
    stream: TokenStream,
}

impl Spanned for Group {
    fn span(&self) -> Span {
        self.delimiter.span()
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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
}

impl Spanned for Trivia {
    fn span(&self) -> Span {
        self.span
    }
}

impl Display for Trivia {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        formatter.write_str(&self.text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellBlock {
    stream: TokenStream,
    span: Span,
}

impl CellBlock {
    pub fn new(stream: TokenStream, span: Span) -> Self {
        Self { stream, span }
    }

    pub fn stream(&self) -> &TokenStream {
        &self.stream
    }

    pub fn into_stream(self) -> TokenStream {
        self.stream
    }
}

impl Spanned for CellBlock {
    fn span(&self) -> Span {
        self.span
    }
}

impl Display for CellBlock {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        self.stream.fmt(formatter)
    }
}

impl Display for Group {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        formatter.write_str(self.delim_kind().opening())?;
        self.stream.fmt(formatter)?;
        formatter.write_str(self.delim_kind().closing())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenTree {
    Group(Group),
    Literal(Literal),
    Punct(Punct),
    Ident(IdentToken),
    Trivia(Trivia),
}

impl Spanned for TokenTree {
    fn span(&self) -> Span {
        match self {
            Self::Group(token) => token.span(),
            Self::Literal(token) => token.span(),
            Self::Punct(token) => token.span(),
            Self::Ident(token) => token.span(),
            Self::Trivia(token) => token.span(),
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

impl Extend<TokenTree> for TokenStream {
    fn extend<T: IntoIterator<Item = TokenTree>>(&mut self, iter: T) {
        self.0.extend(iter);
    }
}

impl IntoIterator for TokenStream {
    type Item = TokenTree;
    type IntoIter = std::vec::IntoIter<TokenTree>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellStream {
    cells: Vec<CellBlock>,
    source_id: crate::SourceId,
}

impl CellStream {
    pub fn new(cells: Vec<CellBlock>, source_id: crate::SourceId) -> Self {
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

    pub fn source_id(&self) -> crate::SourceId {
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
    type IntoIter = std::vec::IntoIter<CellBlock>;

    fn into_iter(self) -> Self::IntoIter {
        self.cells.into_iter()
    }
}

pub trait ToTokens {
    fn to_tokens(&self, output: &mut TokenStream);

    fn tokens(&self) -> TokenStream {
        let mut output = TokenStream::new();
        self.to_tokens(&mut output);
        output
    }

    fn to_m2(&self) -> String {
        self.tokens().to_string()
    }
}

impl<T> ToTokens for &T
where
    T: ToTokens + ?Sized,
{
    fn to_tokens(&self, output: &mut TokenStream) {
        (*self).to_tokens(output);
    }
}

impl<T> ToTokens for Box<T>
where
    T: ToTokens + ?Sized,
{
    fn to_tokens(&self, output: &mut TokenStream) {
        self.as_ref().to_tokens(output);
    }
}

impl<T> ToTokens for Option<T>
where
    T: ToTokens,
{
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
