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

use crate::native::{LambdaArguments, assignment_list, installation_value, lambda_arguments};
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
    /// The parser.
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
    /// The parser.
    parser: tree_sitter::Parser,
    /// The source.
    source: Arc<Source>,
    /// The parse view.
    parse_view: String,
    /// The tree.
    tree: tree_sitter::Tree,
    /// The syntax.
    syntax: SourceFile,
    /// The diagnostics.
    diagnostics: Vec<LexError>,
    /// The changed ranges.
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

/// Internal state for ParsedTree.
struct ParsedTree {
    /// The parse view.
    parse_view: String,
    /// The tree.
    tree: tree_sitter::Tree,
    /// The syntax.
    syntax: SourceFile,
    /// The diagnostics.
    diagnostics: Vec<LexError>,
}

/// Parses recovered tree.
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

/// Handles input edit between.
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

/// Handles point at.
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

    /// Parses source in.
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

/// Handles first error.
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

/// Internal state for Lowerer.
struct Lowerer<'source> {
    /// The source.
    source: &'source [u8],
    /// The source span.
    source_span: Span,
    /// The aliases.
    aliases: Vec<CoreAlias>,
}

/// Internal alternatives for IterationDomain.
enum IterationDomain {
    /// The Range alternative.
    Range(Option<RangeBoundFrom>, Option<RangeBoundTo>),
    /// The In alternative.
    In(Token![in], Box<Expr>),
}

impl<'source> Lowerer<'source> {
    /// Creates a new value.
    fn new(source_owner: Arc<Source>, source: &'source str, aliases: Vec<CoreAlias>) -> Self {
        Self {
            source: source.as_bytes(),
            source_span: Span::ranged(source_owner, 0..source.len()),
            aliases,
        }
    }

    /// Handles source file.
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

    /// Handles cell.
    fn cell(&self, node: tree_sitter::Node<'_>, eoc_offset: usize) -> Result<Cell, ParseError> {
        let (content, semicolon) = if node.kind() == "muted" {
            let semicolon = self.required_kind(node, ";")?;
            (
                self.packed_content(self.children(node))?,
                Some(self.migrate(semicolon)?),
            )
        } else {
            let child = self.only_semantic_child(node)?;
            let content = if child.kind() == "naked_sequence" {
                self.expr_pack(self.children(child))?.into()
            } else {
                self.expr(child)?.into()
            };
            (content, None)
        };
        Ok(Cell::new(
            content,
            semicolon,
            EOC::new("", self.source_span.at(eoc_offset)),
        ))
    }

    /// Handles end-of-cell offset.
    fn eoc_offset(
        &self,
        _node: tree_sitter::Node<'_>,
        next: Option<tree_sitter::Node<'_>>,
    ) -> usize {
        next.map_or(self.source.len(), |next| next.start_byte())
            .min(self.source.len())
    }

