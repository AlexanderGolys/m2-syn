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

#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
/// A literal spelling together with its lexical category and source span.
pub struct LiteralToken {
    /// The literal's lexical category.
    pub kind: LiteralKind,
    span: Span,
}

impl LiteralToken {
    /// Creates a literal token from its source span.
    pub fn from_span(kind: LiteralKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Creates a literal token.
    pub fn new(kind: LiteralKind, text: impl Into<Arc<str>>, span: Span) -> Self {
        Self {
            kind,
            span: span_for_spelling(text.into(), span),
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

impl Lex for LiteralToken {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.0.push(TokenTree::Literal(self.clone()));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
/// An identifier spelling and its source span.
pub struct IdentToken {
    span: Span,
}

impl IdentToken {
    /// Creates an identifier from its source span.
    pub fn from_span(span: Span) -> Self {
        Self { span }
    }

    /// Creates an identifier token.
    pub fn new(text: impl Into<Arc<str>>, span: Span) -> Self {
        Self {
            span: span_for_spelling(text.into(), span),
        }
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
}

impl Lex for IdentToken {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.0.push(TokenTree::Ident(self.clone()));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Spanned)]
/// A zero-width boundary terminating one top-level source cell.
pub struct EocToken {
    span: Span,
    after_semicolon: bool,
}

impl EocToken {
    /// Creates an end-of-cell marker at a source boundary.
    pub fn new(span: Span) -> Self {
        Self {
            span,
            after_semicolon: false,
        }
    }

    /// Creates a boundary produced by an unquoted top-level semicolon.
    #[doc(hidden)]
    pub fn after_semicolon(span: Span) -> Self {
        Self {
            span,
            after_semicolon: true,
        }
    }

    /// Reports whether this boundary was produced by a physical semicolon.
    pub fn follows_semicolon(&self) -> bool {
        self.after_semicolon
    }
}

impl Display for EocToken {
    fn fmt(&self, _formatter: &mut Formatter<'_>) -> Result {
        Ok(())
    }
}

impl Lex for EocToken {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.0.push(TokenTree::Eoc(self.clone()));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// A delimited raw token stream.
pub struct TokenGroup {
    delimiter: Delimiter,
    stream: TokenStream,
}

impl Spanned for TokenGroup {
    fn try_span(&self) -> Option<Span> {
        self.delimiter.try_span()
    }
}

impl TokenGroup {
    /// Creates a group from its delimiter and contents.
    pub fn new(delimiter: Delimiter, stream: TokenStream) -> Self {
        Self { delimiter, stream }
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

    /// Consumes the group and returns its contents.
    pub fn into_stream(self) -> TokenStream {
        self.stream
    }
}

impl Lex for TokenGroup {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.0.push(TokenTree::Group(self.clone()));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The lexical category of source trivia.
pub enum TriviaKind {
    /// Horizontal spacing.
    Whitespace,
    /// A standalone carriage return.
    CarriageReturn,
    /// A line feed.
    LineBreak,
    /// A comment extending to the end of a line.
    LineComment,
    /// A delimited block comment.
    BlockComment,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Spanned)]
/// Whitespace or a comment retained in the raw token stream.
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

    /// Reports whether this trivia contains a line feed.
    pub fn contains_line_break(&self) -> bool {
        self.text().contains('\n')
    }
}

impl Lex for Trivia {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.0.push(TokenTree::Trivia(self.clone()));
    }
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
    /// A zero-width top-level cell boundary inserted by the lexer.
    Eoc(EocToken),
    /// A delimited group.
    Group(TokenGroup),
    /// A literal.
    Literal(LiteralToken),
    /// Punctuation.
    Punct(Punct),
    /// An identifier or keyword spelling.
    Ident(IdentToken),
    /// Retained whitespace or a comment.
    Trivia(Trivia),
}

impl TokenTree {
    /// Returns the token's source spelling when it is a flat lexical atom.
    pub fn spelling(&self) -> Option<&str> {
        match self {
            Self::Eoc(_) => None,
            Self::Literal(token) => Some(token.text()),
            Self::Punct(token) => Some(token.text()),
            Self::Ident(token) => Some(token.text()),
            Self::Group(_) | Self::Trivia(_) => None,
        }
    }

    pub(crate) fn description(&self) -> String {
        match self {
            Self::Eoc(_) => "end of cell".into(),
            Self::Group(group) => group.delim_kind().to_string(),
            Self::Trivia(trivia) => match trivia.kind() {
                TriviaKind::Whitespace => "whitespace".into(),
                TriviaKind::CarriageReturn => "carriage return".into(),
                TriviaKind::LineBreak => "line break".into(),
                TriviaKind::LineComment => "line comment".into(),
                TriviaKind::BlockComment => "block comment".into(),
            },
            token => token.spelling().unwrap_or("").to_owned(),
        }
    }
}
impl Lex for TokenTree {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.0.push(self.clone());
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Spanned)]
/// An owned sequence of raw M2 token trees.
pub struct TokenStream(Vec<TokenTree>);

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

    #[doc(hidden)]
    pub fn starts_with_whitespace(&self) -> bool {
        matches!(self.0.first(), Some(TokenTree::Trivia(_)))
    }

    #[doc(hidden)]
    pub fn ends_with_whitespace(&self) -> bool {
        matches!(self.0.last(), Some(TokenTree::Trivia(_)))
    }

    #[doc(hidden)]
    pub fn ends_with_semicolon(&self) -> bool {
        matches!(self.0.last(), Some(TokenTree::Punct(token)) if token.text() == ";")
    }

    #[doc(hidden)]
    pub fn mark_final_eoc_after_semicolon(&mut self) {
        let Some(TokenTree::Eoc(marker)) = self.0.last_mut() else {
            return;
        };
        *marker = EocToken::after_semicolon(marker.expect_span());
    }

    /// Appends one emitted syntax fragment with any separator required to keep
    /// adjacent word-like fragments lexically distinct.
    ///
    /// This is the composition operation used by [`crate::quote_m2!`]. The
    /// fragment retains all of its internal token structure and trivia; a
    /// synthetic space is inserted only when the last existing token and first
    /// appended token would otherwise merge into one lexical token.
    #[doc(hidden)]
    pub fn append_fragment<T: Lex + ?Sized>(&mut self, fragment: &T) {
        let fragment = fragment.to_token_stream();
        if self
            .last()
            .zip(fragment.iter().next())
            .is_some_and(|(left, right)| needs_separator(left, right))
        {
            self.extend([Trivia::new(
                TriviaKind::Whitespace,
                " ",
                Span::for_text(" "),
            )]);
        }
        self.extend([fragment]);
    }

    /// Removes and returns the final token tree, if any.
    pub fn pop(&mut self) -> Option<TokenTree> {
        self.0.pop()
    }

    /// Reconstructs source text from the shared sources retained by its spans.
    pub fn to_source(&self) -> String {
        self.to_string()
    }
}

impl Display for TokenStream {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> Result {
        write_stream(formatter, self)
    }
}

fn write_stream(formatter: &mut Formatter<'_>, stream: &TokenStream) -> Result {
    let mut previous = None;
    for tree in &stream.0 {
        if let Some(left) = previous
            && needs_separator(left, tree)
        {
            formatter.write_str(" ")?;
        }
        write_tree(formatter, tree)?;
        previous = match tree {
            TokenTree::Trivia(_) | TokenTree::Eoc(_) => None,
            tree => Some(tree),
        };
    }
    Ok(())
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

fn write_tree(formatter: &mut Formatter<'_>, tree: &TokenTree) -> Result {
    match tree {
        TokenTree::Group(group) => {
            formatter.write_str(group.delim_kind().opening())?;
            write_stream(formatter, &group.stream)?;
            if group.delimiter.is_closed() {
                formatter.write_str(group.delim_kind().closing())?;
            }
            Ok(())
        }
        TokenTree::Eoc(marker) => marker.fmt(formatter),
        TokenTree::Literal(literal) => formatter.write_str(literal.text()),
        TokenTree::Punct(punct) => formatter.write_str(punct.text()),
        TokenTree::Ident(ident) => formatter.write_str(ident.text()),
        TokenTree::Trivia(trivia) => formatter.write_str(trivia.text()),
    }
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
        Self(iter.into_iter().collect())
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
        output.0.extend(self.0.iter().cloned());
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
