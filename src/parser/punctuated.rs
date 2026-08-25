//! Nonempty syntax sequences separated by typed punctuation tokens.

use std::iter::{Chain, Once, once};
use std::{slice, vec};

use crate::{Lex, Parse, ParseStream, Spanned, Token, TokenParseError, TokenStream};

/// Two syntax values paired by one punctuation token.
///
/// Unlike [`Punctuated`], this type represents exactly one relation: a left
/// value, its separator, and a right value. It is useful when a grammar field
/// requires exactly two operands rather than an arbitrarily long sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Spanned)]
pub struct PunctPair<L, P = Token![,], R = L> {
    /// The value before the punctuation.
    pub left: L,
    /// The punctuation pairing the two values.
    pub punctuation: P,
    /// The value after the punctuation.
    pub right: R,
}

impl<L, P, R> PunctPair<L, P, R> {
    /// Creates a pair from its two values and their punctuation.
    pub fn new(left: L, punctuation: P, right: R) -> Self {
        Self {
            left,
            punctuation,
            right,
        }
    }

    /// Separates the pair into its two values and punctuation.
    pub fn into_parts(self) -> (L, P, R) {
        (self.left, self.punctuation, self.right)
    }
}

impl<L: Parse, P: Parse, R: Parse> Parse for PunctPair<L, P, R> {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        Ok(Self::new(
            L::parse(input)?,
            P::parse(input)?,
            R::parse(input)?,
        ))
    }
}

impl<L: Lex, P: Lex, R: Lex> Lex for PunctPair<L, P, R> {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.left.to_tokens(output);
        self.punctuation.to_tokens(output);
        self.right.to_tokens(output);
    }
}

/// Names or constructs a nonempty comma-punctuated sequence.
///
/// - `punc!(T)` names `Punctuated<T>`.
/// - `punc!(value first)` constructs one value.
/// - `punc!(pairs first; comma => second, ...)` constructs a sequence while
///   retaining every supplied typed comma and its span.
#[macro_export]
macro_rules! punc {
    (value $value:expr) => {
        $crate::Punctuated::from_single($value)
    };
    (pairs $first:expr; $($punctuation:expr => $next:expr),+ $(,)?) => {{
        let mut values = $crate::Punctuated::from_single($first);
        $(values.push($punctuation, $next);)+
        values
    }};
    ($value:ty) => {
        $crate::Punctuated<$value>
    };
}

/// A nonempty sequence of syntax values separated by typed punctuation.
///
/// Every separator is stored as its actual token, including its source span.
/// The first value is mandatory; optional or empty grammar positions belong in
/// the element type, such as `Option<T>` or a syntax enum containing an empty
/// variant, while an optional whole sequence is `Option<Punctuated<T, P>>`.
///
/// ```
/// use m2_syn::{Lex, Span};
///
/// type Pluses = m2_syn::punc!(m2_syn::Token![+]);
/// let comma = m2_syn::Token![,](Span::detached());
/// let values: Pluses = m2_syn::punc!(
///     pairs m2_syn::Token![+](Span::detached());
///     comma => m2_syn::Token![+](Span::detached()),
/// );
/// assert_eq!(values.to_m2(), "+,+");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
pub struct Punctuated<S, P = Token![,]> {
    first: S,
    rest: Vec<(P, S)>,
}

impl<S, P> Punctuated<S, P> {
    /// Creates a sequence from its first value and remaining punctuation-value
    /// pairs.
    pub fn new(first: S, rest: Vec<(P, S)>) -> Self {
        Self { first, rest }
    }

    /// Creates a sequence containing exactly one value.
    pub fn from_single(value: S) -> Self {
        Self::new(value, Vec::new())
    }

    /// Creates a two-value sequence from one punctuated pair.
    pub fn from_pair(pair: PunctPair<S, P>) -> Self {
        let (left, punctuation, right) = pair.into_parts();
        Self::new(left, vec![(punctuation, right)])
    }

    /// Returns the first value.
    pub fn first(&self) -> &S {
        &self.first
    }

