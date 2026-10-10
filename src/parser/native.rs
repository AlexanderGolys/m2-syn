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
    CellStream, CellStreamError, CellTokenStream, CommentTrivia, Delimited, DelimiterKind,
    DelimiterToken, DelimiterValue, LeadingTrivia, LexError, LexErrorKind, LiteralKind, Parse,
    ParseStream, Parser, Punctuated, Recovered, Span, Spanned, Terminal, Token, TokenParseError,
    TokenStream, TokenTree, Tokenize, TrailingTrivia, Trivia, lex_str, lex_str_recovering,
};

/// Tests a raw token against one typed-token spelling.
macro_rules! is_token {
    ($token:expr, $($pattern:tt)*) => {
        <Token![$($pattern)*] as Token>::matches_token_tree($token)
    };
}

/// Constructs the native parser's unexpected-token diagnostic.
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

/// The PREC CLOSER constant.
const PREC_CLOSER: u8 = 6;
/// The PREC COMMA constant.
const PREC_COMMA: u8 = 10;
/// The PREC CONTROL constant.
const PREC_CONTROL: u8 = 12;
/// The PREC LOOP CLAUSE constant.
const PREC_LOOP_CLAUSE: u8 = 16;
/// The PREC COLLECTION constant.
const PREC_COLLECTION: u8 = 56;
/// The PREC APPLICATION RIGHT constant.
const PREC_APPLICATION_RIGHT: u8 = 61;
/// The PREC APPLICATION constant.
const PREC_APPLICATION: u8 = 62;

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
    /// The source.
    source: Span,
    /// The syntax.
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
    /// The active source.
    active_source: String,
    /// The active tokens.
    active_tokens: TokenStream,
    /// The active syntax.
    active_syntax: Option<crate::nodes::Cell>,
    /// The completed.
    completed: Vec<CompletedNativeCell>,
    /// The lexer diagnostics.
    lexer_diagnostics: Vec<LexError>,
    /// The parse error.
    parse_error: Option<NativeParseError>,
    /// The state.
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

    /// Handles refresh.
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

/// Handles incomplete lex error.
pub(crate) fn incomplete_lex_error(kind: LexErrorKind) -> bool {
    matches!(
        kind,
        LexErrorKind::UnterminatedGroup
            | LexErrorKind::UnterminatedString
            | LexErrorKind::UnterminatedRawString
            | LexErrorKind::UnterminatedBlockComment
    )
}

