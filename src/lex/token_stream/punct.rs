use std::sync::Arc;

use crate::{Span, Spanned};

use super::{Lex, TokenStream, TokenTree};

/// One maximal-munch M2 punctuation token.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Spanned)]
pub struct Punct {
    span: Span,
}

impl Punct {
    /// Creates punctuation from its source span.
    pub fn from_span(span: Span) -> Self {
        Self { span }
    }

    /// Creates punctuation with its source spelling and span.
    pub fn new(text: impl Into<Arc<str>>, span: Span) -> Self {
        Self {
            span: span.with_text(text),
        }
    }

    /// Returns the punctuation spelling.
    ///
    /// # Panics
    ///
    /// Panics when `sources` cannot resolve this token's span.
    pub fn text(&self) -> &str {
        self.span
            .text()
            .expect("punctuation spans must resolve to source text")
    }
}

impl Lex for Punct {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push(TokenTree::Punct(self.clone()));
    }
}
