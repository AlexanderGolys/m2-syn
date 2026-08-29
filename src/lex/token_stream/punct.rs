use std::sync::Arc;

use crate::{Source, Span, Spanned};

use super::{LeadingTrivia, Lex, Terminal, TokenStream, TokenTree, TrailingTrivia};

/// One maximal-munch M2 punctuation token.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Spanned)]
pub struct Punct {
    /// The span.
    span: Span,
    /// The leading trivia.
    leading_trivia: LeadingTrivia,
    /// The trailing trivia.
    trailing_trivia: TrailingTrivia,
}

impl Punct {
    /// Creates punctuation from its source span.
    pub fn from_span(span: Span) -> Self {
        Self {
            span,
            leading_trivia: LeadingTrivia::default(),
            trailing_trivia: TrailingTrivia::default(),
        }
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
        Self {
            span,
            leading_trivia: LeadingTrivia::default(),
            trailing_trivia: TrailingTrivia::default(),
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

impl Terminal for Punct {
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
