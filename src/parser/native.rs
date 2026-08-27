//! Direct construction of the typed graph from a flat token stream.
//!
//! The native precedence engine owns parsing decisions only. Token storage,
//! cursor position, trivia skipping, and newline detection belong to
//! [`ParseStream`], shared with generated [`Parse`]
//! implementations.

use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::cell_stream::{end_span, ends_with_cell_semicolon, frame_recovering};
use crate::lexer::canonical_word_spelling;
use crate::nodes::*;
use crate::parse::{Lookahead, SignificantToken};
use crate::{
    CellStream, CellStreamError, CommentTrivia, Delimited, DelimiterKind, DelimiterToken,
    DelimiterValue, LexError, LexErrorKind, LiteralKind, Parse, ParseStream, Parser, Punctuated,
    Recovered, Span, Spanned, Terminal, Token, TokenParseError, TokenStream, TokenTree, Tokenize,
    TrailingTrivia, lex_str, lex_str_recovering,
};

macro_rules! is_token {
    ($token:expr, $($pattern:tt)*) => {
        <Token![$($pattern)*] as Token>::matches_token_tree($token)
    };
}

macro_rules! unexpected {
    ($token:expr, $expected:expr) => {{
        let token = $token;
        NativeParseError::Unexpected {
            span: token.expect_span(),
            found: token.description(),
            expected: $expected,
        }
    }};
}

/// The built-in parser implemented directly from Macaulay2's `P/B/U` table.
#[derive(Debug)]
pub struct NativeParser;

impl Parser for NativeParser {
    type Error = NativeParseError;

    fn parse_cells(&mut self, cells: CellStream) -> Result<SourceFile, Self::Error> {
        parse_cell_stream(cells)
    }
}

/// Parses a complete source file with the native precedence engine.
///
/// # Errors
///
/// Returns [`NativeParseError`] when lexing fails or the token sequence is not
/// valid complete Macaulay2 syntax.
pub fn parse_native(source: &str) -> Result<SourceFile, NativeParseError> {
    let tokens = lex_str(source).map_err(NativeParseError::Lex)?;
    let cells = tokenize_for_native_parser(tokens)?;
    NativeParser.parse_cells(cells)
}

/// Parses source while retaining valid syntax inside unclosed delimiters.
///
/// Missing closing delimiters become zero-width closing spans at end of input
/// and are returned as non-fatal lexical diagnostics. Other lexical or
/// grammatical failures remain fatal.
///
/// # Errors
///
/// Returns [`NativeParseError`] when source cannot be recovered into a complete
/// typed file.
pub fn parse_native_recovering(
    source: &str,
) -> Result<Recovered<SourceFile, LexError>, NativeParseError> {
    let recovered = lex_str_recovering(source).map_err(NativeParseError::Lex)?;
    let (tokens, diagnostics) = recovered.into_parts();
    let source = tokens.source();
    let framed = frame_recovering(tokens);
    let cells = CellStream::from_cells_with_source(
        framed.cells.into_iter().chain(framed.incomplete),
        source,
    );
    let syntax = NativeParser.parse_cells(cells)?;
    Ok(Recovered::new(syntax, diagnostics))
}

/// The current state of an append-oriented native cell parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NativeCellState {
    /// No submitted syntax is waiting and no completed cell is queued.
    Empty,
    /// The current syntax is complete or a completed shell cell is queued.
    Complete,
    /// The active source is a prefix that more input may complete.
    Incomplete,
    /// The active source contains an error that appended input cannot repair.
    Invalid,
}

/// One completed native cell together with its exact submitted source.
#[derive(Debug)]
pub struct CompletedNativeCell {
    source: Span,
    syntax: crate::nodes::Cell,
}

impl CompletedNativeCell {
    /// Returns the exact source submitted for this evaluation unit.
    ///
    /// # Panics
    ///
    /// Panics if the internally retained source span cannot resolve its text.
    pub fn source(&self) -> &str {
        self.source
            .text()
            .expect("completed cell source spans must resolve to text")
    }

    /// Returns the parsed typed cell.
    pub fn syntax(&self) -> &crate::nodes::Cell {
        &self.syntax
    }

    /// Separates the exact source span from the typed cell.
    pub fn into_parts(self) -> (Span, crate::nodes::Cell) {
        (self.source, self.syntax)
    }
}

/// A retained native parser for line-oriented shell input.
///
/// Each call to [`Self::push_line`] appends one submitted physical line. Cells
/// completed by a top-level newline or semicolon are removed from the active
/// buffer and never parsed again. Only the unfinished tail is lexed and parsed
/// on later submissions.
///
/// Missing delimiter closers and parser requirements at end of input are
/// classified as [`NativeCellState::Incomplete`]. A recoverable typed cell is
/// available through [`Self::active_syntax`] when synthetic delimiter closers
/// were enough to parse its contents.
///
/// ```
/// use m2_syn::{Lex, NativeCellParser, NativeCellState};
///
/// let mut parser = NativeCellParser::new();
/// assert_eq!(parser.push_line("f = x -> ("), NativeCellState::Incomplete);
/// assert!(parser.active_syntax().is_some());
/// assert_eq!(parser.push_line("x + 1"), NativeCellState::Incomplete);
/// assert_eq!(parser.push_line(")"), NativeCellState::Complete);
///
/// let cells = parser.take_completed();
/// assert_eq!(cells[0].syntax().to_m2(), "f = x -> (\nx + 1\n)");
/// ```
#[derive(Debug)]
pub struct NativeCellParser {
    active_source: String,
    active_tokens: TokenStream,
    active_syntax: Option<crate::nodes::Cell>,
    completed: Vec<CompletedNativeCell>,
    lexer_diagnostics: Vec<LexError>,
    parse_error: Option<NativeParseError>,
    state: NativeCellState,
}

impl Default for NativeCellParser {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeCellParser {
    /// Creates an empty native shell parser.
    pub fn new() -> Self {
        Self {
            active_source: String::new(),
            active_tokens: TokenStream::new(),
            active_syntax: None,
            completed: Vec::new(),
            lexer_diagnostics: Vec::new(),
            parse_error: None,
            state: NativeCellState::Empty,
        }
    }

    /// Appends one submitted physical line and refreshes the active cell.
    ///
    /// A line feed is appended when `line` does not already end in one. This is
    /// the submission boundary: a syntactically complete final expression may
    /// become a completed cell, while an operator, required clause, or open
    /// delimiter keeps the cell active.
    pub fn push_line(&mut self, line: &str) -> NativeCellState {
        self.active_source.push_str(line);
        if !line.ends_with('\n') {
            self.active_source.push('\n');
        }
        self.refresh()
    }

    /// Returns the current completion classification.
    pub fn state(&self) -> NativeCellState {
        self.state
    }

    /// Returns the exact source still belonging to the unfinished cell.
    pub fn active_source(&self) -> &str {
        &self.active_source
    }

    /// Returns recovered tokens for the unfinished cell.
    pub fn active_tokens(&self) -> &TokenStream {
        &self.active_tokens
    }

    /// Returns recovered typed syntax for the unfinished cell when available.
    pub fn active_syntax(&self) -> Option<&crate::nodes::Cell> {
        self.active_syntax.as_ref()
    }

    /// Returns lexer diagnostics for the active cell.
    pub fn lexer_diagnostics(&self) -> &[LexError] {
        &self.lexer_diagnostics
    }

    /// Returns the parser failure for an incomplete or invalid active cell.
    pub fn parse_error(&self) -> Option<&NativeParseError> {
        self.parse_error.as_ref()
    }

    /// Returns completed evaluation units that have not yet been taken.
    pub fn completed(&self) -> &[CompletedNativeCell] {
        &self.completed
    }

