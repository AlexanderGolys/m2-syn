use std::borrow::Cow;
use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::{
    CellStream, CstChild, CstNode, LexError, NodeIdentity, Parse, ReconstructError, SourceFile,
    SourceId, Span, TextPoint, TextRange, TokenStream, lex_str, reconstruct,
};

#[derive(Debug)]
pub enum ParseError {
    Lex(LexError),
    Language(tree_sitter::LanguageError),
    Cancelled,
    InvalidSyntax(Span),
    Reconstruct(ReconstructError),
}

impl Display for ParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lex(error) => error.fmt(formatter),
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
            Self::Lex(error) => Some(error),
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

impl From<LexError> for ParseError {
    fn from(error: LexError) -> Self {
        Self::Lex(error)
    }
}

pub struct TreeSitterParser {
    parser: tree_sitter::Parser,
}

impl TreeSitterParser {
    pub fn new() -> Result<Self, ParseError> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_macaulay2::language())
            .map_err(ParseError::Language)?;
        Ok(Self { parser })
    }
}

impl Parse for TreeSitterParser {
    type Error = ParseError;

    fn parse(&mut self, tokens: CellStream) -> Result<SourceFile, Self::Error> {
        let source_id = tokens.source_id();
        let source = tokens.to_string();
        let tree = self
            .parser
            .parse(&source, None)
            .ok_or(ParseError::Cancelled)?;
        if let Some(error) = first_error(tree.root_node()) {
            return Err(ParseError::InvalidSyntax(
                TreeSitterNode::new(error, source.as_bytes(), source_id).span(),
            ));
        }
        reconstruct(TreeSitterNode::new(
            tree.root_node(),
            source.as_bytes(),
            source_id,
        ))
        .map_err(Into::into)
    }
}

pub fn parse_file(source: &str, source_id: SourceId) -> Result<SourceFile, ParseError> {
    let mut parser = TreeSitterParser::new()?;
    parser.parse(lex_str(source, source_id)?)
}

/// Parses an emitted M2 token stream into the complete typed source file.
pub fn parse_tokens(tokens: &TokenStream, source_id: SourceId) -> Result<SourceFile, ParseError> {
    parse_file(&tokens.to_string(), source_id)
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

    fn normalized_node(self) -> tree_sitter::Node<'tree> {
        if self.node.kind() != "cell" {
            return self.node;
        }

        // Grammar 5 represented a global muted form as `cell(muted(...))`.
        // The current grammar exposes that `muted` node directly under the
        // source file, so erase the obsolete adapter-only wrapper here.
        let mut named_children = (0..self.node.named_child_count())
            .filter_map(|index| self.node.named_child(index as u32));
        match (named_children.next(), named_children.next()) {
            (Some(child), None) if child.kind() == "muted" => child,
            _ => self.node,
        }
    }

    fn is_implicit_adjacency(self) -> bool {
        let node = self.normalized_node();
        if node.kind() != "binary_expression" {
            return false;
        }
        let Some(operator) = node.child_by_field_name("operator") else {
            return false;
        };
        operator.kind() == "SPACE"
            && self.source[operator.byte_range()]
                .iter()
                .all(u8::is_ascii_whitespace)
    }
}

impl CstNode for TreeSitterNode<'_, '_> {
    type Children<'syntax>
        = std::vec::IntoIter<CstChild<Self>>
    where
        Self: 'syntax;

    fn identity(&self) -> NodeIdentity<'_> {
        if self.is_implicit_adjacency() {
            return NodeIdentity::new("adjacent_expression", true);
        }
        let node = self.normalized_node();
        NodeIdentity::new(node.kind(), node.is_named())
    }

    fn children(&self) -> Self::Children<'_> {
        let parent = self.normalized_node();
        (0..parent.child_count())
            .filter_map(|index| {
                let index = u32::try_from(index).expect("Tree-sitter child count fits u32");
                parent.child(index).map(|node| CstChild {
                    field: parent.field_name_for_child(index),
                    node: Self::new(node, self.source, self.source_id),
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.source[self.normalized_node().byte_range()])
    }

    fn span(&self) -> Span {
        let node = self.normalized_node();
        let range = TextRange::new(
            TextPoint::new(
                node.start_position().row as u32,
                node.start_position().column as u32,
                node.start_byte(),
            ),
            TextPoint::new(
                node.end_position().row as u32,
                node.end_position().column as u32,
                node.end_byte(),
            ),
        )
        .expect("Tree-sitter node ranges are ordered");
        Span::located(self.source_id, range)
    }

    fn is_extra(&self) -> bool {
        self.node.is_extra()
    }
}
