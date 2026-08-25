//! Typed syntax contained by one of the six delimiter families.

use crate::{
    Delimiter, DelimiterValue, Lex, Parse, ParseStream, PrettyNode, PrettyTree, Spanned,
    TokenParseError, TokenStream, parse2, token_stream::delim::parse_any_group,
};

/// Syntax contained by an arbitrary or statically known delimiter family.
///
/// With its default `D = Delimiter`, `Delimited<S>` accepts any delimiter
/// family. Supply a [`crate::Delimiter!`] type, for example
/// `Delimited<S, Delimiter![()]>`, when the family is statically known. The
/// delimiter retains both boundary spans, while `contents` owns the syntax
/// between them.
///
/// ```
/// use m2_syn::{Lex, Source, Span};
///
/// type ParenthesizedPlus = m2_syn::paren!(m2_syn::Token![+]);
/// let span = Span::unlocated(Source::synthetic(""));
/// let value: ParenthesizedPlus = m2_syn::paren!(
///     m2_syn::Token![+](&span),
///     &span,
/// );
/// assert_eq!(value.to_m2(), "(+)");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
pub struct Delimited<S, D = Delimiter> {
    /// The delimiter family and its exact opening and closing spans.
    pub delimiter: D,
    /// The syntax stored between the delimiter boundaries.
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
    D: DelimiterValue,
{
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let group = parse_any_group(input, D::expected())?;
        let raw_delimiter = group.delimiter().clone();
        let delimiter = D::from_delimiter(raw_delimiter.clone()).ok_or_else(|| {
            TokenParseError::unexpected(
                D::expected(),
                raw_delimiter.kind.opening(),
                raw_delimiter.expect_span(),
            )
        })?;
        let contents = parse2(group.into_stream())?;
        Ok(Self::new(delimiter, contents))
    }
}

impl<S, D> Lex for Delimited<S, D>
where
    S: Lex,
    D: DelimiterValue,
{
    fn to_tokens(&self, output: &mut TokenStream) {
        self.delimiter
            .surround(output, self.contents.to_token_stream());
    }
}

impl<S, D> PrettyTree for Delimited<S, D>
where
    S: PrettyTree + Spanned,
    D: DelimiterValue,
{
    fn pretty_tree(&self) -> PrettyNode {
        let delimiter = self.delimiter.delimiter();
        let mut node = PrettyNode::collection("Delimited", self.try_span());
        node.push("contents", &self.contents);
        node.push_node(
            "delimiter",
            PrettyNode::delimiter(delimiter.kind, delimiter.spans),
        );
        node
    }
}

#[cfg(test)]
mod tests {
    use super::Delimited;
    use crate::{DelimiterKind, Lex, lex_str, parse2};

    #[test]
    fn arbitrary_delimiter_retains_each_family() {
        for (source, expected) in [
            ("(+)", DelimiterKind::Parenthesis),
            ("[+]", DelimiterKind::Bracket),
            ("{+}", DelimiterKind::Brace),
            ("<|+|>", DelimiterKind::AngleBar),
        ] {
            let parsed: Delimited<Token![+]> = parse2(lex_str(source).unwrap()).unwrap();
            assert_eq!(parsed.delimiter.kind, expected);
            assert_eq!(parsed.to_m2(), source);
        }
    }
}