    /// Takes every completed evaluation unit without disturbing an active cell.
    pub fn take_completed(&mut self) -> Vec<CompletedNativeCell> {
        let completed = std::mem::take(&mut self.completed);
        if self.state == NativeCellState::Complete {
            self.state = NativeCellState::Empty;
        }
        completed
    }

    /// Discards the unfinished cell and its diagnostics.
    pub fn reset_active(&mut self) {
        self.active_source.clear();
        self.active_tokens = TokenStream::new();
        self.active_syntax = None;
        self.lexer_diagnostics.clear();
        self.parse_error = None;
        self.state = if self.completed.is_empty() {
            NativeCellState::Empty
        } else {
            NativeCellState::Complete
        };
    }

    fn refresh(&mut self) -> NativeCellState {
        self.active_tokens = TokenStream::new();
        self.active_syntax = None;
        self.lexer_diagnostics.clear();
        self.parse_error = None;

        loop {
            let recovered = match lex_str_recovering(&self.active_source) {
                Ok(recovered) => recovered,
                Err(error) => {
                    self.state = if incomplete_lex_error(error.kind()) {
                        NativeCellState::Incomplete
                    } else {
                        NativeCellState::Invalid
                    };
                    self.lexer_diagnostics.push(error);
                    return self.state;
                }
            };
            let (tokens, diagnostics) = recovered.into_parts();
            let framed = frame_recovering(tokens.clone());
            let has_incomplete = framed.incomplete.is_some();
            let mut cells = framed.cells.into_iter().map(|cell| (cell, true));
            let first = cells
                .next()
                .or_else(|| framed.incomplete.map(|cell| (cell, false)));
            let Some((cell, cell_complete)) = first else {
                self.active_tokens = tokens;
                self.lexer_diagnostics = diagnostics;
                self.state = if self.completed.is_empty() {
                    NativeCellState::Empty
                } else {
                    NativeCellState::Complete
                };
                return self.state;
            };

            let has_following_cell = cells.next().is_some() || has_incomplete;
            let ended_by_semicolon = ends_with_cell_semicolon(&cell);
            let boundary = end_span(&cell);
            let end = boundary.end().unwrap_or(self.active_source.len());
            let source = Span::ranged(std::sync::Arc::clone(boundary.source()), 0..end);

            match parse_raw_cell(cell) {
                Ok(Some(syntax)) if cell_complete && has_following_cell => {
                    self.completed.push(CompletedNativeCell { source, syntax });
                    self.active_source.replace_range(..end, "");
                }
                Ok(Some(syntax)) if cell_complete && diagnostics.is_empty() => {
                    self.completed.push(CompletedNativeCell { source, syntax });
                    self.active_source.clear();
                    self.active_tokens = TokenStream::new();
                    self.state = NativeCellState::Complete;
                    return self.state;
                }
                Ok(Some(syntax)) => {
                    self.active_tokens = tokens;
                    self.active_syntax = Some(syntax);
                    self.lexer_diagnostics = diagnostics;
                    self.state = NativeCellState::Incomplete;
                    return self.state;
                }
                Ok(None) => {
                    self.active_tokens = tokens;
                    self.lexer_diagnostics = diagnostics;
                    self.state = if self.completed.is_empty() {
                        NativeCellState::Empty
                    } else {
                        NativeCellState::Complete
                    };
                    return self.state;
                }
                Err(error) => {
                    let can_append_to_cell =
                        !cell_complete || (!has_following_cell && !ended_by_semicolon);
                    self.state = if can_append_to_cell && incomplete_parse_error(&error) {
                        NativeCellState::Incomplete
                    } else {
                        NativeCellState::Invalid
                    };
                    self.active_tokens = tokens;
                    self.lexer_diagnostics = diagnostics;
                    self.parse_error = Some(error);
                    return self.state;
                }
            }
        }
    }
}

pub(crate) fn incomplete_lex_error(kind: LexErrorKind) -> bool {
    matches!(
        kind,
        LexErrorKind::UnterminatedGroup
            | LexErrorKind::UnterminatedString
            | LexErrorKind::UnterminatedRawString
            | LexErrorKind::UnterminatedBlockComment
    )
}

pub(crate) fn incomplete_parse_error(error: &NativeParseError) -> bool {
    match error {
        NativeParseError::MissingOperand { .. } => true,
        NativeParseError::Unexpected { found, .. } => found == "end of input",
        NativeParseError::Lex(error) => incomplete_lex_error(error.kind()),
        NativeParseError::Tokenize(_) => true,
        NativeParseError::NewlineInApplication { .. }
        | NativeParseError::InvalidAssignmentTarget { .. }
        | NativeParseError::InvalidLambdaParameters { .. }
        | NativeParseError::Unsupported { .. } => false,
    }
}

#[derive(Debug, Spanned)]
/// An error produced by the native lexer or precedence parser.
pub enum NativeParseError {
    /// Tokenization failed before parsing began.
    Lex(LexError),
    /// The raw token stream could not be partitioned into complete cells.
    Tokenize(CellStreamError),
    /// The parser encountered a token that cannot occur in this position.
    Unexpected {
        /// A readable description of the received token.
        found: String,
        /// A readable description of the required syntax.
        expected: &'static str,
        /// The received token or end-of-input position.
        span: Span,
    },
    /// A prefix or binary operator has no operand.
    MissingOperand {
        /// The operator spelling.
        operator: String,
        /// The operator or end-of-input position.
        span: Span,
    },
    /// Implicit function application crossed a top-level line break.
    NewlineInApplication {
        /// The token following the line break.
        span: Span,
    },
    /// The left side of an assignment is not assignable.
    InvalidAssignmentTarget {
        /// The assignment operator span.
        span: Span,
    },
    /// The left side of a lambda arrow is not a parameter form.
    InvalidLambdaParameters {
        /// The lambda arrow span.
        span: Span,
    },
    /// The grammar recognizes syntax that this native backend cannot lower yet.
    Unsupported {
        /// The unsupported syntax spelling.
        syntax: String,
        /// The unsupported token span.
        span: Span,
    },
}

impl NativeParseError {
    fn unexpected_end(expected: &'static str, span: Span) -> Self {
        Self::Unexpected {
            found: "end of input".into(),
            expected,
            span,
        }
    }

    fn missing_operand_for(self, operator: &TokenTree) -> Self {
        if matches!(self, Self::MissingOperand { .. }) {
            Self::MissingOperand {
                operator: operator.description(),
                span: operator.expect_span(),
            }
        } else {
            self
        }
    }
}

impl Display for NativeParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lex(error) => error.fmt(formatter),
            Self::Tokenize(error) => error.fmt(formatter),
            Self::Unexpected {
                found, expected, ..
            } => write!(formatter, "expected {expected}, found `{found}`"),
            Self::MissingOperand { operator, .. } => {
                write!(
                    formatter,
                    "expected an operand after `{operator}`, found end of input"
                )
            }
            Self::NewlineInApplication { .. } => {
                formatter.write_str("implicit application cannot cross a line break")
            }
            Self::InvalidAssignmentTarget { .. } => {
                formatter.write_str("invalid assignment target")
            }
            Self::InvalidLambdaParameters { .. } => {
                formatter.write_str("invalid lambda parameter list")
            }
            Self::Unsupported { syntax, .. } => {
                write!(
                    formatter,
                    "native parsing of `{syntax}` is not implemented yet"
                )
            }
        }
    }
}

