use std::fmt::{Display, Formatter, Result};
use std::sync::Arc;
use std::vec::IntoIter;

use crate::{Source, Span, Spanned, lexer::punctuation_would_merge};

#[path = "token_stream/delim.rs"]
pub mod delim;
use delim::{Delimiter, DelimiterKind};

#[path = "token_stream/punct.rs"]
mod punct;
pub use punct::Punct;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The lexical category of a literal token.
pub enum LiteralKind {
    /// A double-quoted string.
    String,
    /// A slash-delimited raw string.
    RawString,
    /// An integer number.
    Integer,
    /// A floating-point number.
    Float,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A literal spelling together with its lexical category and source span.
pub struct LiteralToken {
    /// The literal's lexical category.
    pub kind: LiteralKind,
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
    ///
    /// # Panics
    ///
    /// Panics when `sources` cannot resolve this token's span.
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
    span: Span,
    source_qualifier: Option<Span>,
    index_qualifier: Option<Span>,
    leading_trivia: LeadingTrivia,
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
    delimiter: Delimiter,
    stream: TokenStream,
    leading_trivia: LeadingTrivia,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The lexical category of source trivia.
pub enum TriviaKind {
    /// A comment extending to the end of a line.
    LineComment,
    /// A delimited block comment.
    BlockComment,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Spanned)]
/// A comment retained alongside a lexical atom.
pub struct Trivia {
    kind: TriviaKind,
    span: Span,
}

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
    outer: Vec<Trivia>,
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

    pub(crate) fn take_from_cell(tokens: &mut TokenStream) -> Self {
        let mut outer = Vec::new();
        if let Some(first) = tokens.0.first_mut() {
            let leading = first.leading_trivia_mut().as_mut_vec();
            let mut retained = Vec::with_capacity(leading.len());
            for trivia in std::mem::take(leading) {
                if trivia.kind() == TriviaKind::LineComment {
                    outer.push(trivia);
                } else {
                    retained.push(trivia);
                }
            }
            *leading = retained;
        }
        let mut inner = take_stream_comments(tokens);
        sort_comments(&mut outer);
        sort_comments(&mut inner);
        Self { outer, inner }
    }

    pub(crate) fn take_inner(tokens: &mut TokenStream) -> Self {
        let mut inner = take_stream_comments(tokens);
        sort_comments(&mut inner);
        Self {
            outer: Vec::new(),
            inner,
        }
    }

    pub(crate) fn append(&mut self, mut comments: Self) {
        self.outer.append(&mut comments.outer);
        self.inner.append(&mut comments.inner);
        sort_comments(&mut self.outer);
        sort_comments(&mut self.inner);
    }

    /// Reattaches owned comments to a raw projection at their source-relative
    /// positions.
    #[doc(hidden)]
    pub fn __apply_to(&self, output: &mut TokenStream) {
        for comment in self.outer.iter().rev().cloned() {
            output.prepend_comment(comment);
        }
        for comment in self.inner.iter().cloned() {
            output.insert_inner_comment(comment);
        }
    }
}

fn sort_comments(comments: &mut [Trivia]) {
    comments.sort_by_key(|comment| comment.span().start().unwrap_or(usize::MAX));
}

