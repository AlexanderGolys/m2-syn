//! Terminal-oriented inspection of source, raw tokens, and typed syntax trees.
//!
//! [`PrettyReport`] combines the three useful views of a parsed file. The
//! renderer deliberately owns only presentation data; the generated typed CST
//! remains the source of truth and implements [`PrettyTree`] directly.

use std::fmt::{Display, Formatter, Result as FmtResult, Write as FmtWrite};
use std::io::{self, IsTerminal as _, Write as IoWrite};

use crate::{
    CellStream, DelimiterKind, Punctuated, Span, Spanned, TextPoint, TextRange, TokenStream,
    TokenTree, punct::Pair, token_stream::delim::Delimiter,
};

const RESET: &str = "\u{1b}[0m";
const BOLD_CYAN: &str = "\u{1b}[1;36m";
const BRIGHT_YELLOW: &str = "\u{1b}[93m";
const NON_TERMINAL_BLUE: &str = "\u{1b}[38;5;75m";
const DIM: &str = "\u{1b}[2m";
const IDENT_COLOR: &str = "\u{1b}[38;5;45m";
const LITERAL_COLOR: &str = "\u{1b}[38;5;207m";
const PUNCT_COLOR: &str = "\u{1b}[38;5;82m";
const OPEN_COLOR: &str = "\u{1b}[38;5;75m";
const CLOSE_COLOR: &str = "\u{1b}[38;5;213m";
const EOC_COLOR: &str = "\u{1b}[38;5;208m";
const TRIVIA_COLOR: &str = "\u{1b}[38;5;245m";

/// Converts typed syntax into the structural presentation used by the pretty
/// printer.
///
/// Implementations for generated nodes are emitted from `syntax_schema!`, so
/// field names, punctuation, and newly added concrete node types appear in the
/// tree without a handwritten registry. Category enums are intentionally
/// transparent because they are storage details rather than CST nodes.
pub trait PrettyTree {
    /// Builds the presentation node for this syntax value.
    fn pretty_tree(&self) -> PrettyNode;
}