    /// Returns the first value mutably.
    pub fn first_mut(&mut self) -> &mut S {
        &mut self.first
    }

    /// Returns the punctuation-value pairs following the first value.
    pub fn rest(&self) -> &[(P, S)] {
        &self.rest
    }

    /// Returns the punctuation-value pairs following the first value mutably.
    pub fn rest_mut(&mut self) -> &mut [(P, S)] {
        &mut self.rest
    }

    /// Separates the sequence into its first value and remaining pairs.
    pub fn into_parts(self) -> (S, Vec<(P, S)>) {
        (self.first, self.rest)
    }

    /// Returns the number of values, excluding punctuation.
    pub fn len(&self) -> usize {
        self.rest.len() + 1
    }

    /// Returns `false`; a punctuated sequence always contains its first value.
    pub const fn is_empty(&self) -> bool {
        false
    }

    /// Returns the number of values and punctuation tokens together.
    pub fn token_len(&self) -> usize {
        self.rest.len() * 2 + 1
    }

    /// Borrows the values without discarding or synthesizing punctuation.
    pub fn iter(&self) -> Values<'_, S, P> {
        once(&self.first).chain(
            self.rest
                .iter()
                .map(rest_value_ref::<S, P> as fn(&(P, S)) -> &S),
        )
    }

    /// Mutably borrows the values without discarding or synthesizing
    /// punctuation.
    pub fn iter_mut(&mut self) -> ValuesMut<'_, S, P> {
        once(&mut self.first).chain(
            self.rest
                .iter_mut()
                .map(rest_value_mut::<S, P> as fn(&mut (P, S)) -> &mut S),
        )
    }

    /// Appends a real punctuation token and the value following it.
    pub fn push(&mut self, punctuation: P, value: S) {
        self.rest.push((punctuation, value));
    }
}

impl<S: Lex, P: Lex> Lex for Punctuated<S, P> {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.first.to_tokens(output);
        for (punctuation, value) in &self.rest {
            punctuation.to_tokens(output);
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
        let mut values = Self::from_single(first);
        while input.peek().is_some_and(P::matches_token_tree) {
            let punctuation = P::parse(input)?;
            values.push(punctuation, S::parse(input)?);
        }
        Ok(values)
    }
}

/// An iterator borrowing only the values of a [`Punctuated`] sequence.
pub type Values<'a, S, P> =
    Chain<Once<&'a S>, std::iter::Map<slice::Iter<'a, (P, S)>, fn(&(P, S)) -> &S>>;

/// An iterator mutably borrowing only the values of a [`Punctuated`] sequence.
pub type ValuesMut<'a, S, P> =
    Chain<Once<&'a mut S>, std::iter::Map<slice::IterMut<'a, (P, S)>, fn(&mut (P, S)) -> &mut S>>;

/// An iterator owning only the values of a [`Punctuated`] sequence.
pub type IntoValues<S, P> = Chain<Once<S>, std::iter::Map<vec::IntoIter<(P, S)>, fn((P, S)) -> S>>;

impl<S, P> IntoIterator for Punctuated<S, P> {
    type Item = S;
    type IntoIter = IntoValues<S, P>;

    fn into_iter(self) -> Self::IntoIter {
        once(self.first).chain(
            self.rest
                .into_iter()
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

fn rest_value<S, P>((_, value): (P, S)) -> S {
    value
}

fn rest_value_ref<S, P>((_, value): &(P, S)) -> &S {
    value
}

fn rest_value_mut<S, P>((_, value): &mut (P, S)) -> &mut S {
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Span;

    #[test]
    fn a_sequence_is_nonempty_and_retains_separator_spans() {
        let comma_span = Span::new(1, 2);
        let values = Punctuated::new(
            Token![+](Span::new(0, 1)),
            vec![(Token![,](&comma_span), Token![+](Span::new(2, 3)))],
        );

        assert_eq!(values.len(), 2);
        assert_eq!(values.token_len(), 3);
        assert_eq!(values.rest()[0].0.span().bounds(), comma_span.bounds());
        assert_eq!(values.to_m2(), "+,+");
    }
}
