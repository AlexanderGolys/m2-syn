use std::fmt::{Display, Formatter, Result};
use std::mem::take;
use std::ops::Deref;
use std::sync::Arc;
use std::vec::IntoIter;

use crate::{Source, Span, Spanned, lexer::punctuation_would_merge};
use these_macros_should_be_illegal::strutuct;

#[path = "token_stream/delim.rs"]
pub mod delim;
use delim::{Delimiter, DelimiterKind};

#[path = "token_stream/punct.rs"]
/// Internal support for punct.
mod punct;
pub use punct::Punct;

strutuct! {
/// A literal spelling together with its lexical category and source span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiteralToken
    /// The literal's lexical category.
    #[derive(Copy, Hash)]
    kind: |LiteralKind| {
        String,
        RawString,
        Integer,
        Float,
    },
    span: Span,
    leading_trivia: LeadingTrivia,
    trailing_trivia: TrailingTrivia,
}

impl LiteralToken {
    /// Creates a literal token from its source span.
    pub fn from_span(kind: LiteralKind, span: Span) -> Self {
        Self {
            kind,
            span,
            leading_trivia: LeadingTrivia::default(),
            trailing_trivia: TrailingTrivia::default(),
        }
    }

    /// Creates a literal token.
    pub fn new(kind: LiteralKind, text: impl Into<Arc<str>>, span: Span) -> Self {
        Self {
            kind,
            span: span_for_spelling(text.into(), span),
            leading_trivia: LeadingTrivia::default(),
            trailing_trivia: TrailingTrivia::default(),
        }
    }

    /// Returns the complete source spelling, including string delimiters.
    pub fn text(&self) -> &str {
        self.span
            .text()
            .expect("literal spans must resolve to source text")
    }
}

impl Spanned for LiteralToken {
    fn try_span(&self) -> Option<Span> {
        Some(self.span.clone())
    }
}

impl Lex for LiteralToken {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push(TokenTree::Literal(self.clone()));
    }
}

impl Terminal for LiteralToken {
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

#[derive(Debug, Clone, PartialEq, Eq)]
/// An identifier spelling and its source span.
pub struct IdentToken {
    /// The span.
    span: Span,
    /// The source qualifier.
    source_qualifier: Option<Span>,
    /// The index qualifier.
    index_qualifier: Option<Span>,
    /// The leading trivia.
    leading_trivia: LeadingTrivia,
    /// The trailing trivia.
    trailing_trivia: TrailingTrivia,
}

impl IdentToken {
    /// Creates an identifier from its source span.
    ///
    /// # Panics
    ///
    /// Panics when the span cannot resolve its source text.
    pub fn from_span(span: Span) -> Self {
        let text = span
            .text()
            .expect("identifier spans must resolve to source text");
        let (source_qualifier, index_qualifier) = identifier_qualifier_spans(text, &span);
        Self {
            span,
            source_qualifier,
            index_qualifier,
            leading_trivia: LeadingTrivia::default(),
            trailing_trivia: TrailingTrivia::default(),
        }
    }

    /// Creates an identifier token.
    pub fn new(text: impl Into<Arc<str>>, span: Span) -> Self {
        Self::from_span(span_for_spelling(text.into(), span))
    }

    /// Returns the identifier spelling.
    ///
    /// # Panics
    ///
    /// Panics when `sources` cannot resolve this token's span.
    pub fn text(&self) -> &str {
        self.span
            .text()
            .expect("identifier spans must resolve to source text")
    }

    /// Returns the `Source$` prefix, including its `$`, when present.
    pub fn source_qualifier(&self) -> Option<&Span> {
        self.source_qualifier.as_ref()
    }

    /// Returns the `$0` suffix, including its `$`, when present.
    pub fn index_qualifier(&self) -> Option<&Span> {
        self.index_qualifier.as_ref()
    }
}

impl Spanned for IdentToken {
    fn try_span(&self) -> Option<Span> {
        Some(self.span.clone())
    }
}

impl Lex for IdentToken {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push(TokenTree::Ident(self.clone()));
    }
}

impl Terminal for IdentToken {
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

#[derive(Debug, Clone, PartialEq, Eq)]
/// A delimited raw token stream.
pub struct TokenGroup {
    /// The delimiter.
    delimiter: Delimiter,
    /// The stream.
    stream: TokenStream,
    /// The leading trivia.
    leading_trivia: LeadingTrivia,
    /// The trailing trivia.
    trailing_trivia: TrailingTrivia,
}

impl Spanned for TokenGroup {
    fn try_span(&self) -> Option<Span> {
        self.delimiter.try_span()
    }
}

impl TokenGroup {
    /// Creates a group from its delimiter and contents.
    pub fn new(delimiter: Delimiter, stream: TokenStream) -> Self {
        Self {
            delimiter,
            stream,
            leading_trivia: LeadingTrivia::default(),
            trailing_trivia: TrailingTrivia::default(),
        }
    }

