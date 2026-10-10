//! Places lexer trivia on a typed graph lowered from another parser's CST.
//!
//! A backend that lowers its own CST rebuilds every terminal and delimiter
//! from source spans, so it sees none of the comments the lexer attached to
//! raw tokens. Each raw atom and its typed counterpart cover the same bytes,
//! which makes the exact source range the key that carries trivia across.

use std::collections::HashMap;
use std::mem::take;
use std::sync::Arc;

use crate::cell_stream::{FramedCells, frame_recovering};
use crate::lexer::lex_source_recovering;
use crate::nodes::visit_mut::{self, VisitMut};
use crate::nodes::{Cell, SourceFile};
use crate::span::Offset;
use crate::{
    CellTokenStream, CommentTrivia, Delimiter, DelimiterValue, LeadingTrivia, LexError, Source,
    Spanned, Terminal, TokenStream, TokenTree, TrailingTrivia, Trivia,
};

/// The byte range of one raw atom.
type Bounds = (Offset, Offset);

/// The trivia surrounding the contents of one delimited group.
struct GroupTrivia {
    /// Comments after the opening delimiter and before the first token.
    opening: TrailingTrivia,
    /// Comments after the closing delimiter.
    closing: TrailingTrivia,
}

/// Trivia settled onto raw atoms, keyed by the range of its owner.
#[derive(Default)]
pub(crate) struct TriviaTransfer {
    /// Comments following each raw atom, keyed by the atom's range.
    terminals: HashMap<Bounds, TrailingTrivia>,
    /// Delimiter trivia, keyed by the opening delimiter's range.
    groups: HashMap<Bounds, GroupTrivia>,
    /// Comments before each cell, keyed by the start of its first token.
    cells: HashMap<Offset, LeadingTrivia>,
    /// Comments following the last cell that has tokens.
    closing: Vec<Trivia>,
}

impl TriviaTransfer {
    /// Lexes and frames `source` exactly as the native parser does, then
    /// collects the trivia of its cells.
    ///
    /// # Errors
    ///
    /// Returns [`LexError`] for lexical failures beyond missing closers.
    pub(crate) fn from_source(source: &Arc<Source>) -> Result<Self, LexError> {
        let FramedCells { cells, incomplete } =
            frame_recovering(lex_source_recovering(source)?.value);
        Ok(Self::from_cells(cells.into_iter().chain(incomplete)))
    }

    /// Collects the trivia of framed cells, giving a comment-only cell's
    /// comments to the next cell with tokens, as the native parser does.
    fn from_cells(cells: impl IntoIterator<Item = CellTokenStream>) -> Self {
        let mut transfer = Self::default();
        let mut detached = Vec::new();
        for cell in cells {
            let mut tokens = cell.into_parser_token_stream();
            let Some(start) = tokens.iter().next().and_then(|first| first.span().start()) else {
                detached.extend(CommentTrivia::of(&tokens).into_trivia());
                continue;
            };
            detached.extend(tokens.take_prefix_trivia().into_vec());
            transfer
                .cells
                .insert(start, LeadingTrivia::new(take(&mut detached)));
            transfer.collect_stream(&tokens);
        }
        transfer.closing = detached;
        transfer
    }

    /// Places the collected trivia on the matching nodes of `source_file`.
    pub(crate) fn apply(mut self, source_file: &mut SourceFile) {
        self.visit_source_file_mut(source_file);
        source_file.set_closing_comments(take(&mut self.closing));
    }

    /// Records the trivia owned by every atom of a settled stream.
    fn collect_stream(&mut self, tokens: &TokenStream) {
        for token in tokens {
            let Some(bounds) = token.span().bounds() else {
                continue;
            };
            if let TokenTree::Group(group) = token {
                self.collect_stream(group.stream());
                let Some(opening) = group.delimiter().spans.open().bounds() else {
                    continue;
                };
                self.groups.insert(
                    opening,
                    GroupTrivia {
                        opening: TrailingTrivia::new(group.stream().prefix_trivia().to_vec()),
                        closing: token.trailing_trivia().clone(),
                    },
                );
            } else if !token.trailing_trivia().is_empty() {
                self.terminals
                    .insert(bounds, token.trailing_trivia().clone());
            }
        }
    }
}

impl VisitMut for TriviaTransfer {
    fn visit_cell_mut(&mut self, cell: &mut Cell) {
        if let Some(leading) = cell
            .try_span()
            .and_then(|span| span.start())
            .and_then(|start| self.cells.remove(&start))
        {
            cell.leading_trivia = leading;
        }
        visit_mut::visit_cell_mut(self, cell);
    }

    fn visit_terminal_mut(&mut self, terminal: &mut dyn Terminal) {
        if let Some(trailing) = terminal
            .try_span()
            .and_then(|span| span.bounds())
            .and_then(|bounds| self.terminals.remove(&bounds))
        {
            *terminal.trailing_trivia_mut() = trailing;
        }
    }

    fn visit_delimiter_mut(&mut self, delimiter: &mut Delimiter) {
        if let Some(GroupTrivia { opening, closing }) = delimiter
            .spans
            .open()
            .bounds()
            .and_then(|bounds| self.groups.remove(&bounds))
        {
            *delimiter.opening_trivia_mut() = opening;
            *delimiter.closing_trivia_mut() = closing;
        }
    }
}
