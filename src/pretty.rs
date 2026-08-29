//! Terminal-oriented inspection of source, raw tokens, and typed syntax trees.
//!
//! [`PrettyReport`] combines the three useful views of a parsed file. The
//! renderer deliberately owns only presentation data; the generated typed CST
//! remains the source of truth and implements [`PrettyTree`] directly.

use std::fmt::{Display, Formatter, Result as FmtResult, Write as FmtWrite};
use std::io::{self, IsTerminal as _, Write as IoWrite};

use crate::{
    CellTokenStream, CommentTrivia, DelimiterKind, DoubleSpan, Punctuated, Span, Spanned,
    TokenStream, TokenTree, Trivia, TriviaKind, token_stream::delim::Delimiter,
};

/// The RESET constant.
const RESET: &str = "\u{1b}[0m";
/// The BOLD CYAN constant.
const BOLD_CYAN: &str = "\u{1b}[1;36m";
/// The BRIGHT YELLOW constant.
const BRIGHT_YELLOW: &str = "\u{1b}[93m";
/// The BRIGHT BLACK constant.
const BRIGHT_BLACK: &str = "\u{1b}[90m";
/// The RED constant.
const RED: &str = "\u{1b}[31m";
/// The GREEN constant.
const GREEN: &str = "\u{1b}[32m";
/// The YELLOW constant.
const YELLOW: &str = "\u{1b}[33m";
/// The BLUE constant.
const BLUE: &str = "\u{1b}[34m";
/// The MAGENTA constant.
const MAGENTA: &str = "\u{1b}[35m";
/// The CYAN constant.
const CYAN: &str = "\u{1b}[36m";
/// The WHITE constant.
const WHITE: &str = "\u{1b}[37m";
/// The NON TERMINAL BLUE constant.
const NON_TERMINAL_BLUE: &str = BLUE;
/// The DIM constant.
const DIM: &str = "\u{1b}[2m";

/// Converts typed syntax into the structural presentation used by the pretty
/// printer.
///
/// Implementations for generated nodes are emitted from `syntax!`, so
/// field names, punctuation, and newly added concrete node types appear in the
/// tree without a handwritten registry. Semantic category enums remain visible
/// because they define visitor dispatch, while generic containers are
/// transparent storage details rather than CST nodes.
pub trait PrettyTree {
    /// Builds the presentation node for this syntax value.
    fn pretty_tree(&self) -> PrettyNode;
}

/// One owned node in a syntax presentation tree.
///
/// The structure is independent of terminal layout. External renderers can
/// recursively inspect [`Self::children`], use [`Self::role`] and
/// [`PrettyDetail::role`] for styling, and apply their own width and glyph
/// policies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrettyNode {
    /// The name.
    name: String,
    /// The detail.
    detail: Option<PrettyDetail>,
    /// The span.
    span: Option<Span>,
    /// The role.
    role: PrettyNodeRole,
    /// The transparent.
    transparent: bool,
    /// The children.
    children: Vec<PrettyEdge>,
}

/// Additional text displayed alongside a [`PrettyNode`].
///
/// Token nodes use details for their source spelling, while virtual syntax
/// elements use them for their conventional display glyph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrettyDetail {
    /// The text.
    text: String,
    /// The role.
    role: PrettyDetailRole,
}

/// The semantic role of a [`PrettyDetail`].
///
/// Renderers can use this role to select their own highlight group without
/// depending on the built-in ANSI palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrettyDetailRole {
    /// Source text belonging to a lexical token.
    Text,
    /// A conventional glyph representing a virtual syntax element.
    Glyph,
}

/// The semantic role of a [`PrettyNode`].
///
/// This is presentation metadata rather than a replacement for the generated
/// typed syntax hierarchy. It lets external renderers distinguish structural
/// nodes, lexical leaves, delimiter boundaries, collection fragments, and
/// absent optional values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrettyNodeRole {
    /// A concrete node from the generated typed syntax hierarchy.
    Syntax,
    /// A semantic category through which visitor specialization dispatches.
    Category,
    /// A lexical token or virtual syntax leaf.
    Token,
    /// One boundary projected from a paired delimiter atom.
    Delimiter,
    /// A transparent syntax container such as a list or delimiter pair.
    Collection,
    /// An optional syntax value that is not present.
    Absent,
}

/// A labeled parent-to-child edge in a [`PrettyNode`] tree.
///
/// The label is normally the generated field name or the index of an element
/// in a collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrettyEdge {
    /// The label.
    label: String,
    /// The node.
    node: PrettyNode,
}

