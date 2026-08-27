//! Tree-sitter parser adapter.
//!
//! Tree-sitter nodes remain backend-local. Complete-file parsing lowers the
//! grammar's concrete nodes directly into the generated typed graph; it never
//! reparses their source through the native precedence engine.
//! [`TreeSitterNode`] is also the generic [`ExternalCstNode`] adapter used for
//! migrating leaves, operators, and tokens with the generated machinery.

use std::borrow::Cow;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::ops::Range;
use std::sync::Arc;
use std::vec::IntoIter;

use crate::native::{assignment_pack, installation_value, lambda_parameters};
use crate::nodes::*;
use crate::{
    CSTNodeClassLabel, CellStream, CellStreamError, DelimiterKind, DelimiterToken, DoubleSpan,
    ExternalCstChild, ExternalCstNode, LexError, Migrate, MigrateError, Parser, Punctuated,
    Recovered, Source, SourceFile, Span, Spanned, TextEdit, TokenStream, TokenTree, Tokenize,
    lex_str, lex_str_recovering,
};

#[derive(Debug)]
/// A failure while parsing or migrating a Tree-sitter syntax tree.
pub enum ParseError {
    /// The renaming hook could not inspect the source token stream.
    Lex(LexError),
    /// The raw token stream could not be partitioned into complete cells.
    Tokenize(CellStreamError),
    /// The bundled grammar could not be installed in the parser.
    Language(tree_sitter::LanguageError),
    /// Tree-sitter returned no tree.
    Cancelled,
    /// A requested source edit did not lie on valid boundaries of the buffer.
    InvalidEdit,
    /// Tree-sitter produced an error or missing node at this span.
    InvalidSyntax(Span),
    /// The untyped Tree-sitter node could not migrate typed syntax.
    Migrate(MigrateError),
    /// A valid concrete node has no representation in the typed schema.
    UnsupportedNode {
        /// The Tree-sitter node kind that could not be lowered.
        kind: String,
        /// The complete node range.
        span: Span,
    },
}

impl Display for ParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lex(error) => error.fmt(formatter),
            Self::Tokenize(error) => error.fmt(formatter),
            Self::Language(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("Tree-sitter parsing was cancelled"),
            Self::InvalidEdit => formatter.write_str("source edit is outside the current buffer"),
            Self::InvalidSyntax(span) if span.line_column_range().is_some() => {
                write!(formatter, "invalid Macaulay2 syntax at {span}")
            }
            Self::InvalidSyntax(_) => formatter.write_str("invalid Macaulay2 syntax"),
            Self::Migrate(error) => error.fmt(formatter),
            Self::UnsupportedNode { kind, .. } => {
                write!(
                    formatter,
                    "Tree-sitter node `{kind}` has no typed representation"
                )
            }
        }
    }
}

impl Error for ParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Lex(error) => Some(error),
            Self::Tokenize(error) => Some(error),
            Self::Language(error) => Some(error),
            Self::Migrate(error) => Some(error),
            Self::Cancelled
            | Self::InvalidEdit
            | Self::InvalidSyntax(_)
            | Self::UnsupportedNode { .. } => None,
        }
    }
}

impl From<MigrateError> for ParseError {
    fn from(error: MigrateError) -> Self {
        Self::Migrate(error)
    }
}

impl From<LexError> for ParseError {
    fn from(error: LexError) -> Self {
        Self::Lex(error)
    }
}

impl From<CellStreamError> for ParseError {
    fn from(error: CellStreamError) -> Self {
        Self::Tokenize(error)
    }
}

/// A reusable Tree-sitter parser and direct typed-CST lowerer.
pub struct TreeSitterParser {
    parser: tree_sitter::Parser,
}

/// An incrementally updated Tree-sitter parse for editor-style source changes.
///
/// The parser retains its previous Tree-sitter tree, applies each [`TextEdit`]
/// to that tree, and supplies it to the next parse. Typed syntax is lowered
/// afresh from the incrementally produced tree, so every public node and span
/// describes the current source snapshot. Missing group closers are recoverable
/// and remain available through [`Self::diagnostics`].
pub struct RealtimeParser {
    parser: tree_sitter::Parser,
    source: Arc<Source>,
    parse_view: String,
    tree: tree_sitter::Tree,
    syntax: SourceFile,
    diagnostics: Vec<LexError>,
    changed_ranges: Vec<Span>,
}

impl RealtimeParser {
    /// Creates a retained parse of the initial source snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] when the grammar cannot parse and lower the
    /// source, apart from recoverable missing group closers.
    pub fn new(source: impl Into<String>) -> Result<Self, ParseError> {
        let source = Source::new(source.into());
        let mut parser = TreeSitterParser::new()?.parser;
        let ParsedTree {
            parse_view,
            tree,
            syntax,
            diagnostics,
        } = parse_recovered_tree(&mut parser, Arc::clone(&source), None)?;
        Ok(Self {
            parser,
            source,
            parse_view,
            tree,
            syntax,
            diagnostics,
            changed_ranges: Vec::new(),
        })
    }