/// One owned node in the terminal presentation tree.
///
/// This is primarily useful to implementations of [`PrettyTree`]. Most users
/// obtain it through [`crate::ParsedFile::pretty_cst`] or [`PrettyReport`].
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrettyNode {
    name: String,
    detail: Option<String>,
    span: Span,
    role: NodeRole,
    children: Vec<PrettyEdge>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeRole {
    Syntax,
    Token,
    Collection,
    Absent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrettyEdge {
    label: String,
    node: PrettyNode,
}

impl PrettyNode {
    /// Creates a generated concrete syntax node.
    #[doc(hidden)]
    pub fn syntax(name: impl Into<String>, span: Span) -> Self {
        Self::new(name, span, NodeRole::Syntax)
    }

    /// Creates a generated enum-variant node.
    #[doc(hidden)]
    pub fn variant(_category: &str, _variant: &str, span: Span, mut value: PrettyNode) -> Self {
        if !matches!(span, Span::Detached) {
            value.span = span;
        }
        value
    }

    /// Creates a delimiter node with explicit opening and closing boundaries.
    #[doc(hidden)]
    pub fn delimiter(kind: DelimiterKind, span: Span) -> Self {
        let delimiter = Delimiter::new(kind, span);
        let mut node = Self::new("Delimiter", span, NodeRole::Token);
        if !kind.opening().is_empty() {
            node.push_node(
                "opening",
                Self::token("Token", kind.opening(), delimiter.opening_span()),
            );
        }
        if !kind.closing().is_empty() {
            node.push_node(
                "closing",
                Self::token("Token", kind.closing(), delimiter.closing_span()),
            );
        }
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
            return Self::delimiter(kind, span);
        }
        let mut node = Self::new(name, span, NodeRole::Token);
        node.detail = Some(text);
        node
    }

    /// Creates a collection wrapper.
    #[doc(hidden)]
    pub fn collection(name: impl Into<String>, span: Span) -> Self {
        Self::new(name, span, NodeRole::Collection)
    }

    /// Creates an absent optional value.
    #[doc(hidden)]
    pub fn absent() -> Self {
        Self::new("None", Span::detached(), NodeRole::Absent)
    }

    fn new(name: impl Into<String>, span: Span, role: NodeRole) -> Self {
        Self {
            name: name.into(),
            detail: None,
            span,
            role,
            children: Vec::new(),
        }
    }

    /// Adds a named child to this node.
    #[doc(hidden)]
    pub fn push(&mut self, label: impl Into<String>, child: impl PrettyTree) {
        self.push_node(label, child.pretty_tree());
    }

    /// Adds an already constructed named child to this node.
    #[doc(hidden)]
    pub fn push_node(&mut self, label: impl Into<String>, node: PrettyNode) {
        let mut node = node;
        if matches!(node.span, Span::Detached) {
            node.span = self.span;
        }
        self.children.push(PrettyEdge {
            label: label.into(),
            node,
        });
    }
}

fn delimiter_notation(notation: &str) -> Option<DelimiterKind> {
    match notation {
        "…" => Some(DelimiterKind::Empty),
        "…;" => Some(DelimiterKind::Semicolon),
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
        let mut node = PrettyNode::collection(item_count("List", self.len()), self.span());
        for (index, value) in self.iter().enumerate() {
            node.push(format!("[{index}]"), value);
        }
        node
    }
}

impl<T: PrettyTree + Spanned> PrettyTree for Punctuated<T> {
    fn pretty_tree(&self) -> PrettyNode {
        let mut node = PrettyNode::collection(item_count("Punctuated", self.len()), self.span());
        let values = self
            .pairs()
            .map(|pair| match pair {
                Pair::Punctuated(value, comma) => (value.pretty_tree(), Some(comma.pretty_tree())),
                Pair::End(value) => (value.pretty_tree(), None),
            })
            .collect::<Vec<_>>();
        for (index, (value, comma)) in values.iter().enumerate() {
            node.push_node(format!("[{index}]"), value.clone());
            if let Some(comma) = comma {
                let mut comma = comma.clone();
                if matches!(comma.span, Span::Detached) {
                    let right = values
                        .get(index + 1)
                        .map_or(node.span, |(value, _)| value.span);
                    comma.span = infer_token_span(value.span, right, ",").unwrap_or(node.span);
                }
                node.push_node("comma", comma);
            }
        }
        node
    }
}

fn item_count(name: &str, count: usize) -> String {
    let suffix = if count == 1 { "item" } else { "items" };
    format!("{name} · {count} {suffix}")
}

/// Formatting choices shared by token, CST, and combined reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrettyOptions {
    /// Emits ANSI terminal styling when enabled.
    pub color: bool,
    /// Includes source ranges beside tokens and CST nodes.
    pub spans: bool,
    /// Includes coalesced whitespace and comment trivia in the flat token view.
    pub trivia: bool,
    /// Preferred width of source headings and horizontal token bands.
    pub width: usize,
}

impl PrettyOptions {
    /// Plain, deterministic output suitable for tests and redirected output.
    pub const PLAIN: Self = Self {
        color: false,
        spans: true,
        trivia: false,
        width: 120,
    };