impl PrettyDetail {
    /// Returns the text carried by this detail.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the semantic role of this detail.
    pub fn role(&self) -> PrettyDetailRole {
        self.role
    }
}

impl PrettyEdge {
    /// Returns the field name or collection index labeling this edge.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the child presentation node.
    pub fn node(&self) -> &PrettyNode {
        &self.node
    }
}

impl PrettyNode {
    /// Returns the display name of this node.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the optional token spelling or virtual-element glyph.
    pub fn detail(&self) -> Option<&PrettyDetail> {
        self.detail.as_ref()
    }

    /// Returns the source span represented by this node, when available.
    pub fn span(&self) -> Option<&Span> {
        self.span.as_ref()
    }

    /// Returns the semantic presentation role of this node.
    pub fn role(&self) -> PrettyNodeRole {
        self.role
    }

    /// Returns the labeled children in source-tree order.
    pub fn children(&self) -> &[PrettyEdge] {
        &self.children
    }

    /// Creates a generated concrete syntax node.
    #[doc(hidden)]
    pub fn syntax(name: impl Into<String>, span: Option<Span>) -> Self {
        Self::new(name, span, PrettyNodeRole::Syntax)
    }

    /// Creates a generated enum-variant node.
    #[doc(hidden)]
    pub fn variant(category: &str, _variant: &str, span: Option<Span>, value: PrettyNode) -> Self {
        let mut node = Self::new(category, span, PrettyNodeRole::Category);
        node.push_node("", value);
        node
    }

    /// Creates a delimiter collection with explicit opening and closing boundaries.
    #[doc(hidden)]
    pub fn delimiter(kind: DelimiterKind, spans: DoubleSpan) -> Self {
        let delimiter = Delimiter::new(kind, spans);
        let mut node = Self::collection(
            format!("{}…{} · 2 tokens", kind.opening(), kind.closing()),
            delimiter.try_span(),
        );
        node.push_node(
            "opening",
            Self::delimiter_boundary(kind, true, delimiter.opening_span()),
        );
        if let Some(closing) = delimiter.closing_span() {
            node.push_node("closing", Self::delimiter_boundary(kind, false, closing));
        }
        node
    }

    /// Creates one displayed boundary of a canonical paired delimiter atom.
    #[doc(hidden)]
    pub fn delimiter_boundary(kind: DelimiterKind, opening: bool, span: Span) -> Self {
        let name = match kind {
            DelimiterKind::Parenthesis => "Delimiter![()]",
            DelimiterKind::Bracket => "Delimiter![[]]",
            DelimiterKind::Brace => "Delimiter![{}]",
            DelimiterKind::AngleBar => "Delimiter![<| |>]",
        };
        let spelling = if opening {
            kind.opening()
        } else {
            kind.closing()
        };
        let mut node = Self::new(name, Some(span), PrettyNodeRole::Delimiter);
        node.detail = Some(PrettyDetail {
            text: spelling.into(),
            role: PrettyDetailRole::Text,
        });
        node
    }

    /// Creates a leaf containing token or source text.
    #[doc(hidden)]
    pub fn token(name: impl Into<String>, text: impl Into<String>, span: Span) -> Self {
        let name = name.into();
        let text = text.into();
        if name == "Delimiter"
            && let Some(kind) = delimiter_notation(&text)
        {
            return Self::delimiter(kind, DoubleSpan::new(span.clone(), span));
        }
        let role = if matches!(text.as_str(), "∅" | "EOC" | "EOF") {
            PrettyDetailRole::Glyph
        } else {
            PrettyDetailRole::Text
        };
        let mut node = Self::new(name, Some(span), PrettyNodeRole::Token);
        node.detail = Some(PrettyDetail { text, role });
        node
    }

    /// Creates a collection wrapper.
    #[doc(hidden)]
    pub fn collection(name: impl Into<String>, span: Option<Span>) -> Self {
        let mut node = Self::new(name, span, PrettyNodeRole::Collection);
        node.transparent = true;
        node
    }

    /// Creates a visible grouping node that has collection styling.
    #[doc(hidden)]
    pub fn group(name: impl Into<String>, span: Option<Span>) -> Self {
        Self::new(name, span, PrettyNodeRole::Collection)
    }

    /// Creates an absent optional value.
    #[doc(hidden)]
    pub fn absent() -> Self {
        Self::new("None", None, PrettyNodeRole::Absent)
    }

