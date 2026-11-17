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

use crate::native::{function_params, is_assignment_target};
use crate::nodes::*;
use crate::{
    CSTNodeClassLabel, CellStream, CellStreamError, CommentKind, DelimiterKind, DelimiterToken,
    DoubleSpan, ExternalCstChild, ExternalCstNode, LexError, Migrate, MigrateError, Parser,
    Punctuated, Recovered, Source, SourceFile, Span, Spanned, TextEdit, TokenStream, TokenTree,
    Tokenize, lex_str, lex_str_recovering,
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
    /// source, apart from recoverable missing group and string-literal closers.
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
        let (view, respellings, diagnostics) = renamed_source_recovering(source.text())?;
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
        let syntax = Lowerer::new(Arc::clone(&source), source.text(), respellings)
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
    let (view, respellings, diagnostics) = renamed_source_recovering(source_text)?;
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
    let syntax = Lowerer::new(Arc::clone(&source), source_text, respellings)
        .source_file(tree.root_node())?;
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
        let (renamed, respellings) = renamed_source(source_text)?;
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
        Lowerer::new(Arc::clone(&source), source_text, respellings).source_file(tree.root_node())
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
        let (renamed, respellings, diagnostics) = renamed_source_recovering(source_text)?;
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
        let syntax = Lowerer::new(Arc::clone(&source), source_text, respellings)
            .source_file(tree.root_node())?;
        Ok(Recovered::new(syntax, diagnostics))
    }
}

impl Parser for TreeSitterParser {
    type Error = ParseError;