    /// Borrows the group's delimiter data.
    pub fn delimiter(&self) -> &Delimiter {
        &self.delimiter
    }

    /// Returns the group's delimiter family.
    pub fn delim_kind(&self) -> DelimiterKind {
        self.delimiter.kind
    }

    /// Borrows the tokens inside the group.
    pub fn stream(&self) -> &TokenStream {
        &self.stream
    }

    /// Mutably borrows the tokens inside the group.
    pub fn stream_mut(&mut self) -> &mut TokenStream {
        &mut self.stream
    }

    /// Consumes the group and returns its contents.
    pub fn into_stream(self) -> TokenStream {
        self.stream
    }

    /// Converts the value into parts.
    pub(crate) fn into_parts(self) -> (Delimiter, TokenStream, LeadingTrivia, TrailingTrivia) {
        (
            self.delimiter,
            self.stream,
            self.leading_trivia,
            self.trailing_trivia,
        )
    }

    /// Creates the value from parts.
    pub(crate) fn from_parts(
        delimiter: Delimiter,
        stream: TokenStream,
        leading_trivia: LeadingTrivia,
        trailing_trivia: TrailingTrivia,
    ) -> Self {
        Self {
            delimiter,
            stream,
            leading_trivia,
            trailing_trivia,
        }
    }
}

impl Lex for TokenGroup {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push(TokenTree::Group(self.clone()));
    }
}

impl Terminal for TokenGroup {
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

strutuct! {
/// A comment retained alongside a lexical atom.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Spanned)]
pub struct Trivia {
    /// The kind.
    #[derive(Copy)]
    kind: |TriviaKind| {
        LineComment,
        BlockComment
    },
    /// The span.
    span: Span,
}}

impl Trivia {
    /// Creates trivia from its category and source span.
    pub fn from_span(kind: TriviaKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Creates trivia from its category, exact text, and span.
    pub fn new(kind: TriviaKind, text: impl Into<Arc<str>>, span: Span) -> Self {
        Self {
            kind,
            span: span_for_spelling(text.into(), span),
        }
    }

    /// Returns the trivia category.
    pub fn kind(&self) -> TriviaKind {
        self.kind
    }

    /// Returns the trivia exactly as it appeared in source.
    ///
    /// # Panics
    ///
    /// Panics when `sources` cannot resolve this trivia's span.
    pub fn text(&self) -> &str {
        self.span
            .text()
            .expect("trivia spans must resolve to source text")
    }
}

impl Lex for Trivia {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_trailing_trivia(self.clone());
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
/// Comments owned by the typed terminal to their left.
pub struct TrailingTrivia(Vec<Trivia>);

impl TrailingTrivia {
    /// Creates retained trailing trivia from raw trivia tokens.
    pub fn new(trivia: impl IntoIterator<Item = Trivia>) -> Self {
        Self(trivia.into_iter().collect())
    }

    /// Borrows the retained trivia in source order.
    pub fn as_slice(&self) -> &[Trivia] {
        &self.0
    }

    /// Mutably borrows the retained trivia in source order.
    pub fn as_mut_vec(&mut self) -> &mut Vec<Trivia> {
        &mut self.0
    }

    /// Reports whether no comments follow the terminal.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Consumes the collection and returns its trivia tokens.
    pub fn into_vec(self) -> Vec<Trivia> {
        self.0
    }
}

impl FromIterator<Trivia> for TrailingTrivia {
    fn from_iter<T: IntoIterator<Item = Trivia>>(iter: T) -> Self {
        Self::new(iter)
    }
}

impl Spanned for TrailingTrivia {
    fn try_span(&self) -> Option<Span> {
        Span::join_all(self.0.iter().map(Spanned::try_span))
    }
}

impl Lex for TrailingTrivia {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.0.to_tokens(output);
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
/// Comments owned by the typed terminal to their right.
pub struct LeadingTrivia(Vec<Trivia>);

impl LeadingTrivia {
    /// Creates leading trivia from raw trivia tokens.
    pub fn new(trivia: impl IntoIterator<Item = Trivia>) -> Self {
        Self(trivia.into_iter().collect())
    }

    /// Borrows the retained trivia in source order.
    pub fn as_slice(&self) -> &[Trivia] {
        &self.0
    }

    /// Mutably borrows the comment storage in source order.
    pub fn as_mut_vec(&mut self) -> &mut Vec<Trivia> {
        &mut self.0
    }