    /// Creates a new value.
    fn new(name: impl Into<String>, span: Option<Span>, role: PrettyNodeRole) -> Self {
        Self {
            name: name.into(),
            detail: None,
            span,
            role,
            transparent: false,
            children: Vec::new(),
        }
    }

    /// Adds a named child to this node.
    #[doc(hidden)]
    pub fn push(&mut self, label: impl Into<String>, child: impl PrettyTree) {
        self.push_node(label, child.pretty_tree());
    }

    /// Adds an already constructed named child to this node.
    ///
    /// Collection nodes are presentation fragments rather than CST nodes, so
    /// their children are inserted directly into this node.
    #[doc(hidden)]
    pub fn push_node(&mut self, label: impl Into<String>, node: PrettyNode) {
        if node.transparent {
            for child in node.children {
                self.push_node(child.label, child.node);
            }
            return;
        }
        let mut node = node;
        if node.span.is_none() {
            node.span.clone_from(&self.span);
        }
        self.children.push(PrettyEdge {
            label: label.into(),
            node,
        });
    }
}

/// Handles delimiter notation.
fn delimiter_notation(notation: &str) -> Option<DelimiterKind> {
    match notation {
        "(…)" => Some(DelimiterKind::Parenthesis),
        "[…]" => Some(DelimiterKind::Bracket),
        "{…}" => Some(DelimiterKind::Brace),
        "<|…|>" => Some(DelimiterKind::AngleBar),
        _ => None,
    }
}

impl<T: PrettyTree + ?Sized> PrettyTree for &T {
    fn pretty_tree(&self) -> PrettyNode {
        T::pretty_tree(self)
    }
}

impl<T: PrettyTree> PrettyTree for Box<T> {
    fn pretty_tree(&self) -> PrettyNode {
        T::pretty_tree(self)
    }
}

impl<T: PrettyTree + Spanned> PrettyTree for Option<T> {
    fn pretty_tree(&self) -> PrettyNode {
        match self {
            Some(value) => value.pretty_tree(),
            None => PrettyNode::absent(),
        }
    }
}

impl<T: PrettyTree + Spanned> PrettyTree for Vec<T> {
    fn pretty_tree(&self) -> PrettyNode {
        let span = Span::join_all(self.iter().map(Spanned::span));
        let mut node = PrettyNode::collection(item_count("List", self.len()), span);
        for (index, value) in self.iter().enumerate() {
            node.push(format!("[{index}]"), value);
        }
        node
    }
}

impl PrettyTree for Trivia {
    fn pretty_tree(&self) -> PrettyNode {
        let name = match self.kind() {
            TriviaKind::LineComment => "LineCommentBlock",
            TriviaKind::BlockComment => "BlockComment",
        };
        PrettyNode::token(name, self.text(), self.span())
    }
}

impl PrettyTree for CommentTrivia {
    fn pretty_tree(&self) -> PrettyNode {
        let count = self.outer().len() + self.inner().len();
        let mut node = PrettyNode::group(item_count("Comments", count), self.try_span());
        for (index, comment) in self.outer().iter().enumerate() {
            node.push(format!("outer[{index}]"), comment);
        }
        for (index, comment) in self.inner().iter().enumerate() {
            node.push(format!("inner[{index}]"), comment);
        }
        node
    }
}

impl<T, P> PrettyTree for Punctuated<T, P>
where
    T: PrettyTree + Spanned,
    P: PrettyTree + Spanned,
{
    fn pretty_tree(&self) -> PrettyNode {
        let mut node =
            PrettyNode::collection(item_count("Punctuated", self.len()), self.try_span());
        node.push("[0]", self.first());
        for (index, (punctuation, value)) in self.pairs().enumerate() {
            node.push("punctuation", punctuation);
            node.push(format!("[{}]", index + 1), value);
        }
        node
    }
}

/// Handles item count.
fn item_count(name: &str, count: usize) -> String {
    let suffix = if count == 1 { "item" } else { "items" };
    format!("{name} · {count} {suffix}")
}

/// Formatting choices shared by token, CST, and combined reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrettyOptions {
    /// Whether to emit ANSI color escapes.
    pub color: bool,
    /// Whether to display source spans.
    pub spans: bool,
    /// Whether to include trivia in raw token views.
    pub trivia: bool,
    /// The preferred output width.
    pub width: usize,
}

impl PrettyOptions {
    /// Plain, deterministic output suitable for tests and redirected output.
    pub const PLAIN: Self = Self {
        color: false,
        spans: true,
        trivia: false,
        width: 150,
    };