    /// Applies one source edit and incrementally refreshes the typed syntax.
    ///
    /// The update is transactional: a failed edit or parse leaves the previous
    /// source, tree, syntax, and diagnostics unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::InvalidEdit`] when the edit is outside the current
    /// UTF-8 buffer, or another [`ParseError`] when the edited source cannot be
    /// recovered and lowered.
    pub fn apply_edit(&mut self, edit: &TextEdit) -> Result<(), ParseError> {
        let source_text = edit
            .apply(self.source.text())
            .ok_or(ParseError::InvalidEdit)?;
        let source = Source::with_file(self.source.file(), source_text);
        let (view, aliases, diagnostics) = renamed_source_recovering(source.text())?;
        let parse_view = view.into_owned();

        let mut edited_tree = self.tree.clone();
        edited_tree.edit(&input_edit_between(&self.parse_view, &parse_view));
        let tree = self
            .parser
            .parse(parse_view.as_bytes(), Some(&edited_tree))
            .ok_or(ParseError::Cancelled)?;
        if let Some(error) = first_error(tree.root_node()) {
            return Err(ParseError::InvalidSyntax(Span::ranged(
                Arc::clone(&source),
                error.start_byte().min(source.text().len())
                    ..error.end_byte().min(source.text().len()),
            )));
        }
        let syntax = Lowerer::new(Arc::clone(&source), source.text(), aliases)
            .source_file(tree.root_node())?;
        let changed_ranges = edited_tree
            .changed_ranges(&tree)
            .map(|range| {
                Span::ranged(
                    Arc::clone(&source),
                    range.start_byte.min(source.text().len())
                        ..range.end_byte.min(source.text().len()),
                )
            })
            .collect();

        self.source = source;
        self.parse_view = parse_view;
        self.tree = tree;
        self.syntax = syntax;
        self.diagnostics = diagnostics;
        self.changed_ranges = changed_ranges;
        Ok(())
    }

    /// Returns the current source snapshot.
    pub fn source(&self) -> &str {
        self.source.text()
    }

    /// Returns the typed syntax for the current snapshot.
    pub fn syntax(&self) -> &SourceFile {
        &self.syntax
    }

    /// Returns the recoverable diagnostics for the current snapshot.
    pub fn diagnostics(&self) -> &[LexError] {
        &self.diagnostics
    }

    /// Returns structural ranges changed by the most recent incremental parse.
    pub fn changed_ranges(&self) -> &[Span] {
        &self.changed_ranges
    }

    /// Consumes the retained parser and returns its current typed syntax.
    pub fn into_syntax(self) -> SourceFile {
        self.syntax
    }
}

struct ParsedTree {
    parse_view: String,
    tree: tree_sitter::Tree,
    syntax: SourceFile,
    diagnostics: Vec<LexError>,
}

fn parse_recovered_tree(
    parser: &mut tree_sitter::Parser,
    source: Arc<Source>,
    old_tree: Option<&tree_sitter::Tree>,
) -> Result<ParsedTree, ParseError> {
    let source_text = source.text();
    let (view, aliases, diagnostics) = renamed_source_recovering(source_text)?;
    let parse_view = view.into_owned();
    let tree = parser
        .parse(parse_view.as_bytes(), old_tree)
        .ok_or(ParseError::Cancelled)?;
    if let Some(error) = first_error(tree.root_node()) {
        return Err(ParseError::InvalidSyntax(Span::ranged(
            Arc::clone(&source),
            error.start_byte().min(source_text.len())..error.end_byte().min(source_text.len()),
        )));
    }
    let syntax =
        Lowerer::new(Arc::clone(&source), source_text, aliases).source_file(tree.root_node())?;
    Ok(ParsedTree {
        parse_view,
        tree,
        syntax,
        diagnostics,
    })
}

fn input_edit_between(old: &str, new: &str) -> tree_sitter::InputEdit {
    let mut start = old
        .as_bytes()
        .iter()
        .zip(new.as_bytes())
        .take_while(|(old, new)| old == new)
        .count();
    while !old.is_char_boundary(start) || !new.is_char_boundary(start) {
        start -= 1;
    }

    let available_suffix = old.len().min(new.len()).saturating_sub(start);
    let mut suffix = old
        .as_bytes()
        .iter()
        .rev()
        .zip(new.as_bytes().iter().rev())
        .take(available_suffix)
        .take_while(|(old, new)| old == new)
        .count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }

    let old_end = old.len() - suffix;
    let new_end = new.len() - suffix;
    tree_sitter::InputEdit {
        start_byte: start,
        old_end_byte: old_end,
        new_end_byte: new_end,
        start_position: point_at(old, start),
        old_end_position: point_at(old, old_end),
        new_end_position: point_at(new, new_end),
    }
}

fn point_at(source: &str, offset: usize) -> tree_sitter::Point {
    let before = &source[..offset];
    let row = before.bytes().filter(|byte| *byte == b'\n').count();
    let column = before
        .as_bytes()
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(before.len(), |newline| before.len() - newline - 1);
    tree_sitter::Point { row, column }
}

impl TreeSitterParser {
    /// Creates a parser loaded with the bundled Macaulay2 grammar.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::Language`] if Tree-sitter rejects the bundled
    /// language definition.
    pub fn new() -> Result<Self, ParseError> {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_macaulay2::language())
            .map_err(ParseError::Language)?;
        Ok(Self { parser })
    }

    fn parse_source_in(&mut self, source: Arc<Source>) -> Result<SourceFile, ParseError> {
        let source_text = source.text();
        let (renamed, aliases) = renamed_source(source_text)?;
        let tree = self
            .parser
            .parse(renamed.as_bytes(), None)
            .ok_or(ParseError::Cancelled)?;
        if let Some(error) = first_error(tree.root_node()) {
            return Err(ParseError::InvalidSyntax(Span::ranged(
                Arc::clone(&source),
                error.start_byte()..error.end_byte(),
            )));
        }
        Lowerer::new(Arc::clone(&source), source_text, aliases).source_file(tree.root_node())
    }

    /// Parses incomplete source while retaining syntax inside unclosed groups.
    ///
    /// The parsing view is completed only for Tree-sitter; migrated nodes
    /// retain the original source spans and zero-width missing closers.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError`] when source has a non-recoverable lexical or
    /// grammatical failure, parsing is cancelled, or lowering fails.
    pub fn parse_recovering(
        &mut self,
        source: &str,
    ) -> Result<Recovered<SourceFile, LexError>, ParseError> {
        let source = Source::new(source);
        let source_text = source.text();
        let (renamed, aliases, diagnostics) = renamed_source_recovering(source_text)?;
        let tree = self
            .parser
            .parse(renamed.as_bytes(), None)
            .ok_or(ParseError::Cancelled)?;
        if let Some(error) = first_error(tree.root_node()) {
            return Err(ParseError::InvalidSyntax(Span::ranged(
                Arc::clone(&source),
                error.start_byte().min(source_text.len())..error.end_byte().min(source_text.len()),
            )));
        }
        let syntax = Lowerer::new(Arc::clone(&source), source_text, aliases)
            .source_file(tree.root_node())?;
        Ok(Recovered::new(syntax, diagnostics))
    }
}

