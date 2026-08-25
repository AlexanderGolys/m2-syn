use std::sync::Arc;

use crate::{Source, Span, Spanned};

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
        let text = text.into();
        let span = if span.text() == Some(text.as_ref()) {
            span
        } else {
            let source = Source::with_file(span.file(), text);
            let len = source.text().len();
            Span::ranged(source, 0..len)
        };
        Self { span }
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
        output.0.push(TokenTree::Punct(self.clone()));
    }
}