    /// ANSI-colored output intended for an interactive terminal.
    pub const ANSI: Self = Self {
        color: true,
        spans: true,
        trivia: false,
        width: 120,
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
    source: &'source str,
    cells: CellStream,
    cst: PrettyNode,
    options: PrettyOptions,
}

impl<'source> PrettyReport<'source> {
    /// Creates a plain report from its three projections.
    #[doc(hidden)]
    pub fn new(source: &'source str, cells: CellStream, cst: PrettyNode) -> Self {
        Self {
            source,
            cells,
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
        section_title(output, "SOURCE", self.options)?;
        write_source(output, self.source, self.options)?;
        section_title(output, "TOKEN STREAM", self.options)?;
        write_cell_tokens(output, &self.cells, self.options)?;
        section_title(output, "TYPED CST", self.options)?;
        write_tree(output, &self.cst, self.options)
    }
}

pub(crate) fn print_report(report: PrettyReport<'_>) -> io::Result<()> {
    let mut output = io::stdout().lock();
    let width = std::env::var("COLUMNS")
        .ok()
        .and_then(|width| width.parse().ok())
        .unwrap_or(120);
    let report = report.ansi(output.is_terminal()).width(width);
    write!(output, "{report}")
}

pub(crate) fn format_tree(node: &PrettyNode, options: PrettyOptions) -> String {
    let mut output = String::new();
    write_tree(&mut output, node, options).expect("writing to a string cannot fail");
    output
}

fn section_title(output: &mut impl FmtWrite, title: &str, options: PrettyOptions) -> FmtResult {
    writeln!(
        output,
        "{}╭─ {} {}",
        paint(DIM, "", options),
        paint(BOLD_CYAN, title, options),
        paint(
            DIM,
            &"─".repeat(options.width.saturating_sub(title.len() + 5)),
            options
        ),
    )
}

fn write_source(output: &mut impl FmtWrite, source: &str, options: PrettyOptions) -> FmtResult {
    let width = source.lines().count().max(1).to_string().len();
    if source.is_empty() {
        writeln!(output, "{}", paint(DIM, "│  ∅", options))?;
    } else {
        for (index, line) in source.lines().enumerate() {
            writeln!(
                output,
                "{} {:>width$} {} {}",
                paint(DIM, "│", options),
                index + 1,
                paint(DIM, "│", options),
                line,
            )?;
        }
    }
    writeln!(output, "{}", paint(DIM, "╰─", options))
}

#[derive(Clone, Copy)]
enum FlatKind {
    Open(DelimiterKind),
    Close(DelimiterKind),
    Ident,
    Literal,
    Punct,
    Trivia,
    Eoc,
    Eof,
}

#[derive(Clone)]
struct FlatToken {
    depth: usize,
    kind: FlatKind,
    text: String,
    span: Span,
}

pub(crate) fn format_cells(cells: &CellStream, options: PrettyOptions) -> String {
    let mut output = String::new();
    write_cell_tokens(&mut output, cells, options).expect("writing to a string cannot fail");
    output
}

fn write_cell_tokens(
    output: &mut impl FmtWrite,
    cells: &CellStream,
    options: PrettyOptions,
) -> FmtResult {
    let mut flat = Vec::new();
    flatten_cells(cells, &mut flat);
    write_flat(output, flat, options)
}

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
    let chunks = structural_chunks(&flat);
    let available = options.width.saturating_sub(8).max(24);
    let mut band = Vec::new();
    let mut used = 0;
    let mut wrote_band = false;
    for chunk in chunks {
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

        if chunk
            .last()
            .is_some_and(|token| matches!(token.kind, FlatKind::Eoc))
        {
            if wrote_band {
                writeln!(output)?;
            }
            write_token_band(output, &band, options)?;
            wrote_band = true;
            band.clear();
            used = 0;
        }
    }
    if !band.is_empty() {
        if wrote_band {
            writeln!(output)?;
        }
        write_token_band(output, &band, options)?;
    }
    Ok(())
}

fn structural_chunks(tokens: &[FlatToken]) -> Vec<&[FlatToken]> {
    let mut chunks = Vec::new();
    let mut start = 0;
    for (index, token) in tokens.iter().enumerate() {
        if matches!(
            token.kind,
            FlatKind::Open(_) | FlatKind::Close(_) | FlatKind::Eoc
        ) {
            chunks.push(&tokens[start..=index]);
            start = index + 1;
        }
    }
    if start < tokens.len() {
        chunks.push(&tokens[start..]);
    }
    chunks
}

fn token_band_width(tokens: &[FlatToken]) -> usize {
    tokens.iter().map(token_column_width).sum::<usize>() + tokens.len().saturating_sub(1)
}

fn token_column_width(token: &FlatToken) -> usize {
    visible_text(&token.text)
        .chars()
        .count()
        .max(flat_kind(token.kind).chars().count())
        .clamp(3, 24)
        + 2
}

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
        write!(output, "{}", paint(BRIGHT_YELLOW, &cell, options))?;
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
        let kind = clipped(&flat_kind(token.kind), width);
        let cell = centered_cell(&kind, width);
        write!(output, "{}", paint(token_color(token.kind), &cell, options))?;
    }
    writeln!(output)
}

fn centered_cell(text: &str, width: usize) -> String {
    let padding = width.saturating_sub(text.chars().count());
    let left = padding / 2;
    let right = padding - left;
    format!(" {}{}{} ", " ".repeat(left), text, " ".repeat(right))
}

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

fn infer_token_span(left: Span, right: Span, text: &str) -> Option<Span> {
    let (source, start) = located_end(left)?;
    let (right_source, limit) = located_start(right)?;
    if source != right_source || start.line != limit.line {
        return None;
    }
    let end = TextPoint::new(
        start.line,
        start.column.saturating_add(text.chars().count() as u32),
        start.byte.saturating_add(text.len()),
    );
    (end <= limit).then(|| Span::new(source, TextRange::new(start, end)))
}