impl Parser for TreeSitterParser {
    type Error = ParseError;

    fn parse_cells(&mut self, cells: CellStream) -> Result<SourceFile, Self::Error> {
        let (cell_comments, comments) = cells.comment_ownership();
        let mut source_file = match cells.complete_source() {
            Some(source) => self.parse_source_in(source),
            None => self.parse_source_in(Source::new(cells.to_string())),
        }?;
        if source_file.cells.len() == cell_comments.len() {
            for (cell, comments) in source_file.cells.iter_mut().zip(cell_comments) {
                cell.comments = comments;
            }
            source_file.comments = comments;
        } else {
            source_file.comments = comments;
            for comments in cell_comments {
                source_file.comments.append(comments);
            }
        }
        Ok(source_file)
    }
}

/// Parses source with Tree-sitter and lowers its CST into the typed syntax tree.
///
/// # Errors
///
/// Returns [`ParseError`] when parsing or typed migration fails.
pub fn parse_file(source: &str) -> Result<SourceFile, ParseError> {
    let mut parser = TreeSitterParser::new()?;
    let cells = lex_str(source)?.to_cell_stream()?;
    parser.parse_cells(cells)
}

/// Parses incomplete source while retaining syntax inside unclosed groups.
///
/// # Errors
///
/// Returns [`ParseError`] for failures other than missing closing delimiters or
/// when the recovered Tree-sitter CST cannot be lowered.
pub fn parse_file_recovering(source: &str) -> Result<Recovered<SourceFile, LexError>, ParseError> {
    TreeSitterParser::new()?.parse_recovering(source)
}

/// Parses an emitted M2 token stream into the complete typed source file.
///
/// # Errors
///
/// Returns [`ParseError`] when the emitted source is invalid or migration
/// fails.
pub fn parse_tokens(tokens: &TokenStream) -> Result<SourceFile, ParseError> {
    let cells = tokens.to_cell_stream()?;
    TreeSitterParser::new()?.parse_cells(cells)
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

struct Lowerer<'source> {
    source: &'source [u8],
    source_span: Span,
    aliases: Vec<CoreAlias>,
}

impl<'source> Lowerer<'source> {
    fn new(source_owner: Arc<Source>, source: &'source str, aliases: Vec<CoreAlias>) -> Self {
        Self {
            source: source.as_bytes(),
            source_span: Span::ranged(source_owner, 0..source.len()),
            aliases,
        }
    }

    fn source_file(&self, root: tree_sitter::Node<'_>) -> Result<SourceFile, ParseError> {
        let mut cells = Vec::new();
        let raw_cells = self.semantic_children(root);
        for (index, child) in raw_cells.iter().copied().enumerate() {
            match child.kind() {
                "cell" | "muted" => {
                    let next = raw_cells.get(index + 1).copied();
                    cells.push(self.cell(child, self.eoc_offset(child, next))?);
                }
                kind => return Err(self.unsupported(kind, child)),
            }
        }
        let eof_offset = cells
            .last()
            .and_then(|cell| cell.eoc.span().start())
            .unwrap_or(self.source.len());
        Ok(SourceFile::new(
            cells,
            EOF::new("", self.source_span.at(eof_offset)),
        ))
    }

    fn cell(&self, node: tree_sitter::Node<'_>, eoc_offset: usize) -> Result<Cell, ParseError> {
        let (content, semicolon) = if node.kind() == "muted" {
            let semicolon = self.required_kind(node, ";")?;
            (
                self.expr_pack(self.children(node))?,
                Some(self.migrate(semicolon)?),
            )
        } else {
            let child = self.only_semantic_child(node)?;
            let pack = if child.kind() == "naked_sequence" {
                self.expr_pack(self.children(child))?
            } else {
                ExprPack::new(Punctuated::from_single(Component::Expr(Box::new(
                    self.expr(child)?,
                ))))
            };
            (pack, None)
        };
        Ok(Cell::new(
            content,
            semicolon,
            EOC::new("", self.source_span.at(eoc_offset)),
        ))
    }

    fn eoc_offset(
        &self,
        _node: tree_sitter::Node<'_>,
        next: Option<tree_sitter::Node<'_>>,
    ) -> usize {
        next.map_or(self.source.len(), |next| next.start_byte())
            .min(self.source.len())
    }

    fn expr_pack(&self, nodes: Vec<tree_sitter::Node<'_>>) -> Result<ExprPack, ParseError> {
        let pack_span = nodes
            .iter()
            .map(|node| self.span(*node))
            .reduce(|left, right| {
                left.join(right)
                    .expect("Tree-sitter nodes share one source snapshot")
            })
            .unwrap_or_else(|| Span::unlocated(Arc::clone(self.source_span.source())));
        let mut values = Vec::new();
        let mut commas = Vec::new();
        for node in nodes.into_iter().filter(|node| !node.is_extra()) {
            match node.kind() {
                "," => commas.push(self.migrate::<Token![,]>(node)?),
                "empty_component" => values.push(Component::Empty(Empty::new("", self.span(node)))),
                ";" | "(" | ")" | "[" | "]" | "{" | "}" | "<|" | "|>" | "muted" => {}
                _ if node.is_named() => values.push(Component::Expr(Box::new(self.expr(node)?))),
                _ => {}
            }
        }
        let mut values = values.into_iter();
        let Some(first) = values.next() else {
            return Err(self.unsupported_span("empty expression pack", pack_span));
        };
        let value_count = values.len() + 1;
        let comma_count = commas.len();
        if value_count != comma_count + 1 {
            return Err(self.unsupported_span("invalid expression pack", pack_span));
        }
        let rest = commas
            .into_iter()
            .zip(values)
            .collect::<Vec<(Token![,], Component)>>();
        Ok(ExprPack::new(Punctuated::new(first, rest)))
    }