impl Error for NativeParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Lex(error) => Some(error),
            Self::Tokenize(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
/// Parser-local instruction for interpreting a token in unary position.
///
/// This is not a syntax category and is never stored in the token stream or
/// typed CST. [`parse_info`] derives it temporarily from the current raw token
/// so the precedence engine can select the corresponding parsing routine.
enum UnaryAction {
    Atom,
    Prefix,
    Statement,
    Control,
    Debug,
    Delimiter,
    Quote,
    Error,
}

#[derive(Debug, Clone, Copy)]
struct ParseInfo {
    precedence: u8,
    binary_strength: Option<u8>,
    unary_strength: Option<u8>,
    unary: UnaryAction,
    postfix: bool,
}

impl From<TokenParseRule> for ParseInfo {
    fn from(rule: TokenParseRule) -> Self {
        match rule {
            TokenParseRule::Precedence(precedence) => Self::stop(precedence),
            TokenParseRule::Postfix(precedence) => Self::postfix(precedence),
            TokenParseRule::Binary(precedence, strength) => Self::binary(precedence, strength),
            TokenParseRule::Prefix(precedence, strength) => {
                Self::unary(precedence, UnaryAction::Prefix, strength)
            }
            TokenParseRule::PrefixBinary(precedence, binary, unary) => {
                Self::prefix_binary(precedence, binary, unary)
            }
            TokenParseRule::Unary(precedence, strength) => {
                Self::unary(precedence, UnaryAction::Error, strength)
            }
        }
    }
}

impl ParseInfo {
    const fn atom(precedence: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: None,
            unary: UnaryAction::Atom,
            postfix: false,
        }
    }

    const fn binary(precedence: u8, strength: u8) -> Self {
        Self {
            precedence,
            binary_strength: Some(strength),
            unary_strength: None,
            unary: UnaryAction::Error,
            postfix: false,
        }
    }

    const fn prefix_binary(precedence: u8, binary_strength: u8, unary_strength: u8) -> Self {
        Self {
            precedence,
            binary_strength: Some(binary_strength),
            unary_strength: Some(unary_strength),
            unary: UnaryAction::Prefix,
            postfix: false,
        }
    }

    /// A token whose action in unary position is to introduce something that
    /// binds at `strength`: a statement, a control form, a timing form, or a
    /// quoted name.
    const fn unary(precedence: u8, unary: UnaryAction, strength: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: Some(strength),
            unary,
            postfix: false,
        }
    }

    const fn postfix(precedence: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: None,
            unary: UnaryAction::Error,
            postfix: true,
        }
    }

    const fn delimiter(precedence: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: Some(PREC_CLOSER),
            unary: UnaryAction::Delimiter,
            postfix: false,
        }
    }

    const fn stop(precedence: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: None,
            unary: UnaryAction::Error,
            postfix: false,
        }
    }
}

struct Engine {
    input: ParseStream,
}

impl Engine {
    fn new(tokens: TokenStream) -> Self {
        Self {
            input: ParseStream::new(tokens),
        }
    }

    fn parse_cell(
        mut self,
        semicolon: Option<Token![;]>,
        eoc_span: Span,
    ) -> Result<Option<crate::nodes::Cell>, NativeParseError> {
        self.skip_trivia();
        if matches!(self.peek(), Lookahead::End(_)) {
            return match semicolon {
                None => Ok(None),
                Some(semicolon) => Err(NativeParseError::Unexpected {
                    found: ";".into(),
                    expected: "a statement before `;`",
                    span: semicolon.expect_span(),
                }),
            };
        }

        let (components, _) = self.parse_components()?;

        if let Lookahead::Token(token) = self.peek() {
            return Err(unexpected!(token.token, "the end of the cell"));
        }

        let eoc = EOC::new("", eoc_span);
        Ok(Some(crate::nodes::Cell::new(
            ExprPack::new(components),
            semicolon,
            eoc,
        )))
    }

    fn parse_components(&mut self) -> Result<(Punctuated<Component>, bool), NativeParseError> {
        let first = if self.at_punctuation(",") {
            Component::Empty(Empty::new("", empty_span_at(&self.peek().expect_span())))
        } else {
            self.parse_required(PREC_COMMA)?.into()
        };
        let mut elements = Punctuated::from_single(first);
        let mut has_comma = false;

        while let Some(comma) = self.take_punctuation(",") {
            has_comma = true;
            let next = if self.at_end() || self.at_punctuation(";") || self.at_punctuation(",") {
                Component::Empty(Empty::new("", empty_span_at(&self.peek().expect_span())))
            } else {
                self.parse_required(PREC_COMMA)?.into()
            };
            elements.push(typed_token(comma), next);
        }

        Ok((elements, has_comma))
    }

    fn parse_optional_components(
        &mut self,
    ) -> Result<Option<(Punctuated<Component>, bool)>, NativeParseError> {
        if self.at_end() || self.at_punctuation(";") {
            Ok(None)
        } else {
            self.parse_components().map(Some)
        }
    }

    fn parse_required(&mut self, level: u8) -> Result<Expr, NativeParseError> {
        let token = match self.consume() {
            Lookahead::Token(token) => token,
            Lookahead::End(span) => {
                return Err(NativeParseError::MissingOperand {
                    operator: "end of input".into(),
                    span,
                });
            }
        };
        self.parse_consumed(token, level)
    }

    fn parse_nullable(&mut self, level: u8) -> Result<Option<Expr>, NativeParseError> {
        let lookahead = self.peek();
        let Lookahead::Token(token) = lookahead else {
            return Ok(None);
        };
        if parse_info(&token.token).precedence <= level {
            return Ok(None);
        }
        let Lookahead::Token(token) = self.consume() else {
            unreachable!("the token just peeked is consumable")
        };
        self.parse_consumed(token, level).map(Some)
    }

    fn parse_consumed(
        &mut self,
        token: SignificantToken,
        level: u8,
    ) -> Result<Expr, NativeParseError> {
        let info = parse_info(&token.token);
        let result = match info.unary {
            UnaryAction::Atom => lower_atom(token)?,
            UnaryAction::Prefix => {
                let strength = info.unary_strength.expect("prefix operators have U");
                let operand = self.parse_required(level.max(strength))?;
                let span = token.token.expect_span();
                let spelling = token.token.description();
                let operator =
                    PrefixOperator::from_token_tree_with_trivia(token.token, token.trailing_trivia)
                        .ok_or({
                            NativeParseError::Unsupported {
                                syntax: spelling,
                                span,
                            }
                        })?;
                PrefixExpr::new(operator, operand).into()
            }
            UnaryAction::Statement => {
                let strength = info
                    .unary_strength
                    .expect("prefix statements have a unary binding strength");
                let operand = if is_token!(&token.token, break)
                    || is_token!(&token.token, continue)
                    || is_token!(&token.token, return)
                {
                    self.parse_nullable(level.max(strength))?
                } else {
                    self.parse_required(level.max(strength))
                        .map_err(|error| error.missing_operand_for(&token.token))?
                        .into()
                };
                lower_prefix_statement(token, operand)
            }
            UnaryAction::Control => {
                if is_token!(&token.token, if) {
                    self.parse_if_statement(token)?
                } else if is_token!(&token.token, for) {
                    self.parse_for_loop(token)?
                } else if is_token!(&token.token, while) {
                    self.parse_while_loop(token)?
                } else if is_token!(&token.token, try) {
                    self.parse_try_statement(token)?
                } else if is_token!(&token.token, new) {
                    self.parse_new_statement(token)?
                } else {
                    unreachable!("control actions are assigned only to control keywords")
                }
            }
            UnaryAction::Debug => self.parse_debug_clause(token)?,
            UnaryAction::Delimiter => parse_delimited(token)?,
            UnaryAction::Quote => {
                let operator = token.token.description();
                let quoted = match self.consume() {
                    Lookahead::Token(quoted) => quoted,
                    Lookahead::End(span) => {
                        return Err(NativeParseError::MissingOperand { operator, span });
                    }
                };
                let specifier = if is_token!(&token.token, symbol) {
                    QuoteSpecifier::Symbol(typed_token(token))
                } else if is_token!(&token.token, local) {
                    QuoteSpecifier::Local(typed_token(token))
                } else if is_token!(&token.token, global) {
                    QuoteSpecifier::Global(typed_token(token))
                } else if is_token!(&token.token, threadVariable) {
                    QuoteSpecifier::ThreadVariable(typed_token(token))
                } else if is_token!(&token.token, threadLocal) {
                    QuoteSpecifier::ThreadLocal(typed_token(token))
                } else {
                    unreachable!("quote actions are assigned only to quote specifiers")
                };
                let quoted_span = quoted.token.expect_span();
                let Some(quoted_text) = quoted.token.spelling().map(str::to_owned) else {
                    return Err(unexpected!(quoted.token, "a quoted token"));
                };
                let quoted = typed_terminal(
                    Symbol::new(quoted_text, quoted_span),
                    quoted.trailing_trivia,
                );
                QuoteExpression::new(specifier, quoted).into()
            }
            UnaryAction::Error => {
                return Err(unexpected!(token.token, "an expression"));
            }
        };
        self.accumulate(result, level)
    }