/// Handles incomplete parse error.
pub(crate) fn incomplete_parse_error(error: &NativeParseError) -> bool {
    match error {
        NativeParseError::MissingOperand { .. } => true,
        NativeParseError::Unexpected { found, .. } => found == "end of input",
        NativeParseError::Lex(error) => incomplete_lex_error(error.kind()),
        NativeParseError::Tokenize(_) => true,
        NativeParseError::NewlineInApplication { .. }
        | NativeParseError::InvalidAssignmentTarget { .. }
        | NativeParseError::InvalidLambdaArguments { .. }
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
    /// The left side of a lambda arrow is not an argument form.
    InvalidLambdaArguments {
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
    /// Handles unexpected end.
    fn unexpected_end(expected: &'static str, span: Span) -> Self {
        Self::Unexpected {
            found: "end of input".into(),
            expected,
            span,
        }
    }

    /// Handles missing operand for.
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
            Self::InvalidLambdaArguments { .. } => {
                formatter.write_str("invalid lambda argument list")
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
    /// The Atom alternative.
    Atom,
    /// The Prefix alternative.
    Prefix,
    /// The Statement alternative.
    Statement,
    /// The Control alternative.
    Control,
    /// The Debug alternative.
    Debug,
    /// The Delimiter alternative.
    Delimiter,
    /// The Quote alternative.
    Quote,
    /// The Error alternative.
    Error,
}

#[derive(Debug, Clone, Copy)]
/// Binding powers and unary behavior for one parser token.
struct ParseInfo {
    /// The precedence.
    precedence: u8,
    /// The binary strength.
    binary_strength: Option<u8>,
    /// The unary strength.
    unary_strength: Option<u8>,
    /// The unary.
    unary: UnaryAction,
    /// The postfix.
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
    /// Handles atom.
    const fn atom(precedence: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: None,
            unary: UnaryAction::Atom,
            postfix: false,
        }
    }

    /// Handles binary.
    const fn binary(precedence: u8, strength: u8) -> Self {
        Self {
            precedence,
            binary_strength: Some(strength),
            unary_strength: None,
            unary: UnaryAction::Error,
            postfix: false,
        }
    }

    /// Handles prefix binary.
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

    /// Handles postfix.
    const fn postfix(precedence: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: None,
            unary: UnaryAction::Error,
            postfix: true,
        }
    }

    /// Handles delimiter.
    const fn delimiter(precedence: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: Some(PREC_CLOSER),
            unary: UnaryAction::Delimiter,
            postfix: false,
        }
    }

    /// Handles stop.
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

/// Internal state for Engine.
struct Engine {
    /// The input.
    input: ParseStream,
}

/// Internal state for Components.
struct Components<T> {
    /// The first.
    first: T,
    /// The rest.
    rest: Vec<(Token![,], T)>,
}

impl<T> Components<T> {
    /// Creates the value from punctuated.
    fn from_punctuated(values: Punctuated<T>) -> Self {
        let (pair, rest) = values.into_parts();
        let (first, punctuation, second) = pair.into_parts();
        let mut pairs = Vec::with_capacity(rest.len() + 1);
        pairs.push((punctuation, second));
        pairs.extend(rest);
        Self { first, rest: pairs }
    }

    /// Returns whether the value is punctuated.
    fn is_punctuated(&self) -> bool {
        !self.rest.is_empty()
    }

    /// Converts the value into punctuated.
    fn into_punctuated(self) -> Result<Punctuated<T>, T> {
        let mut rest = self.rest.into_iter();
        let Some((punctuation, second)) = rest.next() else {
            return Err(self.first);
        };
        let mut values = Punctuated::new(self.first, punctuation, second);
        for (punctuation, value) in rest {
            values.push(punctuation, value);
        }
        Ok(values)
    }

    /// Handles map.
    fn map<U>(self, mut map: impl FnMut(T) -> Option<U>) -> Option<Components<U>> {
        Some(Components {
            first: map(self.first)?,
            rest: self
                .rest
                .into_iter()
                .map(|(punctuation, value)| Some((punctuation, map(value)?)))
                .collect::<Option<Vec<_>>>()?,
        })
    }
}

impl Engine {
    /// Creates a new value.
    fn new(tokens: TokenStream) -> Self {
        Self {
            input: ParseStream::new(tokens),
        }
    }

    /// Parses cell.
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

        let components = self.parse_components()?;

        if let Lookahead::Token(token) = self.peek() {
            return Err(unexpected!(token.token, "the end of the cell"));
        }

        let eoc = EOC::new("", eoc_span);
        Ok(Some(crate::nodes::Cell::new(
            self.packed_content(components)?,
            semicolon,
            eoc,
        )))
    }

    /// Parses components.
    fn parse_components(&mut self) -> Result<Components<Component>, NativeParseError> {
        let first = if self.at_punctuation(",") {
            Component::Empty(Empty::new("", empty_span_at(&self.peek().expect_span())))
        } else {
            self.parse_required(PREC_COMMA)?.into()
        };
        let mut rest = Vec::new();

        while let Some(comma) = self.take_punctuation(",") {
            let next = if self.at_end() || self.at_punctuation(";") || self.at_punctuation(",") {
                Component::Empty(Empty::new("", empty_span_at(&self.peek().expect_span())))
            } else {
                self.parse_required(PREC_COMMA)?.into()
            };
            rest.push((typed_token(comma), next));
        }

        Ok(Components { first, rest })
    }

    /// Parses optional components.
    fn parse_optional_components(
        &mut self,
    ) -> Result<Option<Components<Component>>, NativeParseError> {
        if self.at_end() || self.at_punctuation(";") {
            Ok(None)
        } else {
            self.parse_components().map(Some)
        }
    }

    /// Parses required.
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

    /// Parses nullable.
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

    /// Parses consumed.
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
                Expr!(Op::Prefix {
                    operator,
                    operand: Box::new(operand),
                })
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
                let quoted_span = quoted.token.expect_span();
                let Some(quoted_text) = quoted.token.spelling().map(str::to_owned) else {
                    return Err(unexpected!(quoted.token, "a quoted token"));
                };
                let quoted = typed_terminal(
                    Symbol::new(quoted_text, quoted_span),
                    quoted.trailing_trivia,
                );
                if is_token!(&token.token, symbol) {
                    Expr!(Quote::Symbol {
                        specifier: typed_token(token),
                        symbol: quoted,
                    })
                } else if is_token!(&token.token, local) {
                    Expr!(Quote::Local {
                        specifier: typed_token(token),
                        symbol: quoted,
                    })
                } else if is_token!(&token.token, global) {
                    Expr!(Quote::Global {
                        specifier: typed_token(token),
                        symbol: quoted,
                    })
                } else if is_token!(&token.token, threadVariable) {
                    Expr!(Quote::ThreadVariable {
                        specifier: typed_token(token),
                        symbol: quoted,
                    })
                } else if is_token!(&token.token, threadLocal) {
                    Expr!(Quote::ThreadLocal {
                        specifier: typed_token(token),
                        symbol: quoted,
                    })
                } else {
                    unreachable!("quote actions are assigned only to quote specifiers")
                }
            }
            UnaryAction::Error => {
                return Err(unexpected!(token.token, "an expression"));
            }
        };
        self.accumulate(result, level)
    }

    /// Parses if statement.
    fn parse_if_statement(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let condition = self.required_clause_expression(PREC_CONTROL, &keyword.token)?;
        let then_keyword = self.consume_exact("then")?;
        let then_value = self.required_clause_expression(PREC_CONTROL, &then_keyword.token)?;
        let then_clause = ThenClause::new(typed_token(then_keyword), Box::new(then_value));
        let else_clause = self
            .take_exact("else")
            .map(|else_keyword| {
                let value = self.required_clause_expression(PREC_CONTROL, &else_keyword.token)?;
                Ok::<_, NativeParseError>(ElseClause::new(
                    typed_token(else_keyword),
                    Box::new(value),
                ))
            })
            .transpose()?;
        Ok(Expr!(If {
            if_token: typed_token(keyword),
            condition: Box::new(condition),
            then_clause,
            else_clause,
        }))
    }

    /// Parses for loop.
    fn parse_for_loop(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let variable = self.consume_symbol("a loop variable")?;

        enum Domain {
            /// The Bare alternative.
            Bare,
            /// The Range alternative.
            Range(Option<RangeBoundFrom>, Option<RangeBoundTo>),
            /// The In alternative.
            In(SignificantToken, Box<Expr>),
        }
        let domain = if let Some(token) = self.take_exact("in") {
            let collection = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
            Domain::In(token, Box::new(collection))
        } else {
            let lower_bound = self
                .take_exact("from")
                .map(|token| {
                    let value = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
                    Ok::<_, NativeParseError>(RangeBoundFrom::new(
                        typed_token(token),
                        Box::new(value),
                    ))
                })
                .transpose()?;
            let upper_bound = self
                .take_exact("to")
                .map(|token| {
                    let value = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
                    Ok::<_, NativeParseError>(RangeBoundTo::new(
                        typed_token(token),
                        Box::new(value),
                    ))
                })
                .transpose()?;
            if lower_bound.is_some() || upper_bound.is_some() {
                Domain::Range(lower_bound, upper_bound)
            } else {
                Domain::Bare
            }
        };

        let when_condition = if let Some(token) = self.take_exact("when") {
            let condition = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
            Some(WhenCondition::new(typed_token(token), Box::new(condition)))
        } else {
            None
        };
        let body = self.parse_loop_body()?;

        Ok(match domain {
            Domain::Bare => Expr!(For::Bare {
                for_token: typed_token(keyword),
                variable,
                condition: when_condition,
                body,
            }),
            Domain::Range(lower_bound, upper_bound) => Expr!(For::Range {
                for_token: typed_token(keyword),
                variable,
                lower_bound,
                upper_bound,
                condition: when_condition,
                body,
            }),
            Domain::In(token, collection) => Expr!(For::In {
                for_token: typed_token(keyword),
                variable,
                in_token: typed_token(token),
                collection,
                condition: when_condition,
                body,
            }),
        })
    }

    /// Parses while loop.
    fn parse_while_loop(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let condition = self.required_clause_expression(PREC_CONTROL, &keyword.token)?;
        let body = self.parse_loop_body()?;
        Ok(Expr!(While {
            while_token: typed_token(keyword),
            condition: Box::new(condition),
            body,
        }))
    }

    /// Parses loop body.
    fn parse_loop_body(&mut self) -> Result<LoopBody, NativeParseError> {
        let (list_step, do_step) = if let Some(token) = self.take_exact("list") {
            let listed_value = self.required_clause_expression(PREC_CONTROL, &token.token)?;
            let list_step = Some(ListStep::new(typed_token(token), Box::new(listed_value)));
            if let Some(token) = self.take_exact("do") {
                let ignored_value = self.required_clause_expression(PREC_CONTROL, &token.token)?;
                (
                    list_step,
                    Some(DoStep::new(typed_token(token), Box::new(ignored_value))),
                )
            } else {
                (list_step, None)
            }
        } else if let Some(token) = self.take_exact("do") {
            let ignored_value = self.required_clause_expression(PREC_CONTROL, &token.token)?;
            (
                None,
                Some(DoStep::new(typed_token(token), Box::new(ignored_value))),
            )
        } else {
            return Err(self.expected_at_lookahead("`list` or `do`"));
        };

        Ok(LoopBody::new(list_step, do_step))
    }

    /// Parses try statement.
    fn parse_try_statement(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let value = self.required_clause_expression(PREC_CONTROL, &keyword.token)?;
        let then_clause = self
            .take_exact("then")
            .map(|then_keyword| {
                let value = self.required_clause_expression(PREC_CONTROL, &then_keyword.token)?;
                Ok::<_, NativeParseError>(ThenClause::new(
                    typed_token(then_keyword),
                    Box::new(value),
                ))
            })
            .transpose()?;

        let fallback = if let Some(else_keyword) = self.take_exact("else") {
            let value = self.required_clause_expression(PREC_CONTROL, &else_keyword.token)?;
            Some(TryFallback::Else(ElseClause::new(
                typed_token(else_keyword),
                Box::new(value),
            )))
        } else if let Some(except_keyword) = self.take_exact("except") {
            let exception = self.consume_symbol("an exception symbol")?;
            let do_keyword = self.consume_exact("do")?;
            let value = self.required_clause_expression(PREC_CONTROL, &do_keyword.token)?;
            Some(TryFallback!(Except {
                except_token: typed_token(except_keyword),
                exception,
                do_token: typed_token(do_keyword),
                value: Box::new(value),
            }))
        } else {
            None
        };

        Ok(Expr!(Try {
            try_token: typed_token(keyword),
            value: Box::new(value),
            on_success: then_clause,
            fallback,
        }))
    }

    /// Parses new statement.
    fn parse_new_statement(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let class = self.required_clause_expression(PREC_LOOP_CLAUSE, &keyword.token)?;
        let parent_class = if let Some(token) = self.take_exact("of") {
            let parent = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
            Some(OfClass::new(typed_token(token), Box::new(parent)))
        } else {
            None
        };
        let instance = if let Some(token) = self.take_exact("from") {
            let instance = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
            Some(FromInstance::new(typed_token(token), Box::new(instance)))
        } else {
            None
        };

        Ok(Expr!(New {
            new_token: typed_token(keyword),
            class: Box::new(class),
            parent_class,
            instance,
        }))
    }

    /// Parses debug clause.
    fn parse_debug_clause(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let spelling = parsing_spelling(&keyword.token).to_owned();
        if matches!(
            spelling.as_str(),
            "time" | "timing" | "elapsedTime" | "elapsedTiming" | "profile"
        ) {
            let value = self
                .parse_required(PREC_CONTROL)
                .map_err(|error| error.missing_operand_for(&keyword.token))?;
            return Ok(match spelling.as_str() {
                "time" => Expr!(Profile::Time {
                    modifier: typed_token(keyword),
                    value: Box::new(value),
                }),
                "timing" => Expr!(Profile::Timing {
                    modifier: typed_token(keyword),
                    value: Box::new(value),
                }),
                "elapsedTime" => Expr!(Profile::ElapsedTime {
                    modifier: typed_token(keyword),
                    value: Box::new(value),
                }),
                "elapsedTiming" => Expr!(Profile::ElapsedTiming {
                    modifier: typed_token(keyword),
                    value: Box::new(value),
                }),
                "profile" => Expr!(Profile::Profile {
                    modifier: typed_token(keyword),
                    value: Box::new(value),
                }),
                _ => unreachable!(),
            });
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
        let value = value.map(Box::new);
        Ok(match spelling.as_str() {
            "step" => Expr!(Debug::Step {
                keyword: typed_token(keyword),
                value,
            }),
            "finish" => Expr!(Debug::Finish {
                keyword: typed_token(keyword),
                value,
            }),
            "shield" => Expr!(Debug::Shield {
                keyword: typed_token(keyword),
                value,
            }),
            "TEST" => Expr!(Debug::Test {
                keyword: typed_token(keyword),
                value,
            }),
            "breakpoint" => Expr!(Debug::Breakpoint {
                keyword: typed_token(keyword),
                value,
            }),
            _ => unreachable!("debug actions are assigned only to debug keywords"),
        })
    }

    /// Handles required clause expression.
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

    /// Consumes symbol.
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

    /// Consumes exact.
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

    /// Takes exact.
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

    /// Handles expected at lookahead.
    fn expected_at_lookahead(&self, expected: &'static str) -> NativeParseError {
        match self.peek() {
            Lookahead::Token(token) => unexpected!(token.token, expected),
            Lookahead::End(span) => NativeParseError::unexpected_end(expected, span),
        }
    }

    /// Handles accumulate.
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
                Expr!(Op::Bin::Adj {
                    left_operand: Box::new(left),
                    operator: adjacency,
                    right_operand: Box::new(right),
                })
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
                Expr!(Op::Postfix {
                    operand: Box::new(left),
                    operator,
                })
            } else if !matches!(info.unary, UnaryAction::Error) {
                if operator.crossed_newline {
                    return Err(NativeParseError::NewlineInApplication {
                        span: operator.token.expect_span(),
                    });
                }
                let adjacency_span = operator.token.expect_span().subspan(0, 0);
                let right = self.parse_consumed(operator, PREC_APPLICATION_RIGHT)?;
                Expr!(Op::Bin::Adj {
                    left_operand: Box::new(left),
                    operator: Adj::new("", adjacency_span),
                    right_operand: Box::new(right),
                })
            } else {
                return Err(unexpected!(operator.token, "an operator"));
            };
        }
    }

    /// Handles adjacency info.
    fn adjacency_info(&self) -> ParseInfo {
        let mut input = self.input.fork();
        let _ = input.next_significant();
        input.next_significant().map_or_else(
            || ParseInfo::stop(PREC_APPLICATION),
            |right| ParseInfo::binary(parse_info(&right.token).precedence, PREC_APPLICATION_RIGHT),
        )
    }

    /// Handles only expression.
    fn only_expression(&self, components: Components<Component>) -> Result<Expr, NativeParseError> {
        if components.is_punctuated() {
            return Err(NativeParseError::Unexpected {
                found: "a sequence".into(),
                expected: "one expression",
                span: components.first.expect_span(),
            });
        }
        match components.first {
            Component::Expr(value) => Ok(*value),
            value => Err(NativeParseError::Unexpected {
                found: "an empty component".into(),
                expected: "one expression",
                span: value.expect_span(),
            }),
        }
    }

    /// Handles packed content.
    fn packed_content(
        &self,
        components: Components<Component>,
    ) -> Result<PackedContent, NativeParseError> {
        match components.into_punctuated() {
            Ok(components) => Ok(ExprPack::new(components).into()),
            Err(Component::Expr(expression)) => Ok((*expression).into()),
            Err(value) => Err(NativeParseError::Unexpected {
                found: "an empty component".into(),
                expected: "one expression",
                span: value.expect_span(),
            }),
        }
    }

    /// Handles peek.
    fn peek(&self) -> Lookahead {
        self.input.lookahead()
    }

    /// Handles consume.
    fn consume(&mut self) -> Lookahead {
        self.input.consume_lookahead()
    }

    /// Skips trivia.
    fn skip_trivia(&mut self) {
        self.input.skip_trivia();
    }

    /// Handles at end.
    fn at_end(&self) -> bool {
        matches!(self.peek(), Lookahead::End(_))
    }

    /// Handles at punctuation.
    fn at_punctuation(&self, spelling: &str) -> bool {
        matches!(
            self.peek(),
            Lookahead::Token(token)
                if token.token.spelling() == Some(spelling)
        )
    }

    /// Takes punctuation.
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