    fn expr(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        match node.kind() {
            "symbol" => Ok(self.migrate::<Symbol>(node)?.into()),
            "integer_literal" if self.text(node).contains('p') => {
                Ok(FloatLiteral::new(self.text(node), self.span(node)).into())
            }
            "integer_literal" => Ok(self.migrate::<IntegerLiteral>(node)?.into()),
            "float_literal" => Ok(self.migrate::<FloatLiteral>(node)?.into()),
            "string_literal" => self.string_literal(node),
            "raw_string_literal" => self.raw_string_literal(node),
            "array" | "list" | "angle_bar_list" | "sequence" | "parenthesized_expression" => {
                self.collection(node)
            }
            "binary_expression" => self.binary_expr(node),
            "prefix_expression" => self.prefix_expr(node),
            "postfix_expression" => self.postfix_expr(node),
            "lambda_expression" => self.lambda_expression(node),
            "assignment"
            | "local_assignment"
            | "binary_assignment"
            | "prefix_assignment"
            | "postfix_assignment"
            | "binary_installation"
            | "prefix_installation"
            | "postfix_installation"
            | "structured_binding"
            | "local_structured_binding" => self.assignment(node),
            "evaluated_assignment" => Ok(EvaluatedAssignment::new(
                self.expr(self.required_field(node, "left")?)?,
                self.migrate(self.required_field(node, "operator")?)?,
                self.expr(self.required_field(node, "right")?)?,
            )
            .into()),
            "option" => Ok(OptionExpression::new(
                self.expr(self.required_field(node, "left")?)?,
                self.migrate(self.required_field(node, "operator")?)?,
                self.expr(self.required_field(node, "right")?)?,
            )
            .into()),
            "if_statement" => self.if_statement(node),
            "for_loop" => self.for_loop(node),
            "while_loop" => self.while_loop(node),
            "try_statement" => self.try_statement(node),
            "quote_expression" => self.quote_expression(node),
            "new_statement" => self.new_statement(node),
            "break_statement" => Ok(BreakExpr::new(
                self.migrate(self.required_kind(node, "break")?)?,
                self.first_expression_child(node)
                    .map(|child| self.expr(child))
                    .transpose()?,
            )
            .into()),
            "continue_statement" => Ok(ContinueExpr::new(
                self.migrate(self.required_kind(node, "continue")?)?,
                self.first_expression_child(node)
                    .map(|child| self.expr(child))
                    .transpose()?,
            )
            .into()),
            "return_statement" => Ok(ReturnExpr::new(
                self.migrate(self.required_kind(node, "return")?)?,
                self.first_expression_child(node)
                    .map(|child| self.expr(child))
                    .transpose()?,
            )
            .into()),
            "catch_statement" => Ok(Catch::new(
                self.migrate(self.required_kind(node, "catch")?)?,
                self.expr(self.required_expression_child(node)?)?,
            )
            .into()),
            "throw_statement" => Ok(Throw::new(
                self.migrate(self.required_kind(node, "throw")?)?,
                self.expr(self.required_expression_child(node)?)?,
            )
            .into()),
            "trap_statement" => Ok(Trap::new(
                self.migrate(self.required_kind(node, "trap")?)?,
                self.expr(self.required_expression_child(node)?)?,
            )
            .into()),
            "debug_clause" => self.debug_clause(node),
            kind => Err(self.unsupported(kind, node)),
        }
    }

    fn collection(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let spans = self.delimiter_spans(node)?;
        let statements = self
            .semantic_children(node)
            .into_iter()
            .filter(|child| child.kind() == "muted")
            .map(|child| self.statement(child))
            .collect::<Result<Vec<_>, _>>()?;

        if node.kind() == "parenthesized_expression" {
            let value = self
                .semantic_children(node)
                .into_iter()
                .find(|child| child.kind() != "muted")
                .map(|child| self.expr(child))
                .transpose()?;
            let mut holder = Holder::new(statements, value);
            holder.delimiter = <Delimiter![()] as DelimiterToken>::new(spans);
            return Ok(holder.into());
        }

        let has_elements = self.children(node).into_iter().any(|child| {
            !child.is_extra()
                && (child.kind() == ","
                    || child.kind() == "empty_component"
                    || (child.is_named() && child.kind() != "muted"))
        });
        let elements = has_elements
            .then(|| self.expr_pack(self.children(node)))
            .transpose()?;
        Ok(match node.kind() {
            "array" => {
                let mut value = Array::new(statements, elements);
                value.delimiter = <Delimiter![[]] as DelimiterToken>::new(spans);
                value.into()
            }
            "list" => {
                let mut value = List::new(statements, elements);
                value.delimiter = <Delimiter![{}] as DelimiterToken>::new(spans);
                value.into()
            }
            "angle_bar_list" => {
                let mut value = AngleBarList::new(statements, elements);
                value.delimiter = <Delimiter![<| |>] as DelimiterToken>::new(spans);
                value.into()
            }
            "sequence" => {
                let mut value = Sequence::new(statements, elements);
                value.delimiter = <Delimiter![()] as DelimiterToken>::new(spans);
                value.into()
            }
            kind => return Err(self.unsupported(kind, node)),
        })
    }

    fn statement(&self, node: tree_sitter::Node<'_>) -> Result<Statement, ParseError> {
        Ok(Statement::new(
            self.expr_pack(self.children(node))?,
            self.migrate(self.required_kind(node, ";")?)?,
        ))
    }