    fn parse_if_statement(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let condition = self.required_clause_expression(PREC_CONTROL, &keyword.token)?;
        let then_keyword = self.consume_exact("then")?;
        let then_value = self.required_clause_expression(PREC_CONTROL, &then_keyword.token)?;
        let then_clause = ThenClause::new(typed_token(then_keyword), then_value);
        let else_clause = self
            .take_exact("else")
            .map(|else_keyword| {
                let value = self.required_clause_expression(PREC_CONTROL, &else_keyword.token)?;
                Ok::<_, NativeParseError>(ElseClause::new(typed_token(else_keyword), value))
            })
            .transpose()?;
        Ok(IfStatement::new(typed_token(keyword), condition, then_clause, else_clause).into())
    }

    fn parse_for_loop(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let variable = self.consume_symbol("a loop variable")?;

        let iteration_domain = if let Some(token) = self.take_exact("in") {
            let collection = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
            Some(IterIn::new(typed_token(token), collection).into())
        } else {
            let lower_bound = self
                .take_exact("from")
                .map(|token| {
                    let value = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
                    Ok::<_, NativeParseError>(RangeBoundFrom::new(typed_token(token), value))
                })
                .transpose()?;
            let upper_bound = self
                .take_exact("to")
                .map(|token| {
                    let value = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
                    Ok::<_, NativeParseError>(RangeBoundTo::new(typed_token(token), value))
                })
                .transpose()?;
            (lower_bound.is_some() || upper_bound.is_some())
                .then(|| IterRange::new(lower_bound, upper_bound).into())
        };

        let when_condition = if let Some(token) = self.take_exact("when") {
            let condition = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
            Some(WhenCondition::new(typed_token(token), condition))
        } else {
            None
        };
        let body = self.parse_loop_body()?;

        Ok(ForLoop::new(
            typed_token(keyword),
            variable,
            iteration_domain,
            when_condition,
            body,
        )
        .into())
    }

    fn parse_while_loop(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let condition = self.required_clause_expression(PREC_CONTROL, &keyword.token)?;
        let body = self.parse_loop_body()?;
        Ok(WhileLoop::new(typed_token(keyword), condition, body).into())
    }

    fn parse_loop_body(&mut self) -> Result<LoopBody, NativeParseError> {
        let (list_step, do_step) = if let Some(token) = self.take_exact("list") {
            let listed_value = self.required_clause_expression(PREC_CONTROL, &token.token)?;
            let list_step = Some(ListStep::new(typed_token(token), listed_value));
            if let Some(token) = self.take_exact("do") {
                let ignored_value = self.required_clause_expression(PREC_CONTROL, &token.token)?;
                (
                    list_step,
                    Some(DoStep::new(typed_token(token), ignored_value)),
                )
            } else {
                (list_step, None)
            }
        } else if let Some(token) = self.take_exact("do") {
            let ignored_value = self.required_clause_expression(PREC_CONTROL, &token.token)?;
            (None, Some(DoStep::new(typed_token(token), ignored_value)))
        } else {
            return Err(self.expected_at_lookahead("`list` or `do`"));
        };

        Ok(LoopBody::new(list_step, do_step))
    }

    fn parse_try_statement(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let value = self.required_clause_expression(PREC_CONTROL, &keyword.token)?;
        let then_clause = self
            .take_exact("then")
            .map(|then_keyword| {
                let value = self.required_clause_expression(PREC_CONTROL, &then_keyword.token)?;
                Ok::<_, NativeParseError>(ThenClause::new(typed_token(then_keyword), value))
            })
            .transpose()?;

        let fallback = if let Some(else_keyword) = self.take_exact("else") {
            let value = self.required_clause_expression(PREC_CONTROL, &else_keyword.token)?;
            Some(TryFallback::ElseClause(ElseClause::new(
                typed_token(else_keyword),
                value,
            )))
        } else if let Some(except_keyword) = self.take_exact("except") {
            let exception = self.consume_symbol("an exception symbol")?;
            let do_keyword = self.consume_exact("do")?;
            let value = self.required_clause_expression(PREC_CONTROL, &do_keyword.token)?;
            Some(TryFallback::ExceptDo(ExceptDo::new(
                typed_token(except_keyword),
                exception,
                typed_token(do_keyword),
                value,
            )))
        } else {
            None
        };

        Ok(TryStatement::new(typed_token(keyword), value, then_clause, fallback).into())
    }

    fn parse_new_statement(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let class = self.required_clause_expression(PREC_LOOP_CLAUSE, &keyword.token)?;
        let parent_class = if let Some(token) = self.take_exact("of") {
            let parent = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
            Some(OfClass::new(typed_token(token), parent))
        } else {
            None
        };
        let instance = if let Some(token) = self.take_exact("from") {
            let instance = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
            Some(FromInstance::new(typed_token(token), instance))
        } else {
            None
        };

        Ok(NewStatement::new(typed_token(keyword), class, parent_class, instance).into())
    }

    fn parse_debug_clause(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let spelling = parsing_spelling(&keyword.token).to_owned();
        if matches!(
            spelling.as_str(),
            "time" | "timing" | "elapsedTime" | "elapsedTiming" | "profile"
        ) {
            let value = self
                .parse_required(PREC_CONTROL)
                .map_err(|error| error.missing_operand_for(&keyword.token))?;
            let modifier = match spelling.as_str() {
                "time" => ProfilingModifier::Time(typed_token(keyword)),
                "timing" => ProfilingModifier::Timing(typed_token(keyword)),
                "elapsedTime" => ProfilingModifier::ElapsedTime(typed_token(keyword)),
                "elapsedTiming" => ProfilingModifier::ElapsedTiming(typed_token(keyword)),
                "profile" => ProfilingModifier::Profile(typed_token(keyword)),
                _ => unreachable!(),
            };
            return Ok(ProfilingExpr::new(modifier, value).into());
        }
        let value_is_optional = matches!(spelling.as_str(), "step" | "finish");
        let value = if value_is_optional {
            self.parse_nullable(PREC_CONTROL)?
        } else {
            Some(
                self.parse_required(PREC_CONTROL)
                    .map_err(|error| error.missing_operand_for(&keyword.token))?,
            )
        };
        let keyword = match spelling.as_str() {
            "step" => DebugKeyword::Step(typed_token(keyword)),
            "finish" => DebugKeyword::Finish(typed_token(keyword)),
            "shield" => DebugKeyword::Shield(typed_token(keyword)),
            "TEST" => DebugKeyword::Test(typed_token(keyword)),
            "breakpoint" => DebugKeyword::Breakpoint(typed_token(keyword)),
            _ => unreachable!("debug actions are assigned only to debug keywords"),
        };
        Ok(DebugExpr::new(keyword, value).into())
    }

