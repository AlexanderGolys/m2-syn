//! Syntax sequences containing at least one typed punctuation token.

use std::iter::{Chain, Map, Once, once};
use std::slice;
use std::slice::IterMut;
use std::vec::IntoIter;

use crate::{Lex, Parse, ParseStream, Spanned, Token, TokenParseError, TokenStream};

/// Two syntax values paired by one punctuation token.
///
/// Unlike [`Punctuated`], this type represents exactly one relation: a left
/// value, its separator, and a right value. It is useful when a grammar field
/// requires exactly two operands rather than an arbitrarily long sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Spanned)]
pub struct SeparatedPair<T, P = Token![,]> {
    /// The value before the punctuation.
    pub left: T,
    /// The punctuation pairing the two values.
    pub punctuation: P,
    /// The value after the punctuation.
    pub right: T,
}

impl<T, P> SeparatedPair<T, P> {
    /// Creates a pair from its two values and their punctuation.
    pub fn new(left: T, punctuation: P, right: T) -> Self {
        Self {
            left,
            punctuation,
            right,
        }
    }

    /// Separates the pair into its two values and punctuation.
    pub fn into_parts(self) -> (T, P, T) {
        (self.left, self.punctuation, self.right)
    }
}

impl<T: Parse, P: Parse> Parse for SeparatedPair<T, P> {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        Ok(Self::new(
            T::parse(input)?,
            P::parse(input)?,
            T::parse(input)?,
        ))
    }
}

impl<T: Lex, P: Lex> Lex for SeparatedPair<T, P> {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.left.to_tokens(output);
        self.punctuation.to_tokens(output);
        self.right.to_tokens(output);
    }
}

/// Names or constructs a comma-punctuated sequence.
///
/// - `punc!(T)` names `Punctuated<T>`.
/// - `punc!(pairs first; comma => second, ...)` constructs a sequence while
///   retaining every supplied typed comma and its span.
#[macro_export]
macro_rules! punc {
    (
        pairs $first:expr;
        $punctuation:expr => $second:expr
        $(, $rest_punctuation:expr => $rest_value:expr)*
        $(,)?
    ) => {{
        let mut values = $crate::Punctuated::new($first, $punctuation, $second);
        $(values.push($rest_punctuation, $rest_value);)*
        values
    }};
    ($value:ty) => {
        $crate::Punctuated<$value>
    };
}

/// Two or more syntax values separated by typed punctuation.
///
/// Every separator is stored as its actual token, including its source span.
/// At least one separator and the values on both sides are mandatory. A single
/// value belongs directly in the surrounding syntax type, while an optional
/// whole sequence is `Option<Punctuated<T, P>>`.
///
/// ```
/// use m2_syn::{Lex, Span};
///
/// type Pluses = m2_syn::punc!(m2_syn::Token![+]);
/// let comma = m2_syn::Token![,](Span::for_text(","));
/// let values: Pluses = m2_syn::punc!(
///     pairs m2_syn::Token![+](Span::for_text("+"));
///     comma => m2_syn::Token![+](Span::for_text("+")),
/// );
/// assert_eq!(values.to_m2(), "+,+");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
pub struct Punctuated<S, P = Token![,]> {
    first: S,
    second: (P, S),
    rest: Vec<(P, S)>,
}

impl<S, P> Punctuated<S, P> {
    /// Creates a sequence from the first two values and their punctuation.
    pub fn new(first: S, punctuation: P, second: S) -> Self {
        Self {
            first,
            second: (punctuation, second),
            rest: Vec::new(),
        }
    }

    /// Creates a two-value sequence from one punctuated pair.
    pub fn from_pair(pair: SeparatedPair<S, P>) -> Self {
        Self::from_pair_and_rest(pair, Vec::new())
    }

    /// Creates a sequence from its first separated pair and any later
    /// punctuation-value pairs.
    pub fn from_pair_and_rest(pair: SeparatedPair<S, P>, rest: Vec<(P, S)>) -> Self {
        let (left, punctuation, right) = pair.into_parts();
        Self {
            first: left,
            second: (punctuation, right),
            rest,
        }
    }

    /// Returns the first value.
    pub fn first(&self) -> &S {
        &self.first
    }

    /// Returns the first value mutably.
    pub fn first_mut(&mut self) -> &mut S {
        &mut self.first
    }

