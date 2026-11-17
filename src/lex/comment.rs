//! Comments, found through the spans they surround rather than stored on
//! syntax.
//!
//! The lexer records where every comment of a [`Source`] lies, and the source
//! keeps that index next to its text. A span then answers which comments are
//! next to it. A comment that starts on the line where the preceding atom
//! ends follows that atom; every other comment precedes the next atom:
//!
//! ```text
//! x = 1 -- after `1`
//! -- before `y`
//! y = 2
//! ```

use std::iter::Chain;
use std::ops::Range;
use std::sync::Arc;
use std::vec::IntoIter;

use crate::{Offset, Source, Span, Spanned, TokenStream, TokenTree};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The two comment forms: `-- to the end of the line` and `-* delimited *-`.
pub enum CommentKind {
    /// `-- note`, running to the end of its line.
    Line,
    /// `-* note *-`, possibly spanning lines.
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
/// One comment of a source, as written: `-- note`, `-* note *-`.
pub struct Comment {
    kind: CommentKind,
    span: Span,
}

impl Comment {
    /// Whether it is a line or a block comment.
    pub fn kind(&self) -> CommentKind {
        self.kind
    }

    /// The comment as written, with its `--` or `-* *-` markers.
    pub fn text(&self) -> &str {
        self.span
            .text()
            .expect("comments are located in their source")
    }
}

impl Spanned for Comment {
    fn try_span(&self) -> Option<Span> {
        Some(self.span.clone())
    }
}

/// Where one comment lies in its source.
pub(crate) type CommentRange = (CommentKind, Range<Offset>);

impl Source {
    /// The comments of this source in order, lexing it the first time.
    pub(crate) fn comment_ranges(&self) -> &[CommentRange] {
        self.comments
            .get_or_init(|| crate::lexer::comment_ranges(self.text()))
    }

    /// Records the comments a lexer found while lexing this whole source.
    pub(crate) fn install_comments(&self, comments: Vec<CommentRange>) {
        let _ = self.comments.set(comments.into_boxed_slice());
    }

    /// Whether `range` holds only whitespace and comments.
    pub(crate) fn is_layout(&self, range: Range<Offset>) -> bool {
        let mut cursor = range.start;
        let comments = self.comment_ranges();
        let first = comments.partition_point(|(_, comment)| comment.end <= range.start);
        for (_, comment) in &comments[first..] {
            if comment.start >= range.end {
                break;
            }
            if !self.is_blank(cursor..comment.start.max(cursor)) {
                return false;
            }
            cursor = comment.end;
        }
        cursor >= range.end || self.is_blank(cursor..range.end)
    }

    fn is_blank(&self, range: Range<Offset>) -> bool {
        self.text()[range]
            .bytes()
            .all(|byte| byte.is_ascii_whitespace())
    }

    fn comment(self: &Arc<Self>, (kind, range): &CommentRange) -> Comment {
        Comment {
            kind: *kind,
            span: Span::ranged(Arc::clone(self), range.clone()),
        }
    }

    /// The comments after `end` up to the first line break or atom.
    fn comments_from(self: &Arc<Self>, end: Offset) -> Vec<Comment> {
        let comments = self.comment_ranges();
        let first = comments.partition_point(|(_, comment)| comment.start < end);
        let mut cursor = end;
        comments[first..]
            .iter()
            .take_while(|(_, comment)| {
                let gap = &self.text()[cursor..comment.start];
                let same_line = gap.bytes().all(|byte| matches!(byte, b' ' | b'\t'));
                cursor = comment.end;
                same_line
            })
            .map(|comment| self.comment(comment))
            .collect()
    }
}

impl Span {
    /// The comments between this span and the line break that ends it: the
    /// `-- note` in `x -- note`.
    pub fn comments_after(&self) -> Vec<Comment> {
        if let Some(respelled) = &self.source().respells {
            return respelled.comments_after();
        }
        self.end()
            .map_or_else(Vec::new, |end| self.source().comments_from(end))
    }