    /// Reports whether the block is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Consumes the collection and returns its comments.
    pub fn into_vec(self) -> Vec<Trivia> {
        self.0
    }
}

impl FromIterator<Trivia> for LeadingTrivia {
    fn from_iter<T: IntoIterator<Item = Trivia>>(iter: T) -> Self {
        Self::new(iter)
    }
}

impl Spanned for LeadingTrivia {
    fn try_span(&self) -> Option<Span> {
        Span::join_all(self.0.iter().map(Spanned::try_span))
    }
}

impl Lex for LeadingTrivia {
    fn to_tokens(&self, output: &mut TokenStream) {
        for trivia in self.0.iter().cloned() {
            output.push_leading_trivia(trivia);
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Spanned)]
/// Comment blocks owned by one structural syntax item.
///
/// Outer comments are complete-line `--` blocks documenting the item on
/// their right. Inner comments occur within the item, including delimited
/// block comments and `--` comments following code on the same line.
pub struct CommentTrivia {
    /// The outer.
    outer: Vec<Trivia>,
    /// The inner.
    inner: Vec<Trivia>,
}

impl CommentTrivia {
    /// Borrows the complete-line comment blocks attached from above.
    pub fn outer(&self) -> &[Trivia] {
        &self.outer
    }

    /// Borrows comments contained by the owning syntax item.
    pub fn inner(&self) -> &[Trivia] {
        &self.inner
    }

    /// Reports whether the owner has no retained comments.
    pub fn is_empty(&self) -> bool {
        self.outer.is_empty() && self.inner.is_empty()
    }

    /// Collects the comments a token projection carries, without detaching
    /// them from the tokens that own them.
    ///
    /// Complete-line `--` blocks leading the first token are outer comments;
    /// every other comment, in source order, is inner.
    pub fn of(tokens: &TokenStream) -> Self {
        let mut comments = Self::default();
        let data = tokens.data();
        comments
            .inner
            .extend(data.prefix.as_slice().iter().cloned());
        for (index, token) in data.trees.iter().enumerate() {
            for trivia in token.leading_trivia().as_slice() {
                if index == 0 && trivia.kind() == TriviaKind::LineComment {
                    comments.outer.push(trivia.clone());
                } else {
                    comments.inner.push(trivia.clone());
                }
            }
            if let TokenTree::Group(group) = token {
                comments
                    .inner
                    .extend(Self::of(group.stream()).into_trivia());
            }
            comments
                .inner
                .extend(token.trailing_trivia().as_slice().iter().cloned());
        }
        comments
            .inner
            .extend(data.suffix.as_slice().iter().cloned());
        comments
    }

    /// Consumes the collection and returns its comments in source order.
    pub(crate) fn into_trivia(self) -> impl Iterator<Item = Trivia> {
        self.outer.into_iter().chain(self.inner)
    }
}

/// A typed terminal together with its adjacent comments.
pub trait Terminal: Spanned {
    /// Borrows the comments preceding this terminal.
    fn leading_trivia(&self) -> &LeadingTrivia;

    /// Mutably borrows the comments preceding this terminal.
    fn leading_trivia_mut(&mut self) -> &mut LeadingTrivia;

    /// Borrows the comments following this terminal.
    fn trailing_trivia(&self) -> &TrailingTrivia;

    /// Mutably borrows the comments following this terminal.
    fn trailing_trivia_mut(&mut self) -> &mut TrailingTrivia;

    /// Replaces the comments preceding this terminal.
    fn set_leading_trivia(&mut self, trivia: impl IntoIterator<Item = Trivia>)
    where
        Self: Sized,
    {
        *self.leading_trivia_mut() = LeadingTrivia::new(trivia);
    }