    fn string_literal(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let elements = self
            .children(node)
            .into_iter()
            .filter(|child| child.kind() != "\"")
            .filter(|child| !child.is_extra())
            .map(|child| match child.kind() {
                "escape_sequence" => self.migrate(child).map(StringElement::EscapeSequence),
                "string_content" => self.migrate(child).map(StringElement::StringContent),
                _ => Ok(StringElement::StringContent(StringContent::new(
                    self.text(child),
                    self.span(child),
                ))),
            })
            .collect::<Result<Vec<_>, ParseError>>()?;
        Ok(StringLiteral::new(elements, self.span(node)).into())
    }

    fn raw_string_literal(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let elements = self
            .children(node)
            .into_iter()
            .filter(|child| child.kind() != "///")
            .filter(|child| !child.is_extra())
            .map(|child| match child.kind() {
                "escape_sequence" => self.migrate(child).map(RawStringElement::EscapeSequence),
                "raw_string_content" => self.migrate(child).map(RawStringElement::RawStringContent),
                kind => Err(self.unsupported(kind, child)),
            })
            .collect::<Result<Vec<_>, ParseError>>()?;
        Ok(RawStringLiteral::new(elements, self.span(node)).into())
    }

    fn binary_expr(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let left = self.expr(self.required_field(node, "left")?)?;
        let operator = self.required_field(node, "operator")?;
        let right = self.expr(self.required_field(node, "right")?)?;
        if operator.kind() == "SPACE" {
            Ok(AdjacentExpr::new(left, Adj::new("", self.span(operator)), right).into())
        } else {
            Ok(BinaryExpr::new(left, self.migrate(operator)?, right).into())
        }
    }

    fn prefix_expr(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        Ok(PrefixExpr::new(
            self.migrate(self.required_field(node, "operator")?)?,
            self.expr(self.required_field(node, "operand")?)?,
        )
        .into())
    }

    fn postfix_expr(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        Ok(PostfixExpr::new(
            self.expr(self.required_field(node, "operand")?)?,
            self.migrate(self.required_field(node, "operator")?)?,
        )
        .into())
    }

    fn lambda_expression(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let parameter_expr = self.expr(self.required_field(node, "parameters")?)?;
        let parameters = lambda_parameters(parameter_expr)
            .ok_or_else(|| self.unsupported("lambda parameters", node))?;
        Ok(LambdaExpression::new(
            parameters,
            self.migrate(self.required_field(node, "operator")?)?,
            self.expr(self.required_field(node, "body")?)?,
        )
        .into())
    }

    fn assignment(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let right = self.expr(self.required_field(node, "right")?)?;
        Ok(match node.kind() {
            "assignment" => GlobalBinding::new(
                self.migrate(self.required_field(node, "left")?)?,
                self.migrate(self.required_field(node, "operator")?)?,
                right,
            )
            .into(),
            "local_assignment" => LocalBinding::new(
                self.migrate(self.required_field(node, "left")?)?,
                self.migrate(self.required_field(node, "operator")?)?,
                right,
            )
            .into(),
            "binary_assignment" | "prefix_assignment" | "postfix_assignment" => {
                let Expr::OperatorExpr(left) = self.expr(self.required_field(node, "left")?)?
                else {
                    return Err(self.unsupported("operator assignment target", node));
                };
                OperatorAssignment::new(
                    left,
                    self.migrate(self.required_field(node, "operator")?)?,
                    right,
                )
                .into()
            }
            "binary_installation" | "prefix_installation" | "postfix_installation" => {
                let Expr::OperatorExpr(left) = self.expr(self.required_field(node, "left")?)?
                else {
                    return Err(self.unsupported("operator installation target", node));
                };
                let (typical_value, function) = installation_value(right);
                match left {
                    OperatorExpr::AdjacentExpr(application) => MethodInstallation::new(
                        *application.left_operand,
                        *application.right_operand,
                        self.migrate(self.required_field(node, "operator")?)?,
                        typical_value,
                        function,
                    )
                    .into(),
                    domain => OperatorInstallation::new(
                        domain,
                        self.migrate(self.required_field(node, "operator")?)?,
                        typical_value,
                        function,
                    )
                    .into(),
                }
            }
            "structured_binding" | "local_structured_binding" => {
                let pack_expr = self.expr(self.required_field(node, "binding_pack")?)?;
                let pack = assignment_pack(pack_expr)
                    .ok_or_else(|| self.unsupported("assignment pack", node))?;
                if node.kind() == "local_structured_binding" {
                    LocalParallelAssignment::new(
                        pack,
                        self.migrate(self.required_field(node, "operator")?)?,
                        right,
                    )
                    .into()
                } else {
                    GlobalParallelAssignment::new(
                        pack,
                        self.migrate(self.required_field(node, "operator")?)?,
                        right,
                    )
                    .into()
                }
            }
            kind => return Err(self.unsupported(kind, node)),
        })
    }

    fn then_clause(&self, node: tree_sitter::Node<'_>) -> Result<ThenClause, ParseError> {
        Ok(ThenClause::new(
            self.migrate(self.required_kind(node, "then")?)?,
            self.expr(self.required_expression_child(node)?)?,
        ))
    }

    fn else_clause(&self, node: tree_sitter::Node<'_>) -> Result<ElseClause, ParseError> {
        Ok(ElseClause::new(
            self.migrate(self.required_kind(node, "else")?)?,
            self.expr(self.required_expression_child(node)?)?,
        ))
    }

    fn if_statement(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        Ok(IfStatement::new(
            self.migrate(self.required_kind(node, "if")?)?,
            self.expr(self.required_field(node, "condition")?)?,
            self.then_clause(self.required_kind(node, "then_clause")?)?,
            self.optional_kind(node, "else_clause")
                .map(|child| self.else_clause(child))
                .transpose()?,
        )
        .into())
    }