    /// The comments between this span and the preceding atom, except those on
    /// that atom's own line: `-- doc` in `x = 1\n-- doc\ny`, for the span of `y`.
    pub fn comments_before(&self) -> Vec<Comment> {
        if let Some(respelled) = &self.source().respells {
            return respelled.comments_before();
        }
        let Some(start) = self.start() else {
            return Vec::new();
        };
        let source = self.source();
        let comments = source.comment_ranges();
        let mut first = comments.partition_point(|(_, comment)| comment.end <= start);
        let mut cursor = start;
        while first > 0 && source.is_blank(comments[first - 1].1.end..cursor) {
            first -= 1;
            cursor = comments[first].1.start;
        }
        let preceding = source.text()[..cursor].trim_end_matches(|c: char| c.is_ascii_whitespace());
        let owned_by_preceding = if preceding.is_empty() {
            0
        } else {
            source.comments_from(preceding.len()).len()
        };
        comments[first..]
            .iter()
            .take_while(|(_, comment)| comment.end <= start)
            .skip(owned_by_preceding)
            .map(|comment| source.comment(comment))
            .collect()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Spanned)]
/// The comments of one syntax item, split by what they document.
///
/// Outer comments are the complete-line `--` blocks right above the item,
/// documenting it; consecutive lines form one block. Inner comments are the
/// other comments above it, those within it, and those after it on its last
/// line:
///
/// ```text
/// -- outer: documents `f`
/// f = x -> x -* inner *- -- inner
/// ```
pub struct Comments {
    outer: Vec<Comment>,
    inner: Vec<Comment>,
}

impl Comments {
    /// The comment blocks above the item, one per run of consecutive lines.
    pub fn outer(&self) -> &[Comment] {
        &self.outer
    }

    /// The comments within the item, and after it on its last line.
    pub fn inner(&self) -> &[Comment] {
        &self.inner
    }

    /// Whether the item has no comments at all.
    pub fn is_empty(&self) -> bool {
        self.outer.is_empty() && self.inner.is_empty()
    }

    /// Every comment within `span`, as inner comments.
    pub fn within(span: &Span) -> Self {
        let (Some(start), Some(end)) = (span.start(), span.end()) else {
            return Self::default();
        };
        let source = span.source();
        let inner = source
            .comment_ranges()
            .iter()
            .filter(|(_, comment)| start <= comment.start && comment.end <= end)
            .map(|comment| source.comment(comment))
            .collect();
        Self {
            outer: Vec::new(),
            inner,
        }
    }

    /// The comments of the syntax a token stream spells.
    pub fn of(tokens: &TokenStream) -> Self {
        let mut atoms = tokens.iter().map(TokenTree::span);
        let (Some(first), last) = (atoms.next(), tokens.last().map(TokenTree::span)) else {
            return Self::default();
        };
        let last = last.expect("a stream with a first token has a last one");
        let (outer, mut inner): (Vec<_>, Vec<_>) = first
            .comments_before()
            .into_iter()
            .partition(|comment| comment.kind == CommentKind::Line && starts_its_line(comment));
        let outer = outer.into_iter().fold(Vec::new(), join_line_block);
        inner.extend(
            first
                .join(last.clone())
                .map_or_else(Vec::new, |span| Self::within(&span).inner),
        );
        inner.extend(last.comments_after());
        Self { outer, inner }
    }
}

impl IntoIterator for Comments {
    type Item = Comment;
    type IntoIter = Chain<IntoIter<Comment>, IntoIter<Comment>>;

    /// Every comment, in source order.
    fn into_iter(self) -> Self::IntoIter {
        self.outer.into_iter().chain(self.inner)
    }
}

fn starts_its_line(comment: &Comment) -> bool {
    let start = comment.span.start().expect("comments are located");
    let text = comment.span.source().text();
    text[..start]
        .rsplit('\n')
        .next()
        .is_some_and(|indent| indent.bytes().all(|byte| matches!(byte, b' ' | b'\t')))
}

/// Appends a complete-line comment, merging it into the previous block when
/// it is on the very next line.
fn join_line_block(mut blocks: Vec<Comment>, comment: Comment) -> Vec<Comment> {
    let next_line = blocks.last().is_some_and(|previous| {
        let gap = &comment.span.source().text()
            [previous.span.end().unwrap()..comment.span.start().unwrap()];
        gap.trim_start_matches([' ', '\t'])
            .strip_prefix("\r\n")
            .or_else(|| gap.trim_start_matches([' ', '\t']).strip_prefix('\n'))
            .is_some_and(|indent| indent.bytes().all(|byte| matches!(byte, b' ' | b'\t')))
    });
    match blocks.last_mut() {
        Some(previous) if next_line => {
            previous.span = previous
                .span
                .join(comment.span)
                .expect("blocks share a source");
        }
        _ => blocks.push(comment),
    }
    blocks
}