    /// ANSI-colored output intended for an interactive terminal.
    pub const ANSI: Self = Self {
        color: true,
        spans: true,
        trivia: false,
        width: 150,
    };
}

impl Default for PrettyOptions {
    fn default() -> Self {
        Self::PLAIN
    }
}

/// A displayable three-part inspection report for one parsed source file.
///
/// The report shows the retained input snapshot, a recursively flattened raw
/// token table, and the complete generated typed CST. Use [`Self::ansi`] for
/// explicit terminal color or [`crate::ParsedFile::print_pretty`] for automatic
/// terminal detection.
pub struct PrettyReport<'source> {
    /// The source.
    source: &'source str,
    /// The tokens.
    tokens: TokenStream,
    /// The CST.
    cst: PrettyNode,
    /// The options.
    options: PrettyOptions,
}

impl<'source> PrettyReport<'source> {
    /// Creates a plain report from its three projections.
    #[doc(hidden)]
    pub fn new(source: &'source str, tokens: TokenStream, cst: PrettyNode) -> Self {
        Self {
            source,
            tokens,
            cst,
            options: PrettyOptions::PLAIN,
        }
    }

    /// Enables or disables ANSI terminal styling.
    pub fn ansi(mut self, enabled: bool) -> Self {
        self.options.color = enabled;
        self
    }

    /// Enables or disables source ranges.
    pub fn spans(mut self, enabled: bool) -> Self {
        self.options.spans = enabled;
        self
    }

    /// Includes or hides coalesced trivia in the flat token view.
    pub fn trivia(mut self, enabled: bool) -> Self {
        self.options.trivia = enabled;
        self
    }

    /// Sets the preferred report width used to wrap horizontal token bands.
    pub fn width(mut self, width: usize) -> Self {
        self.options.width = width.max(40);
        self
    }
}

impl Display for PrettyReport<'_> {
    fn fmt(&self, output: &mut Formatter<'_>) -> FmtResult {
        let mut source = String::new();
        write_source(&mut source, self.source)?;
        write_box(output, "SOURCE", &source, self.options)?;
        writeln!(output)?;

        let mut tokens = String::new();
        write_tokens(&mut tokens, &self.tokens, self.options)?;
        write_box(output, "TOKEN STREAM", &tokens, self.options)?;
        writeln!(output)?;

        let mut cst = String::new();
        write_tree(&mut cst, &self.cst, self.options)?;
        write_box(output, "TYPED CST", &cst, self.options)
    }
}

/// Handles print report.
pub(crate) fn print_report(report: PrettyReport<'_>) -> io::Result<()> {
    let mut output = io::stdout().lock();
    let width = std::env::var("COLUMNS")
        .ok()
        .and_then(|width| width.parse().ok())
        .unwrap_or(120);
    let report = report.ansi(output.is_terminal()).width(width);
    write!(output, "{report}")
}

/// Formats tree.
pub(crate) fn format_tree(node: &PrettyNode, options: PrettyOptions) -> String {
    let mut output = String::new();
    write_tree(&mut output, node, options).expect("writing to a string cannot fail");
    output
}

/// Formats one typed syntax value as a Unicode tree without terminal colors.
pub fn format_pretty_tree(value: &impl PrettyTree) -> String {
    format_tree(&value.pretty_tree(), PrettyOptions::PLAIN)
}

/// Writes box.
fn write_box(
    output: &mut impl FmtWrite,
    title: &str,
    contents: &str,
    options: PrettyOptions,
) -> FmtResult {
    let inner_width = options.width.saturating_sub(4);
    writeln!(
        output,
        "{}{}{}{}",
        paint(DIM, "╭─ ", options),
        paint(BOLD_CYAN, title, options),
        paint(
            DIM,
            &format!(
                " {}",
                "─".repeat(options.width.saturating_sub(title.chars().count() + 5))
            ),
            options,
        ),
        paint(DIM, "╮", options),
    )?;
    for line in contents.lines() {
        let line = clip_ansi(line, inner_width);
        let line_width = ansi_width(&line);
        writeln!(
            output,
            "{} {}{} {}",
            paint(DIM, "│", options),
            line,
            " ".repeat(inner_width.saturating_sub(line_width)),
            paint(DIM, "│", options),
        )?;
    }
    writeln!(
        output,
        "{}{}{}",
        paint(DIM, "╰", options),
        paint(DIM, &"─".repeat(options.width.saturating_sub(2)), options),
        paint(DIM, "╯", options),
    )
}