    fn loop_body(&self, node: tree_sitter::Node<'_>) -> Result<LoopBody, ParseError> {
        let list_step = self
            .optional_kind(node, "list")
            .map(|token| {
                Ok::<_, ParseError>(ListStep::new(
                    self.migrate(token)?,
                    self.expr(self.required_field(node, "listed_value")?)?,
                ))
            })
            .transpose()?;
        let do_step = self
            .optional_kind(node, "do")
            .map(|token| {
                Ok::<_, ParseError>(DoStep::new(
                    self.migrate(token)?,
                    self.expr(self.required_field(node, "ignored_value")?)?,
                ))
            })
            .transpose()?;
        Ok(LoopBody::new(list_step, do_step))
    }

    fn for_loop(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let iteration_domain = self
            .optional_kind(node, "iteration_range")
            .map(|range| self.iteration_domain(range))
            .transpose()?;
        let when_condition = self
            .optional_kind(node, "when")
            .map(|token| {
                Ok::<_, ParseError>(WhenCondition::new(
                    self.migrate(token)?,
                    self.expr(self.required_field(node, "filter")?)?,
                ))
            })
            .transpose()?;
        Ok(ForLoop::new(
            self.migrate(self.required_kind(node, "for")?)?,
            self.migrate(self.required_field(node, "variable")?)?,
            iteration_domain,
            when_condition,
            self.loop_body(self.required_kind(node, "loop_body")?)?,
        )
        .into())
    }

    fn iteration_domain(&self, node: tree_sitter::Node<'_>) -> Result<IterationDomain, ParseError> {
        if let Some(token) = self.optional_kind(node, "in") {
            return Ok(IterIn::new(
                self.migrate(token)?,
                self.expr(self.required_field(node, "iterated_collection")?)?,
            )
            .into());
        }
        let lower_bound = self
            .optional_kind(node, "from")
            .map(|token| {
                Ok::<_, ParseError>(RangeBoundFrom::new(
                    self.migrate(token)?,
                    self.expr(self.required_field(node, "range_start")?)?,
                ))
            })
            .transpose()?;
        let upper_bound = self
            .optional_kind(node, "to")
            .map(|token| {
                Ok::<_, ParseError>(RangeBoundTo::new(
                    self.migrate(token)?,
                    self.expr(self.required_field(node, "range_end")?)?,
                ))
            })
            .transpose()?;
        Ok(IterRange::new(lower_bound, upper_bound).into())
    }

    fn while_loop(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        Ok(WhileLoop::new(
            self.migrate(self.required_kind(node, "while")?)?,
            self.expr(self.required_field(node, "condition")?)?,
            self.loop_body(self.required_kind(node, "loop_body")?)?,
        )
        .into())
    }

    fn new_statement(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let mut installation = None;
        let class =
            self.new_statement_element(self.required_field(node, "class")?, &mut installation)?;
        let parent_class = self
            .optional_kind(node, "of")
            .map(|token| {
                Ok::<_, ParseError>(OfClass::new(
                    self.migrate(token)?,
                    self.new_statement_element(
                        self.required_field(node, "parent")?,
                        &mut installation,
                    )?,
                ))
            })
            .transpose()?;
        let instance = self
            .optional_kind(node, "from")
            .map(|token| {
                Ok::<_, ParseError>(FromInstance::new(
                    self.migrate(token)?,
                    self.new_statement_element(
                        self.required_field(node, "instance")?,
                        &mut installation,
                    )?,
                ))
            })
            .transpose()?;
        let statement = NewStatement::new(
            self.migrate(self.required_kind(node, "new")?)?,
            class,
            parent_class,
            instance,
        );
        Ok(match installation {
            Some((operator, function)) => {
                NewInstallation::new(statement, operator, function).into()
            }
            None => statement.into(),
        })
    }

    fn new_statement_element(
        &self,
        node: tree_sitter::Node<'_>,
        installation: &mut Option<(Token![:=], Expr)>,
    ) -> Result<Expr, ParseError> {
        match node.kind() {
            "assignment" => return Err(self.unsupported("`new` assignment with `=`", node)),
            "local_assignment" => {}
            _ => return self.expr(node),
        }
        *installation = Some((
            self.migrate(self.required_field(node, "operator")?)?,
            self.expr(self.required_field(node, "right")?)?,
        ));
        self.expr(self.required_field(node, "left")?)
    }

    fn try_statement(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let value = self
            .semantic_children(node)
            .into_iter()
            .find(|child| {
                child.kind() != "then_clause"
                    && child.kind() != "else_clause"
                    && child.kind() != "except_clause"
            })
            .ok_or_else(|| self.unsupported("try value", node))?;
        let on_success = self
            .optional_kind(node, "then_clause")
            .map(|child| self.then_clause(child))
            .transpose()?;
        let fallback = self
            .child_by_field(node, "fallback")
            .map(|child| match child.kind() {
                "else_clause" => self.else_clause(child).map(TryFallback::ElseClause),
                "except_clause" => self.except_do(child).map(TryFallback::ExceptDo),
                kind => Err(self.unsupported(kind, child)),
            })
            .transpose()?;
        Ok(TryStatement::new(
            self.migrate(self.required_kind(node, "try")?)?,
            self.expr(value)?,
            on_success,
            fallback,
        )
        .into())
    }

    fn except_do(&self, node: tree_sitter::Node<'_>) -> Result<ExceptDo, ParseError> {
        Ok(ExceptDo::new(
            self.migrate(self.required_kind(node, "except")?)?,
            self.migrate(self.required_field(node, "exception")?)?,
            self.migrate(self.required_kind(node, "do")?)?,
            self.expr(self.required_expression_child(node)?)?,
        ))
    }

