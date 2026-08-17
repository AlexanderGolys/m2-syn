//! Typed syntax contained by one matched delimiter pair.

use crate::{
    DelimiterToken, Parse, ParseStream, Span, Spanned, ToTokens, TokenParseError, TokenStream,
    TokenTree, parse2,
};

/// Syntax contained by a statically known delimiter family.
///
/// `D` is normally named through [`crate::Delimiter!`], for example
/// `Delimited<S, Delimiter![()]>`. The delimiter retains both boundary spans,
/// while `contents` owns the syntax between them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delimited<S, D> {
    pub delimiter: D,
    pub contents: S,
}

#[macro_export]
macro_rules! paren {
    ($contents:ty) => {
        $crate::delimited::Delimited<$contents, $crate::Delimiter![()]>
    };
}

#[macro_export]
macro_rules! braces {
    ($contents:ty) => {
        $crate::delimited::Delimited<$contents, $crate::Delimiter![{}]>
    };
}

#[macro_export]
macro_rules! brackets {
    ($contents:ty) => {
        $crate::delimited::Delimited<$contents, $crate::Delimiter![[]]>
    };
}

#[macro_export]
macro_rules! angle_bars {
    ($contents:ty) => {
        $crate::delimited::Delimited<$contents, $crate::Delimiter![<||>]>
    };
}

#[macro_export]
macro_rules! naked {
    ($contents:ty) => {
        $crate::delimited::Delimited<$contents, $crate::Delimiter![]>
    };
}

impl<S, D> Delimited<S, D> {
    /// Creates a delimited value from its delimiter and contents.
    pub fn new(delimiter: D, contents: S) -> Self {
        Self {
            delimiter,
            contents,
        }
    }

    /// Separates the delimiter from the contained syntax.
    pub fn into_parts(self) -> (D, S) {
        (self.delimiter, self.contents)
    }
}

impl<S, D: DelimiterToken> Spanned for Delimited<S, D> {
    fn span(&self) -> Span {
        self.delimiter.span()
    }
}

impl<S, D> Parse for Delimited<S, D>
where
    S: Parse,
    D: DelimiterToken,
{
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let Some(token) = input.next_token() else {
            return Err(TokenParseError::UnexpectedEnd {
                expected: D::KIND.opening(),
                span: input.eof_span(),
            });
        };
        let span = token.span();
        let TokenTree::Group(group) = token else {
            return Err(TokenParseError::UnexpectedToken {
                expected: D::KIND.opening(),
                found: token.spelling().unwrap_or("trivia").to_owned(),
                span,
            });
        };
        if group.delim_kind() != D::KIND {
            return Err(TokenParseError::UnexpectedToken {
                expected: D::KIND.opening(),
                found: group.delim_kind().to_string(),
                span,
            });
        }

        let delimiter = D::new(group.double_span());
        let contents = parse2(group.into_stream())?;
        Ok(Self::new(delimiter, contents))
    }
}

impl<S, D> ToTokens for Delimited<S, D>
where
    S: ToTokens,
    D: DelimiterToken,
{
    fn to_tokens(&self, output: &mut TokenStream) {
        self.delimiter
            .surround(output, self.contents.to_token_stream());
    }
}
