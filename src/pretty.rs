//! Terminal-oriented inspection of source, raw tokens, and typed syntax trees.
//!
//! [`PrettyReport`] combines the three useful views of a parsed file. The
//! renderer deliberately owns only presentation data; the generated typed CST
//! remains the source of truth and implements [`PrettyTree`] directly.

use std::fmt::{Display, Formatter, Result as FmtResult, Write as FmtWrite};
use std::io::{self, IsTerminal as _, Write as IoWrite};

use crate::{
    DelimiterKind, DoubleSpan, Punctuated, Span, Spanned, TokenStream, TokenTree,
    token_stream::delim::Delimiter,
};

const RESET: &str = "\u{1b}[0m";
const BOLD_CYAN: &str = "\u{1b}[1;36m";
const BRIGHT_YELLOW: &str = "\u{1b}[93m";
const BRIGHT_BLACK: &str = "\u{1b}[90m";
const RED: &str = "\u{1b}[31m";
const GREEN: &str = "\u{1b}[32m";
const YELLOW: &str = "\u{1b}[33m";
const BLUE: &str = "\u{1b}[34m";
const MAGENTA: &str = "\u{1b}[35m";
const CYAN: &str = "\u{1b}[36m";
const WHITE: &str = "\u{1b}[37m";
const NON_TERMINAL_BLUE: &str = BLUE;
const DIM: &str = "\u{1b}[2m";

/// Converts typed syntax into the structural presentation used by the pretty
/// printer.
///
/// Implementations for generated nodes are emitted from `syntax!`, so
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
    detail: Option<PrettyDetail>,
    span: Option<Span>,
    role: NodeRole,
    children: Vec<PrettyEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrettyDetail {
    text: String,
    role: DetailRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailRole {
    Text,
    Glyph,
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
    pub fn syntax(name: impl Into<String>, span: Option<Span>) -> Self {
        Self::new(name, span, NodeRole::Syntax)
    }

    /// Creates a generated enum-variant node.
    #[doc(hidden)]
    pub fn variant(
        _category: &str,
        _variant: &str,
        span: Option<Span>,
        mut value: PrettyNode,
    ) -> Self {
        value.span = span;
        value
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
            Self::token("Token", kind.opening(), delimiter.opening_span()),
        );
        node.push_node(
            "closing",
            Self::token("Token", kind.closing(), delimiter.closing_span()),
        );
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
        let role = if matches!(text.as_str(), "∅" | "EOC") {
            DetailRole::Glyph
        } else {
            DetailRole::Text
        };
        let mut node = Self::new(name, Some(span), NodeRole::Token);
        node.detail = Some(PrettyDetail { text, role });
        node
    }

    /// Creates a collection wrapper.
    #[doc(hidden)]
    pub fn collection(name: impl Into<String>, span: Option<Span>) -> Self {
        Self::new(name, span, NodeRole::Collection)
    }

    /// Creates an absent optional value.
    #[doc(hidden)]
    pub fn absent() -> Self {
        Self::new("None", None, NodeRole::Absent)
    }

    fn new(name: impl Into<String>, span: Option<Span>, role: NodeRole) -> Self {
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
        if node.span.is_none() {
            node.span.clone_from(&self.span);
        }
        self.children.push(PrettyEdge {
            label: label.into(),
            node,
        });
    }
}

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

