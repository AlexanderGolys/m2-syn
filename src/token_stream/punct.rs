use std::fmt::{Display, Formatter};

use crate::{Span, Spanned};

/// One maximal-munch M2 punctuation token.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
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

impl Spanned for Punct {
    fn span(&self) -> Span {
        self.span
    }
}

impl Display for Punct {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.text())
    }
}
