//! Typed syntax contained by one of the six delimiter families.

use crate::{
    CellStream, DelimiterKind, DelimiterToken, Parse, ParseStream, Span, Spanned, TextRange,
    ToCells, ToTokens, TokenParseError, TokenStream, TokenTree, parse1,
};

/// Syntax contained by a statically known delimiter family.
///
/// `D` is normally named through [`crate::Delimiter!`], for example
/// `Delimited<S, Delimiter![()]>`. The delimiter retains both boundary spans,
/// while `contents` owns the syntax between them.
///
/// ```
/// use m2_syn::{Span, ToTokens};
///
/// type ParenthesizedPlus = m2_syn::paren!(m2_syn::Token![+]);
/// let value: ParenthesizedPlus = m2_syn::paren!(
///     m2_syn::Token![+](Span::detached()),
///     Span::detached(),
/// );
/// assert_eq!(value.to_m2(), "(+)");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
pub struct Delimited<S, D> {
    /// The typed delimiter family and its two boundary spans.
    pub delimiter: D,
    /// The syntax enclosed or terminated by the delimiter.
    pub contents: S,
}

/// Names or constructs parenthesized syntax.
///
/// `paren!(T)` is `Delimited<T, Delimiter![()]>`. The constructor form is
/// `paren!(value, span)` where `span` describes the complete parenthesized value.
#[macro_export]
macro_rules! paren {
    ($contents:expr, $span:expr $(,)?) => {
        $crate::Delimited::new($crate::Delimiter![()]($span), $contents)
    };
    ($contents:ty) => {
        $crate::Delimited<$contents, $crate::Delimiter![()]>
    };
}

/// Names or constructs brace-delimited syntax.
///
/// Use `braces!(T)` as a type and `braces!(value, spans)` as a constructor.
#[macro_export]
macro_rules! braces {
    ($contents:expr, $span:expr $(,)?) => {
        $crate::Delimited::new($crate::Delimiter![{}]($span), $contents)
    };
    ($contents:ty) => {
        $crate::Delimited<$contents, $crate::Delimiter![{}]>
    };
}

/// Names or constructs bracket-delimited syntax.
///
/// Use `brackets!(T)` as a type and `brackets!(value, spans)` as a constructor.
#[macro_export]
macro_rules! brackets {
    ($contents:expr, $span:expr $(,)?) => {
        $crate::Delimited::new($crate::Delimiter![[]]($span), $contents)
    };
    ($contents:ty) => {
        $crate::Delimited<$contents, $crate::Delimiter![[]]>
    };
}

/// Names or constructs angle-bar-delimited syntax.
///
/// Use `angle_bars!(T)` as a type and `angle_bars!(value, spans)` as a
/// constructor.
#[macro_export]
macro_rules! angle_bars {
    ($contents:expr, $span:expr $(,)?) => {
        $crate::Delimited::new($crate::Delimiter![<| |>]($span), $contents)
    };
    ($contents:ty) => {
        $crate::Delimited<$contents, $crate::Delimiter![<| |>]>
    };
}

/// Names or constructs syntax with an implicit empty delimiter.
///
/// Use `naked!(T)` as a type and `naked!(value, spans)` as a constructor.
#[macro_export]
macro_rules! naked {
    ($contents:expr, $span:expr $(,)?) => {
        $crate::Delimited::new($crate::Delimiter![]($span), $contents)
    };
    ($contents:ty) => {
        $crate::Delimited<$contents, $crate::Delimiter![]>
    };
}

/// Names or constructs syntax terminated by a semicolon delimiter.
///
/// Use `semicolon!(T)` as a type and `semicolon!(value, spans)` as a
/// constructor.
#[macro_export]
macro_rules! semicolon {
    ($contents:expr, $span:expr $(,)?) => {
        $crate::Delimited::new($crate::Delimiter![;]($span), $contents)
    };
    ($contents:ty) => {
        $crate::Delimited<$contents, $crate::Delimiter![;]>
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

impl<S, D> Parse for Delimited<S, D>
where
    S: Parse,
    D: DelimiterToken,
{
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        if D::KIND == DelimiterKind::Empty {
            let opening = input
                .peek()
                .map(|token| boundary_before(token.span()))
                .unwrap_or_else(|| input.eof_span());
            let contents = S::parse(input)?;
            let delimiter = D::new(opening.join(input.eof_span()));
            return Ok(Self::new(delimiter, contents));
        }

        if D::KIND == DelimiterKind::Semicolon {
            if input
                .peek()
                .is_some_and(<Token![;] as crate::Token>::matches_token_tree)
            {
                let semicolon = input.peek().unwrap();
                return Err(TokenParseError::UnexpectedToken {
                    expected: "nonempty syntax before `;`",
                    found: ";".to_owned(),
                    span: semicolon.span(),
                });
            }
            let opening = input
                .peek()
                .map(|token| boundary_before(token.span()))
                .unwrap_or_else(|| input.eof_span());
            let contents = S::parse(input)?;
            let semicolon = <Token![;]>::parse(input)?;
            let delimiter = D::new(opening.join(semicolon.span()));
            return Ok(Self::new(delimiter, contents));
        }

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

        let delimiter = D::new(group.span());
        let contents = parse1(group.into_stream())?;
        Ok(Self::new(delimiter, contents))
    }
}

fn boundary_before(span: Span) -> Span {
    span.source()
        .ok()
        .zip(span.start_point().ok())
        .map(|(source, point)| Span::new(source, TextRange::from_point(point)))
        .unwrap_or_else(Span::detached)
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

impl<S, D> ToCells for Delimited<S, D>
where
    S: ToCells,
{
    fn to_cells(&self, output: &mut CellStream) {
        self.contents.to_cells(output);
    }
}