    fn required_clause_expression(
        &mut self,
        level: u8,
        keyword: &TokenTree,
    ) -> Result<Expr, NativeParseError> {
        match self.parse_nullable(level)? {
            None => Err(NativeParseError::MissingOperand {
                operator: keyword.description(),
                span: keyword.expect_span(),
            }),
            Some(expression) => Ok(expression),
        }
    }

    fn consume_symbol(&mut self, expected: &'static str) -> Result<Symbol, NativeParseError> {
        match self.consume() {
            Lookahead::Token(token)
                if matches!(token.token, TokenTree::Ident(_))
                    && matches!(parse_info(&token.token).unary, UnaryAction::Atom) =>
            {
                let span = token.token.expect_span();
                let text = token.token.description();
                Ok(typed_terminal(
                    Symbol::new(text, span),
                    token.trailing_trivia,
                ))
            }
            Lookahead::Token(token) => Err(unexpected!(token.token, expected)),
            Lookahead::End(span) => Err(NativeParseError::unexpected_end(expected, span)),
        }
    }

    fn consume_exact(
        &mut self,
        spelling: &'static str,
    ) -> Result<SignificantToken, NativeParseError> {
        match self.consume() {
            Lookahead::Token(token) if parsing_spelling(&token.token) == spelling => Ok(token),
            Lookahead::Token(token) => Err(unexpected!(token.token, spelling)),
            Lookahead::End(span) => Err(NativeParseError::unexpected_end(spelling, span)),
        }
    }

    fn take_exact(&mut self, spelling: &'static str) -> Option<SignificantToken> {
        if matches!(
            self.peek(),
            Lookahead::Token(token)
                if parsing_spelling(&token.token) == spelling
        ) {
            let Lookahead::Token(token) = self.consume() else {
                unreachable!("the token just peeked is consumable")
            };
            Some(token)
        } else {
            None
        }
    }

    fn expected_at_lookahead(&self, expected: &'static str) -> NativeParseError {
        match self.peek() {
            Lookahead::Token(token) => unexpected!(token.token, expected),
            Lookahead::End(span) => NativeParseError::unexpected_end(expected, span),
        }
    }

    fn accumulate(&mut self, mut left: Expr, level: u8) -> Result<Expr, NativeParseError> {
        loop {
            let Lookahead::Token(next) = self.peek() else {
                return Ok(left);
            };
            let info = if matches!(&next.token, TokenTree::Punct(punct) if punct.text().is_empty())
            {
                self.adjacency_info()
            } else {
                parse_info(&next.token)
            };
            if info.precedence <= level {
                return Ok(left);
            }
            let Lookahead::Token(operator) = self.consume() else {
                unreachable!("the token just peeked is consumable")
            };
            left = if matches!(&operator.token, TokenTree::Punct(punct) if punct.text().is_empty())
            {
                let right = self
                    .parse_required(PREC_APPLICATION_RIGHT)
                    .map_err(|error| error.missing_operand_for(&operator.token))?;
                let adjacency = typed_terminal(
                    Adj::new("", operator.token.expect_span()),
                    operator.trailing_trivia,
                );
                AdjacentExpr::new(left, adjacency, right).into()
            } else if let Some(strength) = info.binary_strength {
                let left = BinaryLeft::prepare(left, &operator.token)?;
                let right = self
                    .parse_required(strength)
                    .map_err(|error| error.missing_operand_for(&operator.token))?;
                lower_binary(left, operator, right)?
            } else if info.postfix {
                let span = operator.token.expect_span();
                let spelling = operator.token.description();
                let operator = PostfixOperator::from_token_tree_with_trivia(
                    operator.token,
                    operator.trailing_trivia,
                )
                .ok_or(NativeParseError::Unsupported {
                    syntax: spelling,
                    span,
                })?;
                PostfixExpr::new(left, operator).into()
            } else if !matches!(info.unary, UnaryAction::Error) {
                if operator.crossed_newline {
                    return Err(NativeParseError::NewlineInApplication {
                        span: operator.token.expect_span(),
                    });
                }
                let adjacency_span = operator.token.expect_span().subspan(0, 0);
                let right = self.parse_consumed(operator, PREC_APPLICATION_RIGHT)?;
                AdjacentExpr::new(left, Adj::new("", adjacency_span), right).into()
            } else {
                return Err(unexpected!(operator.token, "an operator"));
            };
        }
    }

    fn adjacency_info(&self) -> ParseInfo {
        let mut input = self.input.fork();
        let _ = input.next_significant();
        input.next_significant().map_or_else(
            || ParseInfo::stop(PREC_APPLICATION),
            |right| ParseInfo::binary(parse_info(&right.token).precedence, PREC_APPLICATION_RIGHT),
        )
    }

    fn only_expression(&self, components: Punctuated<Component>) -> Result<Expr, NativeParseError> {
        let mut components = components.into_iter();
        match (components.next(), components.next()) {
            (Some(Component::Expr(value)), None) => Ok(*value),
            (Some(value), _) => Err(NativeParseError::Unexpected {
                found: "a sequence".into(),
                expected: "one expression",
                span: value.expect_span(),
            }),
            (None, _) => Err(NativeParseError::MissingOperand {
                operator: "empty expression".into(),
                span: self.input.eof_span(),
            }),
        }
    }

    fn peek(&self) -> Lookahead {
        self.input.lookahead()
    }

    fn consume(&mut self) -> Lookahead {
        self.input.consume_lookahead()
    }

    fn skip_trivia(&mut self) {
        self.input.skip_trivia();
    }

    fn at_end(&self) -> bool {
        matches!(self.peek(), Lookahead::End(_))
    }

    fn at_punctuation(&self, spelling: &str) -> bool {
        matches!(
            self.peek(),
            Lookahead::Token(token)
                if token.token.spelling() == Some(spelling)
        )
    }

    fn take_punctuation(&mut self, spelling: &str) -> Option<SignificantToken> {
        if !self.at_punctuation(spelling) {
            return None;
        }
        let Lookahead::Token(token) = self.consume() else {
            unreachable!("the punctuation just peeked is consumable")
        };
        Some(token)
    }
}

fn parse_delimited(token: SignificantToken) -> Result<Expr, NativeParseError> {
    let group_trailing = token.trailing_trivia;
    let group = match token.token {
        TokenTree::Group(group) => group,
        token => return Err(unexpected!(token, "a delimited group")),
    };
    let kind = group.delim_kind();
    let delimiter_spans = group.delimiter().spans.clone();
    let mut group_contents = group.into_stream();
    let opening_trivia = group_contents.take_leading_trivia();
    let mut contents = Engine::new(group_contents);
    let mut muted = Vec::new();
    let (elements, comma) = loop {
        let elements = contents.parse_optional_components()?;
        let Some(separator) = contents.take_punctuation(";") else {
            break elements.map_or((None, false), |(elements, comma)| (Some(elements), comma));
        };
        let Some((elements, _)) = elements else {
            return Err(unexpected!(separator.token, "a statement before `;`"));
        };
        muted.push(Statement::new(
            ExprPack::new(elements),
            typed_token(separator),
        ));
    };
    if let Lookahead::Token(token) = contents.peek() {
        return Err(unexpected!(token.token, "the end of the group"));
    }

    let expression: Expr = match kind {
        DelimiterKind::Bracket => {
            let mut value = Array::new(muted, elements.map(ExprPack::new));
            value.delimiter = typed_delimiter(delimiter_spans, opening_trivia, group_trailing);
            value.into()
        }
        DelimiterKind::Brace => {
            let mut value = List::new(muted, elements.map(ExprPack::new));
            value.delimiter = typed_delimiter(delimiter_spans, opening_trivia, group_trailing);
            value.into()
        }
        DelimiterKind::AngleBar => {
            let mut value = AngleBarList::new(muted, elements.map(ExprPack::new));
            value.delimiter = typed_delimiter(delimiter_spans, opening_trivia, group_trailing);
            value.into()
        }
        DelimiterKind::Parenthesis if !muted.is_empty() || comma => {
            let mut value = if comma {
                Sequence::new(
                    muted,
                    Some(ExprPack::new(
                        elements.expect("a comma pack always contains components"),
                    )),
                )
            } else {
                let value = elements
                    .map(|elements| contents.only_expression(elements))
                    .transpose()?;
                let mut value = Holder::new(muted, value);
                value.delimiter = typed_delimiter(delimiter_spans, opening_trivia, group_trailing);
                return Ok(value.into());
            };
            value.delimiter = typed_delimiter(delimiter_spans, opening_trivia, group_trailing);
            value.into()
        }
        DelimiterKind::Parenthesis => {
            let value = elements
                .map(|elements| contents.only_expression(elements))
                .transpose()?;
            let mut value = Holder::new(Vec::new(), value);
            value.delimiter = typed_delimiter(delimiter_spans, opening_trivia, group_trailing);
            value.into()
        }
    };
    Ok(expression)
}