/// Handles ansi width.
fn ansi_width(text: &str) -> usize {
    let mut characters = text.chars().peekable();
    let mut width = 0;
    while let Some(character) = characters.next() {
        if character == '\u{1b}' && characters.peek() == Some(&'[') {
            for escaped in characters.by_ref() {
                if escaped == 'm' {
                    break;
                }
            }
        } else {
            width += 1;
        }
    }
    width
}

/// Handles clip ansi.
fn clip_ansi(text: &str, width: usize) -> String {
    if ansi_width(text) <= width {
        return text.to_owned();
    }

    let limit = width.saturating_sub(1);
    let mut output = String::new();
    let mut characters = text.chars().peekable();
    let mut visible = 0;
    let mut styled = false;
    while let Some(character) = characters.next() {
        if character == '\u{1b}' && characters.peek() == Some(&'[') {
            styled = true;
            output.push(character);
            for escaped in characters.by_ref() {
                output.push(escaped);
                if escaped == 'm' {
                    break;
                }
            }
        } else if visible < limit {
            output.push(character);
            visible += 1;
        } else {
            break;
        }
    }
    output.push('…');
    if styled {
        output.push_str(RESET);
    }
    output
}

/// Writes source.
fn write_source(output: &mut impl FmtWrite, source: &str) -> FmtResult {
    let width = source.lines().count().max(1).to_string().len();
    if source.is_empty() {
        writeln!(output, "∅")?;
    } else {
        for (index, line) in source.lines().enumerate() {
            writeln!(output, "{:>width$} │ {}", index + 1, line)?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
/// Internal alternatives for FlatKind.
enum FlatKind {
    /// The Adj alternative.
    Adj,
    /// The Open alternative.
    Open,
    /// The Close alternative.
    Close,
    /// The Ident alternative.
    Ident,
    /// The Literal alternative.
    Literal,
    /// The Punct alternative.
    Punct,
    /// The Trivia alternative.
    Trivia,
}

#[derive(Clone)]
/// Internal state for FlatToken.
struct FlatToken {
    /// The depth.
    depth: usize,
    /// The kind.
    kind: FlatKind,
    /// The text.
    text: String,
    /// The span.
    span: Span,
}

/// Formats tokens.
pub(crate) fn format_tokens(tokens: &TokenStream, options: PrettyOptions) -> String {
    let mut output = String::new();
    write_tokens(&mut output, tokens, options).expect("writing to a string cannot fail");
    output
}

/// Formats a raw token stream as a table with explicit group boundaries.
pub fn format_pretty_tokens(tokens: &TokenStream) -> String {
    format_tokens(tokens, PrettyOptions::PLAIN)
}

/// Formats one framed cell as a table with explicit inferred adjacency.
pub fn format_pretty_cell_tokens(tokens: &CellTokenStream) -> String {
    format_tokens(
        &tokens.clone().into_parser_token_stream(),
        PrettyOptions::PLAIN,
    )
}

/// Writes tokens.
fn write_tokens(
    output: &mut impl FmtWrite,
    tokens: &TokenStream,
    options: PrettyOptions,
) -> FmtResult {
    let mut flat = Vec::new();
    flatten(tokens, 0, &mut flat);
    write_flat(output, flat, options)
}

/// Writes flat.
fn write_flat(
    output: &mut impl FmtWrite,
    mut flat: Vec<FlatToken>,
    options: PrettyOptions,
) -> FmtResult {
    if !options.trivia {
        flat.retain(|token| !matches!(token.kind, FlatKind::Trivia));
    }
    if flat.is_empty() {
        return writeln!(output, "{}", paint(DIM, "∅", options));
    }
    let available = options.width.saturating_sub(12).max(24);
    let mut band = Vec::new();
    let mut used = 0;
    let mut wrote_band = false;
    for chunk in flat.iter().map(std::slice::from_ref) {
        let chunk_width = token_band_width(chunk);
        if !band.is_empty() && used + 1 + chunk_width > available {
            if wrote_band {
                writeln!(output)?;
            }
            write_token_band(output, &band, options)?;
            wrote_band = true;
            band.clear();
            used = 0;
        }
        if !band.is_empty() {
            used += 1;
        }
        used += chunk_width;
        band.extend_from_slice(chunk);
    }
    if !band.is_empty() {
        if wrote_band {
            writeln!(output)?;
        }
        write_token_band(output, &band, options)?;
    }
    Ok(())
}

/// Handles token band width.
fn token_band_width(tokens: &[FlatToken]) -> usize {
    tokens.iter().map(token_column_width).sum::<usize>() + tokens.len().saturating_sub(1)
}

/// Handles token column width.
fn token_column_width(token: &FlatToken) -> usize {
    visible_text(&token.text)
        .chars()
        .count()
        .max(flat_kind(token.kind).chars().count())
        .clamp(3, 24)
        + 2
}

/// Writes token band.
fn write_token_band(
    output: &mut impl FmtWrite,
    tokens: &[FlatToken],
    options: PrettyOptions,
) -> FmtResult {
    write!(
        output,
        "{} {} ",
        paint(BRIGHT_YELLOW, "token", options),
        paint(DIM, "│", options)
    )?;
    for (index, token) in tokens.iter().enumerate() {
        if index != 0 {
            write!(output, "{}", paint(DIM, "│", options))?;
        }
        let width = token_column_width(token) - 2;
        let text = clipped(&visible_text(&token.text), width);
        let cell = centered_cell(&text, width);
        write!(output, "{}", paint(token_color(token.kind), &cell, options))?;
    }
    writeln!(output)?;

    write!(
        output,
        "{} {} ",
        paint(BRIGHT_YELLOW, " type", options),
        paint(DIM, "│", options)
    )?;
    for (index, token) in tokens.iter().enumerate() {
        if index != 0 {
            write!(output, "{}", paint(DIM, "│", options))?;
        }
        let width = token_column_width(token) - 2;
        let kind = clipped(flat_kind(token.kind), width);
        let cell = centered_cell(&kind, width);
        write!(output, "{}", paint(token_color(token.kind), &cell, options))?;
    }
    writeln!(output)
}

/// Handles centered cell.
fn centered_cell(text: &str, width: usize) -> String {
    let padding = width.saturating_sub(text.chars().count());
    let left = padding / 2;
    let right = padding - left;
    format!(" {}{}{} ", " ".repeat(left), text, " ".repeat(right))
}

/// Handles clipped.
fn clipped(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_owned();
    }
    text.chars()
        .take(width.saturating_sub(1))
        .chain(std::iter::once('…'))
        .collect()
}

/// Handles flatten.
fn flatten(tokens: &TokenStream, depth: usize, output: &mut Vec<FlatToken>) {
    flatten_trivia(tokens.prefix_trivia(), depth, output);
    for token in tokens.iter() {
        flatten_trivia(token.leading_trivia().as_slice(), depth, output);
        match token {
            TokenTree::Group(group) => {
                let kind = group.delim_kind();
                push_flat(
                    output,
                    FlatToken {
                        depth,
                        kind: FlatKind::Open,
                        text: kind.opening().into(),
                        span: group.delimiter().opening_span(),
                    },
                );
                flatten(group.stream(), depth + 1, output);
                if let Some(closing) = group.delimiter().closing_span() {
                    push_flat(
                        output,
                        FlatToken {
                            depth,
                            kind: FlatKind::Close,
                            text: kind.closing().into(),
                            span: closing,
                        },
                    );
                }
            }
            TokenTree::Ident(token) => push_flat(
                output,
                FlatToken {
                    depth,
                    kind: FlatKind::Ident,
                    text: token.text().into(),
                    span: token.expect_span(),
                },
            ),
            TokenTree::Literal(token) => push_flat(
                output,
                FlatToken {
                    depth,
                    kind: FlatKind::Literal,
                    text: token.text().into(),
                    span: token.expect_span(),
                },
            ),
            TokenTree::Punct(token) => push_flat(
                output,
                FlatToken {
                    depth,
                    kind: if token.text().is_empty() {
                        FlatKind::Adj
                    } else {
                        FlatKind::Punct
                    },
                    text: if token.text().is_empty() {
                        "Adj".into()
                    } else {
                        token.text().into()
                    },
                    span: token.expect_span(),
                },
            ),
        }
        flatten_trivia(token.trailing_trivia().as_slice(), depth, output);
    }
    flatten_trivia(tokens.suffix_trivia(), depth, output);
}

/// Handles flatten trivia.
fn flatten_trivia(trivia: &[crate::Trivia], depth: usize, output: &mut Vec<FlatToken>) {
    for token in trivia {
        push_flat(
            output,
            FlatToken {
                depth,
                kind: FlatKind::Trivia,
                text: token.text().into(),
                span: token.expect_span(),
            },
        );
    }
}

/// Pushes flat.
fn push_flat(output: &mut Vec<FlatToken>, token: FlatToken) {
    if matches!(token.kind, FlatKind::Trivia)
        && let Some(previous) = output.last_mut()
        && matches!(previous.kind, FlatKind::Trivia)
        && previous.depth == token.depth
    {
        previous.text.push_str(&token.text);
        if let Some(span) = previous.span.join(token.span.clone()) {
            previous.span = span;
        } else {
            output.push(token);
        }
    } else {
        output.push(token);
    }
}

/// Handles flat kind.
fn flat_kind(kind: FlatKind) -> &'static str {
    match kind {
        FlatKind::Adj => "ADJ",
        FlatKind::Open => "DELIM->",
        FlatKind::Close => "<-DELIM",
        FlatKind::Ident => "IDENT",
        FlatKind::Literal => "LITERAL",
        FlatKind::Punct => "PUNCT",
        FlatKind::Trivia => "TRIVIA",
    }
}

/// Handles token color.
fn token_color(kind: FlatKind) -> &'static str {
    match kind {
        FlatKind::Adj => BRIGHT_BLACK,
        FlatKind::Open => BLUE,
        FlatKind::Close => MAGENTA,
        FlatKind::Ident => CYAN,
        FlatKind::Literal => YELLOW,
        FlatKind::Punct => GREEN,
        FlatKind::Trivia => RED,
    }
}

