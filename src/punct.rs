//! Comma-punctuated syntax sequences and their iterator views.

use std::{option, slice, vec};

use crate::cst::Token;
use crate::{Parse, ParseStream, Span, Spanned, TextRange, ToTokens, TokenParseError, TokenStream};

type Comma = Token![,];
type Semicolon = Token![;];

/// A zero-width component occupying an omitted position beside a comma.
///
/// Empty components evaluate as null but emit no token text. Their spans mark
/// the source position at which the omitted component occurs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Empty {
    span: Span,
}

impl Empty {
    /// Creates an empty component at `span`.
    pub fn new(span: Span) -> Self {
        Self { span }
    }
}

impl Spanned for Empty {
    fn span(&self) -> Span {
        self.span
    }
}

impl ToTokens for Empty {
    fn to_tokens(&self, _output: &mut TokenStream) {}
}

/// One value together with its following punctuation, or the unpunctuated end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pair<S, P = Comma> {
    Punctuated(S, P),
    End(S),
}

impl<S, P> Pair<S, P> {
    /// Returns the value independently of whether it has following punctuation.
    pub fn value(&self) -> &S {
        match self {
            Self::Punctuated(value, _) | Self::End(value) => value,
        }
    }

    /// Separates the value from its optional following punctuation.
    pub fn into_tuple(self) -> (S, Option<P>) {
        match self {
            Self::Punctuated(value, punctuation) => (value, Some(punctuation)),
            Self::End(value) => (value, None),
        }
    }
}

/// A sequence of syntax values separated by typed comma tokens.
///
/// Ordinary iteration exposes only values. Use [`Punctuated::pairs`] or
/// [`Punctuated::into_pairs`] when punctuation and its spans are significant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Punctuated<S> {
    inner: Vec<(S, Comma)>,
    last: Option<Box<S>>,
}

impl<S> Punctuated<S> {
    /// Creates an empty sequence.
    pub const fn new() -> Self {
        Self {
            inner: Vec::new(),
            last: None,
        }
    }

    /// Returns the number of values, excluding punctuation.
    pub fn len(&self) -> usize {
        self.inner.len() + usize::from(self.last.is_some())
    }

    /// Reports whether the sequence contains no values or punctuation.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty() && self.last.is_none()
    }

    /// Creates a sequence containing one value.
    pub fn from_value(value: S) -> Self {
        Self {
            inner: Vec::new(),
            last: Some(Box::new(value)),
        }
    }

    /// Borrows the values without exposing punctuation.
    pub fn iter(&self) -> Iter<'_, S> {
        Iter {
            inner: self.pairs(),
        }
    }

    /// Mutably borrows the values without exposing punctuation.
    pub fn iter_mut(&mut self) -> IterMut<'_, S> {
        IterMut {
            inner: self.pairs_mut(),
        }
    }

    /// Borrows values together with their optional following commas.
    pub fn pairs(&self) -> Pairs<'_, S> {
        Pairs {
            inner: self.inner.iter(),
            last: self.last.as_deref().into_iter(),
        }
    }

    /// Mutably borrows values together with their optional following commas.
    pub fn pairs_mut(&mut self) -> PairsMut<'_, S> {
        PairsMut {
            inner: self.inner.iter_mut(),
            last: self.last.as_deref_mut().into_iter(),
        }
    }

    /// Consumes the sequence while preserving commas in the yielded pairs.
    pub fn into_pairs(self) -> IntoPairs<S> {
        IntoPairs {
            inner: self.inner.into_iter(),
            last: self.last.into_iter(),
        }
    }

    /// Adds a comma and the component on its right.
    ///
    /// Panics if the sequence is empty. Empty components must be represented
    /// explicitly by converting [`Empty`] into `S` before calling this method.
    pub fn push(&mut self, punctuation: Comma, next: S) {
        let previous = self
            .last
            .take()
            .expect("cannot punctuate an empty sequence");
        self.inner.push((*previous, punctuation));
        self.last = Some(Box::new(next));
    }
}

impl<S> Default for Punctuated<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Parse for Punctuated<S>
where
    S: Parse + From<Empty>,
{
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let mut values = Self::new();
        if input.is_empty() {
            return Ok(values);
        }

        let first = if input
            .peek()
            .is_some_and(<Comma as Token>::matches_token_tree)
        {
            S::from(Empty::new(empty_before(input.peek().unwrap().span())))
        } else {
            S::parse(input)?
        };
        values = Self::from_value(first);

        while input
            .peek()
            .is_some_and(<Comma as Token>::matches_token_tree)
        {
            let punctuation = Comma::parse(input)?;
            let next = if input.is_empty()
                || input.peek().is_some_and(|token| {
                    <Comma as Token>::matches_token_tree(token)
                        || <Semicolon as Token>::matches_token_tree(token)
                }) {
                let span = input
                    .peek()
                    .map(|token| empty_before(token.span()))
                    .unwrap_or_else(|| input.eof_span());
                S::from(Empty::new(span))
            } else {
                S::parse(input)?
            };
            values.push(punctuation, next);
        }
        Ok(values)
    }
}

impl<S: Spanned> Spanned for Punctuated<S> {
    fn span(&self) -> Span {
        Span::join_all(
            self.iter()
                .map(Spanned::span)
                .chain(self.inner.iter().map(|(_, punctuation)| punctuation.span())),
        )
    }
}