/// Parses delimited.
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
    let elements = loop {
        let elements = contents.parse_optional_components()?;
        let Some(separator) = contents.take_punctuation(";") else {
            break elements;
        };
        let Some(elements) = elements else {
            return Err(unexpected!(separator.token, "a statement before `;`"));
        };
        muted.push(Statement::new(
            contents.packed_content(elements)?,
            typed_token(separator),
        ));
    };
    if let Lookahead::Token(token) = contents.peek() {
        return Err(unexpected!(token.token, "the end of the group"));
    }

    let expression: Expr = match kind {
        DelimiterKind::Bracket => {
            let elements = elements
                .map(|elements| contents.packed_content(elements))
                .transpose()?;
            let mut value = Array::new(muted, elements);
            value.delimiter = typed_delimiter(delimiter_spans, opening_trivia, group_trailing);
            value.into()
        }
        DelimiterKind::Brace => {
            let elements = elements
                .map(|elements| contents.packed_content(elements))
                .transpose()?;
            let mut value = List::new(muted, elements);
            value.delimiter = typed_delimiter(delimiter_spans, opening_trivia, group_trailing);
            value.into()
        }
        DelimiterKind::AngleBar => {
            let elements = elements
                .map(|elements| contents.packed_content(elements))
                .transpose()?;
            let mut value = AngleBarList::new(muted, elements);
            value.delimiter = typed_delimiter(delimiter_spans, opening_trivia, group_trailing);
            value.into()
        }
        DelimiterKind::Parenthesis
            if !muted.is_empty() || elements.as_ref().is_some_and(Components::is_punctuated) =>
        {
            let mut value = if elements.as_ref().is_some_and(Components::is_punctuated) {
                Sequence::new(
                    muted,
                    elements
                        .map(|elements| contents.packed_content(elements))
                        .transpose()?,
                )
            } else {
                let value = elements
                    .map(|elements| contents.only_expression(elements))
                    .transpose()?;
                let mut value = Holder::new(muted, value.map(Box::new));
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
            let mut value = Holder::new(Vec::new(), value.map(Box::new));
            value.delimiter = typed_delimiter(delimiter_spans, opening_trivia, group_trailing);
            value.into()
        }
    };
    Ok(expression)
}