    /// Borrows every punctuation-value pair following the first value.
    pub fn pairs(&self) -> Pairs<'_, S, P> {
        once(&self.second).chain(self.rest.iter())
    }

    /// Mutably borrows every punctuation-value pair following the first value.
    pub fn pairs_mut(&mut self) -> PairsMut<'_, S, P> {
        once(&mut self.second).chain(self.rest.iter_mut())
    }

    /// Separates the sequence into its first pair and any later pairs.
    pub fn into_parts(self) -> (SeparatedPair<S, P>, Vec<(P, S)>) {
        let (punctuation, second) = self.second;
        (
            SeparatedPair::new(self.first, punctuation, second),
            self.rest,
        )
    }

    /// Returns the number of values, excluding punctuation.
    pub fn len(&self) -> usize {
        self.rest.len() + 2
    }

    /// Returns `false`; a punctuated sequence always contains at least two values.
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// Returns the number of values and punctuation tokens together.
    pub fn token_len(&self) -> usize {
        self.rest.len() * 2 + 3
    }

    /// Borrows the values without discarding or synthesizing punctuation.
    pub fn iter(&self) -> Values<'_, S, P> {
        once(&self.first).chain(
            self.pairs()
                .map(rest_value_ref::<S, P> as fn(&(P, S)) -> &S),
        )
    }

    /// Mutably borrows the values without discarding or synthesizing
    /// punctuation.
    pub fn iter_mut(&mut self) -> ValuesMut<'_, S, P> {
        once(&mut self.first).chain(
            once(&mut self.second)
                .chain(self.rest.iter_mut())
                .map(rest_value_mut::<S, P> as fn(&mut (P, S)) -> &mut S),
        )
    }

    /// Appends a real punctuation token and the value following it.
    pub fn push(&mut self, punctuation: P, value: S) {
        self.rest.push((punctuation, value));
    }

    /// Maps the values while retaining every punctuation token unchanged.
    pub fn map_values<T>(self, mut map: impl FnMut(S) -> T) -> Punctuated<T, P> {
        let (pair, rest) = self.into_parts();
        let (first, punctuation, second) = pair.into_parts();
        Punctuated::from_pair_and_rest(
            SeparatedPair::new(map(first), punctuation, map(second)),
            rest.into_iter()
                .map(|(punctuation, value)| (punctuation, map(value)))
                .collect(),
        )
    }

    /// Fallibly maps both values and punctuation while retaining their order.
    ///
    /// # Errors
    ///
    /// Returns the first error produced by either mapping function.
    pub fn try_map<T, Q, E>(
        self,
        mut map_value: impl FnMut(S) -> Result<T, E>,
        mut map_punct: impl FnMut(P) -> Result<Q, E>,
    ) -> Result<Punctuated<T, Q>, E> {
        let (pair, rest) = self.into_parts();
        let (first, punct, second) = pair.into_parts();
        Ok(Punctuated::from_pair_and_rest(
            SeparatedPair::new(map_value(first)?, map_punct(punct)?, map_value(second)?),
            rest.into_iter()
                .map(|(punct, value)| Ok((map_punct(punct)?, map_value(value)?)))
                .collect::<Result<Vec<_>, E>>()?,
        ))
    }
}

impl<S: Lex, P: Lex> Lex for Punctuated<S, P> {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.first.to_tokens(output);
        for (punct, value) in self.pairs() {
            punct.to_tokens(output);
            value.to_tokens(output);
        }
    }
}

impl<S, P> Parse for Punctuated<S, P>
where
    S: Parse,
    P: Parse + Token,
{
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let first = S::parse(input)?;
        let punct = P::parse(input)?;
        let second = S::parse(input)?;
        let mut values = Self::new(first, punct, second);
        while input.peek().is_some_and(P::matches_token_tree) {
            let punct = P::parse(input)?;
            values.push(punct, S::parse(input)?);
        }
        Ok(values)
    }
}

/// An iterator borrowing punctuation-value pairs from a [`Punctuated`] sequence.
pub type Pairs<'a, S, P> = Chain<Once<&'a (P, S)>, slice::Iter<'a, (P, S)>>;

/// An iterator mutably borrowing punctuation-value pairs from a [`Punctuated`] sequence.
pub type PairsMut<'a, S, P> = Chain<Once<&'a mut (P, S)>, IterMut<'a, (P, S)>>;

/// An iterator borrowing only the values of a [`Punctuated`] sequence.
pub type Values<'a, S, P> = Chain<Once<&'a S>, Map<Pairs<'a, S, P>, fn(&(P, S)) -> &S>>;

/// An iterator mutably borrowing only the values of a [`Punctuated`] sequence.
pub type ValuesMut<'a, S, P> =
    Chain<Once<&'a mut S>, Map<PairsMut<'a, S, P>, fn(&mut (P, S)) -> &mut S>>;

/// An iterator owning punctuation-value pairs from a [`Punctuated`] sequence.
pub type IntoPairs<S, P> = Chain<Once<(P, S)>, IntoIter<(P, S)>>;

/// An iterator owning only the values of a [`Punctuated`] sequence.
pub type IntoValues<S, P> = Chain<Once<S>, Map<IntoPairs<S, P>, fn((P, S)) -> S>>;

impl<S, P> IntoIterator for Punctuated<S, P> {
    type Item = S;
    type IntoIter = IntoValues<S, P>;

    fn into_iter(self) -> Self::IntoIter {
        once(self.first).chain(
            once(self.second)
                .chain(self.rest)
                .map(rest_value::<S, P> as fn((P, S)) -> S),
        )
    }
}

impl<'a, S, P> IntoIterator for &'a Punctuated<S, P> {
    type Item = &'a S;
    type IntoIter = Values<'a, S, P>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, S, P> IntoIterator for &'a mut Punctuated<S, P> {
    type Item = &'a mut S;
    type IntoIter = ValuesMut<'a, S, P>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// Handles rest value.
fn rest_value<S, P>((_, value): (P, S)) -> S {
    value
}

/// Handles rest value ref.
fn rest_value_ref<S, P>((_, value): &(P, S)) -> &S {
    value
}

/// Handles rest value mut.
fn rest_value_mut<S, P>((_, value): &mut (P, S)) -> &mut S {
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sequence_is_nonempty_and_retains_separator_spans() {
        let source = "+,+";
        let values: Punctuated<Token![+], Token![,]> = crate::parse_fragment_str(source).unwrap();
        let comma_span = values.pairs().next().unwrap().0.expect_span();

        assert_eq!(values.len(), 2);
        assert_eq!(values.token_len(), 3);
        assert_eq!(comma_span.text(), Some(","));
        assert_eq!(comma_span.bounds(), Some((1, 2)));
        assert_eq!(values.to_m2(), "+,+");
    }

    #[test]
    fn a_single_value_is_not_a_punctuated_sequence() {
        let result = crate::parse_fragment_str::<Punctuated<Token![+]>>("+");

        assert!(result.is_err());
    }
}