    fn parse_cells(&mut self, cells: CellStream) -> Result<SourceFile, Self::Error> {
        match cells.complete_source() {
            Some(source) => self.parse_source_in(source),
            None => self.parse_source_in(Source::new(cells.to_string())),
        }
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

/// Parses incomplete source while retaining unclosed groups and string literals.
///
/// # Errors
///
/// Returns [`ParseError`] for failures other than missing group or
/// string-literal closers, or when the recovered Tree-sitter CST cannot be
/// lowered.
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
    respellings: Vec<Respelling>,
}

impl<'source> Lowerer<'source> {
    /// Creates a new value.
    fn new(source_owner: Arc<Source>, source: &'source str, respellings: Vec<Respelling>) -> Self {
        Self {
            source: source.as_bytes(),
            source_span: Span::ranged(source_owner, 0..source.len()),
            respellings,
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
        Ok(SourceFile::new(
            Token![BOF](self.source_span.at(0)),
            cells,
            Token![EOF](self.source_span.at(self.source_span.source().text().len())),
        ))
    }

    /// Handles cell.
    fn cell(&self, node: tree_sitter::Node<'_>, eoc_offset: usize) -> Result<Cell, ParseError> {
        let (elems, semicolon) = if node.kind() == "muted" {
            let semicolon = self.required_kind(node, ";")?;
            (
                self.elements(self.children(node))?,
                Some(self.migrate(semicolon)?),
            )
        } else {
            let child = self.only_semantic_child(node)?;
            let elems = if child.kind() == "naked_sequence" {
                Elements::Pack(self.element_pack(self.children(child))?)
            } else {
                Elements::Single(Box::new(self.expr(child)?))
            };
            (elems, None)
        };
        Ok(Cell::new(
            elems,
            semicolon,
            Token![EOC](self.source_span.at(eoc_offset)),
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

    /// Lowers a comma-separated pack of at least two elements.
    fn element_pack(&self, nodes: Vec<tree_sitter::Node<'_>>) -> Result<ElementsPack, ParseError> {
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
                "empty_component" => values.push(Element::Empty(Empty::new("", self.span(node)))),
                ";" | "(" | ")" | "[" | "]" | "{" | "}" | "<|" | "|>" | "muted" => {}
                _ if node.is_named() => values.push(Element::Expr(Box::new(self.expr(node)?))),
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
            .collect::<Vec<(Token![,], Element)>>();
        let mut rest = rest.into_iter();
        let Some((punctuation, second)) = rest.next() else {
            return Err(self.unsupported_span("expression pack without a comma", pack_span));
        };
        let mut pack = Punctuated::new(first, punctuation, second);
        for (punctuation, value) in rest {
            pack.push(punctuation, value);
        }
        Ok(ElementsPack::new(pack))
    }

    /// Lowers a single expression or a comma-separated pack.
    fn elements(&self, nodes: Vec<tree_sitter::Node<'_>>) -> Result<Elements, ParseError> {
        if nodes.iter().any(|node| node.kind() == ",") {
            return self.element_pack(nodes).map(Elements::Pack);
        }
        let mut expressions = nodes
            .into_iter()
            .filter(|node| !node.is_extra() && node.is_named() && node.kind() != "muted");
        let Some(expression) = expressions.next() else {
            return Err(self.unsupported_span("empty elements", self.source_span.clone()));
        };
        if expressions.next().is_some() {
            return Err(self.unsupported("unseparated elements", expression));
        }
        Ok(Elements::Single(Box::new(self.expr(expression)?)))
    }

    /// Handles expr.
    fn expr(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        match node.kind() {
            "symbol" => Ok(self.migrate::<Symbol>(node)?.into()),
            "integer_literal" | "float_literal" | "string_literal" | "raw_string_literal" => {
                self.literal(node).map(Expr::Literal)
            }
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
            "evaluated_assignment" => Ok(Expr::from(ExprAssign {
                left: Box::new(self.expr(self.required_field(node, "left")?)?),
                op: AssignOperator::Evaluated(
                    self.migrate(self.required_field(node, "operator")?)?,
                ),
                right: Box::new(self.expr(self.required_field(node, "right")?)?),
            })),
            "option" => Ok(Expr::from(ExprOption {
                key: Box::new(self.expr(self.required_field(node, "left")?)?),
                fat_arrow_token: self.migrate(self.required_field(node, "operator")?)?,
                value: Box::new(self.expr(self.required_field(node, "right")?)?),
            })),
            "if_statement" => self.if_statement(node),
            "for_loop" => self.for_loop(node),
            "while_loop" => self.while_loop(node),
            "try_statement" => self.try_statement(node),
            "quote_expression" => self.quote_expression(node),
            "new_statement" => self.new_statement(node),
            "break_statement" => Ok(Expr::from(ExprBreak {
                break_token: self.migrate(self.required_kind(node, "break")?)?,
                expr: self
                    .first_expression_child(node)
                    .map(|child| self.expr(child).map(Box::new))
                    .transpose()?,
            })),
            "continue_statement" => Ok(Expr::from(ExprContinue {
                continue_token: self.migrate(self.required_kind(node, "continue")?)?,
                expr: self
                    .first_expression_child(node)
                    .map(|child| self.expr(child).map(Box::new))
                    .transpose()?,
            })),
            "return_statement" => Ok(Expr::from(ExprReturn {
                return_token: self.migrate(self.required_kind(node, "return")?)?,
                expr: self
                    .first_expression_child(node)
                    .map(|child| self.expr(child).map(Box::new))
                    .transpose()?,
            })),
            "catch_statement" => Ok(Expr::from(ExprCatch {
                catch_token: self.migrate(self.required_kind(node, "catch")?)?,
                expr: Box::new(self.expr(self.required_expression_child(node)?)?),
            })),
            "throw_statement" => Ok(Expr::from(ExprThrow {
                throw_token: self.migrate(self.required_kind(node, "throw")?)?,
                expr: Box::new(self.expr(self.required_expression_child(node)?)?),
            })),
            "trap_statement" => Ok(Expr::from(ExprTrap {
                trap_token: self.migrate(self.required_kind(node, "trap")?)?,
                expr: Box::new(self.expr(self.required_expression_child(node)?)?),
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
            .map(|child| self.stmt(child))
            .collect::<Result<Vec<_>, _>>()?;

        if node.kind() == "parenthesized_expression" {
            let value = self
                .semantic_children(node)
                .into_iter()
                .find(|child| child.kind() != "muted")
                .map(|child| self.expr(child))
                .transpose()?;
            let mut paren = ExprParen::new(statements, value.map(Box::new));
            paren.delimiter = <Token![()] as DelimiterToken>::new(spans);
            return Ok(paren.into());
        }

        let has_elements = self.children(node).into_iter().any(|child| {
            !child.is_extra()
                && (child.kind() == ","
                    || child.kind() == "empty_component"
                    || (child.is_named() && child.kind() != "muted"))
        });
        let elements = has_elements
            .then(|| self.elements(self.children(node)))
            .transpose()?;
        Ok(match node.kind() {
            "array" => {
                let mut value = ExprArray::new(statements, elements);
                value.delimiter = <Token![[]] as DelimiterToken>::new(spans);
                value.into()
            }
            "list" => {
                let mut value = ExprList::new(statements, elements);
                value.delimiter = <Token![{}] as DelimiterToken>::new(spans);
                value.into()
            }
            "angle_bar_list" => {
                let mut value = ExprAngleBar::new(statements, elements);
                value.delimiter = <Token![<||>] as DelimiterToken>::new(spans);
                value.into()
            }
            "sequence" => {
                let mut value = ExprSequence::new(statements, elements);
                value.delimiter = <Token![()] as DelimiterToken>::new(spans);
                value.into()
            }
            kind => return Err(self.unsupported(kind, node)),
        })
    }

    /// Lowers elements terminated by `;`.
    fn stmt(&self, node: tree_sitter::Node<'_>) -> Result<Stmt, ParseError> {
        Ok(Stmt::new(
            self.elements(self.children(node))?,
            self.migrate(self.required_kind(node, ";")?)?,
        ))
    }

    /// Handles string literal.
    /// A hexadecimal integer with a `p` exponent is a float.
    fn literal(&self, node: tree_sitter::Node<'_>) -> Result<Literal, ParseError> {
        Ok(match node.kind() {
            "integer_literal" if self.text(node).contains('p') => {
                FloatLiteral::new(self.text(node), self.span(node)).into()
            }
            "integer_literal" => self.migrate::<IntegerLiteral>(node)?.into(),
            "float_literal" => self.migrate::<FloatLiteral>(node)?.into(),
            "string_literal" => self.string_literal(node)?.into(),
            _ => self.raw_string_literal(node)?.into(),
        })
    }

    fn string_literal(&self, node: tree_sitter::Node<'_>) -> Result<StringLiteral, ParseError> {
        let elements = self
            .children(node)
            .into_iter()
            .filter(|child| child.kind() != "\"")
            .filter(|child| !child.is_extra())
            .map(|child| match child.kind() {
                "escape_sequence" => self.migrate(child).map(StringFragment::Escape),
                "string_content" => self.migrate(child).map(StringFragment::Content),
                _ => Ok(StringFragment::Content(StringContent::new(
                    self.text(child),
                    self.span(child),
                ))),
            })
            .collect::<Result<Vec<_>, ParseError>>()?;
        Ok(StringLiteral::new(elements, self.span(node)))
    }

    /// Handles raw string literal.
    fn raw_string_literal(
        &self,
        node: tree_sitter::Node<'_>,
    ) -> Result<RawStringLiteral, ParseError> {
        let elements = self
            .children(node)
            .into_iter()
            .filter(|child| child.kind() != "///")
            .filter(|child| !child.is_extra())
            .map(|child| match child.kind() {
                "escape_sequence" => self.migrate(child).map(RawStringFragment::Escape),
                "raw_string_content" => self.migrate(child).map(RawStringFragment::Content),
                kind => Err(self.unsupported(kind, child)),
            })
            .collect::<Result<Vec<_>, ParseError>>()?;
        Ok(RawStringLiteral::new(elements, self.span(node)))
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
        let (left, right) = (Box::new(left), Box::new(right));
        Ok(match self.text(operator).as_ref() {
            spelling if GENERATED_AUGMENTED_OPERATOR_SPELLINGS.contains(&spelling) => {
                Expr::from(ExprAssign {
                    left,
                    op: AssignOperator::Augmented(self.migrate(operator)?),
                    right,
                })
            }
            spelling @ ("." | ".?") => {
                let op = if spelling == "." {
                    FieldOperator::Dot(self.migrate(operator)?)
                } else {
                    FieldOperator::OptionalDot(self.migrate(operator)?)
                };
                let Expr::Symbol(member) = *right else {
                    return Err(self.unsupported("member that is not a symbol", node));
                };
                Expr::from(ExprField {
                    base: left,
                    op,
                    member,
                })
            }
            _ => Expr::from(ExprBinary {
                left,
                op: self.migrate(operator)?,
                right,
            }),
        })
    }

    /// Handles prefix expr.
    fn prefix_expr(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        Ok(Expr::from(ExprPrefix {
            op: self.migrate(self.required_field(node, "operator")?)?,
            expr: Box::new(self.expr(self.required_field(node, "operand")?)?),
        }))
    }

    /// Handles postfix expr.
    fn postfix_expr(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        Ok(Expr::from(ExprPostfix {
            expr: Box::new(self.expr(self.required_field(node, "operand")?)?),
            op: self.migrate(self.required_field(node, "operator")?)?,
        }))
    }

    /// Lowers a function.
    fn lambda_expression(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let params = function_params(self.expr(self.required_field(node, "parameters")?)?)
            .ok_or_else(|| self.unsupported("function parameters", node))?;
        Ok(Expr::from(ExprFunction {
            params,
            arrow_token: self.migrate(self.required_field(node, "operator")?)?,
            body: Box::new(self.expr(self.required_field(node, "body")?)?),
        }))
    }

    /// Lowers an assignment or an installation.
    fn assignment(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let left_field = match node.kind() {
            "structured_binding" | "local_structured_binding" => "binding_pack",
            _ => "left",
        };
        let left = self.expr(self.required_field(node, left_field)?)?;
        let operator = self.required_field(node, "operator")?;
        let op = match node.kind() {
            "assignment" | "binary_assignment" | "prefix_assignment" | "postfix_assignment"
            | "structured_binding" => AssignOperator::Global(self.migrate(operator)?),
            "local_assignment"
            | "binary_installation"
            | "prefix_installation"
            | "postfix_installation"
            | "local_structured_binding" => AssignOperator::Local(self.migrate(operator)?),
            kind => return Err(self.unsupported(kind, node)),
        };
        if !is_assignment_target(&left, matches!(op, AssignOperator::Local(_))) {
            return Err(self.unsupported("assignment target", node));
        }
        Ok(Expr::from(ExprAssign {
            left: Box::new(left),
            op,
            right: Box::new(self.expr(self.required_field(node, "right")?)?),
        }))
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
        Ok(Expr::from(ExprIf {
            if_token: self.migrate(self.required_kind(node, "if")?)?,
            cond: Box::new(self.expr(self.required_field(node, "condition")?)?),
            then_clause: self.then_clause(self.required_kind(node, "then_clause")?)?,
            else_clause: self
                .optional_kind(node, "else_clause")
                .map(|child| self.else_clause(child))
                .transpose()?,
        }))
    }

    /// Handles loop body.
    fn loop_body(&self, node: tree_sitter::Node<'_>) -> Result<LoopBody, ParseError> {
        let list_clause = self
            .optional_kind(node, "list")
            .map(|token| {
                Ok::<_, ParseError>(ListClause::new(
                    self.migrate(token)?,
                    Box::new(self.expr(self.required_field(node, "listed_value")?)?),
                ))
            })
            .transpose()?;
        let do_clause = self
            .optional_kind(node, "do")
            .map(|token| {
                Ok::<_, ParseError>(DoClause::new(
                    self.migrate(token)?,
                    Box::new(self.expr(self.required_field(node, "ignored_value")?)?),
                ))
            })
            .transpose()?;
        Ok(LoopBody::new(list_clause, do_clause))
    }

    /// Lowers a `for` loop.
    fn for_loop(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let iter = self
            .optional_kind(node, "iteration_range")
            .map(|range| self.for_iter(range))
            .transpose()?;
        let when_clause = self
            .optional_kind(node, "when")
            .map(|token| {
                Ok::<_, ParseError>(WhenClause::new(
                    self.migrate(token)?,
                    Box::new(self.expr(self.required_field(node, "filter")?)?),
                ))
            })
            .transpose()?;
        Ok(Expr::from(ExprFor {
            for_token: self.migrate(self.required_kind(node, "for")?)?,
            var: self.migrate(self.required_field(node, "variable")?)?,
            iter,
            when_clause,
            body: self.loop_body(self.required_kind(node, "loop_body")?)?,
        }))
    }

    /// Lowers what a `for` loop iterates over.
    fn for_iter(&self, node: tree_sitter::Node<'_>) -> Result<ForIter, ParseError> {
        if let Some(token) = self.optional_kind(node, "in") {
            return Ok(ForIter::In(InClause::new(
                self.migrate(token)?,
                Box::new(self.expr(self.required_field(node, "iterated_collection")?)?),
            )));
        }
        let from_clause = self
            .optional_kind(node, "from")
            .map(|token| {
                Ok::<_, ParseError>(FromClause::new(
                    self.migrate(token)?,
                    Box::new(self.expr(self.required_field(node, "range_start")?)?),
                ))
            })
            .transpose()?;
        let to_clause = self
            .optional_kind(node, "to")
            .map(|token| {
                Ok::<_, ParseError>(ToClause::new(
                    self.migrate(token)?,
                    Box::new(self.expr(self.required_field(node, "range_end")?)?),
                ))
            })
            .transpose()?;
        Ok(ForIter::Range(ForRange::new(from_clause, to_clause)))
    }

    /// Handles while loop.
    fn while_loop(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        Ok(Expr::from(ExprWhile {
            while_token: self.migrate(self.required_kind(node, "while")?)?,
            cond: Box::new(self.expr(self.required_field(node, "condition")?)?),
            body: self.loop_body(self.required_kind(node, "loop_body")?)?,
        }))
    }

    /// Handles new statement.
    fn new_statement(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let mut installation = None;
        let class =
            self.new_statement_element(self.required_field(node, "class")?, &mut installation)?;
        let of_clause = self
            .optional_kind(node, "of")
            .map(|token| {
                Ok::<_, ParseError>(OfClause::new(
                    self.migrate(token)?,
                    Box::new(self.new_statement_element(
                        self.required_field(node, "parent")?,
                        &mut installation,
                    )?),
                ))
            })
            .transpose()?;
        let from_clause = self
            .optional_kind(node, "from")
            .map(|token| {
                Ok::<_, ParseError>(FromClause::new(
                    self.migrate(token)?,
                    Box::new(self.new_statement_element(
                        self.required_field(node, "instance")?,
                        &mut installation,
                    )?),
                ))
            })
            .transpose()?;
        let new = Expr::from(ExprNew {
            new_token: self.migrate(self.required_kind(node, "new")?)?,
            class: Box::new(class),
            of_clause,
            from_clause,
        });
        Ok(match installation {
            Some((operator, function)) => Expr::from(ExprAssign {
                left: Box::new(new),
                op: AssignOperator::Local(operator),
                right: Box::new(function),
            }),
            None => new,
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
        let then_clause = self
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
        Ok(Expr::from(ExprTry {
            try_token: self.migrate(self.required_kind(node, "try")?)?,
            expr: Box::new(self.expr(value)?),
            then_clause,
            fallback,
        }))
    }

    /// Handles except do.
    fn except_do(&self, node: tree_sitter::Node<'_>) -> Result<TryFallback, ParseError> {
        Ok(TryFallback::Except(ExceptClause::new(
            self.migrate(self.required_kind(node, "except")?)?,
            self.migrate(self.required_field(node, "exception")?)?,
            self.migrate(self.required_kind(node, "do")?)?,
            Box::new(self.expr(self.required_expression_child(node)?)?),
        )))
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
        let keyword = match token.kind() {
            "symbol" => QuoteKeyword::Symbol(self.migrate(token)?),
            "local" => QuoteKeyword::Local(self.migrate(token)?),
            "global" => QuoteKeyword::Global(self.migrate(token)?),
            "threadLocal" | "threadVariable" => QuoteKeyword::ThreadLocal(self.migrate(token)?),
            kind => return Err(self.unsupported(kind, token)),
        };
        Ok(Expr::from(ExprQuote { keyword, symbol }))
    }

    /// Handles debug clause.
    fn debug_clause(&self, node: tree_sitter::Node<'_>) -> Result<Expr, ParseError> {
        let keyword = self.required_field(node, "keyword")?;
        let value = self
            .first_expression_child(node)
            .map(|child| self.expr(child))
            .transpose()?;
        let timing_keyword = match keyword.kind() {
            "elapsedTime" => Some(TimingKeyword::ElapsedTime(self.migrate(keyword)?)),
            "elapsedTiming" => Some(TimingKeyword::ElapsedTiming(self.migrate(keyword)?)),
            "profile" => Some(TimingKeyword::Profile(self.migrate(keyword)?)),
            "time" => Some(TimingKeyword::Time(self.migrate(keyword)?)),
            "timing" => Some(TimingKeyword::Timing(self.migrate(keyword)?)),
            _ => None,
        };
        if let Some(keyword) = timing_keyword {
            let value = value.ok_or_else(|| self.unsupported("timing operand", node))?;
            return Ok(Expr::from(ExprTiming {
                keyword,
                expr: Box::new(value),
            }));
        }
        let keyword = match keyword.kind() {
            "TEST" => DebugKeyword::Test(self.migrate(keyword)?),
            "step" => DebugKeyword::Step(self.migrate(keyword)?),
            "finish" => DebugKeyword::Finish(self.migrate(keyword)?),
            "shield" => DebugKeyword::Shield(self.migrate(keyword)?),
            "breakpoint" => DebugKeyword::Breakpoint(self.migrate(keyword)?),
            kind => return Err(self.unsupported(kind, keyword)),
        };
        Ok(Expr::from(ExprDebug {
            keyword,
            expr: value.map(Box::new),
        }))
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
        self.respellings
            .iter()
            .find(|respelling| respelling.read == range)
            .map_or_else(
                || {
                    self.source_span.subspan(
                        node.start_byte().min(self.source.len()),
                        node.end_byte().min(self.source.len()),
                    )
                },
                |respelling| {
                    self.source_span
                        .subspan(respelling.written.start, respelling.written.end)
                },
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
/// A token the grammar reads in its declared spelling, at the same offsets.
///
/// The declared spelling goes after one blank, and blanks pad the rest:
/// `Core$if` is read as ` if    `, `≠` as ` !=`, and `threadVariable` as
/// ` threadLocal  `.
struct Respelling {
    written: Range<usize>,
    read: Range<usize>,
    declared: &'static str,
}

impl Respelling {
    /// Respells the token at `span`, unless a word would have no blank
    /// between it and a word beside it: `a∧b` cannot be read as `aandb`.
    fn new(source: &[u8], span: Span, declared: &'static str) -> Result<Self, ParseError> {
        let (start, end) = span.bounds().expect("lexed tokens lie in the source");
        let written = start..end;
        let padding = written.len() - declared.len();
        let read = written.start + padding.min(1)..written.start + padding.min(1) + declared.len();
        let touches_word = |byte: Option<&u8>| {
            byte.is_some_and(|byte| byte.is_ascii_alphanumeric() || !byte.is_ascii())
        };
        let glued = declared.starts_with(|c: char| c.is_ascii_alphabetic())
            && (read.start == written.start && touches_word(source[..written.start].last())
                || read.end == written.end && touches_word(source.get(written.end)));
        if glued {
            return Err(ParseError::UnsupportedNode {
                kind: format!("{} beside a word", span.text().unwrap_or(declared)),
                span,
            });
        }
        Ok(Self {
            written,
            read,
            declared,
        })
    }

    fn apply(&self, view: &mut [u8]) {
        view[self.written.clone()].fill(b' ');
        view[self.read.clone()].copy_from_slice(self.declared.as_bytes());
    }
}

/// Handles renamed source.
fn renamed_source(source: &str) -> Result<(Cow<'_, str>, Vec<Respelling>), ParseError> {
    let tokens = lex_str(source)?;
    let mut respellings = Vec::new();
    collect_respellings(&tokens, source.as_bytes(), &mut respellings)?;
    if respellings.is_empty() {
        return Ok((Cow::Borrowed(source), respellings));
    }

    let mut renamed = source.as_bytes().to_vec();
    respellings
        .iter()
        .for_each(|respelling| respelling.apply(&mut renamed));
    Ok((
        Cow::Owned(String::from_utf8(renamed).expect("respelling writes whole tokens in ASCII")),
        respellings,
    ))
}

/// Internal alias for RecoveredSource.
type RecoveredSource<'source> = (Cow<'source, str>, Vec<Respelling>, Vec<LexError>);

/// Handles renamed source recovering.
fn renamed_source_recovering(source: &str) -> Result<RecoveredSource<'_>, ParseError> {
    let recovered = lex_str_recovering(source)?;
    let (tokens, diagnostics) = recovered.into_parts();
    let mut respellings = Vec::new();
    collect_respellings(&tokens, source.as_bytes(), &mut respellings)?;
    let mut missing = Vec::new();
    collect_missing_closers(&tokens, &mut missing);
    let missing_literal = diagnostics
        .iter()
        .find_map(|diagnostic| match diagnostic.kind() {
            crate::LexErrorKind::UnterminatedString => Some("\""),
            crate::LexErrorKind::UnterminatedRawString => Some("///"),
            _ => None,
        });

    if respellings.is_empty() && missing.is_empty() && missing_literal.is_none() {
        return Ok((Cow::Borrowed(source), respellings, diagnostics));
    }

    let mut renamed = source.as_bytes().to_vec();
    respellings
        .iter()
        .for_each(|respelling| respelling.apply(&mut renamed));
    // A line comment runs to the end of its line, so appended closers must
    // start on a new one to remain syntax.
    if !missing.is_empty() && ends_in_line_comment(&tokens, source.len()) {
        renamed.push(b'\n');
    }
    if let Some(closer) = missing_literal {
        renamed.extend_from_slice(closer.as_bytes());
    }
    for kind in missing {
        renamed.extend_from_slice(kind.closing().as_bytes());
    }
    Ok((
        Cow::Owned(String::from_utf8(renamed).expect("ASCII recovery preserves UTF-8")),
        respellings,
        diagnostics,
    ))
}

/// Reports whether the source ends inside a line comment.
fn ends_in_line_comment(tokens: &TokenStream, source_len: usize) -> bool {
    tokens.source().is_some_and(|source| {
        source
            .comment_ranges()
            .last()
            .is_some_and(|(kind, range)| *kind == CommentKind::Line && range.end == source_len)
    })
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

/// Collects the tokens the grammar reads in their declared spelling: the
/// `Core$` aliases and M2's synonyms, which the grammar does not know.
///
/// The alias of a word that only continues a construct is a syntax error, as
/// in M2; respelling it would make `if x Core$then y` parse.
fn collect_respellings(
    tokens: &TokenStream,
    source: &[u8],
    respellings: &mut Vec<Respelling>,
) -> Result<(), ParseError> {
    for token in tokens.iter() {
        if let TokenTree::Group(group) = token {
            collect_respellings(group.stream(), source, respellings)?;
            continue;
        }
        let Some(spelling) = token.spelling() else {
            continue;
        };
        if crate::lexer::is_continuation_alias(spelling) {
            return Err(ParseError::InvalidSyntax(token.expect_span()));
        }
        if let Some(declared) = crate::lexer::declared_spelling(spelling) {
            respellings.push(Respelling::new(source, token.expect_span(), declared)?);
        }
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn expressions(source: &str) -> [Expr; 2] {
        [
            parse_file(source).unwrap(),
            crate::parse_native(source).unwrap(),
        ]
        .map(|mut file| {
            assert_eq!(file.cells.len(), 1, "source: {source}");
            let Elements::Single(value) = file.cells.pop().unwrap().elems else {
                panic!("expected one expression for {source}");
            };
            *value
        })
    }

    #[test]
    fn empty_parentheses_are_empty_sequences_in_both_backends() {
        for source in ["()", "( )", "(-* comment *-)", "(-- comment\n)"] {
            for expression in expressions(source) {
                let Expr::Sequence(sequence) = expression else {
                    panic!("expected an empty sequence for {source}, found {expression:?}");
                };
                assert!(sequence.stmts.is_empty(), "source: {source}");
                assert!(sequence.elems.is_none(), "source: {source}");
                assert_eq!(sequence.span().text(), Some(source));
            }
        }
    }

    #[test]
    fn parenthesized_values_and_statements_are_parenthesized_expressions() {
        for source in ["(x)", "(x;)", "(x; y)"] {
            for expression in expressions(source) {
                assert!(matches!(expression, Expr::Paren(_)), "source: {source}");
            }
        }
    }
}