fn typed_delimiter<D: DelimiterToken>(
    spans: crate::DoubleSpan,
    opening_trivia: TrailingTrivia,
    closing_trivia: TrailingTrivia,
) -> D {
    let mut delimiter = D::new(spans);
    *delimiter.opening_trivia_mut() = opening_trivia;
    *delimiter.closing_trivia_mut() = closing_trivia;
    delimiter
}

fn lower_prefix_statement(keyword: SignificantToken, value: Option<Expr>) -> Expr {
    if is_token!(&keyword.token, break) {
        BreakExpr::new(typed_token(keyword), value).into()
    } else if is_token!(&keyword.token, continue) {
        ContinueExpr::new(typed_token(keyword), value).into()
    } else if is_token!(&keyword.token, return) {
        ReturnExpr::new(typed_token(keyword), value).into()
    } else {
        let value = value.expect("catch, throw, and trap require a value");
        if is_token!(&keyword.token, catch) {
            Catch::new(typed_token(keyword), value).into()
        } else if is_token!(&keyword.token, throw) {
            Throw::new(typed_token(keyword), value).into()
        } else if is_token!(&keyword.token, trap) {
            Trap::new(typed_token(keyword), value).into()
        } else {
            unreachable!("statement actions are assigned only to statement keywords")
        }
    }
}

fn lower_atom(token: SignificantToken) -> Result<Expr, NativeParseError> {
    match token.token {
        TokenTree::Ident(raw) => Ok(typed_terminal(
            Symbol::new(raw.text(), raw.expect_span()),
            token.trailing_trivia,
        )
        .into()),
        TokenTree::Literal(raw) => {
            let span = raw.expect_span();
            Ok(match raw.kind {
                LiteralKind::Integer => {
                    typed_terminal(IntegerLiteral::new(raw.text(), span), token.trailing_trivia)
                        .into()
                }
                LiteralKind::Float => {
                    typed_terminal(FloatLiteral::new(raw.text(), span), token.trailing_trivia)
                        .into()
                }
                LiteralKind::String => {
                    lower_string_literal(raw.text(), span, token.trailing_trivia)
                }
                LiteralKind::RawString => {
                    lower_raw_string_literal(raw.text(), span, token.trailing_trivia)
                }
            })
        }
        token => Err(unexpected!(token, "an expression")),
    }
}

/// A left operand whose operator-specific structural requirements are valid.
enum BinaryLeft {
    Expression(Expr),
    Assignment {
        target: AssignmentTarget,
        local: bool,
    },
    Lambda(LambdaParameters),
}

impl BinaryLeft {
    fn prepare(left: Expr, operator: &TokenTree) -> Result<Self, NativeParseError> {
        let span = operator.expect_span();
        Ok(match operator.spelling() {
            Some("=") => Self::Assignment {
                target: AssignmentTarget::parse(left, false, span)?,
                local: false,
            },
            Some(":=") => Self::Assignment {
                target: AssignmentTarget::parse(left, true, span)?,
                local: true,
            },
            Some("->") => Self::Lambda(
                lambda_parameters(left)
                    .ok_or(NativeParseError::InvalidLambdaParameters { span })?,
            ),
            _ => Self::Expression(left),
        })
    }
}

/// A parsed expression that can occur to the left of an assignment operator.
enum AssignmentTarget {
    Symbol(Symbol),
    Operator(OperatorExpr),
    New(NewStatement),
    Pack(AssignmentPack),
}

impl AssignmentTarget {
    fn parse(expression: Expr, local: bool, span: Span) -> Result<Self, NativeParseError> {
        match expression {
            Expr::Symbol(variable) => Ok(Self::Symbol(variable)),
            Expr::OperatorExpr(expression) => Ok(Self::Operator(expression)),
            Expr::NewStatement(statement) if local => Ok(Self::New(statement)),
            expression => assignment_pack(expression)
                .map(Self::Pack)
                .ok_or(NativeParseError::InvalidAssignmentTarget { span }),
        }
    }
}

fn lower_binary(
    left: BinaryLeft,
    operator: SignificantToken,
    right: Expr,
) -> Result<Expr, NativeParseError> {
    let span = operator.token.expect_span();
    let spelling = operator
        .token
        .spelling()
        .expect("binary actions are assigned only to spelled tokens");
    match left {
        BinaryLeft::Assignment { target, local } => {
            Ok(lower_assignment(target, operator, right, local))
        }
        BinaryLeft::Lambda(parameters) => {
            Ok(LambdaExpression::new(parameters, typed_token(operator), right).into())
        }
        BinaryLeft::Expression(left) if spelling == "<-" => {
            Ok(EvaluatedAssignment::new(left, typed_token(operator), right).into())
        }
        BinaryLeft::Expression(left) if spelling == "=>" => {
            Ok(OptionExpression::new(left, typed_token(operator), right).into())
        }
        BinaryLeft::Expression(left) => {
            let spelling = spelling.to_owned();
            let operator = BinaryOperator::from_token_tree_with_trivia(
                operator.token,
                operator.trailing_trivia,
            )
            .ok_or(NativeParseError::Unsupported {
                syntax: spelling,
                span,
            })?;
            Ok(BinaryExpr::new(left, operator, right).into())
        }
    }
}

fn lower_assignment(
    target: AssignmentTarget,
    operator: SignificantToken,
    right: Expr,
    local: bool,
) -> Expr {
    match (local, target) {
        (false, AssignmentTarget::Symbol(variable)) => {
            GlobalBinding::new(variable, typed_token(operator), right).into()
        }
        (true, AssignmentTarget::Symbol(variable)) => {
            LocalBinding::new(variable, typed_token(operator), right).into()
        }
        (false, AssignmentTarget::Operator(argument_expr)) => {
            OperatorAssignment::new(argument_expr, typed_token(operator), right).into()
        }
        (true, AssignmentTarget::Operator(OperatorExpr::AdjacentExpr(application))) => {
            let (typical_value, function) = installation_value(right);
            MethodInstallation::new(
                *application.left_operand,
                *application.right_operand,
                typed_token(operator),
                typical_value,
                function,
            )
            .into()
        }
        (true, AssignmentTarget::Operator(domain_expr)) => {
            let (typical_value, function) = installation_value(right);
            OperatorInstallation::new(domain_expr, typed_token(operator), typical_value, function)
                .into()
        }
        (true, AssignmentTarget::New(new_statement)) => {
            NewInstallation::new(new_statement, typed_token(operator), right).into()
        }
        (local, AssignmentTarget::Pack(argument_pack)) => {
            if local {
                LocalParallelAssignment::new(argument_pack, typed_token(operator), right).into()
            } else {
                GlobalParallelAssignment::new(argument_pack, typed_token(operator), right).into()
            }
        }
        (false, AssignmentTarget::New(_)) => {
            unreachable!("global assignment preparation rejects new statements")
        }
    }
}