/// Handles visible text.
fn visible_text(text: &str) -> String {
    let mut visible = String::new();
    for character in text.chars() {
        match character {
            ' ' => visible.push_str("' '"),
            '\t' => visible.push('⇥'),
            '\n' => visible.push('↵'),
            '\r' => visible.push('␍'),
            character => visible.push(character),
        }
    }
    visible
}

/// Writes tree.
fn write_tree(output: &mut impl FmtWrite, root: &PrettyNode, options: PrettyOptions) -> FmtResult {
    write_node(output, root, "", None, true, options)
}

/// Writes node.
fn write_node(
    output: &mut impl FmtWrite,
    node: &PrettyNode,
    prefix: &str,
    label: Option<&str>,
    last: bool,
    options: PrettyOptions,
) -> FmtResult {
    let connector = if label.is_none() {
        ""
    } else if last {
        "└─ "
    } else {
        "├─ "
    };
    write!(
        output,
        "{}{}",
        paint(DIM, prefix, options),
        paint(DIM, connector, options)
    )?;
    if let Some(detail) = &node.detail {
        write!(
            output,
            "{}  ",
            paint(
                detail_color(detail.role),
                &visible_text(&detail.text),
                options
            )
        )?;
    }
    if let Some(label) = label.filter(|label| !label.is_empty()) {
        write!(
            output,
            "{} {} ",
            paint(BRIGHT_YELLOW, label, options),
            paint(DIM, "→", options)
        )?;
    }
    write!(
        output,
        "{}",
        paint(node_color(node.role), &node.name, options)
    )?;
    if options.spans {
        write!(
            output,
            "  {}",
            paint(
                DIM,
                &node.span.as_ref().map_or_else(|| "@?".into(), format_span),
                options,
            )
        )?;
    }
    writeln!(output)?;

    let child_prefix = if label.is_none() {
        String::new()
    } else {
        format!("{prefix}{}", if last { "   " } else { "│  " })
    };
    let child_count = node.children.len();
    for (index, child) in node.children.iter().enumerate() {
        write_node(
            output,
            &child.node,
            &child_prefix,
            Some(&child.label),
            index + 1 == child_count,
            options,
        )?;
    }
    Ok(())
}

