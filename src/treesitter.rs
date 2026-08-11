use std::borrow::Cow;
use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::{
    CstChild, CstNode, NodeIdentity, Reconstruct, ReconstructError, SourceFile, SourceId, Span,
    TextPoint, TextRange,
};

#[derive(Debug)]
pub enum ParseError {
    Language(tree_sitter::LanguageError),
    Cancelled,
    InvalidSyntax(Span),
    Reconstruct(ReconstructError),
}

impl Display for ParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Language(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("Tree-sitter parsing was cancelled"),
            Self::InvalidSyntax(span) => match span.range() {
                Ok(range) => write!(
                    formatter,
                    "invalid Macaulay2 syntax at byte {}",
                    range.start.byte
                ),
                Err(_) => formatter.write_str("invalid Macaulay2 syntax"),
            },
            Self::Reconstruct(error) => error.fmt(formatter),
        }
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Language(error) => Some(error),
            Self::Reconstruct(error) => Some(error),
            Self::Cancelled | Self::InvalidSyntax(_) => None,
        }
    }
}

impl From<ReconstructError> for ParseError {
    fn from(error: ReconstructError) -> Self {
        Self::Reconstruct(error)
    }
}

pub fn parse_file(source: &str, source_id: SourceId) -> Result<SourceFile, ParseError> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_macaulay2::language())
        .map_err(ParseError::Language)?;
    let tree = parser.parse(source, None).ok_or(ParseError::Cancelled)?;
    if let Some(error) = first_error(tree.root_node()) {
        return Err(ParseError::InvalidSyntax(
            TreeSitterNode::new(error, source.as_bytes(), source_id).span(),
        ));
    }
    SourceFile::reconstruct(TreeSitterNode::new(
        tree.root_node(),
        source.as_bytes(),
        source_id,
    ))
    .map_err(Into::into)
}

fn first_error(root: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if node.is_error() || node.is_missing() {
            return Some(node);
        }
        pending.extend((0..node.child_count()).rev().filter_map(|index| {
            node.child(u32::try_from(index).expect("Tree-sitter child count fits u32"))
        }));
    }
    None
}

#[derive(Clone, Copy)]
pub struct TreeSitterNode<'tree, 'source> {
    node: tree_sitter::Node<'tree>,
    source: &'source [u8],
    source_id: SourceId,
}

impl<'tree, 'source> TreeSitterNode<'tree, 'source> {
    pub fn new(node: tree_sitter::Node<'tree>, source: &'source [u8], source_id: SourceId) -> Self {
        Self {
            node,
            source,
            source_id,
        }
    }

    pub fn raw(self) -> tree_sitter::Node<'tree> {
        self.node
    }
}

impl CstNode for TreeSitterNode<'_, '_> {
    type Children<'syntax>
        = std::vec::IntoIter<CstChild<Self>>
    where
        Self: 'syntax;

    fn identity(&self) -> NodeIdentity<'_> {
        NodeIdentity::new(self.node.kind(), self.node.is_named())
    }

    fn children(&self) -> Self::Children<'_> {
        (0..self.node.child_count())
            .filter_map(|index| {
                let index = u32::try_from(index).expect("Tree-sitter child count fits u32");
                self.node.child(index).map(|node| CstChild {
                    field: self.node.field_name_for_child(index),
                    node: Self::new(node, self.source, self.source_id),
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.source[self.node.byte_range()])
    }

    fn span(&self) -> Span {
        let range = TextRange::new(
            TextPoint::new(
                self.node.start_position().row as u32,
                self.node.start_position().column as u32,
                self.node.start_byte(),
            ),
            TextPoint::new(
                self.node.end_position().row as u32,
                self.node.end_position().column as u32,
                self.node.end_byte(),
            ),
        )
        .expect("Tree-sitter node ranges are ordered");
        Span::located(self.source_id, range)
    }

    fn is_extra(&self) -> bool {
        self.node.is_extra()
    }
}