/// Handles typed delimiter.
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

/// Lowers prefix statement.
fn lower_prefix_statement(keyword: SignificantToken, value: Option<Expr>) -> Expr {
    if is_token!(&keyword.token, break) {
        Expr!(Control::Break {
            break_token: typed_token(keyword),
            value: value.map(Box::new),
        })
    } else if is_token!(&keyword.token, continue) {
        Expr!(Control::Continue {
            continue_token: typed_token(keyword),
            value: value.map(Box::new),
        })
    } else if is_token!(&keyword.token, return) {
        Expr!(Control::Return {
            return_token: typed_token(keyword),
            value: value.map(Box::new),
        })
    } else {
        let value = value.expect("catch, throw, and trap require a value");
        if is_token!(&keyword.token, catch) {
            Expr!(Catch {
                catch_token: typed_token(keyword),
                value: Box::new(value),
            })
        } else if is_token!(&keyword.token, throw) {
            Expr!(Throw {
                throw_token: typed_token(keyword),
                value: Box::new(value),
            })
        } else if is_token!(&keyword.token, trap) {
            Expr!(Trap {
                trap_token: typed_token(keyword),
                value: Box::new(value),
            })
        } else {
            unreachable!("statement actions are assigned only to statement keywords")
        }
    }
}

/// Lowers atom.
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
    /// The Expression alternative.
    Expression(Box<Expr>),
    /// The Assignment alternative.
    Assignment {
        /// The target.
        target: AssignmentTarget,
        /// The local.
        local: bool,
    },
    /// The Lambda alternative.
    Lambda(LambdaArguments),
}