    /// Handles expr pack.
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
        let mut rest = rest.into_iter();
        let Some((punctuation, second)) = rest.next() else {
            return Err(self.unsupported_span("expression pack without a comma", pack_span));
        };
        let mut pack = Punctuated::new(first, punctuation, second);
        for (punctuation, value) in rest {
            pack.push(punctuation, value);
        }
        Ok(ExprPack::new(pack))
    }

    /// Handles packed content.
    fn packed_content(
        &self,
        nodes: Vec<tree_sitter::Node<'_>>,
    ) -> Result<PackedContent, ParseError> {
        if nodes.iter().any(|node| node.kind() == ",") {
            return self.expr_pack(nodes).map(Into::into);
        }
        let mut expressions = nodes
            .into_iter()
            .filter(|node| !node.is_extra() && node.is_named() && node.kind() != "muted");
        let Some(expression) = expressions.next() else {
            return Err(self.unsupported_span("empty packed content", self.source_span.clone()));
        };
        if expressions.next().is_some() {
            return Err(self.unsupported("unseparated packed content", expression));
        }
        self.expr(expression).map(Into::into)
    }

    /// Handles expr.
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
            "evaluated_assignment" => Ok(Expr!(Assign::Evaluated {
                evaluated_domain: Box::new(self.expr(self.required_field(node, "left")?)?),
                operator: self.migrate(self.required_field(node, "operator")?)?,
                value: Box::new(self.expr(self.required_field(node, "right")?)?),
            })),
            "option" => Ok(Expr!(Option {
                key: Box::new(self.expr(self.required_field(node, "left")?)?),
                operator: self.migrate(self.required_field(node, "operator")?)?,
                value: Box::new(self.expr(self.required_field(node, "right")?)?),
            })),
            "if_statement" => self.if_statement(node),
            "for_loop" => self.for_loop(node),
            "while_loop" => self.while_loop(node),
            "try_statement" => self.try_statement(node),
            "quote_expression" => self.quote_expression(node),
            "new_statement" => self.new_statement(node),
            "break_statement" => Ok(Expr!(Control::Break {
                break_token: self.migrate(self.required_kind(node, "break")?)?,
                value: self
                    .first_expression_child(node)
                    .map(|child| self.expr(child).map(Box::new))
                    .transpose()?,
            })),
            "continue_statement" => Ok(Expr!(Control::Continue {
                continue_token: self.migrate(self.required_kind(node, "continue")?)?,
                value: self
                    .first_expression_child(node)
                    .map(|child| self.expr(child).map(Box::new))
                    .transpose()?,
            })),
            "return_statement" => Ok(Expr!(Control::Return {
                return_token: self.migrate(self.required_kind(node, "return")?)?,
                value: self
                    .first_expression_child(node)
                    .map(|child| self.expr(child).map(Box::new))
                    .transpose()?,
            })),
            "catch_statement" => Ok(Expr!(Catch {
                catch_token: self.migrate(self.required_kind(node, "catch")?)?,
                value: Box::new(self.expr(self.required_expression_child(node)?)?),
            })),
            "throw_statement" => Ok(Expr!(Throw {
                throw_token: self.migrate(self.required_kind(node, "throw")?)?,
                value: Box::new(self.expr(self.required_expression_child(node)?)?),
            })),
            "trap_statement" => Ok(Expr!(Trap {
                trap_token: self.migrate(self.required_kind(node, "trap")?)?,
                value: Box::new(self.expr(self.required_expression_child(node)?)?),
            })),
            "debug_clause" => self.debug_clause(node),
            kind => Err(self.unsupported(kind, node)),
        }
    }

    /// Handles collection.
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
            let mut holder = Holder::new(statements, value.map(Box::new));
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
            .then(|| self.packed_content(self.children(node)))
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

    /// Handles statement.
    fn statement(&self, node: tree_sitter::Node<'_>) -> Result<Statement, ParseError> {
        Ok(Statement::new(
            self.packed_content(self.children(node))?,
            self.migrate(self.required_kind(node, ";")?)?,
        ))
    }

    /// Handles string literal.
    fn string_literal(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let elements = self
            .children(node)
            .into_iter()
            .filter(|child| child.kind() != "\"")
            .filter(|child| !child.is_extra())
            .map(|child| match child.kind() {
                "escape_sequence" => self.migrate(child).map(StringElement::Escape),
                "string_content" => self.migrate(child).map(StringElement::Content),
                _ => Ok(StringElement::Content(StringContent::new(
                    self.text(child),
                    self.span(child),
                ))),
            })
            .collect::<Result<Vec<_>, ParseError>>()?;
        Ok(StringLiteral::new(elements, self.span(node)).into())
    }

    /// Handles raw string literal.
    fn raw_string_literal(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let elements = self
            .children(node)
            .into_iter()
            .filter(|child| child.kind() != "///")
            .filter(|child| !child.is_extra())
            .map(|child| match child.kind() {
                "escape_sequence" => self.migrate(child).map(RawStringElement::Escape),
                "raw_string_content" => self.migrate(child).map(RawStringElement::Content),
                kind => Err(self.unsupported(kind, child)),
            })
            .collect::<Result<Vec<_>, ParseError>>()?;
        Ok(RawStringLiteral::new(elements, self.span(node)).into())
    }

    /// Handles binary expr.
    fn binary_expr(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let left = self.expr(
            self.child_by_field(node, "left_operand")
                .or_else(|| self.child_by_field(node, "left"))
                .ok_or_else(|| self.unsupported("left operand", node))?,
        )?;
        let operator = self.required_field(node, "operator")?;
        let right = self.expr(self.required_field(node, "right")?)?;
        if operator.kind() == "SPACE" {
            Ok(Expr!(Op::Bin::Adj {
                left_operand: Box::new(left),
                operator: Adj::new("", self.span(operator)),
                right_operand: Box::new(right),
            }))
        } else {
            match self.text(operator).as_ref() {
                spelling if GENERATED_AUGMENTED_OPERATOR_SPELLINGS.contains(&spelling) => {
                    Ok(Expr!(Assign::Augmented {
                        target: Box::new(left),
                        operator: self.migrate(operator)?,
                        value: Box::new(right),
                    }))
                }
                "." => Ok(Expr!(Op::Bin::Member::Dot {
                    receiver: Box::new(left),
                    operator: self.migrate(operator)?,
                    member: Box::new(right),
                })),
                ".?" => Ok(Expr!(Op::Bin::Member::Optional {
                    receiver: Box::new(left),
                    operator: self.migrate(operator)?,
                    member: Box::new(right),
                })),
                "#" => Ok(Expr!(Op::Bin::Index {
                    receiver: Box::new(left),
                    operator: self.migrate(operator)?,
                    index: Box::new(right),
                })),
                _ => Ok(Expr!(Op::Bin::Regular {
                    left_operand: Box::new(left),
                    operator: self.migrate(operator)?,
                    right_operand: Box::new(right),
                })),
            }
        }
    }

    /// Handles prefix expr.
    fn prefix_expr(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        Ok(Expr!(Op::Prefix {
            operator: self.migrate(self.required_field(node, "operator")?)?,
            operand: Box::new(self.expr(self.required_field(node, "operand")?)?),
        }))
    }

    /// Handles postfix expr.
    fn postfix_expr(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        Ok(Expr!(Op::Postfix {
            operand: Box::new(self.expr(self.required_field(node, "operand")?)?),
            operator: self.migrate(self.required_field(node, "operator")?)?,
        }))
    }

    /// Handles lambda expression.
    fn lambda_expression(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let argument_expr = self.expr(self.required_field(node, "parameters")?)?;
        let arguments = lambda_arguments(argument_expr)
            .ok_or_else(|| self.unsupported("lambda arguments", node))?;
        let operator = self.migrate(self.required_field(node, "operator")?)?;
        let body = Box::new(self.expr(self.required_field(node, "body")?)?);
        Ok(match arguments {
            LambdaArguments::Variadic(argument) => Expr!(Lambda::Variadic {
                argument,
                operator,
                body,
            }),
            LambdaArguments::List(arguments) => Expr!(Lambda::List {
                arguments: *arguments,
                operator,
                body,
            }),
        })
    }

    /// Handles assignment.
    fn assignment(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let right = self.expr(self.required_field(node, "right")?)?;
        Ok(match node.kind() {
            "assignment" => Expr!(Assign::Simple::Global {
                variable: self.migrate(self.required_field(node, "left")?)?,
                operator: self.migrate(self.required_field(node, "operator")?)?,
                value: Box::new(right),
            }),
            "local_assignment" => Expr!(Assign::Simple::Local {
                variable: self.migrate(self.required_field(node, "left")?)?,
                operator: self.migrate(self.required_field(node, "operator")?)?,
                value: Box::new(right),
            }),
            "binary_assignment" | "prefix_assignment" | "postfix_assignment" => {
                let Expr::Op(left) = self.expr(self.required_field(node, "left")?)? else {
                    return Err(self.unsupported("operator assignment target", node));
                };
                Expr!(Assign::Op {
                    argument_expr: left,
                    operator: self.migrate(self.required_field(node, "operator")?)?,
                    value_expr: Box::new(right),
                })
            }
            "binary_installation" | "prefix_installation" | "postfix_installation" => {
                let Expr::Op(left) = self.expr(self.required_field(node, "left")?)? else {
                    return Err(self.unsupported("operator installation target", node));
                };
                let (typical_value, function) = installation_value(right);
                match left {
                    ExprOp::Bin(ExprOpBin::Adj(application)) => Expr!(Assign::Install::Method {
                        method: application.left_operand,
                        operator: application.operator,
                        signature: application.right_operand,
                        assign_op: self.migrate(self.required_field(node, "operator")?)?,
                        typical_value,
                        function: Box::new(function),
                    }),
                    domain_expr => Expr!(Assign::Install::Op {
                        domain_expr,
                        assign_op: self.migrate(self.required_field(node, "operator")?)?,
                        typical_value,
                        function: Box::new(function),
                    }),
                }
            }
            "structured_binding" | "local_structured_binding" => {
                let list_expr = self.expr(self.required_field(node, "binding_pack")?)?;
                let argument_list = assignment_list(list_expr)
                    .ok_or_else(|| self.unsupported("assignment argument list", node))?;
                if node.kind() == "local_structured_binding" {
                    Expr!(Assign::Parallel::Local {
                        argument_list: Box::new(argument_list),
                        operator: self.migrate(self.required_field(node, "operator")?)?,
                        value: Box::new(right),
                    })
                } else {
                    Expr!(Assign::Parallel::Global {
                        argument_list: Box::new(argument_list),
                        operator: self.migrate(self.required_field(node, "operator")?)?,
                        value: Box::new(right),
                    })
                }
            }
            kind => return Err(self.unsupported(kind, node)),
        })
    }

    /// Handles then clause.
    fn then_clause(&self, node: tree_sitter::Node<'_>) -> Result<ThenClause, ParseError> {
        Ok(ThenClause::new(
            self.migrate(self.required_kind(node, "then")?)?,
            Box::new(self.expr(self.required_expression_child(node)?)?),
        ))
    }

    /// Handles else clause.
    fn else_clause(&self, node: tree_sitter::Node<'_>) -> Result<ElseClause, ParseError> {
        Ok(ElseClause::new(
            self.migrate(self.required_kind(node, "else")?)?,
            Box::new(self.expr(self.required_expression_child(node)?)?),
        ))
    }

    /// Handles if statement.
    fn if_statement(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        Ok(Expr!(If {
            if_token: self.migrate(self.required_kind(node, "if")?)?,
            condition: Box::new(self.expr(self.required_field(node, "condition")?)?),
            then_clause: self.then_clause(self.required_kind(node, "then_clause")?)?,
            else_clause: self
                .optional_kind(node, "else_clause")
                .map(|child| self.else_clause(child))
                .transpose()?,
        }))
    }

    /// Handles loop body.
    fn loop_body(&self, node: tree_sitter::Node<'_>) -> Result<LoopBody, ParseError> {
        let list_step = self
            .optional_kind(node, "list")
            .map(|token| {
                Ok::<_, ParseError>(ListStep::new(
                    self.migrate(token)?,
                    Box::new(self.expr(self.required_field(node, "listed_value")?)?),
                ))
            })
            .transpose()?;
        let do_step = self
            .optional_kind(node, "do")
            .map(|token| {
                Ok::<_, ParseError>(DoStep::new(
                    self.migrate(token)?,
                    Box::new(self.expr(self.required_field(node, "ignored_value")?)?),
                ))
            })
            .transpose()?;
        Ok(LoopBody::new(list_step, do_step))
    }

    /// Handles for loop.
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
                    Box::new(self.expr(self.required_field(node, "filter")?)?),
                ))
            })
            .transpose()?;
        let for_token = self.migrate(self.required_kind(node, "for")?)?;
        let variable = self.migrate(self.required_field(node, "variable")?)?;
        let body = self.loop_body(self.required_kind(node, "loop_body")?)?;
        Ok(match iteration_domain {
            None => Expr!(For::Bare {
                for_token,
                variable,
                condition: when_condition,
                body,
            }),
            Some(IterationDomain::Range(lower_bound, upper_bound)) => Expr!(For::Range {
                for_token,
                variable,
                lower_bound,
                upper_bound,
                condition: when_condition,
                body,
            }),
            Some(IterationDomain::In(in_token, collection)) => Expr!(For::In {
                for_token,
                variable,
                in_token,
                collection,
                condition: when_condition,
                body,
            }),
        })
    }

    /// Handles iteration domain.
    fn iteration_domain(&self, node: tree_sitter::Node<'_>) -> Result<IterationDomain, ParseError> {
        if let Some(token) = self.optional_kind(node, "in") {
            return Ok(IterationDomain::In(
                self.migrate(token)?,
                Box::new(self.expr(self.required_field(node, "iterated_collection")?)?),
            ));
        }
        let lower_bound = self
            .optional_kind(node, "from")
            .map(|token| {
                Ok::<_, ParseError>(RangeBoundFrom::new(
                    self.migrate(token)?,
                    Box::new(self.expr(self.required_field(node, "range_start")?)?),
                ))
            })
            .transpose()?;
        let upper_bound = self
            .optional_kind(node, "to")
            .map(|token| {
                Ok::<_, ParseError>(RangeBoundTo::new(
                    self.migrate(token)?,
                    Box::new(self.expr(self.required_field(node, "range_end")?)?),
                ))
            })
            .transpose()?;
        Ok(IterationDomain::Range(lower_bound, upper_bound))
    }

    /// Handles while loop.
    fn while_loop(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        Ok(Expr!(While {
            while_token: self.migrate(self.required_kind(node, "while")?)?,
            condition: Box::new(self.expr(self.required_field(node, "condition")?)?),
            body: self.loop_body(self.required_kind(node, "loop_body")?)?,
        }))
    }

    /// Handles new statement.
    fn new_statement(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let mut installation = None;
        let class =
            self.new_statement_element(self.required_field(node, "class")?, &mut installation)?;
        let parent_class = self
            .optional_kind(node, "of")
            .map(|token| {
                Ok::<_, ParseError>(OfClass::new(
                    self.migrate(token)?,
                    Box::new(self.new_statement_element(
                        self.required_field(node, "parent")?,
                        &mut installation,
                    )?),
                ))
            })
            .transpose()?;
        let instance = self
            .optional_kind(node, "from")
            .map(|token| {
                Ok::<_, ParseError>(FromInstance::new(
                    self.migrate(token)?,
                    Box::new(self.new_statement_element(
                        self.required_field(node, "instance")?,
                        &mut installation,
                    )?),
                ))
            })
            .transpose()?;
        let statement = Expr!(New {
            new_token: self.migrate(self.required_kind(node, "new")?)?,
            class: Box::new(class),
            parent_class,
            instance,
        });
        Ok(match installation {
            Some((operator, function)) => {
                let Expr::New(new_statement) = statement else {
                    unreachable!("the constructor selected the new-statement specialization")
                };
                Expr!(Assign::Install::New {
                    new_statement,
                    operator,
                    function: Box::new(function),
                })
            }
            None => statement,
        })
    }

    /// Handles new statement element.
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

    /// Attempts to statement.
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
                "else_clause" => self.else_clause(child).map(TryFallback::Else),
                "except_clause" => self.except_do(child),
                kind => Err(self.unsupported(kind, child)),
            })
            .transpose()?;
        Ok(Expr!(Try {
            try_token: self.migrate(self.required_kind(node, "try")?)?,
            value: Box::new(self.expr(value)?),
            on_success,
            fallback,
        }))
    }

    /// Handles except do.
    fn except_do(&self, node: tree_sitter::Node<'_>) -> Result<TryFallback, ParseError> {
        Ok(TryFallback!(Except {
            except_token: self.migrate(self.required_kind(node, "except")?)?,
            exception: self.migrate(self.required_field(node, "exception")?)?,
            do_token: self.migrate(self.required_kind(node, "do")?)?,
            value: Box::new(self.expr(self.required_expression_child(node)?)?),
        }))
    }

    /// Handles quote expression.
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
        let quoted = self.required_field(node, "token")?;
        let symbol = Symbol::new(self.text(quoted), self.span(quoted));
        Ok(match token.kind() {
            "symbol" => Expr!(Quote::Symbol {
                specifier: self.migrate(token)?,
                symbol,
            }),
            "local" => Expr!(Quote::Local {
                specifier: self.migrate(token)?,
                symbol,
            }),
            "global" => Expr!(Quote::Global {
                specifier: self.migrate(token)?,
                symbol,
            }),
            "threadVariable" => Expr!(Quote::ThreadVariable {
                specifier: self.migrate(token)?,
                symbol,
            }),
            "threadLocal" => Expr!(Quote::ThreadLocal {
                specifier: self.migrate(token)?,
                symbol,
            }),
            kind => return Err(self.unsupported(kind, token)),
        })
    }

    /// Handles debug clause.
    fn debug_clause(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let keyword = self.required_field(node, "keyword")?;
        let value = self
            .first_expression_child(node)
            .map(|child| self.expr(child))
            .transpose()?;
        Ok(match keyword.kind() {
            "elapsedTime" => Expr!(Profile::ElapsedTime {
                modifier: self.migrate(keyword)?,
                value: Box::new(value.ok_or_else(|| self.unsupported("profiling operand", node))?,),
            }),
            "elapsedTiming" => Expr!(Profile::ElapsedTiming {
                modifier: self.migrate(keyword)?,
                value: Box::new(value.ok_or_else(|| self.unsupported("profiling operand", node))?,),
            }),
            "profile" => Expr!(Profile::Profile {
                modifier: self.migrate(keyword)?,
                value: Box::new(value.ok_or_else(|| self.unsupported("profiling operand", node))?,),
            }),
            "time" => Expr!(Profile::Time {
                modifier: self.migrate(keyword)?,
                value: Box::new(value.ok_or_else(|| self.unsupported("profiling operand", node))?,),
            }),
            "timing" => Expr!(Profile::Timing {
                modifier: self.migrate(keyword)?,
                value: Box::new(value.ok_or_else(|| self.unsupported("profiling operand", node))?,),
            }),
            "TEST" => Expr!(Debug::Test {
                keyword: self.migrate(keyword)?,
                value: value.map(Box::new),
            }),
            "step" => Expr!(Debug::Step {
                keyword: self.migrate(keyword)?,
                value: value.map(Box::new),
            }),
            "finish" => Expr!(Debug::Finish {
                keyword: self.migrate(keyword)?,
                value: value.map(Box::new),
            }),
            "shield" => Expr!(Debug::Shield {
                keyword: self.migrate(keyword)?,
                value: value.map(Box::new),
            }),
            "breakpoint" => Expr!(Debug::Breakpoint {
                keyword: self.migrate(keyword)?,
                value: value.map(Box::new),
            }),
            kind => return Err(self.unsupported(kind, keyword)),
        })
    }

    /// Handles migrate.
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

    /// Handles delimiter spans.
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

    /// Handles children.
    fn children<'tree>(&self, node: tree_sitter::Node<'tree>) -> Vec<tree_sitter::Node<'tree>> {
        (0..node.child_count())
            .filter_map(|index| node.child(u32::try_from(index).expect("child count fits u32")))
            .collect()
    }

    /// Handles semantic children.
    fn semantic_children<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
    ) -> Vec<tree_sitter::Node<'tree>> {
        self.children(node)
            .into_iter()
            .filter(|child| child.is_named() && !child.is_extra())
            .collect()
    }

    /// Handles only semantic child.
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

    /// Handles first expression child.
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

    /// Handles required expression child.
    fn required_expression_child<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
    ) -> Result<tree_sitter::Node<'tree>, ParseError> {
        self.first_expression_child(node)
            .ok_or_else(|| self.unsupported("expression child", node))
    }

    /// Handles child by field.
    fn child_by_field<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
        field: &str,
    ) -> Option<tree_sitter::Node<'tree>> {
        node.child_by_field_name(field)
    }

    /// Handles required field.
    fn required_field<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
        field: &str,
    ) -> Result<tree_sitter::Node<'tree>, ParseError> {
        self.child_by_field(node, field)
            .ok_or_else(|| self.unsupported(field, node))
    }

    /// Handles optional kind.
    fn optional_kind<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
        kind: &str,
    ) -> Option<tree_sitter::Node<'tree>> {
        self.children(node)
            .into_iter()
            .find(|child| !child.is_extra() && child.kind() == kind)
    }

    /// Handles required kind.
    fn required_kind<'tree>(
        &self,
        node: tree_sitter::Node<'tree>,
        kind: &str,
    ) -> Result<tree_sitter::Node<'tree>, ParseError> {
        self.optional_kind(node, kind)
            .ok_or_else(|| self.unsupported(kind, node))
    }

    /// Returns the source text.
    fn text(&self, node: tree_sitter::Node<'_>) -> Cow<'_, str> {
        let range = self
            .span(node)
            .bounds()
            .map_or_else(|| node.byte_range(), |(start, end)| start..end);
        String::from_utf8_lossy(&self.source[range])
    }

    /// Returns the source span.
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

    /// Handles unsupported.
    fn unsupported(&self, kind: impl Into<String>, node: tree_sitter::Node<'_>) -> ParseError {
        self.unsupported_span(kind, self.span(node))
    }

    /// Handles unsupported span.
    fn unsupported_span(&self, kind: impl Into<String>, span: Span) -> ParseError {
        ParseError::UnsupportedNode {
            kind: kind.into(),
            span,
        }
    }
}

#[derive(Clone)]
/// Internal state for CoreAlias.
struct CoreAlias {
    /// The full.
    full: Range<usize>,
    /// The bare.
    bare: Range<usize>,
}

/// Handles renamed source.
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

/// Internal alias for RecoveredSource.
type RecoveredSource<'source> = (Cow<'source, str>, Vec<CoreAlias>, Vec<LexError>);

/// Handles renamed source recovering.
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

/// Collects missing closers.
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

/// Collects core aliases.
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
    /// The node.
    node: tree_sitter::Node<'tree>,
    /// The source.
    source: &'source [u8],
    /// The span.
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

    /// Handles with span.
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