impl<T, P> PrettyTree for Punctuated<T, P>
where
    T: PrettyTree + Spanned,
    P: PrettyTree + Spanned,
{
    fn pretty_tree(&self) -> PrettyNode {
        let mut node =
            PrettyNode::collection(item_count("Punctuated", self.len()), self.try_span());
        node.push("[0]", self.first());
        for (index, (punctuation, value)) in self.rest().iter().enumerate() {
            node.push("punctuation", punctuation);
            node.push(format!("[{}]", index + 1), value);
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
    source: &'source str,
    tokens: TokenStream,
    cst: PrettyNode,
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

/// Formats one typed syntax value as a Unicode tree without terminal colors.
pub fn format_pretty_tree(value: &impl PrettyTree) -> String {
    format_tree(&value.pretty_tree(), PrettyOptions::PLAIN)
}

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
enum FlatKind {
    Eoc,
    Open,
    Close,
    Ident,
    Literal,
    Punct,
    Trivia,
}

#[derive(Clone)]
struct FlatToken {
    depth: usize,
    kind: FlatKind,
    text: String,
    span: Span,
}

pub(crate) fn format_tokens(tokens: &TokenStream, options: PrettyOptions) -> String {
    let mut output = String::new();
    write_tokens(&mut output, tokens, options).expect("writing to a string cannot fail");
    output
}

/// Formats a raw token stream as a table with explicit group and cell markers.
pub fn format_pretty_tokens(tokens: &TokenStream) -> String {
    format_tokens(tokens, PrettyOptions::PLAIN)
}

fn write_tokens(
    output: &mut impl FmtWrite,
    tokens: &TokenStream,
    options: PrettyOptions,
) -> FmtResult {
    let mut flat = Vec::new();
    flatten(tokens, 0, &mut flat);
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
        if matches!(chunk.last().map(|token| token.kind), Some(FlatKind::Eoc)) {
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

fn flatten(tokens: &TokenStream, depth: usize, output: &mut Vec<FlatToken>) {
    for token in tokens.iter() {
        match token {
            TokenTree::Eoc(token) => push_flat(
                output,
                FlatToken {
                    depth,
                    kind: FlatKind::Eoc,
                    text: "EOC".into(),
                    span: token.expect_span(),
                },
            ),
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
                push_flat(
                    output,
                    FlatToken {
                        depth,
                        kind: FlatKind::Close,
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
                    kind: FlatKind::Punct,
                    text: token.text().into(),
                    span: token.expect_span(),
                },
            ),
            TokenTree::Trivia(token) => push_flat(
                output,
                FlatToken {
                    depth,
                    kind: FlatKind::Trivia,
                    text: token.text().into(),
                    span: token.expect_span(),
                },
            ),
        }
    }
}

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

fn flat_kind(kind: FlatKind) -> &'static str {
    match kind {
        FlatKind::Eoc => "EOC",
        FlatKind::Open => "DELIM->",
        FlatKind::Close => "<-DELIM",
        FlatKind::Ident => "IDENT",
        FlatKind::Literal => "LITERAL",
        FlatKind::Punct => "PUNCT",
        FlatKind::Trivia => "TRIVIA",
    }
}

fn token_color(kind: FlatKind) -> &'static str {
    match kind {
        FlatKind::Eoc => BRIGHT_BLACK,
        FlatKind::Open => BLUE,
        FlatKind::Close => MAGENTA,
        FlatKind::Ident => CYAN,
        FlatKind::Literal => YELLOW,
        FlatKind::Punct => GREEN,
        FlatKind::Trivia => RED,
    }
}

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

fn node_color(role: NodeRole) -> &'static str {
    match role {
        NodeRole::Syntax => NON_TERMINAL_BLUE,
        NodeRole::Token => BOLD_CYAN,
        NodeRole::Collection => BRIGHT_YELLOW,
        NodeRole::Absent => DIM,
    }
}

fn detail_color(role: DetailRole) -> &'static str {
    match role {
        DetailRole::Text => WHITE,
        DetailRole::Glyph => BRIGHT_BLACK,
    }
}

fn format_span(span: &Span) -> String {
    let Some((start, end)) = span.bounds() else {
        return "@unlocated".into();
    };
    // Byte offsets rather than `line:column`, because turning an offset into a
    // line and column is a question about the buffer's contents and the pretty
    // printer does not hold the buffer.
    if start == end {
        format!("@{start}")
    } else {
        format!("@{start}–{end}")
    }
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
    use crate::lex_str;

    #[test]
    fn adjacent_trivia_is_hidden_or_rendered_as_one_token() {
        let source = " \n";
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
        assert!(shown.contains("' '"));
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
        for color in [BRIGHT_BLACK, RED, GREEN, YELLOW, BLUE, MAGENTA, CYAN] {
            assert!(colored.contains(color));
        }
    }

    #[test]
    fn eoc_starts_a_new_token_band() {
        let source = "first\nsecond";
        let tokens = lex_str(source).unwrap();
        let rendered = format_tokens(&tokens, PrettyOptions::PLAIN);

        assert_eq!(rendered.matches("token │").count(), 2);
        let bands = rendered.split("\n\n").collect::<Vec<_>>();
        assert_eq!(bands.len(), 2);
        assert!(bands[0].contains("EOC"));
        assert!(bands[1].contains("second"));
    }

    #[test]
    fn token_cells_are_centered() {
        assert_eq!(centered_cell("x", 5), "   x   ");
        assert_eq!(centered_cell("xy", 5), "  xy   ");
    }

    #[test]
    fn enum_presentation_is_transparent() {
        let span = Span::for_text("x");
        let value = PrettyNode::token("Symbol", "x", span.clone());
        let rendered = PrettyNode::variant("Expr", "Symbol", Some(span), value);

        assert_eq!(rendered.name, "Symbol");
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

        assert_eq!(delimiter.role, NodeRole::Collection);
        assert_eq!(delimiter.name, "(…) · 2 tokens");
        assert!(!format_tree(&delimiter, PrettyOptions::PLAIN).contains("Delimiter"));
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