    fn quote_expression(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let token = self
            .children(node)
            .into_iter()
            .find(|child| {
                matches!(
                    child.kind(),
                    "symbol" | "local" | "global" | "threadVariable" | "threadLocal"
                ) && self.child_by_field(node, "token") != Some(*child)
            })
            .ok_or_else(|| self.unsupported("quote specifier", node))?;
        let specifier = match token.kind() {
            "symbol" => QuoteSpecifier::Symbol(self.migrate(token)?),
            "local" => QuoteSpecifier::Local(self.migrate(token)?),
            "global" => QuoteSpecifier::Global(self.migrate(token)?),
            "threadVariable" => QuoteSpecifier::ThreadVariable(self.migrate(token)?),
            "threadLocal" => QuoteSpecifier::ThreadLocal(self.migrate(token)?),
            kind => return Err(self.unsupported(kind, token)),
        };
        let quoted = self.required_field(node, "token")?;
        Ok(
            QuoteExpression::new(specifier, Symbol::new(self.text(quoted), self.span(quoted)))
                .into(),
        )
    }

    fn debug_clause(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let keyword = self.required_field(node, "keyword")?;
        let value = self
            .first_expression_child(node)
            .map(|child| self.expr(child))
            .transpose()?;
        Ok(match keyword.kind() {
            "elapsedTime" => ProfilingExpr::new(
                ProfilingModifier::ElapsedTime(self.migrate(keyword)?),
                value.ok_or_else(|| self.unsupported("profiling operand", node))?,
            )
            .into(),
            "elapsedTiming" => ProfilingExpr::new(
                ProfilingModifier::ElapsedTiming(self.migrate(keyword)?),
                value.ok_or_else(|| self.unsupported("profiling operand", node))?,
            )
            .into(),
            "profile" => ProfilingExpr::new(
                ProfilingModifier::Profile(self.migrate(keyword)?),
                value.ok_or_else(|| self.unsupported("profiling operand", node))?,
            )
            .into(),
            "time" => ProfilingExpr::new(
                ProfilingModifier::Time(self.migrate(keyword)?),
                value.ok_or_else(|| self.unsupported("profiling operand", node))?,
            )
            .into(),
            "timing" => ProfilingExpr::new(
                ProfilingModifier::Timing(self.migrate(keyword)?),
                value.ok_or_else(|| self.unsupported("profiling operand", node))?,
            )
            .into(),
            "TEST" => DebugExpr::new(DebugKeyword::Test(self.migrate(keyword)?), value).into(),
            "step" => DebugExpr::new(DebugKeyword::Step(self.migrate(keyword)?), value).into(),
            "finish" => DebugExpr::new(DebugKeyword::Finish(self.migrate(keyword)?), value).into(),
            "shield" => DebugExpr::new(DebugKeyword::Shield(self.migrate(keyword)?), value).into(),
            "breakpoint" => {
                DebugExpr::new(DebugKeyword::Breakpoint(self.migrate(keyword)?), value).into()
            }
            kind => return Err(self.unsupported(kind, keyword)),
        })
    }

    fn migrate<T>(&self, node: tree_sitter::Node<'_>) -> Result<T, ParseError>
    where
        for<'tree> T: Migrate<TreeSitterNode<'tree, 'source>>,
    {
        T::migrate(TreeSitterNode::with_span(
            node,
            self.source,
            self.span(node),
        ))
        .map_err(Into::into)
    }

    fn delimiter_spans(&self, node: tree_sitter::Node<'_>) -> Result<DoubleSpan, ParseError> {
        let kind = match node.kind() {
            "parenthesized_expression" | "sequence" => DelimiterKind::Parenthesis,
            "array" => DelimiterKind::Bracket,
            "list" => DelimiterKind::Brace,
            "angle_bar_list" => DelimiterKind::AngleBar,
            other => return Err(self.unsupported(other, node)),
        };
        let opening = self.span(self.required_kind(node, kind.opening())?);
        let closing = self.required_kind(node, kind.closing())?;
        if closing.start_byte() >= self.source.len() {
            Ok(DoubleSpan::unclosed(opening))
        } else {
            Ok(DoubleSpan::new(opening, self.span(closing)))
        }
    }

    fn children<'tree>(&self, node: tree_sitter::Node<'tree>) -> Vec<tree_sitter::Node<'tree>> {
        (0..node.child_count())
            .filter_map(|index| node.child(u32::try_from(index).expect("child count fits u32")))
            .collect()
    }

    fn semantic_children<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
    ) -> Vec<tree_sitter::Node<'tree>> {
        self.children(node)
            .into_iter()
            .filter(|child| child.is_named() && !child.is_extra())
            .collect()
    }

    fn only_semantic_child<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
    ) -> Result<tree_sitter::Node<'tree>, ParseError> {
        let children = self.semantic_children(node);
        match children.as_slice() {
            [child] => Ok(*child),
            _ => Err(self.unsupported("cell contents", node)),
        }
    }

    fn first_expression_child<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
    ) -> Option<tree_sitter::Node<'tree>> {
        self.semantic_children(node).into_iter().find(|child| {
            !matches!(
                child.kind(),
                "then_clause" | "else_clause" | "except_clause" | "iteration_range" | "loop_body"
            )
        })
    }

    fn required_expression_child<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
    ) -> Result<tree_sitter::Node<'tree>, ParseError> {
        self.first_expression_child(node)
            .ok_or_else(|| self.unsupported("expression child", node))
    }

    fn child_by_field<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
        field: &str,
    ) -> Option<tree_sitter::Node<'tree>> {
        node.child_by_field_name(field)
    }

    fn required_field<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
        field: &str,
    ) -> Result<tree_sitter::Node<'tree>, ParseError> {
        self.child_by_field(node, field)
            .ok_or_else(|| self.unsupported(field, node))
    }

    fn optional_kind<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
        kind: &str,
    ) -> Option<tree_sitter::Node<'tree>> {
        self.children(node)
            .into_iter()
            .find(|child| !child.is_extra() && child.kind() == kind)
    }

    fn required_kind<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
        kind: &str,
    ) -> Result<tree_sitter::Node<'tree>, ParseError> {
        self.optional_kind(node, kind)
            .ok_or_else(|| self.unsupported(kind, node))
    }

    fn text(&self, node: tree_sitter::Node<'_>) -> Cow<'_, str> {
        let range = self
            .span(node)
            .bounds()
            .map_or_else(|| node.byte_range(), |(start, end)| start..end);
        String::from_utf8_lossy(&self.source[range])
    }

    fn span(&self, node: tree_sitter::Node<'_>) -> Span {
        let range = node.byte_range();
        self.aliases
            .iter()
            .find(|alias| alias.bare == range)
            .map_or_else(
                || {
                    self.source_span.subspan(
                        node.start_byte().min(self.source.len()),
                        node.end_byte().min(self.source.len()),
                    )
                },
                |alias| self.source_span.subspan(alias.full.start, alias.full.end),
            )
    }

    fn unsupported(&self, kind: impl Into<String>, node: tree_sitter::Node<'_>) -> ParseError {
        self.unsupported_span(kind, self.span(node))
    }

    fn unsupported_span(&self, kind: impl Into<String>, span: Span) -> ParseError {
        ParseError::UnsupportedNode {
            kind: kind.into(),
            span,
        }
    }
}