/// Internal alternatives for LambdaArguments.
pub(crate) enum LambdaArguments {
    /// The Variadic alternative.
    Variadic(Symbol),
    /// The List alternative.
    List(Box<Delimited<Option<ArgumentListContent>>>),
}

impl BinaryLeft {
    /// Handles prepare.
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
                lambda_arguments(left).ok_or(NativeParseError::InvalidLambdaArguments { span })?,
            ),
            _ => Self::Expression(Box::new(left)),
        })
    }
}

/// A parsed expression that can occur to the left of an assignment operator.
enum AssignmentTarget {
    /// The Symbol alternative.
    Symbol(Symbol),
    /// The Operator alternative.
    Operator(ExprOp),
    /// The New alternative.
    New(ExprNew),
    /// The List alternative.
    List(Box<AssignmentList>),
}

impl AssignmentTarget {
    /// Handles parse.
    fn parse(expression: Expr, local: bool, span: Span) -> Result<Self, NativeParseError> {
        match expression {
            Expr::Symbol(variable) => Ok(Self::Symbol(variable)),
            Expr::Op(expression) => Ok(Self::Operator(expression)),
            Expr::New(statement) if local => Ok(Self::New(statement)),
            expression => assignment_list(expression)
                .map(Box::new)
                .map(Self::List)
                .ok_or(NativeParseError::InvalidAssignmentTarget { span }),
        }
    }
}