pub(crate) fn installation_value(value: Expr) -> (Option<TypeAnnotation>, Expr) {
    match value {
        Expr::OptionExpression(value) => (
            Some(TypeAnnotation::new(*value.key, value.operator)),
            *value.value,
        ),
        value => (None, value),
    }
}

fn parse_cell_stream(cells: CellStream) -> Result<SourceFile, NativeParseError> {
    let mut elements = Vec::new();
    let eof_span = cells.physical_eof();
    let mut comments = CommentTrivia::default();
    let mut cells = cells.into_iter().peekable();
    while let Some(mut cell) = cells.next() {
        if !cell.has_significant_tokens() {
            comments.append(CommentTrivia::take_inner(&mut cell));
            continue;
        }
        let eoc = cells
            .peek()
            .and_then(cell_start_span)
            .unwrap_or_else(|| end_span(&cell));
        if let Some(element) = parse_raw_cell_at(cell, eoc)? {
            elements.push(element);
        }
    }
    let mut source_file = SourceFile::new(elements, EOF::new("", eof_span));
    source_file.comments = comments;
    Ok(source_file)
}

fn tokenize_for_native_parser(tokens: TokenStream) -> Result<CellStream, NativeParseError> {
    match tokens.to_cell_stream() {
        Ok(cells) => Ok(cells),
        Err(tokenize_error) => {
            let incomplete = frame_recovering(tokens).incomplete;
            if let Some(cell) = incomplete
                && let Err(parse_error) = parse_raw_cell(cell)
            {
                return Err(parse_error);
            }
            Err(NativeParseError::Tokenize(tokenize_error))
        }
    }
}

pub(crate) fn parse_raw_cell(
    tokens: TokenStream,
) -> Result<Option<crate::nodes::Cell>, NativeParseError> {
    let eoc = end_span(&tokens);
    parse_raw_cell_at(tokens, eoc)
}

fn parse_raw_cell_at(
    mut tokens: TokenStream,
    eoc: Span,
) -> Result<Option<crate::nodes::Cell>, NativeParseError> {
    let comments = CommentTrivia::take_from_cell(&mut tokens);
    let semicolon = ends_with_cell_semicolon(&tokens)
        .then(|| tokens.take_final_punctuation(";"))
        .flatten()
        .map(|(punctuation, trailing_trivia)| {
            <Token![;] as Token>::from_token_tree_with_trivia(
                TokenTree::Punct(punctuation),
                trailing_trivia,
            )
            .expect("a semicolon punctuation token lowers to Token![;]")
        });
    let mut cell = Engine::new(tokens).parse_cell(semicolon, eoc)?;
    if let Some(cell) = &mut cell {
        cell.comments = comments;
    }
    Ok(cell)
}

fn cell_start_span(tokens: &TokenStream) -> Option<Span> {
    let token = tokens.iter().next()?;
    let span = token
        .leading_trivia()
        .as_slice()
        .first()
        .map_or_else(|| token.span(), Spanned::span);
    span.start().map(|start| span.at(start))
}

impl Parse for Expr {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let tokens = input.by_ref().collect::<TokenStream>();
        let mut engine = Engine::new(tokens);
        engine.skip_trivia();
        let value = engine
            .parse_required(PREC_CLOSER)
            .map_err(token_parse_error)?;
        engine.skip_trivia();
        match engine.peek() {
            Lookahead::End(_) => Ok(value),
            Lookahead::Token(token) => Err(TokenParseError::TrailingToken {
                found: token.token.description(),
                span: token.token.expect_span(),
            }),
        }
    }
}

impl Parse for SourceFile {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let tokens = input.by_ref().collect();
        tokenize_for_native_parser(tokens)
            .and_then(parse_cell_stream)
            .map_err(token_parse_error)
    }
}

fn token_parse_error(error: NativeParseError) -> TokenParseError {
    let span = error.expect_span();
    TokenParseError::Syntax {
        message: error.to_string(),
        span,
    }
}

fn parsing_spelling(token: &TokenTree) -> &str {
    token.spelling().map_or("", canonical_word_spelling)
}

fn typed_token<T: Token>(token: SignificantToken) -> T {
    let spelling = token.token.description();
    T::from_token_tree_with_trivia(token.token, token.trailing_trivia)
        .unwrap_or_else(|| panic!("`{spelling}` cannot be lowered to `{}`", T::SPELLING))
}

fn typed_terminal<T: Terminal>(mut terminal: T, trailing_trivia: TrailingTrivia) -> T {
    *terminal.trailing_trivia_mut() = trailing_trivia;
    terminal
}

fn empty_span_at(lookahead: &Span) -> Span {
    lookahead.subspan(0, 0)
}

fn lower_string_literal(text: &str, span: Span, trailing_trivia: TrailingTrivia) -> Expr {
    let body = &text[1..text.len() - 1];
    let mut elements = Vec::new();
    let mut content_start = 0;
    let mut cursor = 0;

    while cursor < body.len() {
        if body.as_bytes()[cursor] != b'\\' {
            cursor += 1;
            continue;
        }
        if content_start < cursor {
            elements.push(StringElement::StringContent(StringContent::new(
                &body[content_start..cursor],
                span.subspan(content_start + 1, cursor + 1),
            )));
        }
        let escape_end = match body.as_bytes()[cursor + 1] {
            b'x' => cursor + 4,
            b'u' => cursor + 6,
            b'0'..=b'7' => {
                let mut end = cursor + 2;
                while end < body.len()
                    && end < cursor + 4
                    && body.as_bytes()[end].is_ascii_digit()
                    && body.as_bytes()[end] < b'8'
                {
                    end += 1;
                }
                end
            }
            _ => cursor + 2,
        };
        elements.push(StringElement::EscapeSequence(EscapeSequence::new(
            &body[cursor..escape_end],
            span.subspan(cursor + 1, escape_end + 1),
        )));
        cursor = escape_end;
        content_start = cursor;
    }
    if content_start < body.len() {
        elements.push(StringElement::StringContent(StringContent::new(
            &body[content_start..],
            span.subspan(content_start + 1, text.len() - 1),
        )));
    }
    typed_terminal(StringLiteral::new(elements, span), trailing_trivia).into()
}

fn lower_raw_string_literal(text: &str, span: Span, trailing_trivia: TrailingTrivia) -> Expr {
    let body = &text[3..text.len() - 3];
    let mut elements = Vec::new();
    let mut content_start = 0;
    let mut cursor = 0;

    while cursor + 1 < body.len() {
        if &body.as_bytes()[cursor..cursor + 2] != b"//" {
            cursor += 1;
            continue;
        }
        if content_start < cursor {
            elements.push(RawStringElement::RawStringContent(RawStringContent::new(
                &body[content_start..cursor],
                span.subspan(content_start + 3, cursor + 3),
            )));
        }
        elements.push(RawStringElement::EscapeSequence(EscapeSequence::new(
            "//",
            span.subspan(cursor + 3, cursor + 5),
        )));
        cursor += 2;
        content_start = cursor;
    }
    if content_start < body.len() {
        elements.push(RawStringElement::RawStringContent(RawStringContent::new(
            &body[content_start..],
            span.subspan(content_start + 3, text.len() - 3),
        )));
    }
    typed_terminal(RawStringLiteral::new(elements, span), trailing_trivia).into()
}