#[derive(Clone)]
struct CoreAlias {
    full: Range<usize>,
    bare: Range<usize>,
}

fn renamed_source(source: &str) -> Result<(Cow<'_, str>, Vec<CoreAlias>), LexError> {
    let tokens = lex_str(source)?;
    let mut aliases = Vec::new();
    collect_core_aliases(&tokens, &mut aliases);
    if aliases.is_empty() {
        return Ok((Cow::Borrowed(source), aliases));
    }

    let mut renamed = source.as_bytes().to_vec();
    for alias in &aliases {
        renamed[alias.full.start..alias.bare.start].fill(b' ');
    }
    Ok((
        Cow::Owned(String::from_utf8(renamed).expect("replacing ASCII bytes preserves UTF-8")),
        aliases,
    ))
}

type RecoveredSource<'source> = (Cow<'source, str>, Vec<CoreAlias>, Vec<LexError>);

fn renamed_source_recovering(source: &str) -> Result<RecoveredSource<'_>, LexError> {
    let recovered = lex_str_recovering(source)?;
    let (tokens, diagnostics) = recovered.into_parts();
    let mut aliases = Vec::new();
    collect_core_aliases(&tokens, &mut aliases);
    let mut missing = Vec::new();
    collect_missing_closers(&tokens, &mut missing);

    if aliases.is_empty() && missing.is_empty() {
        return Ok((Cow::Borrowed(source), aliases, diagnostics));
    }

    let mut renamed = source.as_bytes().to_vec();
    for alias in &aliases {
        renamed[alias.full.start..alias.bare.start].fill(b' ');
    }
    for kind in missing {
        renamed.extend_from_slice(kind.closing().as_bytes());
    }
    Ok((
        Cow::Owned(String::from_utf8(renamed).expect("ASCII recovery preserves UTF-8")),
        aliases,
        diagnostics,
    ))
}

fn collect_missing_closers(tokens: &TokenStream, missing: &mut Vec<DelimiterKind>) {
    for token in tokens.iter() {
        let TokenTree::Group(group) = token else {
            continue;
        };
        collect_missing_closers(group.stream(), missing);
        if !group.delimiter().is_closed() {
            missing.push(group.delim_kind());
        }
    }
}

fn collect_core_aliases(tokens: &TokenStream, aliases: &mut Vec<CoreAlias>) {
    for token in tokens.iter() {
        match token {
            TokenTree::Group(group) => collect_core_aliases(group.stream(), aliases),
            TokenTree::Ident(identifier) => {
                let spelling = identifier.text();
                let bare = crate::lexer::canonical_word_spelling(spelling);
                if bare == spelling {
                    continue;
                }
                let Some((start, end)) = identifier.expect_span().bounds() else {
                    continue;
                };
                aliases.push(CoreAlias {
                    full: start..end,
                    bare: end - bare.len()..end,
                });
            }
            TokenTree::Literal(_) | TokenTree::Punct(_) => {}
        }
    }
}

#[derive(Clone)]
/// A borrowed Tree-sitter node paired with the source bytes it spans.
pub struct TreeSitterNode<'tree, 'source> {
    node: tree_sitter::Node<'tree>,
    source: &'source [u8],
    span: Option<Span>,
}

impl<'tree, 'source> TreeSitterNode<'tree, 'source> {
    /// Creates an external migration node.
    pub fn new(node: tree_sitter::Node<'tree>, source: &'source [u8]) -> Self {
        Self {
            node,
            source,
            span: None,
        }
    }

    fn with_span(node: tree_sitter::Node<'tree>, source: &'source [u8], span: Span) -> Self {
        Self {
            node,
            source,
            span: Some(span),
        }
    }
}

impl ExternalCstNode for TreeSitterNode<'_, '_> {
    type Children<'syntax>
        = IntoIter<ExternalCstChild<Self>>
    where
        Self: 'syntax;

    fn identity(&self) -> CSTNodeClassLabel<'_> {
        CSTNodeClassLabel::new(self.node.kind(), self.node.is_named())
    }

    fn children(&self) -> Self::Children<'_> {
        let parent = self.node;
        (0..parent.child_count())
            .filter_map(|index| {
                let index = u32::try_from(index).expect("Tree-sitter child count fits u32");
                parent.child(index).map(|node| ExternalCstChild {
                    field: parent.field_name_for_child(index),
                    node: Self::new(node, self.source),
                })
            })
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn text(&self) -> Cow<'_, str> {
        let range = self
            .span
            .as_ref()
            .and_then(Span::bounds)
            .map_or_else(|| self.node.byte_range(), |(start, end)| start..end);
        String::from_utf8_lossy(&self.source[range])
    }

    fn is_extra(&self) -> bool {
        self.node.is_extra()
    }
}

impl Spanned for TreeSitterNode<'_, '_> {
    fn try_span(&self) -> Option<Span> {
        self.span.clone()
    }
}