/// Lowers binary.
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
        BinaryLeft::Lambda(arguments) => Ok(match arguments {
            LambdaArguments::Variadic(argument) => Expr!(Lambda::Variadic {
                argument,
                operator: typed_token(operator),
                body: Box::new(right),
            }),
            LambdaArguments::List(arguments) => Expr!(Lambda::List {
                arguments: *arguments,
                operator: typed_token(operator),
                body: Box::new(right),
            }),
        }),
        BinaryLeft::Expression(left) if spelling == "<-" => Ok(Expr!(Assign::Evaluated {
            evaluated_domain: left,
            operator: typed_token(operator),
            value: Box::new(right),
        })),
        BinaryLeft::Expression(left) if spelling == "=>" => Ok(Expr!(Option {
            key: left,
            operator: typed_token(operator),
            value: Box::new(right),
        })),
        BinaryLeft::Expression(left)
            if GENERATED_AUGMENTED_OPERATOR_SPELLINGS.contains(&spelling) =>
        {
            let operator = AugmentedOperator::from_token_tree_with_trivia(
                operator.token,
                operator.trailing_trivia,
            )
            .expect("the augmented operator table and category have the same source");
            Ok(Expr!(Assign::Augmented {
                target: left,
                operator,
                value: Box::new(right),
            }))
        }
        BinaryLeft::Expression(left) if matches!(spelling, "." | ".?") => Ok(match spelling {
            "." => Expr!(Op::Bin::Member::Dot {
                receiver: left,
                operator: typed_token::<Token![.]>(operator),
                member: Box::new(right),
            }),
            ".?" => Expr!(Op::Bin::Member::Optional {
                receiver: left,
                operator: typed_token::<Token![.?]>(operator),
                member: Box::new(right),
            }),
            _ => unreachable!(),
        }),
        BinaryLeft::Expression(left) if spelling == "#" => Ok(Expr!(Op::Bin::Index {
            receiver: left,
            operator: typed_token(operator),
            index: Box::new(right),
        })),
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
            Ok(Expr!(Op::Bin::Regular {
                left_operand: left,
                operator,
                right_operand: Box::new(right),
            }))
        }
    }
}

/// Lowers assignment.
fn lower_assignment(
    target: AssignmentTarget,
    operator: SignificantToken,
    right: Expr,
    local: bool,
) -> Expr {
    match (local, target) {
        (false, AssignmentTarget::Symbol(variable)) => {
            Expr!(Assign::Simple::Global {
                variable,
                operator: typed_token(operator),
                value: Box::new(right),
            })
        }
        (true, AssignmentTarget::Symbol(variable)) => {
            Expr!(Assign::Simple::Local {
                variable,
                operator: typed_token(operator),
                value: Box::new(right),
            })
        }
        (false, AssignmentTarget::Operator(argument_expr)) => {
            Expr!(Assign::Op {
                argument_expr,
                operator: typed_token(operator),
                value_expr: Box::new(right),
            })
        }
        (true, AssignmentTarget::Operator(ExprOp::Bin(ExprOpBin::Adj(application)))) => {
            let (typical_value, function) = installation_value(right);
            Expr!(Assign::Install::Method {
                method: application.left_operand,
                operator: application.operator,
                signature: application.right_operand,
                assign_op: typed_token(operator),
                typical_value,
                function: Box::new(function),
            })
        }
        (true, AssignmentTarget::Operator(domain_expr)) => {
            let (typical_value, function) = installation_value(right);
            Expr!(Assign::Install::Op {
                domain_expr,
                assign_op: typed_token(operator),
                typical_value,
                function: Box::new(function),
            })
        }
        (true, AssignmentTarget::New(new_statement)) => {
            Expr!(Assign::Install::New {
                new_statement,
                operator: typed_token(operator),
                function: Box::new(right),
            })
        }
        (local, AssignmentTarget::List(argument_list)) => {
            if local {
                Expr!(Assign::Parallel::Local {
                    argument_list,
                    operator: typed_token(operator),
                    value: Box::new(right),
                })
            } else {
                Expr!(Assign::Parallel::Global {
                    argument_list,
                    operator: typed_token(operator),
                    value: Box::new(right),
                })
            }
        }
        (false, AssignmentTarget::New(_)) => {
            unreachable!("global assignment preparation rejects new statements")
        }
    }
}

