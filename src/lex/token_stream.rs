use std::fmt::{Display, Formatter, Result};
use std::ops::Deref;
use std::sync::Arc;
use std::vec::IntoIter;

use crate::lexer::{punctuation_would_merge, qualifier_spans};
use crate::{CommentKind, Comments, Source, Span, Spanned};
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
#[strutuct(pub, allow_missing_docs)]
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
            span: span.with_text(text),
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

#[derive(Debug, Clone, PartialEq, Eq)]
/// An identifier spelling and its source span.
pub struct IdentToken {
    /// The span.
    span: Span,
    package_qualifier: Option<Span>,
    index_qualifier: Option<Span>,
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
        let (package_qualifier, index_qualifier) = qualifier_spans(text, &span);
        Self {
            span,
            package_qualifier,
            index_qualifier,
        }
    }

    /// Creates an identifier token.
    pub fn new(text: impl Into<Arc<str>>, span: Span) -> Self {
        Self::from_span(span.with_text(text))
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

    /// The package name before the `$`: `Core` in `Core$x` and `Core$x$1`.
    ///
    /// Only name resolution confirms it: `Core$sin` is `sin`, but `a$b` is a
    /// symbol of its own when no package `a` exists.
    pub fn package_qualifier(&self) -> Option<&Span> {
        self.package_qualifier.as_ref()
    }

    /// The decimal index after the last `$`: `1` in `x$1` and `Core$x$1`.
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

#[derive(Debug, Clone, PartialEq, Eq)]
/// A delimited raw token stream.
pub struct TokenGroup {
    /// The delimiter.
    delimiter: Delimiter,
    /// The stream.
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

    /// Mutably borrows the tokens inside the group.
    pub fn stream_mut(&mut self) -> &mut TokenStream {
        &mut self.stream
    }

    /// Consumes the group and returns its contents.
    pub fn into_stream(self) -> TokenStream {
        self.stream
    }

    pub(crate) fn into_parts(self) -> (Delimiter, TokenStream) {
        (self.delimiter, self.stream)
    }
}

impl Lex for TokenGroup {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push(TokenTree::Group(self.clone()));
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
    /// The whole source this stream was lexed from, which its layout spans.
    source: Option<Arc<Source>>,
}

impl PartialEq for TokenStreamData {
    fn eq(&self, other: &Self) -> bool {
        self.trees == other.trees
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
        Span::join_all(self.iter().filter_map(Spanned::try_span))
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

    pub(crate) fn into_parts(self) -> (Vec<TokenTree>, Option<Arc<Source>>) {
        let TokenStreamData { trees, source } = self.into_data();
        (trees, source)
    }

    pub(crate) fn from_parts(trees: Vec<TokenTree>, source: Option<Arc<Source>>) -> Self {
        Self(Arc::new(TokenStreamData { trees, source }))
    }

    /// Handles shares storage.
    pub(crate) fn shares_storage(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
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
    pub(crate) fn take_final_punctuation(&mut self, spelling: &str) -> Option<Punct> {
        let position = self.data().trees.len().checked_sub(1)?;
        let TokenTree::Punct(punctuation) = self.data().trees.get(position)? else {
            return None;
        };
        if punctuation.text() != spelling {
            return None;
        }
        let TokenTree::Punct(punctuation) = self.data_mut().trees.remove(position) else {
            unreachable!("the final significant token was punctuation")
        };
        Some(punctuation)
    }

    /// Handles push.
    pub(crate) fn push(&mut self, token: TokenTree) {
        let data = self.data_mut();
        if let Some(TokenTree::Group(group)) = data.trees.last_mut()
            && !group.delimiter().is_closed()
        {
            group.stream_mut().push(token);
        } else {
            data.trees.push(token);
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

/// Handles coherent stream source.
fn coherent_stream_source(stream: &TokenStream, source: &mut Option<Arc<Source>>) -> bool {
    for token in stream.iter() {
        if !record_source(token.span(), source) {
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
        write_standalone(formatter, self)
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
        || matches!((left, right), (TokenTree::Punct(_) | TokenTree::Ident(_), TokenTree::Punct(_))
            if left.spelling().zip(right.spelling())
                .is_some_and(|(left, right)| punctuation_would_merge(left, right)))
}

/// Writes a stream's atoms and the layout between them, starting after
/// `previous`, the span written just before it, such as an opening delimiter.
/// Returns the span written last.
pub(crate) fn write_stream(
    formatter: &mut Formatter<'_>,
    stream: &TokenStream,
    mut previous: Option<Span>,
) -> std::result::Result<Option<Span>, std::fmt::Error> {
    let mut previous_atom = None::<&TokenTree>;
    for tree in stream {
        let span = tree.span();
        let wrote_layout = match &previous {
            Some(left) => write_gap(formatter, Some(left), Some(&span))?,
            None => false,
        };
        if !wrote_layout && previous_atom.is_some_and(|left| needs_separator(left, tree)) {
            formatter.write_str(" ")?;
        }
        write_tree(formatter, tree)?;
        previous = Some(span);
        previous_atom = Some(tree);
    }
    Ok(previous)
}

/// Writes a stream printed on its own, with the comments at its edges. A
/// stream lexed from a whole source keeps that source's comments around its
/// tokens too.
fn write_standalone(formatter: &mut Formatter<'_>, stream: &TokenStream) -> Result {
    let first = stream.first().map(TokenTree::span);
    let whole = stream.data().source.as_ref();
    let text = whole.map(|source| source.text());
    match (whole, &first) {
        (Some(source), first) => {
            let text = text.unwrap();
            let start = first.as_ref().and_then(Span::start).unwrap_or(text.len());
            if source.is_layout(0..start) {
                // From the line of the first comment, keeping its indentation.
                let layout = &text[..start];
                let written = layout.len() - layout.trim_start().len();
                let line = layout[..written]
                    .rfind('\n')
                    .map_or(0, |newline| newline + 1);
                if written < start {
                    formatter.write_str(&layout[line..])?;
                }
            } else {
                write_gap(formatter, None, first.as_ref())?;
            }
        }
        (None, first) => {
            write_gap(formatter, None, first.as_ref())?;
        }
    }
    let Some(last) = write_stream(formatter, stream, None)? else {
        return Ok(());
    };
    match whole {
        Some(source) if source.is_layout(last.end().unwrap()..source.text().len()) => {
            formatter.write_str(source.text()[last.end().unwrap()..].trim_end())
        }
        _ => write_gap(formatter, Some(&last), None).map(drop),
    }
}

/// The source between two atoms, when it holds only whitespace and comments.
pub(crate) fn layout_between<'a>(left: &'a Span, right: &Span) -> Option<&'a str> {
    let (end, start) = (left.end()?, right.start()?);
    (left.shares_source(right) && end <= start && left.source().is_layout(end..start))
        .then(|| &left.source().text()[end..start])
}

/// Writes the layout between two atoms, either of which may be absent at an
/// edge: the source between them when it is only layout, otherwise the
/// comments after `left` on its line and those on the lines before `right`.
/// Returns whether anything was written.
pub(crate) fn write_gap(
    formatter: &mut Formatter<'_>,
    left: Option<&Span>,
    right: Option<&Span>,
) -> std::result::Result<bool, std::fmt::Error> {
    if let (Some(left), Some(right)) = (left, right)
        && let Some(layout) = layout_between(left, right)
    {
        formatter.write_str(layout)?;
        return Ok(true);
    }
    let mut wrote = false;
    let mut ends_line = left.is_none();
    if let Some(left) = left
        && let Some(last) = left.comments_after().last()
    {
        let text = left.source().text();
        formatter.write_str(&text[left.end().unwrap()..last.span().end().unwrap()])?;
        ends_line = last.kind() == CommentKind::Line;
        if right.is_some() {
            formatter.write_str(if ends_line { "\n" } else { " " })?;
        }
        wrote = true;
    }
    if let Some(right) = right
        && let Some(first) = right.comments_before().first()
    {
        if !ends_line {
            formatter.write_str("\n")?;
        }
        let text = right.source().text();
        formatter.write_str(&text[first.span().start().unwrap()..right.start().unwrap()])?;
        wrote = true;
    }
    Ok(wrote)
}

/// Writes tree.
fn write_tree(formatter: &mut Formatter<'_>, tree: &TokenTree) -> Result {
    match tree {
        TokenTree::Group(group) => {
            formatter.write_str(group.delim_kind().opening())?;
            let opening = group.delimiter().opening_span();
            let last = write_stream(formatter, &group.stream, Some(opening.clone()))?;
            if let Some(closing) = group.delimiter().closing_span() {
                write_gap(formatter, Some(&last.unwrap_or(opening)), Some(&closing))?;
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
    let span = token.span();
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
        let preserve_source = output.is_empty();
        for token in self.iter().cloned() {
            output.push(token);
        }
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

    /// The comments documenting and within this value, found in its source.
    fn comments(&self) -> Comments {
        Comments::of(&self.to_token_stream())
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