fn take_stream_comments(stream: &mut TokenStream) -> Vec<Trivia> {
    let mut comments = std::mem::take(&mut stream.1).into_vec();
    for token in &mut stream.0 {
        comments.extend(std::mem::take(token.leading_trivia_mut()).into_vec());
        if let TokenTree::Group(group) = token {
            comments.extend(take_stream_comments(group.stream_mut()));
        }
        comments.extend(std::mem::take(token.trailing_trivia_mut()).into_vec());
    }
    comments.extend(std::mem::take(&mut stream.2).into_vec());
    comments
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

fn is_identifier_byte(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphanumeric()
}

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

#[derive(Debug, Clone, Default, PartialEq, Eq, Spanned)]
/// An owned sequence of raw M2 token trees.
pub struct TokenStream(Vec<TokenTree>, TrailingTrivia, LeadingTrivia, Option<Span>);

impl TokenStream {
    /// Creates an empty token stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Iterates over the stream's top-level token trees.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &TokenTree> + ExactSizeIterator {
        self.0.iter()
    }

    /// Reports whether the stream contains no token trees.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn get(&self, position: usize) -> Option<&TokenTree> {
        self.0.get(position)
    }

    pub(crate) fn last(&self) -> Option<&TokenTree> {
        self.0.last()
    }

    pub(crate) fn has_significant_tokens(&self) -> bool {
        !self.0.is_empty()
    }

    pub(crate) fn has_trivia(&self) -> bool {
        !self.1.is_empty() || !self.2.is_empty()
    }

    pub(crate) fn prefix_trivia(&self) -> &[Trivia] {
        self.1.as_slice()
    }

    pub(crate) fn suffix_trivia(&self) -> &[Trivia] {
        self.2.as_slice()
    }

    pub(crate) fn source(&self) -> Option<Arc<Source>> {
        let mut source = self.3.as_ref().map(|span| Arc::clone(span.source()));
        coherent_stream_source(self, &mut source)
            .then_some(source)
            .flatten()
    }

    pub(crate) fn set_physical_eof(&mut self, eof: Span) {
        self.3 = Some(eof);
    }

    pub(crate) fn take_physical_eof(&mut self) -> Option<Span> {
        self.3.take()
    }

    pub(crate) fn take_final_punctuation(
        &mut self,
        spelling: &str,
    ) -> Option<(Punct, TrailingTrivia)> {
        let position = self.0.len().checked_sub(1)?;
        let TokenTree::Punct(punctuation) = self.0.get(position)? else {
            return None;
        };
        if punctuation.text() != spelling {
            return None;
        }
        let TokenTree::Punct(mut punctuation) = self.0.remove(position) else {
            unreachable!("the final significant token was punctuation")
        };
        let trailing_trivia = std::mem::take(punctuation.trailing_trivia_mut());
        Some((punctuation, trailing_trivia))
    }

    pub(crate) fn take_leading_trivia(&mut self) -> TrailingTrivia {
        let mut trivia = std::mem::take(&mut self.1).into_vec();
        trivia.extend(std::mem::take(&mut self.2).into_vec());
        TrailingTrivia::new(trivia)
    }

    pub(crate) fn take_prefix_trivia(&mut self) -> TrailingTrivia {
        std::mem::take(&mut self.1)
    }

    pub(crate) fn take_suffix_trivia(&mut self) -> LeadingTrivia {
        std::mem::take(&mut self.2)
    }

    pub(crate) fn push(&mut self, mut token: TokenTree) {
        if let Some(TokenTree::Group(group)) = self.0.last_mut()
            && !group.delimiter().is_closed()
        {
            group.stream_mut().push(token);
        } else {
            token
                .leading_trivia_mut()
                .as_mut_vec()
                .splice(..0, std::mem::take(&mut self.2).into_vec());
            self.0.push(token);
        }
    }

    pub(crate) fn push_line_comment(&mut self, comment: Trivia) {
        if let Some(TokenTree::Group(group)) = self.0.last_mut()
            && !group.delimiter().is_closed()
        {
            group.stream_mut().push_line_comment(comment);
        } else if let Some(comment) = complete_line_comment(&comment) {
            push_line_comment_block(self.2.as_mut_vec(), comment);
        } else if let Some(token) = self.0.last_mut() {
            token.trailing_trivia_mut().as_mut_vec().push(comment);
        } else {
            self.1.as_mut_vec().push(comment);
        }
    }

    #[doc(hidden)]
    pub fn push_leading_trivia(&mut self, trivia: Trivia) {
        if let Some(TokenTree::Group(group)) = self.0.last_mut()
            && !group.delimiter().is_closed()
        {
            group.stream_mut().push_leading_trivia(trivia);
        } else {
            self.2.as_mut_vec().push(trivia);
        }
    }

    pub(crate) fn push_block_comment(&mut self, comment: Trivia) {
        if let Some(TokenTree::Group(group)) = self.0.last_mut()
            && !group.delimiter().is_closed()
        {
            group.stream_mut().push_block_comment(comment);
        } else if let Some(token) = self.0.last_mut() {
            token.trailing_trivia_mut().as_mut_vec().push(comment);
        } else if !self.2.is_empty() {
            self.2.as_mut_vec().push(comment);
        } else {
            self.1.as_mut_vec().push(comment);
        }
    }

    pub(crate) fn push_trailing_trivia(&mut self, trivia: Trivia) {
        if let Some(token) = self.0.last_mut() {
            token.trailing_trivia_mut().as_mut_vec().push(trivia);
        } else {
            self.1.as_mut_vec().push(trivia);
        }
    }

    pub(crate) fn complete_last_group(&mut self, kind: DelimiterKind, close: Span) -> bool {
        let Some(TokenTree::Group(group)) = self.0.last_mut() else {
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

    pub(crate) fn last_innermost(&self) -> Option<&TokenTree> {
        let last = self.last()?;
        match last {
            TokenTree::Group(group) if !group.delimiter().is_closed() => {
                group.stream().last_innermost().or(Some(last))
            }
            _ => Some(last),
        }
    }

    fn prepend_comment(&mut self, comment: Trivia) {
        if let Some(first) = self.0.first_mut() {
            first.leading_trivia_mut().as_mut_vec().insert(0, comment);
        } else {
            self.1.as_mut_vec().insert(0, comment);
        }
    }

    fn insert_inner_comment(&mut self, comment: Trivia) {
        let start = comment.span().start();
        if let Some(start) = start {
            for token in &mut self.0 {
                if let TokenTree::Group(group) = token {
                    let span = group.span();
                    if span.shares_source(&comment.span())
                        && span.start().is_some_and(|open| open <= start)
                        && span.end().is_some_and(|close| start <= close)
                    {
                        group.stream_mut().insert_inner_comment(comment);
                        return;
                    }
                }
            }

            if let Some(token) = self.0.iter_mut().rev().find(|token| {
                let span = token.span();
                span.shares_source(&comment.span())
                    && span.start().is_some_and(|token_start| token_start <= start)
            }) {
                token.trailing_trivia_mut().as_mut_vec().push(comment);
                return;
            }

            if self
                .0
                .iter()
                .any(|token| token.span().shares_source(&comment.span()))
            {
                self.1.as_mut_vec().push(comment);
                return;
            }
        }

        if let Some(last) = self.0.last_mut() {
            last.trailing_trivia_mut().as_mut_vec().push(comment);
        } else {
            self.1.as_mut_vec().push(comment);
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
        self.0
            .last()
            .is_some_and(|token| gap_after_contains(token, '\n'))
    }

    #[doc(hidden)]
    pub fn ends_with_semicolon(&self) -> bool {
        matches!(self.0.last(), Some(TokenTree::Punct(token)) if token.text() == ";")
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
        self.0.pop()
    }

    /// Migrates source text from the shared sources retained by its spans.
    pub fn to_source(&self) -> String {
        self.to_string()
    }
}

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

fn write_stream(formatter: &mut Formatter<'_>, stream: &TokenStream) -> Result {
    let mut previous_span = write_trivia(formatter, None, stream.1.as_slice())?;
    let mut previous_atom = None::<&TokenTree>;
    for tree in &stream.0 {
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
    write_trivia(formatter, previous_span, stream.2.as_slice())?;
    Ok(())
}

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
        Self(
            iter.into_iter().collect(),
            TrailingTrivia::default(),
            LeadingTrivia::default(),
            None,
        )
    }
}

impl IntoIterator for TokenStream {
    type Item = TokenTree;
    type IntoIter = IntoIter<TokenTree>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl Lex for TokenStream {
    fn to_tokens(&self, output: &mut TokenStream) {
        let preserve_eof = output.0.is_empty() && output.1.is_empty() && output.2.is_empty();
        for trivia in self.1.as_slice().iter().cloned() {
            output.push_trailing_trivia(trivia);
        }
        for token in self.0.iter().cloned() {
            output.push(token);
        }
        output
            .2
            .as_mut_vec()
            .extend(self.2.as_slice().iter().cloned());
        output.3 = preserve_eof.then(|| self.3.clone()).flatten();
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
