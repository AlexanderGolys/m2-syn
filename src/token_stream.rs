use std::fmt::{Display, Formatter, Result};

use crate::{Span, Spanned, token_stream::punct::Punct};
use delim::{Delimiter, DelimiterKind, DoubleSpan};

pub mod delim;
mod punct;

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentToken {
    text: String,
    span: Span,
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
    stream: Box<TokenStream>,
}

impl Spanned for Group {
    fn span(&self) -> Span {
        self.delimiter.span()
    }
}

impl Group {
    pub fn delimiter(&self) -> &Delimiter {
        &self.delimiter
    }

    pub fn delim_kind(&self) -> DelimiterKind {
        self.delimiter.kind
    }

    pub fn double_span(&self) -> DoubleSpan {
        self.delimiter.span2
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

    pub fn push_text(&mut self, text: impl Into<String>, span: Span) {
        self.0.push(TokenTree::Ident(IdentToken {
            text: text.into(),
            span,
        }));
    }

    pub fn push_synthetic(&mut self, text: impl Into<String>) {
        self.push_text(text, Span::detached());
    }

    pub fn push_space(&mut self) {
        self.push_synthetic(" ");
    }

    pub fn push_group(&mut self, kind: DelimiterKind, stream: TokenStream, span: Span) {
        self.0.push(TokenTree::Group(Group {
            delimiter: Delimiter::new(kind, DoubleSpan::new(span, span)),
            stream: Box::new(stream),
        }));
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