fn located_start(span: Span) -> Option<(crate::SourceId, TextPoint)> {
    span.source().ok().zip(span.start_point().ok())
}

fn located_end(span: Span) -> Option<(crate::SourceId, TextPoint)> {
    span.source().ok().zip(span.end_point().ok())
}

fn flatten(tokens: &TokenStream, depth: usize, output: &mut Vec<FlatToken>) {
    for token in tokens.iter() {
        match token {
            TokenTree::Group(group) => {
                let kind = group.delim_kind();
                push_flat(
                    output,
                    FlatToken {
                        depth,
                        kind: FlatKind::Open(kind),
                        text: kind.opening().into(),
                        span: group.delimiter().opening_span(),
                    },
                );
                flatten(group.stream(), depth + 1, output);
                push_flat(
                    output,
                    FlatToken {
                        depth,
                        kind: FlatKind::Close(kind),
                        text: kind.closing().into(),
                        span: group.delimiter().closing_span(),
                    },
                );
            }
            TokenTree::Ident(token) => push_flat(
                output,
                FlatToken {
                    depth,
                    kind: FlatKind::Ident,
                    text: token.text().into(),
                    span: token.span(),
                },
            ),
            TokenTree::Literal(token) => push_flat(
                output,
                FlatToken {
                    depth,
                    kind: FlatKind::Literal,
                    text: token.text().into(),
                    span: token.span(),
                },
            ),
            TokenTree::Punct(token) => push_flat(
                output,
                FlatToken {
                    depth,
                    kind: FlatKind::Punct,
                    text: token.text().into(),
                    span: token.span(),
                },
            ),
            TokenTree::Trivia(token) => push_flat(
                output,
                FlatToken {
                    depth,
                    kind: FlatKind::Trivia,
                    text: token.text().into(),
                    span: token.span(),
                },
            ),
            TokenTree::Eof(span) => push_flat(
                output,
                FlatToken {
                    depth,
                    kind: FlatKind::Eof,
                    text: String::new(),
                    span: *span,
                },
            ),
        }
    }
}

fn flatten_cells(cells: &CellStream, output: &mut Vec<FlatToken>) {
    for cell in cells.iter() {
        flatten(cell.stream(), 0, output);
        if cell.delim_kind() == DelimiterKind::Semicolon {
            push_flat(
                output,
                FlatToken {
                    depth: 0,
                    kind: FlatKind::Close(DelimiterKind::Semicolon),
                    text: ";".into(),
                    span: cell.delimiter().closing_span(),
                },
            );
        }
        push_flat(
            output,
            FlatToken {
                depth: 0,
                kind: FlatKind::Eoc,
                text: "EOC".into(),
                span: cell.span(),
            },
        );
    }
}

fn push_flat(output: &mut Vec<FlatToken>, token: FlatToken) {
    if matches!(token.kind, FlatKind::Trivia)
        && let Some(previous) = output.last_mut()
        && matches!(previous.kind, FlatKind::Trivia)
        && previous.depth == token.depth
    {
        previous.text.push_str(&token.text);
        previous.span = previous.span.join(token.span);
    } else {
        output.push(token);
    }
}

fn flat_kind(kind: FlatKind) -> String {
    match kind {
        FlatKind::Open(kind) => format!("OPEN·{kind:?}"),
        FlatKind::Close(kind) => format!("CLOSE·{kind:?}"),
        FlatKind::Ident => "IDENT".into(),
        FlatKind::Literal => "LITERAL".into(),
        FlatKind::Punct => "PUNCT".into(),
        FlatKind::Trivia => "TRIVIA".into(),
        FlatKind::Eoc => "EOC".into(),
        FlatKind::Eof => "EOF".into(),
    }
}

fn token_color(kind: FlatKind) -> &'static str {
    match kind {
        FlatKind::Open(_) => OPEN_COLOR,
        FlatKind::Close(_) => CLOSE_COLOR,
        FlatKind::Ident => IDENT_COLOR,
        FlatKind::Literal => LITERAL_COLOR,
        FlatKind::Punct => PUNCT_COLOR,
        FlatKind::Trivia => TRIVIA_COLOR,
        FlatKind::Eoc => EOC_COLOR,
        FlatKind::Eof => DIM,
    }
}

