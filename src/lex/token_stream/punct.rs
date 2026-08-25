use std::fmt::{Display, Formatter, Result};
use std::sync::Arc;

use crate::{Span, Spanned};

use super::{Lex, TokenStream, TokenTree};

/// One maximal-munch M2 punctuation token.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Spanned)]
pub struct Punct {
    span: Span,
}

impl Punct {
    /// Creates punctuation whose spelling is already available through its span.
    ///
    /// # Panics
    ///
    /// Panics when `span` has no source backing.
    pub fn from_span(span: Span) -> Self {
        assert!(
            span.text().is_some(),
            "punctuation spans must name source text"
        );
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
    /// Panics if this token's invariant that its span is source-backed was violated.
    pub fn text(&self) -> &str {
        self.span
            .text()
            .expect("punctuation tokens must have source-backed spans")
    }
}

impl Display for Punct {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        formatter.write_str(self.text())
    }
}

impl Lex for Punct {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.0.push(TokenTree::Punct(self.clone()));
    }
}
