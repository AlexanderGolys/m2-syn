use std::fmt::{Display, Formatter, Result};

use crate::{Span, Spanned};

use super::{ToTokens, TokenStream};

/// One maximal-munch M2 punctuation token.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Spanned)]
pub struct Punct {
    text: String,
    span: Span,
}

impl Punct {
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

impl Display for Punct {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        formatter.write_str(self.text())
    }
}

impl ToTokens for Punct {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_punct(self.clone());
    }
}