fn visible_text(text: &str) -> String {
    text.chars()
        .flat_map(|character| match character {
            ' ' => "␠".chars().collect::<Vec<_>>(),
            '\t' => "⇥".chars().collect(),
            '\n' => "↵".chars().collect(),
            '\r' => "␍".chars().collect(),
            character => vec![character],
        })
        .collect()
}

fn write_tree(output: &mut impl FmtWrite, root: &PrettyNode, options: PrettyOptions) -> FmtResult {
    write_node(output, root, "", None, true, options)
}

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
    if let Some(label) = label.filter(|label| !label.is_empty()) {
        write!(
            output,
            "{} {} ",
            paint(BRIGHT_YELLOW, label, options),
            paint(DIM, "→", options)
        )?;
    }
    if let Some(detail) = &node.detail {
        write!(
            output,
            "{}  ",
            paint(BRIGHT_YELLOW, &visible_text(detail), options)
        )?;
    }
    write!(
        output,
        "{}",
        paint(node_color(node.role), &node.name, options)
    )?;
    if options.spans {
        write!(output, "  {}", paint(DIM, &format_span(node.span), options))?;
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

fn node_color(role: NodeRole) -> &'static str {
    match role {
        NodeRole::Syntax => NON_TERMINAL_BLUE,
        NodeRole::Token => BOLD_CYAN,
        NodeRole::Collection => BRIGHT_YELLOW,
        NodeRole::Absent => DIM,
    }
}

fn format_span(span: Span) -> String {
    let Ok(range) = span.range() else {
        return "@detached".into();
    };
    let (Some(start), Some(end)) = (range.start(), range.end()) else {
        return "@empty".into();
    };
    let source = span
        .source()
        .map(|source| format!("s{}·", source.0))
        .unwrap_or_default();
    if start == end {
        format!("@{source}{}", point(start))
    } else if start.line == end.line {
        format!(
            "@{source}{}:{}–{}",
            start.line + 1,
            start.column + 1,
            end.column + 1
        )
    } else {
        format!("@{source}{}–{}", point(start), point(end))
    }
}

fn point(point: TextPoint) -> String {
    format!("{}:{}", point.line + 1, point.column + 1)
}

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
    use crate::{CellBlock, SourceId, Trivia};

    #[test]
    fn adjacent_trivia_is_hidden_or_rendered_as_one_token() {
        let mut tokens = TokenStream::new();
        tokens.push_trivia(Trivia::new(
            crate::TriviaKind::Whitespace,
            " ",
            Span::detached(),
        ));
        tokens.push_trivia(Trivia::new(
            crate::TriviaKind::LineBreak,
            "\n",
            Span::detached(),
        ));

        let cells = CellStream::new(
            vec![CellBlock::new(
                Delimiter::new(DelimiterKind::Empty, Span::detached()),
                tokens,
            )],
            SourceId(1),
        );

        let hidden = format_cells(&cells, PrettyOptions::PLAIN);
        assert!(!hidden.contains("TRIVIA"));
        assert!(hidden.contains("EOC"));
        let shown = format_cells(
            &cells,
            PrettyOptions {
                trivia: true,
                ..PrettyOptions::PLAIN
            },
        );
        assert_eq!(shown.matches("TRIVIA").count(), 1);
        assert!(shown.contains("␠↵"));
    }

    #[test]
    fn token_bands_end_at_cells_and_use_distinct_class_colors() {
        let cells = crate::lex_str("name = 1\nnext", SourceId(2)).unwrap();
        let plain = format_cells(&cells, PrettyOptions::PLAIN);

        assert_eq!(plain.matches("token │").count(), 2);
        assert_eq!(plain.matches(" EOC ").count(), 4);

        let colored = format_cells(&cells, PrettyOptions::ANSI);
        assert!(colored.contains(&format!("{BRIGHT_YELLOW} name ")));
        for color in [IDENT_COLOR, PUNCT_COLOR, LITERAL_COLOR, EOC_COLOR] {
            assert!(colored.contains(color));
        }
    }

    #[test]
    fn token_cells_are_centered() {
        assert_eq!(centered_cell("x", 5), "   x   ");
        assert_eq!(centered_cell("xy", 5), "  xy   ");
    }

    #[test]
    fn enum_presentation_is_transparent() {
        let value = PrettyNode::token("Symbol", "x", Span::detached());
        let rendered = PrettyNode::variant("Expr", "Symbol", Span::detached(), value);

        assert_eq!(rendered.name, "Symbol");
    }
}