impl<S: ToTokens> ToTokens for Punctuated<S> {
    fn to_tokens(&self, output: &mut TokenStream) {
        for pair in self.pairs() {
            match pair {
                Pair::Punctuated(value, punctuation) => {
                    value.to_tokens(output);
                    punctuation.to_tokens(output);
                }
                Pair::End(value) => value.to_tokens(output),
            }
        }
    }
}

impl<S> IntoIterator for Punctuated<S> {
    type Item = S;
    type IntoIter = IntoIter<S>;

    fn into_iter(self) -> Self::IntoIter {
        let mut values = Vec::with_capacity(self.len());
        values.extend(self.inner.into_iter().map(|(value, _)| value));
        values.extend(self.last.map(|value| *value));
        IntoIter {
            inner: values.into_iter(),
        }
    }
}

impl<'a, S> IntoIterator for &'a Punctuated<S> {
    type Item = &'a S;
    type IntoIter = Iter<'a, S>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a, S> IntoIterator for &'a mut Punctuated<S> {
    type Item = &'a mut S;
    type IntoIter = IterMut<'a, S>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

/// An iterator over owned values that intentionally discards commas.
pub struct IntoIter<S> {
    inner: vec::IntoIter<S>,
}

impl<S> Iterator for IntoIter<S> {
    type Item = S;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.inner.len(), Some(self.inner.len()))
    }
}

impl<S> DoubleEndedIterator for IntoIter<S> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back()
    }
}

impl<S> ExactSizeIterator for IntoIter<S> {}

/// An iterator over borrowed values that hides commas.
pub struct Iter<'a, S> {
    inner: Pairs<'a, S>,
}

impl<'a, S> Iterator for Iter<'a, S> {
    type Item = &'a S;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|pair| pair.into_tuple().0)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S> DoubleEndedIterator for Iter<'_, S> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|pair| pair.into_tuple().0)
    }
}

impl<S> ExactSizeIterator for Iter<'_, S> {}

/// An iterator over mutably borrowed values that hides commas.
pub struct IterMut<'a, S> {
    inner: PairsMut<'a, S>,
}

impl<'a, S> Iterator for IterMut<'a, S> {
    type Item = &'a mut S;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next().map(|pair| pair.into_tuple().0)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.inner.size_hint()
    }
}

impl<S> DoubleEndedIterator for IterMut<'_, S> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.inner.next_back().map(|pair| pair.into_tuple().0)
    }
}

impl<S> ExactSizeIterator for IterMut<'_, S> {}

/// An iterator over borrowed value/comma pairs.
pub struct Pairs<'a, S> {
    inner: slice::Iter<'a, (S, Comma)>,
    last: option::IntoIter<&'a S>,
}

impl<'a, S> Iterator for Pairs<'a, S> {
    type Item = Pair<&'a S, &'a Comma>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(value, punctuation)| Pair::Punctuated(value, punctuation))
            .or_else(|| self.last.next().map(Pair::End))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.inner.len() + self.last.len();
        (len, Some(len))
    }
}

impl<S> DoubleEndedIterator for Pairs<'_, S> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.last.next().map(Pair::End).or_else(|| {
            self.inner
                .next_back()
                .map(|(value, punctuation)| Pair::Punctuated(value, punctuation))
        })
    }
}

impl<S> ExactSizeIterator for Pairs<'_, S> {}

/// An iterator over mutably borrowed value/comma pairs.
pub struct PairsMut<'a, S> {
    inner: slice::IterMut<'a, (S, Comma)>,
    last: option::IntoIter<&'a mut S>,
}

impl<'a, S> Iterator for PairsMut<'a, S> {
    type Item = Pair<&'a mut S, &'a mut Comma>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(value, punctuation)| Pair::Punctuated(value, punctuation))
            .or_else(|| self.last.next().map(Pair::End))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.inner.len() + self.last.len();
        (len, Some(len))
    }
}

impl<S> DoubleEndedIterator for PairsMut<'_, S> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.last.next().map(Pair::End).or_else(|| {
            self.inner
                .next_back()
                .map(|(value, punctuation)| Pair::Punctuated(value, punctuation))
        })
    }
}

impl<S> ExactSizeIterator for PairsMut<'_, S> {}

/// An iterator over owned value/comma pairs.
pub struct IntoPairs<S> {
    inner: vec::IntoIter<(S, Comma)>,
    last: option::IntoIter<Box<S>>,
}

impl<S> Iterator for IntoPairs<S> {
    type Item = Pair<S>;

    fn next(&mut self) -> Option<Self::Item> {
        self.inner
            .next()
            .map(|(value, punctuation)| Pair::Punctuated(value, punctuation))
            .or_else(|| self.last.next().map(|value| Pair::End(*value)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.inner.len() + self.last.len();
        (len, Some(len))
    }
}

impl<S> DoubleEndedIterator for IntoPairs<S> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.last.next().map(|value| Pair::End(*value)).or_else(|| {
            self.inner
                .next_back()
                .map(|(value, punctuation)| Pair::Punctuated(value, punctuation))
        })
    }
}

impl<S> ExactSizeIterator for IntoPairs<S> {}

fn empty_before(span: Span) -> Span {
    span.source()
        .ok()
        .zip(span.start_point().ok())
        .map(|(source, point)| Span::new(source, TextRange::from_point(point)))
        .unwrap_or_else(Span::detached)
}