/// Handles installation value.
pub(crate) fn installation_value(value: Expr) -> (Option<TypeAnnotation>, Expr) {
    match value {
        Expr::Option(value) => (
            Some(TypeAnnotation::new(value.key, value.operator)),
            *value.value,
        ),
        value => (None, value),
    }
}

/// Parses cell stream.
fn parse_cell_stream(cells: CellStream) -> Result<SourceFile, NativeParseError> {
    let mut elements = Vec::new();
    let eof_span = cells.physical_eof();
    // Comments of a comment-only cell lead the next cell that has tokens, or
    // close the file when no such cell follows.
    let mut detached = Vec::new();
    let mut cells = cells.into_iter().peekable();
    while let Some(cell) = cells.next() {
        if cell.is_empty() {
            detached.extend(CommentTrivia::of(&cell.into_parser_token_stream()).into_trivia());
            continue;
        }
        let eoc = cells
            .peek()
            .and_then(cell_start_span)
            .unwrap_or_else(|| end_span(&cell));
        if let Some(element) = parse_raw_cell_at(cell, eoc, std::mem::take(&mut detached))? {
            elements.push(element);
        }
    }
    let mut source_file = SourceFile::new(elements, EOF::new("", eof_span));
    source_file.set_closing_comments(detached);
    Ok(source_file)
}

/// Handles tokenize for native parser.
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

/// Parses raw cell.
pub(crate) fn parse_raw_cell(
    tokens: CellTokenStream,
) -> Result<Option<crate::nodes::Cell>, NativeParseError> {
    let eoc = end_span(&tokens);
    parse_raw_cell_at(tokens, eoc, Vec::new())
}

/// Parses raw cell at.
///
/// `detached` comments, which precede the cell outside it, lead the comments
/// before its first token in the cell's leading trivia.
fn parse_raw_cell_at(
    tokens: CellTokenStream,
    eoc: Span,
    mut detached: Vec<Trivia>,
) -> Result<Option<crate::nodes::Cell>, NativeParseError> {
    let ends_with_semicolon = ends_with_cell_semicolon(&tokens);
    let mut tokens = tokens.into_parser_token_stream();
    detached.extend(tokens.take_prefix_trivia().into_vec());
    let semicolon = ends_with_semicolon
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
        cell.leading_trivia = LeadingTrivia::new(detached);
    }
    Ok(cell)
}