fn collection_contents(expression: Expr) -> Option<(crate::Delimiter, Option<ExprPack>)> {
    match expression {
        Expr::Collection(Collection::Sequence(value)) if value.statements.is_empty() => {
            Some((DelimiterValue::delimiter(&value.delimiter), value.elements))
        }
        Expr::Collection(Collection::List(value)) if value.statements.is_empty() => {
            Some((DelimiterValue::delimiter(&value.delimiter), value.elements))
        }
        Expr::Collection(Collection::Array(value)) if value.statements.is_empty() => {
            Some((DelimiterValue::delimiter(&value.delimiter), value.elements))
        }
        Expr::Collection(Collection::AngleBarList(value)) if value.statements.is_empty() => {
            Some((DelimiterValue::delimiter(&value.delimiter), value.elements))
        }
        Expr::Holder(value) if value.muted.is_empty() => {
            let elements = value
                .value
                .map(|value| ExprPack::new(Punctuated::from_single(Component::Expr(value))));
            Some((DelimiterValue::delimiter(&value.delimiter), elements))
        }
        _ => None,
    }
}

pub(crate) fn assignment_pack(expression: Expr) -> Option<AssignmentPack> {
    let (delimiter, elements) = collection_contents(expression)?;
    let contents = match elements {
        Some(elements) => Some(map_punctuated(elements.pack, assignment_pack_component)?),
        None => None,
    };
    Some(AssignmentPack::new(Delimited::new(delimiter, contents)))
}

fn assignment_pack_component(component: Component) -> Option<AssignmentPackComponent> {
    match component {
        Component::Empty(value) => Some(AssignmentPackComponent::Empty(value)),
        Component::Expr(value) => match *value {
            Expr::Symbol(value) => Some(AssignmentPackComponent::Symbol(value)),
            Expr::OperatorExpr(value) => Some(AssignmentPackComponent::OperatorExpr(value)),
            value => assignment_pack(value).map(Into::into),
        },
    }
}

fn expr_pack_symbols(elements: ExprPack) -> Option<Punctuated<Symbol>> {
    map_punctuated(elements.pack, |component| match component {
        Component::Expr(value) => match *value {
            Expr::Symbol(value) => Some(value),
            _ => None,
        },
        Component::Empty(_) => None,
    })
}

fn map_punctuated<S, T>(
    values: Punctuated<S>,
    mut map: impl FnMut(S) -> Option<T>,
) -> Option<Punctuated<T>> {
    let (first, rest) = values.into_parts();
    let first = map(first)?;
    let rest = rest
        .into_iter()
        .map(|(punctuation, value)| Some((punctuation, map(value)?)))
        .collect::<Option<Vec<_>>>()?;
    Some(Punctuated::new(first, rest))
}

pub(crate) fn lambda_parameters(expression: Expr) -> Option<LambdaParameters> {
    match expression {
        Expr::Symbol(value) => Some(LambdaParameters::Variadic(Variadic::new(value))),
        expression => {
            let (delimiter, elements) = collection_contents(expression)?;
            let parameters = match elements {
                Some(elements) => Some(expr_pack_symbols(elements)?),
                None => None,
            };
            Some(LambdaParameters::FixedArity(FixedArity::new(
                Delimited::new(delimiter, parameters),
            )))
        }
    }
}

/// The rule declared for a spelling, found in the generated table.
fn token_parse_rule(spelling: &str) -> Option<TokenParseRule> {
    TOKEN_PARSE_RULES
        .binary_search_by_key(&spelling, |(declared, _)| *declared)
        .ok()
        .map(|index| TOKEN_PARSE_RULES[index].1)
}

fn keyword_action(spelling: &str) -> Option<UnaryAction> {
    Some(match spelling {
        "break" | "continue" | "return" | "catch" | "throw" | "trap" => UnaryAction::Statement,
        "if" | "while" | "try" | "for" | "new" => UnaryAction::Control,
        "TEST" | "time" | "timing" | "elapsedTime" | "elapsedTiming" | "breakpoint" | "profile"
        | "shield" | "step" | "finish" => UnaryAction::Debug,
        "symbol" | "global" | "local" | "threadLocal" | "threadVariable" => UnaryAction::Quote,
        _ => return None,
    })
}

fn parse_info(token: &TokenTree) -> ParseInfo {
    if matches!(token, TokenTree::Punct(punctuation) if punctuation.text().is_empty()) {
        return ParseInfo::binary(PREC_APPLICATION, PREC_APPLICATION_RIGHT);
    }
    if let TokenTree::Group(group) = token {
        return match group.delim_kind() {
            DelimiterKind::Bracket | DelimiterKind::AngleBar => {
                ParseInfo::delimiter(PREC_COLLECTION)
            }
            DelimiterKind::Parenthesis | DelimiterKind::Brace => {
                ParseInfo::delimiter(PREC_APPLICATION)
            }
        };
    }

    let text = parsing_spelling(token);
    if let Some(rule) = token_parse_rule(text) {
        let mut info = ParseInfo::from(rule);
        if let Some(action) = keyword_action(text) {
            info.unary = action;
        }
        return info;
    }

    match token {
        TokenTree::Ident(_) | TokenTree::Literal(_) => ParseInfo::atom(PREC_APPLICATION),
        _ => ParseInfo::stop(PREC_APPLICATION),
    }
}

#[cfg(test)]
mod tests {
    use super::{NativeCellParser, NativeCellState};
    use crate::cell_stream::frame_recovering;
    use crate::{Spanned, lex_str};

    /// Cell boundaries are decided here, from the flat stream, using the same
    /// operator/clause/quote state the grammar needs. Verified against M2
    /// v1.26.05 for each case.
    #[test]
    fn cells_use_operator_clause_and_delimiter_state() {
        fn cells(source: &str) -> usize {
            let framed = frame_recovering(lex_str(source).unwrap());
            framed.cells.len() + usize::from(framed.incomplete.is_some())
        }

        assert_eq!(cells("1\n2"), 2);
        assert_eq!(cells("1 +\n2"), 1);
        assert_eq!(cells("not\n1"), 1);
        assert_eq!(cells("1!\n2"), 2);
        assert_eq!(cells("if condition\nthen value"), 1);
        assert_eq!(cells("if condition\nthen value\nnext"), 2);
        assert_eq!(cells("symbol if\nnext"), 2);
        assert_eq!(cells("symbol +\nnext"), 2);
        assert_eq!(cells("1 -* comment\n*- 2"), 2);
        assert_eq!(cells("1 + -* comment\n*- 2"), 1);
        assert_eq!(cells("1;2"), 2);
        // A `;` inside brackets is an ordinary token, so the group stays one
        // cell and its contents are untouched by the embedding.
        assert_eq!(cells("(1\n2;3)"), 1);
        // The quoted `;` is data, not a boundary.
        assert_eq!(cells("symbol ;"), 1);
    }

    #[test]
    fn invalid_binary_left_side_wins_over_a_missing_right_operand() {
        for (source, state, diagnostic, diagnostic_text) in [
            (
                "1 + 1 =",
                NativeCellState::Incomplete,
                "expected an operand after `=`, found end of input",
                "=",
            ),
            (
                "2 =",
                NativeCellState::Invalid,
                "invalid assignment target",
                "=",
            ),
            (
                "(x, y) ->",
                NativeCellState::Incomplete,
                "expected an operand after `->`, found end of input",
                "->",
            ),
            (
                "(x + y) ->",
                NativeCellState::Invalid,
                "invalid lambda parameter list",
                "->",
            ),
        ] {
            let mut parser = NativeCellParser::new();
            assert_eq!(parser.push_line(source), state, "source: `{source}`");
            let error = parser
                .parse_error()
                .unwrap_or_else(|| panic!("expected a diagnostic for `{source}`"));
            assert_eq!(error.to_string(), diagnostic, "source: `{source}`");
            assert_eq!(error.span().text(), Some(diagnostic_text));
        }
    }
}