    /// Replaces the comments and layout characters following this terminal.
    fn set_trailing_trivia(&mut self, trivia: impl IntoIterator<Item = Trivia>)
    where
        Self: Sized,
    {
        *self.trailing_trivia_mut() = TrailingTrivia::new(trivia);
    }
}

/// Handles identifier qualifier spans.
fn identifier_qualifier_spans(spelling: &str, span: &Span) -> (Option<Span>, Option<Span>) {
    let separators = spelling
        .match_indices('$')
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let source = separators.first().and_then(|separator| {
        let name = &spelling[..*separator];
        (!name.is_empty() && name.bytes().all(is_identifier_byte))
            .then(|| span.subspan(0, separator + 1))
    });
    let index = separators.last().and_then(|separator| {
        let digits = &spelling[separator + 1..];
        (!digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| span.subspan(*separator, spelling.len()))
    });
    (source, index)
}

/// Returns whether the value is identifier byte.
fn is_identifier_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

/// Handles span for spelling.
fn span_for_spelling(text: Arc<str>, span: Span) -> Span {
    if span.text() == Some(text.as_ref()) {
        span
    } else {
        let source = Source::with_file(span.file(), text);
        let len = source.text().len();
        Span::ranged(source, 0..len)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
/// One raw M2 token tree.
pub enum TokenTree {
    /// A delimited group.
    Group(TokenGroup),
    /// A literal.
    Literal(LiteralToken),
    /// Punctuation.
    Punct(Punct),
    /// An identifier or keyword spelling.
    Ident(IdentToken),
}

impl TokenTree {
    /// Returns the token's source spelling when it is a flat lexical atom.
    pub fn spelling(&self) -> Option<&str> {
        match self {
            Self::Literal(token) => Some(token.text()),
            Self::Punct(token) => Some(token.text()),
            Self::Ident(token) => Some(token.text()),
            Self::Group(_) => None,
        }
    }

    /// Handles description.
    pub(crate) fn description(&self) -> String {
        match self {
            Self::Group(group) => group.delim_kind().to_string(),
            token => token.spelling().unwrap_or("").to_owned(),
        }
    }

    /// Borrows the comments attached before this atom.
    pub fn leading_trivia(&self) -> &LeadingTrivia {
        match self {
            Self::Group(token) => token.leading_trivia(),
            Self::Literal(token) => token.leading_trivia(),
            Self::Punct(token) => token.leading_trivia(),
            Self::Ident(token) => token.leading_trivia(),
        }
    }

    /// Mutably borrows the comments attached before this atom.
    pub fn leading_trivia_mut(&mut self) -> &mut LeadingTrivia {
        match self {
            Self::Group(token) => token.leading_trivia_mut(),
            Self::Literal(token) => token.leading_trivia_mut(),
            Self::Punct(token) => token.leading_trivia_mut(),
            Self::Ident(token) => token.leading_trivia_mut(),
        }
    }

    /// Borrows the comments attached after this atom.
    pub fn trailing_trivia(&self) -> &TrailingTrivia {
        match self {
            Self::Group(token) => token.trailing_trivia(),
            Self::Literal(token) => token.trailing_trivia(),
            Self::Punct(token) => token.trailing_trivia(),
            Self::Ident(token) => token.trailing_trivia(),
        }
    }

    /// Mutably borrows the comments attached after this atom.
    pub fn trailing_trivia_mut(&mut self) -> &mut TrailingTrivia {
        match self {
            Self::Group(token) => token.trailing_trivia_mut(),
            Self::Literal(token) => token.trailing_trivia_mut(),
            Self::Punct(token) => token.trailing_trivia_mut(),
            Self::Ident(token) => token.trailing_trivia_mut(),
        }
    }
}

impl Lex for TokenTree {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push(self.clone());
    }
}

#[derive(Debug, Clone, Default)]
/// Internal state for TokenStreamData.
struct TokenStreamData {
    /// The trees.
    trees: Vec<TokenTree>,
    /// The prefix.
    prefix: TrailingTrivia,
    /// The suffix.
    suffix: LeadingTrivia,
    /// The source.
    source: Option<Arc<Source>>,
}

/// Internal state for TokenStreamParts.
pub(crate) struct TokenStreamParts {
    /// The trees.
    pub(crate) trees: Vec<TokenTree>,
    /// The prefix.
    pub(crate) prefix: TrailingTrivia,
    /// The suffix.
    pub(crate) suffix: LeadingTrivia,
    /// The source.
    pub(crate) source: Option<Arc<Source>>,
}

impl PartialEq for TokenStreamData {
    fn eq(&self, other: &Self) -> bool {
        self.trees == other.trees
            && self.prefix == other.prefix
            && self.suffix == other.suffix
            && match (&self.source, &other.source) {
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                (None, None) => true,
                _ => false,
            }
    }
}

impl Eq for TokenStreamData {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// A persistent sequence of raw M2 token trees.
///
/// Clones and [`Self::fork`] share immutable backing storage. The first
/// mutation of either value detaches that value through copy-on-write, so
/// speculative consumers can retain the original stream without cloning its
/// token trees eagerly.
pub struct TokenStream(Arc<TokenStreamData>);

impl Spanned for TokenStream {
    fn try_span(&self) -> Option<Span> {
        Span::join_all(
            self.data()
                .prefix
                .try_span()
                .into_iter()
                .chain(self.iter().filter_map(Spanned::try_span))
                .chain(self.data().suffix.try_span()),
        )
    }
}

impl Deref for TokenStream {
    type Target = [TokenTree];

    fn deref(&self) -> &Self::Target {
        &self.data().trees
    }
}

impl AsRef<[TokenTree]> for TokenStream {
    fn as_ref(&self) -> &[TokenTree] {
        self
    }
}

impl TokenStream {
    /// Creates an empty token stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an independently mutable view over the same token storage.
    ///
    /// This operation is constant time. Mutating either stream preserves the
    /// other stream and copies only the backing structures on the modified
    /// path.
    pub fn fork(&self) -> Self {
        self.clone()
    }

    /// Borrows the shared backing data.
    fn data(&self) -> &TokenStreamData {
        &self.0
    }

    /// Borrows the shared backing data mutably.
    fn data_mut(&mut self) -> &mut TokenStreamData {
        Arc::make_mut(&mut self.0)
    }

    /// Converts the value into data.
    fn into_data(self) -> TokenStreamData {
        Arc::try_unwrap(self.0).unwrap_or_else(|data| (*data).clone())
    }

    /// Converts the value into parts.
    pub(crate) fn into_parts(self) -> TokenStreamParts {
        let TokenStreamData {
            trees,
            prefix,
            suffix,
            source,
        } = self.into_data();
        TokenStreamParts {
            trees,
            prefix,
            suffix,
            source,
        }
    }

    /// Creates the value from parts.
    pub(crate) fn from_parts(parts: TokenStreamParts) -> Self {
        Self(Arc::new(TokenStreamData {
            trees: parts.trees,
            prefix: parts.prefix,
            suffix: parts.suffix,
            source: parts.source,
        }))
    }

    /// Handles shares storage.
    pub(crate) fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Returns whether the value has trivia.
    pub(crate) fn has_trivia(&self) -> bool {
        !self.data().prefix.is_empty() || !self.data().suffix.is_empty()
    }

    /// Handles prefix trivia.
    pub(crate) fn prefix_trivia(&self) -> &[Trivia] {
        self.data().prefix.as_slice()
    }

    /// Handles suffix trivia.
    pub(crate) fn suffix_trivia(&self) -> &[Trivia] {
        self.data().suffix.as_slice()
    }

    /// Returns the associated source snapshot.
    pub(crate) fn source(&self) -> Option<Arc<Source>> {
        let mut source = self.data().source.clone();
        coherent_stream_source(self, &mut source)
            .then_some(source)
            .flatten()
    }

    /// Handles set source.
    pub(crate) fn set_source(&mut self, source: Arc<Source>) {
        self.data_mut().source = Some(source);
    }

    /// Takes final punctuation.
    pub(crate) fn take_final_punctuation(
        &mut self,
        spelling: &str,
    ) -> Option<(Punct, TrailingTrivia)> {
        let position = self.data().trees.len().checked_sub(1)?;
        let TokenTree::Punct(punctuation) = self.data().trees.get(position)? else {
            return None;
        };
        if punctuation.text() != spelling {
            return None;
        }
        let TokenTree::Punct(mut punctuation) = self.data_mut().trees.remove(position) else {
            unreachable!("the final significant token was punctuation")
        };
        let trailing_trivia = take(punctuation.trailing_trivia_mut());
        Some((punctuation, trailing_trivia))
    }

    /// Takes leading trivia.
    pub(crate) fn take_leading_trivia(&mut self) -> TrailingTrivia {
        let data = self.data_mut();
        let mut trivia = take(&mut data.prefix).into_vec();
        trivia.extend(take(&mut data.suffix).into_vec());
        TrailingTrivia::new(trivia)
    }

    /// Takes prefix trivia.
    pub(crate) fn take_prefix_trivia(&mut self) -> TrailingTrivia {
        take(&mut self.data_mut().prefix)
    }

    /// Takes suffix trivia.
    pub(crate) fn take_suffix_trivia(&mut self) -> LeadingTrivia {
        take(&mut self.data_mut().suffix)
    }

    /// Moves every comment onto the trailing trivia of the token it follows.
    ///
    /// Afterwards no token has leading trivia and the suffix is empty: comments
    /// before the first token remain in the prefix, which a delimiter or cell
    /// owns, and the emitted order of tokens and comments is unchanged.
    pub(crate) fn settle_trivia(&mut self) {
        let data = self.data_mut();
        let mut previous: Option<&mut TokenTree> = None;
        for token in &mut data.trees {
            let leading = take(token.leading_trivia_mut()).into_vec();
            match previous {
                Some(previous) => previous.trailing_trivia_mut().as_mut_vec().extend(leading),
                None => data.prefix.as_mut_vec().extend(leading),
            }
            if let TokenTree::Group(group) = token {
                group.stream_mut().settle_trivia();
            }
            previous = Some(token);
        }
        let suffix = take(&mut data.suffix).into_vec();
        match previous {
            Some(last) => last.trailing_trivia_mut().as_mut_vec().extend(suffix),
            None => data.prefix.as_mut_vec().extend(suffix),
        }
    }

    /// Handles push.
    pub(crate) fn push(&mut self, mut token: TokenTree) {
        let data = self.data_mut();
        if let Some(TokenTree::Group(group)) = data.trees.last_mut()
            && !group.delimiter().is_closed()
        {
            group.stream_mut().push(token);
        } else {
            token
                .leading_trivia_mut()
                .as_mut_vec()
                .splice(..0, take(&mut data.suffix).into_vec());
            data.trees.push(token);
        }
    }

    /// Pushes line comment.
    pub(crate) fn push_line_comment(&mut self, comment: Trivia) {
        let data = self.data_mut();
        if let Some(TokenTree::Group(group)) = data.trees.last_mut()
            && !group.delimiter().is_closed()
        {
            group.stream_mut().push_line_comment(comment);
        } else if let Some(comment) = complete_line_comment(&comment) {
            push_line_comment_block(data.suffix.as_mut_vec(), comment);
        } else if let Some(token) = data.trees.last_mut() {
            token.trailing_trivia_mut().as_mut_vec().push(comment);
        } else {
            data.prefix.as_mut_vec().push(comment);
        }
    }

    #[doc(hidden)]
    pub fn push_leading_trivia(&mut self, trivia: Trivia) {
        let data = self.data_mut();
        if let Some(TokenTree::Group(group)) = data.trees.last_mut()
            && !group.delimiter().is_closed()
        {
            group.stream_mut().push_leading_trivia(trivia);
        } else {
            data.suffix.as_mut_vec().push(trivia);
        }
    }

    /// Pushes block comment.
    pub(crate) fn push_block_comment(&mut self, comment: Trivia) {
        let data = self.data_mut();
        if let Some(TokenTree::Group(group)) = data.trees.last_mut()
            && !group.delimiter().is_closed()
        {
            group.stream_mut().push_block_comment(comment);
        } else if let Some(token) = data.trees.last_mut() {
            token.trailing_trivia_mut().as_mut_vec().push(comment);
        } else if !data.suffix.is_empty() {
            data.suffix.as_mut_vec().push(comment);
        } else {
            data.prefix.as_mut_vec().push(comment);
        }
    }

    /// Pushes trailing trivia.
    pub(crate) fn push_trailing_trivia(&mut self, trivia: Trivia) {
        let data = self.data_mut();
        if let Some(token) = data.trees.last_mut() {
            token.trailing_trivia_mut().as_mut_vec().push(trivia);
        } else {
            data.prefix.as_mut_vec().push(trivia);
        }
    }

    /// Handles complete last group.
    pub(crate) fn complete_last_group(&mut self, kind: DelimiterKind, close: Span) -> bool {
        let Some(TokenTree::Group(group)) = self.data_mut().trees.last_mut() else {
            return false;
        };
        if group.stream.complete_last_group(kind, close.clone()) {
            return true;
        }
        if group.delim_kind() != kind || group.delimiter().is_closed() {
            return false;
        }
        group.delimiter.spans.complete(close);
        true
    }

    /// Handles last innermost.
    pub(crate) fn last_innermost(&self) -> Option<&TokenTree> {
        let last = self.last()?;
        match last {
            TokenTree::Group(group) if !group.delimiter().is_closed() => {
                group.stream().last_innermost().or(Some(last))
            }
            _ => Some(last),
        }
    }

    #[doc(hidden)]
    pub fn starts_with_whitespace(&self) -> bool {
        false
    }

    #[doc(hidden)]
    pub fn ends_with_whitespace(&self) -> bool {
        false
    }

    #[doc(hidden)]
    pub fn ends_with_line_break(&self) -> bool {
        self.data()
            .trees
            .last()
            .is_some_and(|token| gap_after_contains(token, '\n'))
    }

    #[doc(hidden)]
    pub fn ends_with_semicolon(&self) -> bool {
        matches!(self.data().trees.last(), Some(TokenTree::Punct(token)) if token.text() == ";")
    }

    /// Appends one emitted syntax fragment with any separator required to keep
    /// adjacent word-like fragments lexically distinct.
    ///
    /// This is the composition operation used by [`crate::quote_m2!`]. The
    /// fragment retains all of its internal token structure and comments.
    /// Rendering inserts a conventional space when independently sourced atoms
    /// would otherwise merge into one lexical token.
    #[doc(hidden)]
    pub fn append_fragment<T: Lex + ?Sized>(&mut self, fragment: &T) {
        let fragment = fragment.to_token_stream();
        self.extend([fragment]);
    }

    /// Removes and returns the final token tree, if any.
    pub fn pop(&mut self) -> Option<TokenTree> {
        self.data_mut().trees.pop()
    }

    /// Migrates source text from the shared sources retained by its spans.
    pub fn to_source(&self) -> String {
        self.to_string()
    }
}

/// Handles complete line comment.
fn complete_line_comment(comment: &Trivia) -> Option<Trivia> {
    let span = comment.span();
    let (Some(start), Some(end)) = (span.start(), span.end()) else {
        return None;
    };
    let source = span.source();
    let line_start = source.text()[..start]
        .rfind('\n')
        .map_or(0, |newline| newline + 1);
    source.text()[line_start..start]
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t' | b'\r'))
        .then(|| {
            Trivia::from_span(
                TriviaKind::LineComment,
                Span::ranged(Arc::clone(source), line_start..end),
            )
        })
}

/// Pushes line comment block.
fn push_line_comment_block(comments: &mut Vec<Trivia>, comment: Trivia) {
    let Some(previous) = comments.last_mut() else {
        comments.push(comment);
        return;
    };
    if previous.kind() != TriviaKind::LineComment || !previous.span().shares_source(&comment.span())
    {
        comments.push(comment);
        return;
    }
    let (Some(previous_start), Some(previous_end), Some(comment_start), Some(comment_end)) = (
        previous.span().start(),
        previous.span().end(),
        comment.span().start(),
        comment.span().end(),
    ) else {
        comments.push(comment);
        return;
    };
    let previous_span = previous.span();
    let gap = &previous_span.source().text()[previous_end..comment_start];
    let same_block = gap
        .strip_prefix("\r\n")
        .or_else(|| gap.strip_prefix('\n'))
        .is_some_and(|indent| indent.bytes().all(|byte| matches!(byte, b' ' | b'\t')));
    if same_block {
        *previous = Trivia::from_span(
            TriviaKind::LineComment,
            Span::ranged(
                Arc::clone(previous.span().source()),
                previous_start..comment_end,
            ),
        );
    } else {
        comments.push(comment);
    }
}

/// Handles coherent stream source.
fn coherent_stream_source(stream: &TokenStream, source: &mut Option<Arc<Source>>) -> bool {
    for trivia in stream.prefix_trivia().iter().chain(stream.suffix_trivia()) {
        if !record_source(trivia.span(), source) {
            return false;
        }
    }
    for token in stream.iter() {
        if !record_source(token.span(), source)
            || !token
                .leading_trivia()
                .as_slice()
                .iter()
                .chain(token.trailing_trivia().as_slice())
                .all(|trivia| record_source(trivia.span(), source))
        {
            return false;
        }
        if let TokenTree::Group(group) = token
            && !coherent_stream_source(group.stream(), source)
        {
            return false;
        }
    }
    true
}

/// Handles record source.
fn record_source(span: Span, source: &mut Option<Arc<Source>>) -> bool {
    match source {
        Some(expected) => Arc::ptr_eq(expected, span.source()),
        None => {
            *source = Some(Arc::clone(span.source()));
            true
        }
    }
}

impl Display for TokenStream {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        write_stream(formatter, self)
    }
}
/// Handles needs separator.
fn needs_separator(left: &TokenTree, right: &TokenTree) -> bool {
    let left_span = left.span();
    let right_span = right.span();
    if left_span.shares_source(&right_span) && left_span.end() == right_span.start() {
        return false;
    }
    let wordlike = |tree: &TokenTree| {
        matches!(
            tree,
            TokenTree::Ident(_) | TokenTree::Literal(_) | TokenTree::Group(_)
        )
    };
    (wordlike(left) && wordlike(right))
        || matches!((left, right), (TokenTree::Punct(left), TokenTree::Punct(right))
            if punctuation_would_merge(left.text(), right.text()))
}

/// Writes stream.
fn write_stream(formatter: &mut Formatter<'_>, stream: &TokenStream) -> Result {
    let mut previous_span = write_trivia(formatter, None, stream.data().prefix.as_slice())?;
    let mut previous_atom = None::<&TokenTree>;
    for tree in stream {
        if !tree.leading_trivia().is_empty() {
            previous_span =
                write_trivia(formatter, previous_span, tree.leading_trivia().as_slice())?;
            previous_atom = None;
        }
        if let Some(left) = previous_span {
            let recovered_space = write_source_spaces(formatter, left, tree.span())?;
            if !recovered_space && previous_atom.is_some_and(|left| needs_separator(left, tree)) {
                formatter.write_str(" ")?;
            }
        }
        write_tree(formatter, tree)?;
        previous_span = Some(tree.span());
        previous_atom = Some(tree);
        if !tree.trailing_trivia().is_empty() {
            previous_span =
                write_trivia(formatter, previous_span, tree.trailing_trivia().as_slice())?;
            previous_atom = None;
        }
    }
    write_trivia(formatter, previous_span, stream.data().suffix.as_slice())?;
    Ok(())
}

/// Writes trivia.
fn write_trivia(
    formatter: &mut Formatter<'_>,
    mut previous: Option<Span>,
    trivia: &[Trivia],
) -> std::result::Result<Option<Span>, std::fmt::Error> {
    for item in trivia {
        if let Some(left) = previous {
            write_source_spaces(formatter, left, item.span())?;
        }
        formatter.write_str(item.text())?;
        previous = Some(item.span());
    }
    Ok(previous)
}

/// Writes source spaces.
fn write_source_spaces(
    formatter: &mut Formatter<'_>,
    left: Span,
    right: Span,
) -> std::result::Result<bool, std::fmt::Error> {
    if !left.shares_source(&right) {
        return Ok(false);
    }
    let (Some(end), Some(start)) = (left.end(), right.start()) else {
        return Ok(false);
    };
    if end > start {
        return Ok(false);
    }
    let gap = &left.source().text()[end..start];
    if gap.bytes().all(|byte| byte.is_ascii_whitespace()) {
        formatter.write_str(gap)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

/// Writes tree.
fn write_tree(formatter: &mut Formatter<'_>, tree: &TokenTree) -> Result {
    match tree {
        TokenTree::Group(group) => {
            formatter.write_str(group.delim_kind().opening())?;
            let opening = group.delimiter().opening_span();
            if let Some(first) = group.stream.iter().next() {
                write_source_spaces(formatter, opening.clone(), first.span())?;
            }
            write_stream(formatter, &group.stream)?;
            if let Some(closing) = group.delimiter().closing_span() {
                let left = group
                    .stream
                    .iter()
                    .next_back()
                    .map_or(opening, Spanned::span);
                write_source_spaces(formatter, left, closing)?;
                formatter.write_str(group.delim_kind().closing())?;
            }
            Ok(())
        }
        TokenTree::Literal(literal) => formatter.write_str(literal.text()),
        TokenTree::Punct(punct) => formatter.write_str(punct.text()),
        TokenTree::Ident(ident) => formatter.write_str(ident.text()),
    }
}

/// Handles gap after contains.
fn gap_after_contains(token: &TokenTree, character: char) -> bool {
    let span = token
        .trailing_trivia()
        .as_slice()
        .last()
        .map_or_else(|| token.span(), Spanned::span);
    let Some(end) = span.end() else {
        return false;
    };
    span.source().text()[end..].contains(character)
}

/// Appends anything that can emit tokens, one item at a time.
///
/// `Lex` already states which types produce tokens; this states how a
/// `TokenStream` accepts them, by forwarding to `Lex::to_tokens`. It
/// subsumes the two concrete impls this replaced (`TokenTree` and
/// `TokenStream` itself both implement `Lex`), and works for any other `Lex`
/// type without a bespoke impl.
impl<T: Lex> Extend<T> for TokenStream {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        for item in iter {
            item.to_tokens(self);
        }
    }
}

impl FromIterator<TokenTree> for TokenStream {
    fn from_iter<T: IntoIterator<Item = TokenTree>>(iter: T) -> Self {
        Self(Arc::new(TokenStreamData {
            trees: iter.into_iter().collect(),
            prefix: TrailingTrivia::default(),
            suffix: LeadingTrivia::default(),
            source: None,
        }))
    }
}

impl IntoIterator for TokenStream {
    type Item = TokenTree;
    type IntoIter = IntoIter<TokenTree>;
    fn into_iter(self) -> Self::IntoIter {
        self.into_data().trees.into_iter()
    }
}

impl<'syntax> IntoIterator for &'syntax TokenStream {
    type Item = &'syntax TokenTree;
    type IntoIter = std::slice::Iter<'syntax, TokenTree>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl Lex for TokenStream {
    fn to_tokens(&self, output: &mut TokenStream) {
        let preserve_source =
            output.is_empty() && output.data().prefix.is_empty() && output.data().suffix.is_empty();
        for trivia in self.data().prefix.as_slice().iter().cloned() {
            output.push_trailing_trivia(trivia);
        }
        for token in self.iter().cloned() {
            output.push(token);
        }
        output
            .data_mut()
            .suffix
            .as_mut_vec()
            .extend(self.data().suffix.as_slice().iter().cloned());
        output.data_mut().source = preserve_source.then(|| self.source()).flatten();
    }
}

/// Emits a typed or raw syntax value into the shared token representation.
pub trait Lex {
    /// Appends this value's token trees to `output`.
    fn to_tokens(&self, output: &mut TokenStream);

    /// Emits this value into a fresh token stream.
    fn to_token_stream(&self) -> TokenStream {
        let mut output = TokenStream::new();
        self.to_tokens(&mut output);
        output
    }

    /// Renders this value as normalized Macaulay2 source.
    fn to_m2(&self) -> String {
        self.to_token_stream().to_source()
    }
}

impl<T: Lex + ?Sized> Lex for &T {
    fn to_tokens(&self, output: &mut TokenStream) {
        (*self).to_tokens(output);
    }
}

impl<T: Lex + ?Sized> Lex for Box<T> {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.as_ref().to_tokens(output);
    }
}

impl<T: Lex> Lex for Option<T> {
    fn to_tokens(&self, output: &mut TokenStream) {
        if let Some(value) = self {
            value.to_tokens(output);
        }
    }
}

impl<T: Lex> Lex for [T] {
    fn to_tokens(&self, output: &mut TokenStream) {
        for token in self {
            token.to_tokens(output);
        }
    }
}

impl<T: Lex> Lex for Vec<T> {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.as_slice().to_tokens(output);
    }
}

#[cfg(test)]
mod tests {
    use crate::lex_str;

    #[test]
    fn fork_shares_tokens_until_the_first_mutation() {
        let original = lex_str("left + right").unwrap();
        let mut fork = original.fork();

        assert!(original.shares_storage(&fork));
        assert_eq!((&original).into_iter().count(), original.len());

        fork.pop();

        assert!(!original.shares_storage(&fork));
        assert_eq!(original.to_string(), "left + right");
        assert_eq!(fork.to_string(), "left +");
    }
}