/// Handles node color.
fn node_color(role: PrettyNodeRole) -> &'static str {
    match role {
        PrettyNodeRole::Syntax => NON_TERMINAL_BLUE,
        PrettyNodeRole::Category => BRIGHT_YELLOW,
        PrettyNodeRole::Token => BOLD_CYAN,
        PrettyNodeRole::Delimiter => MAGENTA,
        PrettyNodeRole::Collection => BRIGHT_YELLOW,
        PrettyNodeRole::Absent => DIM,
    }
}

/// Handles detail color.
fn detail_color(role: PrettyDetailRole) -> &'static str {
    match role {
        PrettyDetailRole::Text => WHITE,
        PrettyDetailRole::Glyph => BRIGHT_BLACK,
    }
}

/// Formats span.
fn format_span(span: &Span) -> String {
    let Some(range) = span.line_column_range() else {
        return "@unlocated".into();
    };
    format!("@{range}")
}

/// Handles paint.
fn paint(style: &'static str, text: &str, options: PrettyOptions) -> String {
    if options.color && !text.is_empty() {
        format!("{style}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lex_str;

    #[test]
    fn adjacent_trivia_is_hidden_or_rendered_as_one_token() {
        let source = "-- note";
        let tokens = lex_str(source).unwrap();

        let hidden = format_tokens(&tokens, PrettyOptions::PLAIN);
        assert!(!hidden.contains("TRIVIA"));
        let shown = format_tokens(
            &tokens,
            PrettyOptions {
                trivia: true,
                ..PrettyOptions::PLAIN
            },
        );
        assert_eq!(shown.matches("TRIVIA").count(), 1);
    }

    #[test]
    fn token_bands_use_distinct_class_colors() {
        let source = "name = 1\n(next) -- note";
        let tokens = lex_str(source).unwrap();
        let plain = format_tokens(&tokens, PrettyOptions::PLAIN);

        assert!(plain.contains("token │"));

        let colored = format_tokens(
            &tokens,
            PrettyOptions {
                trivia: true,
                ..PrettyOptions::ANSI
            },
        );
        for color in [RED, GREEN, YELLOW, BLUE, MAGENTA, CYAN] {
            assert!(colored.contains(color));
        }
    }

    #[test]
    fn raw_tokens_have_no_virtual_cell_boundaries() {
        let source = "first\nsecond";
        let tokens = lex_str(source).unwrap();
        let rendered = format_tokens(&tokens, PrettyOptions::PLAIN);

        assert_eq!(rendered.matches("token │").count(), 1);
        assert!(!rendered.contains("EOC"));
        assert!(rendered.contains("second"));
    }

    #[test]
    fn token_cells_are_centered() {
        assert_eq!(centered_cell("x", 5), "   x   ");
        assert_eq!(centered_cell("xy", 5), "  xy   ");
    }

    #[test]
    fn enum_presentation_retains_the_dispatch_category() {
        let span = Span::for_text("x");
        let value = PrettyNode::token("Symbol", "x", span.clone());
        let rendered = PrettyNode::variant("Expr", "Symbol", Some(span), value);

        assert_eq!(rendered.name, "Expr");
        assert_eq!(rendered.role, PrettyNodeRole::Category);
        assert_eq!(rendered.children.len(), 1);
        assert_eq!(rendered.children[0].node.name, "Symbol");
    }

    #[test]
    fn tree_values_lead_their_field_and_distinguish_text_from_glyphs() {
        let source = Span::for_text(";");
        let mut root = PrettyNode::syntax("Root", Some(source.clone()));
        root.push_node(
            "semicolon",
            PrettyNode::token("Token![;]", ";", source.clone()),
        );
        root.push_node("operator", PrettyNode::token("Adj", "∅", source.at(1)));

        let plain = format_tree(&root, PrettyOptions::PLAIN);
        assert!(plain.contains(";  semicolon → Token![;]"));
        assert!(plain.contains("∅  operator → Adj"));

        let colored = format_tree(&root, PrettyOptions::ANSI);
        assert!(colored.contains(&format!("{WHITE};{RESET}")));
        assert!(colored.contains(&format!("{BRIGHT_BLACK}∅{RESET}")));
    }

    #[test]
    fn delimiter_is_presented_as_a_collection() {
        let source = Span::for_text("()");
        let delimiter = PrettyNode::delimiter(
            DelimiterKind::Parenthesis,
            DoubleSpan::new(source.subspan(0, 1), source.subspan(1, 2)),
        );

        assert_eq!(delimiter.role, PrettyNodeRole::Collection);
        assert_eq!(delimiter.name, "(…) · 2 tokens");
        assert!(delimiter.children.iter().all(|edge| {
            edge.node.name == "Delimiter![()]" && edge.node.role == PrettyNodeRole::Delimiter
        }));
    }

    #[test]
    fn report_boxes_have_four_fixed_width_walls() {
        let mut rendered = String::new();
        let long_line = "x".repeat(PrettyOptions::PLAIN.width * 2);
        write_box(
            &mut rendered,
            "TEST",
            &format!("one\ntwo\n{long_line}"),
            PrettyOptions::PLAIN,
        )
        .unwrap();

        for line in rendered.lines() {
            assert_eq!(line.chars().count(), PrettyOptions::PLAIN.width);
        }
        assert!(rendered.contains('…'));
        assert!(rendered.starts_with("╭─ TEST "));
        assert!(rendered.lines().next().unwrap().ends_with('╮'));
        assert!(rendered.lines().nth(1).unwrap().ends_with('│'));
        assert!(rendered.lines().last().unwrap().ends_with('╯'));
    }
}