/// Handles cell start span.
fn cell_start_span(tokens: &CellTokenStream) -> Option<Span> {
    let token = tokens.iter().next()?;
    let span = token
        .leading_trivia()
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

/// Handles token parse error.
fn token_parse_error(error: NativeParseError) -> TokenParseError {
    let span = error.expect_span();
    TokenParseError::Syntax {
        message: error.to_string(),
        span,
    }
}

/// Handles parsing spelling.
fn parsing_spelling(token: &TokenTree) -> &str {
    token.spelling().map_or("", canonical_word_spelling)
}

/// Handles typed token.
fn typed_token<T: Token>(token: SignificantToken) -> T {
    let spelling = token.token.description();
    T::from_token_tree_with_trivia(token.token, token.trailing_trivia)
        .unwrap_or_else(|| panic!("`{spelling}` cannot be lowered to `{}`", T::SPELLING))
}

/// Handles typed terminal.
fn typed_terminal<T: Terminal>(mut terminal: T, trailing_trivia: TrailingTrivia) -> T {
    *terminal.trailing_trivia_mut() = trailing_trivia;
    terminal
}

/// Handles empty span at.
fn empty_span_at(lookahead: &Span) -> Span {
    lookahead.subspan(0, 0)
}

/// Lowers string literal.
fn lower_string_literal(text: &str, span: Span, trailing_trivia: TrailingTrivia) -> Expr {
    let body_end = if text.len() > 1 && text.ends_with('"') {
        text.len() - 1
    } else {
        text.len()
    };
    let body = &text[1..body_end];
    let mut elements = Vec::new();
    let mut content_start = 0;
    let mut cursor = 0;

    while cursor < body.len() {
        if body.as_bytes()[cursor] != b'\\' {
            cursor += 1;
            continue;
        }
        if content_start < cursor {
            elements.push(StringElement::Content(StringContent::new(
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
        elements.push(StringElement::Escape(EscapeSequence::new(
            &body[cursor..escape_end],
            span.subspan(cursor + 1, escape_end + 1),
        )));
        cursor = escape_end;
        content_start = cursor;
    }
    if content_start < body.len() {
        elements.push(StringElement::Content(StringContent::new(
            &body[content_start..],
            span.subspan(content_start + 1, body_end),
        )));
    }
    typed_terminal(StringLiteral::new(elements, span), trailing_trivia).into()
}

/// Lowers raw string literal.
fn lower_raw_string_literal(text: &str, span: Span, trailing_trivia: TrailingTrivia) -> Expr {
    let body_end = if text.len() >= 6 && text.ends_with("///") {
        text.len() - 3
    } else {
        text.len()
    };
    let body = &text[3..body_end];
    let mut elements = Vec::new();
    let mut content_start = 0;
    let mut cursor = 0;

    while cursor + 1 < body.len() {
        if &body.as_bytes()[cursor..cursor + 2] != b"//" {
            cursor += 1;
            continue;
        }
        if content_start < cursor {
            elements.push(RawStringElement::Content(RawStringContent::new(
                &body[content_start..cursor],
                span.subspan(content_start + 3, cursor + 3),
            )));
        }
        elements.push(RawStringElement::Escape(EscapeSequence::new(
            "//",
            span.subspan(cursor + 3, cursor + 5),
        )));
        cursor += 2;
        content_start = cursor;
    }
    if content_start < body.len() {
        elements.push(RawStringElement::Content(RawStringContent::new(
            &body[content_start..],
            span.subspan(content_start + 3, body_end),
        )));
    }
    typed_terminal(RawStringLiteral::new(elements, span), trailing_trivia).into()
}

/// Handles collection contents.
fn collection_contents(expression: Expr) -> Option<(crate::Delimiter, Option<PackedContent>)> {
    match expression {
        Expr::Collection(ExprCollection::Sequence(value)) if value.statements.is_empty() => {
            Some((DelimiterValue::delimiter(&value.delimiter), value.elements))
        }
        Expr::Collection(ExprCollection::List(value)) if value.statements.is_empty() => {
            Some((DelimiterValue::delimiter(&value.delimiter), value.elements))
        }
        Expr::Collection(ExprCollection::Array(value)) if value.statements.is_empty() => {
            Some((DelimiterValue::delimiter(&value.delimiter), value.elements))
        }
        Expr::Collection(ExprCollection::AngleBar(value)) if value.statements.is_empty() => {
            Some((DelimiterValue::delimiter(&value.delimiter), value.elements))
        }
        Expr::Holder(value) if value.muted.is_empty() => {
            let elements = value.value.map(|value| (*value).into());
            Some((DelimiterValue::delimiter(&value.delimiter), elements))
        }
        _ => None,
    }
}

/// Handles assignment list.
pub(crate) fn assignment_list(expression: Expr) -> Option<AssignmentList> {
    let (delimiter, elements) = collection_contents(expression)?;
    let contents = match elements {
        Some(elements) => {
            let components = packed_components(elements).map(assignment_argument)?;
            Some(match components.into_punctuated() {
                Ok(arguments) => AssignmentListContent!(Pack { arguments }),
                Err(argument) => argument.into(),
            })
        }
        None => None,
    };
    Some(AssignmentList::new(Delimited::new(delimiter, contents)))
}

/// Handles assignment argument.
fn assignment_argument(component: Component) -> Option<AssignmentArgument> {
    match component {
        Component::Empty(value) => Some(AssignmentArgument::Empty(value)),
        Component::Expr(value) => match *value {
            Expr::Symbol(value) => Some(AssignmentArgument::Symbol(value)),
            Expr::Op(value) => Some(AssignmentArgument::Op(value)),
            Expr::Assign(ExprAssign::Evaluated(value)) => {
                Some(AssignmentArgument::Evaluated(value))
            }
            value => assignment_list(value).map(Into::into),
        },
    }
}

/// Handles expr pack symbols.
fn expr_pack_symbols(elements: ExprPack) -> Option<Punctuated<Symbol>> {
    elements
        .pack
        .try_map(
            |component| match component {
                Component::Expr(value) => match *value {
                    Expr::Symbol(value) => Ok(value),
                    _ => Err(()),
                },
                Component::Empty(_) => Err(()),
            },
            Ok,
        )
        .ok()
}

/// Handles packed components.
fn packed_components(content: PackedContent) -> Components<Component> {
    match content {
        PackedContent::Expr(expression) => Components {
            first: Component::Expr(expression),
            rest: Vec::new(),
        },
        PackedContent::Pack(pack) => Components::from_punctuated(pack.pack),
    }
}

/// Handles lambda arguments.
pub(crate) fn lambda_arguments(expression: Expr) -> Option<LambdaArguments> {
    match expression {
        Expr::Symbol(value) => Some(LambdaArguments::Variadic(value)),
        expression => {
            let (delimiter, elements) = collection_contents(expression)?;
            let arguments = match elements {
                Some(PackedContent::Expr(expression)) => match *expression {
                    Expr::Symbol(value) => Some(ArgumentListContent::Symbol(value)),
                    _ => return None,
                },
                Some(PackedContent::Pack(pack)) => Some(ArgumentListContent!(Pack {
                    arguments: expr_pack_symbols(pack)?,
                })),
                None => None,
            };
            Some(LambdaArguments::List(Box::new(Delimited::new(
                delimiter, arguments,
            ))))
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

/// Handles keyword action.
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

/// Parses info.
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
                "invalid lambda argument list",
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
