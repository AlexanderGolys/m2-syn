//! Direct construction of the typed graph from a flat token stream.
//!
//! The native precedence engine owns parsing decisions only. Token storage,
//! cursor position and newline detection belong to
//! [`ParseStream`], shared with generated [`Parse`]
//! implementations.

use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::cell_stream::{end_span, ends_with_cell_semicolon, frame_recovering};
use crate::lexer::canonical_spelling;
use crate::nodes::*;
use crate::parse::{Lookahead, SignificantToken};
use crate::{
    CellStream, CellStreamError, CellTokenStream, Delimited, DelimiterKind, DelimiterToken,
    DelimiterValue, LexError, LexErrorKind, LiteralKind, Parse, ParseStream, Parser, Punctuated,
    Recovered, Span, Spanned, Token, TokenParseError, TokenStream, TokenTree, Tokenize, lex_str,
    lex_str_recovering,
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

        let eoc = Token![EOC](eoc_span);
        Ok(Some(crate::nodes::Cell::new(
            self.packed_content(components)?,
            semicolon,
            eoc,
        )))
    }

    /// Parses components.
    fn parse_components(&mut self) -> Result<Components<Element>, NativeParseError> {
        let first = if self.at_punctuation(",") {
            Element::Empty(Empty::new("", empty_span_at(&self.peek().expect_span())))
        } else {
            self.parse_required(PREC_COMMA)?.into()
        };
        let mut rest = Vec::new();

        while let Some(comma) = self.take_punctuation(",") {
            let next = if self.at_end() || self.at_punctuation(";") || self.at_punctuation(",") {
                Element::Empty(Empty::new("", empty_span_at(&self.peek().expect_span())))
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
    ) -> Result<Option<Components<Element>>, NativeParseError> {
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
                let operator = PrefixOperator::from_token_tree(token.token).ok_or({
                    NativeParseError::Unsupported {
                        syntax: spelling,
                        span,
                    }
                })?;
                Expr::from(ExprPrefix {
                    op: operator,
                    expr: Box::new(operand),
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
                if matches!(
                    &quoted.token,
                    TokenTree::Literal(literal)
                        if matches!(literal.kind, LiteralKind::String | LiteralKind::RawString)
                ) {
                    return Err(unexpected!(
                        quoted.token,
                        "a quoted token other than a string literal"
                    ));
                }
                let quoted_span = quoted.token.expect_span();
                let Some(quoted_text) = quoted.token.spelling().map(str::to_owned) else {
                    return Err(unexpected!(quoted.token, "a quoted token"));
                };
                let quoted = Symbol::new(quoted_text, quoted_span);
                let keyword = if is_token!(&token.token, symbol) {
                    QuoteKeyword::Symbol(typed_token(token))
                } else if is_token!(&token.token, local) {
                    QuoteKeyword::Local(typed_token(token))
                } else if is_token!(&token.token, global) {
                    QuoteKeyword::Global(typed_token(token))
                } else if is_token!(&token.token, threadLocal) {
                    QuoteKeyword::ThreadLocal(typed_token(token))
                } else {
                    unreachable!("quote actions are assigned only to quote keywords")
                };
                Expr::from(ExprQuote {
                    keyword,
                    symbol: quoted,
                })
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
        Ok(Expr::from(ExprIf {
            if_token: typed_token(keyword),
            cond: Box::new(condition),
            then_clause,
            else_clause,
        }))
    }

    /// Parses for loop.
    fn parse_for_loop(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let var = self.consume_symbol("a loop variable")?;
        let iter = if let Some(token) = self.take_exact("in") {
            let collection = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
            Some(ForIter::In(InClause::new(
                typed_token(token),
                Box::new(collection),
            )))
        } else {
            let from_clause = self
                .take_exact("from")
                .map(|token| {
                    let value = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
                    Ok::<_, NativeParseError>(FromClause::new(typed_token(token), Box::new(value)))
                })
                .transpose()?;
            let to_clause = self
                .take_exact("to")
                .map(|token| {
                    let value = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
                    Ok::<_, NativeParseError>(ToClause::new(typed_token(token), Box::new(value)))
                })
                .transpose()?;
            (from_clause.is_some() || to_clause.is_some())
                .then(|| ForIter::Range(ForRange::new(from_clause, to_clause)))
        };
        let when_clause = self
            .take_exact("when")
            .map(|token| {
                let condition = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
                Ok::<_, NativeParseError>(WhenClause::new(typed_token(token), Box::new(condition)))
            })
            .transpose()?;
        let body = self.parse_loop_body()?;

        Ok(Expr::from(ExprFor {
            for_token: typed_token(keyword),
            var,
            iter,
            when_clause,
            body,
        }))
    }

    /// Parses while loop.
    fn parse_while_loop(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let condition = self.required_clause_expression(PREC_CONTROL, &keyword.token)?;
        let body = self.parse_loop_body()?;
        Ok(Expr::from(ExprWhile {
            while_token: typed_token(keyword),
            cond: Box::new(condition),
            body,
        }))
    }

    /// Parses loop body.
    fn parse_loop_body(&mut self) -> Result<LoopBody, NativeParseError> {
        let (list_clause, do_clause) = if let Some(token) = self.take_exact("list") {
            let listed_value = self.required_clause_expression(PREC_CONTROL, &token.token)?;
            let list_clause = Some(ListClause::new(typed_token(token), Box::new(listed_value)));
            if let Some(token) = self.take_exact("do") {
                let ignored_value = self.required_clause_expression(PREC_CONTROL, &token.token)?;
                (
                    list_clause,
                    Some(DoClause::new(typed_token(token), Box::new(ignored_value))),
                )
            } else {
                (list_clause, None)
            }
        } else if let Some(token) = self.take_exact("do") {
            let ignored_value = self.required_clause_expression(PREC_CONTROL, &token.token)?;
            (
                None,
                Some(DoClause::new(typed_token(token), Box::new(ignored_value))),
            )
        } else {
            return Err(self.expected_at_lookahead("`list` or `do`"));
        };

        Ok(LoopBody::new(list_clause, do_clause))
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
            Some(TryFallback::Except(ExceptClause::new(
                typed_token(except_keyword),
                exception,
                typed_token(do_keyword),
                Box::new(value),
            )))
        } else {
            None
        };

        Ok(Expr::from(ExprTry {
            try_token: typed_token(keyword),
            expr: Box::new(value),
            then_clause,
            fallback,
        }))
    }

    /// Parses new statement.
    fn parse_new_statement(&mut self, keyword: SignificantToken) -> Result<Expr, NativeParseError> {
        let class = self.required_clause_expression(PREC_LOOP_CLAUSE, &keyword.token)?;
        let of_clause = if let Some(token) = self.take_exact("of") {
            let parent = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
            Some(OfClause::new(typed_token(token), Box::new(parent)))
        } else {
            None
        };
        let from_clause = if let Some(token) = self.take_exact("from") {
            let instance = self.required_clause_expression(PREC_LOOP_CLAUSE, &token.token)?;
            Some(FromClause::new(typed_token(token), Box::new(instance)))
        } else {
            None
        };

        Ok(Expr::from(ExprNew {
            new_token: typed_token(keyword),
            class: Box::new(class),
            of_clause,
            from_clause,
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
            let keyword = match spelling.as_str() {
                "time" => TimingKeyword::Time(typed_token(keyword)),
                "timing" => TimingKeyword::Timing(typed_token(keyword)),
                "elapsedTime" => TimingKeyword::ElapsedTime(typed_token(keyword)),
                "elapsedTiming" => TimingKeyword::ElapsedTiming(typed_token(keyword)),
                "profile" => TimingKeyword::Profile(typed_token(keyword)),
                _ => unreachable!("timing actions are assigned only to timing keywords"),
            };
            return Ok(Expr::from(ExprTiming {
                keyword,
                expr: Box::new(value),
            }));
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
        let keyword = match spelling.as_str() {
            "step" => DebugKeyword::Step(typed_token(keyword)),
            "finish" => DebugKeyword::Finish(typed_token(keyword)),
            "shield" => DebugKeyword::Shield(typed_token(keyword)),
            "TEST" => DebugKeyword::Test(typed_token(keyword)),
            "breakpoint" => DebugKeyword::Breakpoint(typed_token(keyword)),
            _ => unreachable!("debug actions are assigned only to debug keywords"),
        };
        Ok(Expr::from(ExprDebug {
            keyword,
            expr: value,
        }))
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
                Ok(Symbol::new(text, span))
            }
            Lookahead::Token(token) => Err(unexpected!(token.token, expected)),
            Lookahead::End(span) => Err(NativeParseError::unexpected_end(expected, span)),
        }
    }

    /// Consumes the word a construct continues with, which M2 matches by its
    /// exact spelling: `Core$then` does not continue an `if`.
    fn consume_exact(
        &mut self,
        spelling: &'static str,
    ) -> Result<SignificantToken, NativeParseError> {
        match self.consume() {
            Lookahead::Token(token) if token.token.spelling() == Some(spelling) => Ok(token),
            Lookahead::Token(token) => Err(unexpected!(token.token, spelling)),
            Lookahead::End(span) => Err(NativeParseError::unexpected_end(spelling, span)),
        }
    }

    /// Takes the word a construct may continue with, matched like
    /// [`Self::consume_exact`].
    fn take_exact(&mut self, spelling: &'static str) -> Option<SignificantToken> {
        if matches!(
            self.peek(),
            Lookahead::Token(token) if token.token.spelling() == Some(spelling)
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
                adjacency(left, typed_token(operator), right)
            } else if let Some(strength) = info.binary_strength {
                let left = BinaryLeft::prepare(left, &operator.token)?;
                let right = self
                    .parse_required(strength)
                    .map_err(|error| error.missing_operand_for(&operator.token))?;
                lower_binary(left, operator, right)?
            } else if info.postfix {
                let span = operator.token.expect_span();
                let spelling = operator.token.description();
                let operator = PostfixOperator::from_token_tree(operator.token).ok_or(
                    NativeParseError::Unsupported {
                        syntax: spelling,
                        span,
                    },
                )?;
                Expr::from(ExprPostfix {
                    expr: Box::new(left),
                    op: operator,
                })
            } else if !matches!(info.unary, UnaryAction::Error) {
                if operator.crossed_newline {
                    return Err(NativeParseError::NewlineInApplication {
                        span: operator.token.expect_span(),
                    });
                }
                let adjacency_span = operator.token.expect_span().subspan(0, 0);
                let right = self.parse_consumed(operator, PREC_APPLICATION_RIGHT)?;
                adjacency(left, Token![SPACE](adjacency_span), right)
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
    fn only_expression(&self, components: Components<Element>) -> Result<Expr, NativeParseError> {
        if components.is_punctuated() {
            return Err(NativeParseError::Unexpected {
                found: "a sequence".into(),
                expected: "one expression",
                span: components.first.expect_span(),
            });
        }
        match components.first {
            Element::Expr(value) => Ok(*value),
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
        components: Components<Element>,
    ) -> Result<Elements, NativeParseError> {
        match components.into_punctuated() {
            Ok(components) => Ok(Elements::Pack(ElementsPack::new(components))),
            Err(Element::Expr(expression)) => Ok((*expression).into()),
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
    let group = match token.token {
        TokenTree::Group(group) => group,
        token => return Err(unexpected!(token, "a delimited group")),
    };
    let kind = group.delim_kind();
    let delimiter_spans = group.delimiter().spans.clone();
    let mut contents = Engine::new(group.into_stream());
    let mut muted = Vec::new();
    let elements = loop {
        let elements = contents.parse_optional_components()?;
        let Some(separator) = contents.take_punctuation(";") else {
            break elements;
        };
        let Some(elements) = elements else {
            return Err(unexpected!(separator.token, "a statement before `;`"));
        };
        muted.push(Stmt::new(
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
            let mut value = ExprArray::new(muted, elements);
            value.delimiter = DelimiterToken::new(delimiter_spans.clone());
            value.into()
        }
        DelimiterKind::Brace => {
            let elements = elements
                .map(|elements| contents.packed_content(elements))
                .transpose()?;
            let mut value = ExprList::new(muted, elements);
            value.delimiter = DelimiterToken::new(delimiter_spans.clone());
            value.into()
        }
        DelimiterKind::AngleBar => {
            let elements = elements
                .map(|elements| contents.packed_content(elements))
                .transpose()?;
            let mut value = ExprAngleBar::new(muted, elements);
            value.delimiter = DelimiterToken::new(delimiter_spans.clone());
            value.into()
        }
        DelimiterKind::Parenthesis if muted.is_empty() && elements.is_none() => {
            let mut value = ExprSequence::new(Vec::new(), None);
            value.delimiter = DelimiterToken::new(delimiter_spans.clone());
            value.into()
        }
        DelimiterKind::Parenthesis
            if !muted.is_empty() || elements.as_ref().is_some_and(Components::is_punctuated) =>
        {
            let mut value = if elements.as_ref().is_some_and(Components::is_punctuated) {
                ExprSequence::new(
                    muted,
                    elements
                        .map(|elements| contents.packed_content(elements))
                        .transpose()?,
                )
            } else {
                let value = elements
                    .map(|elements| contents.only_expression(elements))
                    .transpose()?;
                let mut value = ExprParen::new(muted, value.map(Box::new));
                value.delimiter = DelimiterToken::new(delimiter_spans.clone());
                return Ok(value.into());
            };
            value.delimiter = DelimiterToken::new(delimiter_spans.clone());
            value.into()
        }
        DelimiterKind::Parenthesis => {
            let value = elements
                .map(|elements| contents.only_expression(elements))
                .transpose()?;
            let mut value = ExprParen::new(Vec::new(), value.map(Box::new));
            value.delimiter = DelimiterToken::new(delimiter_spans.clone());
            value.into()
        }
    };
    Ok(expression)
}

/// Lowers prefix statement.
fn lower_prefix_statement(keyword: SignificantToken, value: Option<Expr>) -> Expr {
    if is_token!(&keyword.token, break) {
        Expr::from(ExprBreak {
            break_token: typed_token(keyword),
            expr: value.map(Box::new),
        })
    } else if is_token!(&keyword.token, continue) {
        Expr::from(ExprContinue {
            continue_token: typed_token(keyword),
            expr: value.map(Box::new),
        })
    } else if is_token!(&keyword.token, return) {
        Expr::from(ExprReturn {
            return_token: typed_token(keyword),
            expr: value.map(Box::new),
        })
    } else {
        let value = value.expect("catch, throw, and trap require a value");
        if is_token!(&keyword.token, catch) {
            Expr::from(ExprCatch {
                catch_token: typed_token(keyword),
                expr: Box::new(value),
            })
        } else if is_token!(&keyword.token, throw) {
            Expr::from(ExprThrow {
                throw_token: typed_token(keyword),
                expr: Box::new(value),
            })
        } else if is_token!(&keyword.token, trap) {
            Expr::from(ExprTrap {
                trap_token: typed_token(keyword),
                expr: Box::new(value),
            })
        } else {
            unreachable!("statement actions are assigned only to statement keywords")
        }
    }
}

/// Lowers atom.
fn lower_atom(token: SignificantToken) -> Result<Expr, NativeParseError> {
    match token.token {
        TokenTree::Ident(raw) => Ok(Symbol::new(raw.text(), raw.expect_span()).into()),
        TokenTree::Literal(raw) => {
            let span = raw.expect_span();
            Ok(Expr::Literal(match raw.kind {
                LiteralKind::Integer => IntegerLiteral::new(raw.text(), span).into(),
                LiteralKind::Float => FloatLiteral::new(raw.text(), span).into(),
                LiteralKind::String => lower_string_literal(raw.text(), span),
                LiteralKind::RawString => lower_raw_string_literal(raw.text(), span),
            }))
        }
        token => Err(unexpected!(token, "an expression")),
    }
}

/// A left operand whose operator-specific structural requirements are valid.
enum BinaryLeft {
    /// An operand of an operator without structural requirements.
    Expression(Box<Expr>),
    /// The parameters left of `->`.
    Function(Box<FunctionParams>),
}

impl BinaryLeft {
    /// Validates the left operand against the requirements of `operator`.
    fn prepare(left: Expr, operator: &TokenTree) -> Result<Self, NativeParseError> {
        let span = operator.expect_span();
        Ok(match Some(parsing_spelling(operator)) {
            Some("=") if !is_assignment_target(&left, false) => {
                return Err(NativeParseError::InvalidAssignmentTarget { span });
            }
            Some(":=") if !is_assignment_target(&left, true) => {
                return Err(NativeParseError::InvalidAssignmentTarget { span });
            }
            Some("->") => Self::Function(Box::new(
                function_params(left).ok_or(NativeParseError::InvalidLambdaArguments { span })?,
            )),
            _ => Self::Expression(Box::new(left)),
        })
    }
}

/// Lowers binary.
fn lower_binary(
    left: BinaryLeft,
    operator: SignificantToken,
    right: Expr,
) -> Result<Expr, NativeParseError> {
    let span = operator.token.expect_span();
    let spelling = parsing_spelling(&operator.token).to_owned();
    let left = match left {
        BinaryLeft::Function(params) => {
            return Ok(Expr::from(ExprFunction {
                params: *params,
                arrow_token: typed_token(operator),
                body: Box::new(right),
            }));
        }
        BinaryLeft::Expression(left) => left,
    };
    let right = Box::new(right);
    Ok(match spelling.as_str() {
        "=" => assign(left, AssignOperator::Global(typed_token(operator)), right),
        ":=" => assign(left, AssignOperator::Local(typed_token(operator)), right),
        "<-" => assign(
            left,
            AssignOperator::Evaluated(typed_token(operator)),
            right,
        ),
        "=>" => Expr::from(ExprOption {
            key: left,
            fat_arrow_token: typed_token(operator),
            value: right,
        }),
        spelling if GENERATED_AUGMENTED_OPERATOR_SPELLINGS.contains(&spelling) => {
            let operator = AugmentedOperator::from_token_tree(operator.token)
                .expect("the augmented operator table and category have the same source");
            assign(left, AssignOperator::Augmented(operator), right)
        }
        "." | ".?" => {
            let op = if spelling == "." {
                FieldOperator::Dot(typed_token(operator))
            } else {
                FieldOperator::OptionalDot(typed_token(operator))
            };
            let Expr::Symbol(member) = *right else {
                return Err(NativeParseError::Unexpected {
                    found: "an expression".into(),
                    expected: "a symbol to the right of `.`",
                    span: right.expect_span(),
                });
            };
            Expr::from(ExprField {
                base: left,
                op,
                member,
            })
        }
        _ => {
            let op = BinaryOperator::from_token_tree(operator.token).ok_or(
                NativeParseError::Unsupported {
                    syntax: spelling.clone(),
                    span,
                },
            )?;
            Expr::from(ExprBinary { left, op, right })
        }
    })
}

/// Builds an assignment.
fn assign(left: Box<Expr>, op: AssignOperator, right: Box<Expr>) -> Expr {
    Expr::from(ExprAssign { left, op, right })
}

/// Parses cell stream.
fn parse_cell_stream(cells: CellStream) -> Result<SourceFile, NativeParseError> {
    let mut elements = Vec::new();
    let bof_span = cells.physical_bof();
    let eof_span = cells.physical_eof();
    let mut cells = cells.into_iter().peekable();
    while let Some(cell) = cells.next() {
        let eoc = cells
            .peek()
            .and_then(cell_start_span)
            .unwrap_or_else(|| end_span(&cell));
        if let Some(element) = parse_raw_cell_at(cell, eoc)? {
            elements.push(element);
        }
    }
    Ok(SourceFile::new(
        Token![BOF](bof_span),
        elements,
        Token![EOF](eof_span),
    ))
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
    parse_raw_cell_at(tokens, eoc)
}

/// Parses raw cell at.
fn parse_raw_cell_at(
    tokens: CellTokenStream,
    eoc: Span,
) -> Result<Option<crate::nodes::Cell>, NativeParseError> {
    let ends_with_semicolon = ends_with_cell_semicolon(&tokens);
    let mut tokens = tokens.into_parser_token_stream();
    let semicolon = ends_with_semicolon
        .then(|| tokens.take_final_punctuation(";"))
        .flatten()
        .map(|punctuation| {
            <Token![;] as Token>::from_token_tree(TokenTree::Punct(punctuation))
                .expect("a semicolon punctuation token lowers to Token![;]")
        });
    Engine::new(tokens).parse_cell(semicolon, eoc)
}

/// Handles cell start span.
fn cell_start_span(tokens: &CellTokenStream) -> Option<Span> {
    let span = tokens.iter().next()?.span();
    span.start().map(|start| span.at(start))
}

impl Parse for Expr {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let tokens = input.by_ref().collect::<TokenStream>();
        let mut engine = Engine::new(tokens);
        let value = engine
            .parse_required(PREC_CLOSER)
            .map_err(token_parse_error)?;
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
    token.spelling().map_or("", canonical_spelling)
}

/// Handles typed token.
fn typed_token<T: Token>(token: SignificantToken) -> T {
    let spelling = token.token.description();
    T::from_token_tree(token.token)
        .unwrap_or_else(|| panic!("`{spelling}` cannot be lowered to `{}`", T::SPELLING))
}

/// Handles empty span at.
fn empty_span_at(lookahead: &Span) -> Span {
    lookahead.subspan(0, 0)
}

/// Lowers string literal.
fn lower_string_literal(text: &str, span: Span) -> Literal {
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
            elements.push(StringFragment::Content(StringContent::new(
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
        elements.push(StringFragment::Escape(EscapeSequence::new(
            &body[cursor..escape_end],
            span.subspan(cursor + 1, escape_end + 1),
        )));
        cursor = escape_end;
        content_start = cursor;
    }
    if content_start < body.len() {
        elements.push(StringFragment::Content(StringContent::new(
            &body[content_start..],
            span.subspan(content_start + 1, body_end),
        )));
    }
    StringLiteral::new(elements, span).into()
}

/// Lowers raw string literal.
fn lower_raw_string_literal(text: &str, span: Span) -> Literal {
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
            elements.push(RawStringFragment::Content(RawStringContent::new(
                &body[content_start..cursor],
                span.subspan(content_start + 3, cursor + 3),
            )));
        }
        elements.push(RawStringFragment::Escape(EscapeSequence::new(
            "//",
            span.subspan(cursor + 3, cursor + 5),
        )));
        cursor += 2;
        content_start = cursor;
    }
    if content_start < body.len() {
        elements.push(RawStringFragment::Content(RawStringContent::new(
            &body[content_start..],
            span.subspan(content_start + 3, body_end),
        )));
    }
    RawStringLiteral::new(elements, span).into()
}

/// The delimiter and elements of a collection without statements.
fn collection_contents(expression: Expr) -> Option<(crate::Delimiter, Option<Elements>)> {
    match expression {
        Expr::Sequence(value) if value.stmts.is_empty() => {
            Some((DelimiterValue::delimiter(&value.delimiter), value.elems))
        }
        Expr::List(value) if value.stmts.is_empty() => {
            Some((DelimiterValue::delimiter(&value.delimiter), value.elems))
        }
        Expr::Array(value) if value.stmts.is_empty() => {
            Some((DelimiterValue::delimiter(&value.delimiter), value.elems))
        }
        Expr::AngleBar(value) if value.stmts.is_empty() => {
            Some((DelimiterValue::delimiter(&value.delimiter), value.elems))
        }
        Expr::Paren(value) if value.stmts.is_empty() => Some((
            DelimiterValue::delimiter(&value.delimiter),
            value.expr.map(Elements::Single),
        )),
        _ => None,
    }
}

/// The elements of a collection without statements; `None` marks an empty slot.
fn collection_values(expression: &Expr) -> Option<Vec<Option<&Expr>>> {
    let (stmts, elems) = match expression {
        Expr::Sequence(value) => (&value.stmts, &value.elems),
        Expr::List(value) => (&value.stmts, &value.elems),
        Expr::Array(value) => (&value.stmts, &value.elems),
        Expr::AngleBar(value) => (&value.stmts, &value.elems),
        Expr::Paren(value) if value.stmts.is_empty() => {
            return Some(value.expr.as_deref().into_iter().map(Some).collect());
        }
        _ => return None,
    };
    if !stmts.is_empty() {
        return None;
    }
    Some(match elems {
        None => Vec::new(),
        Some(Elements::Single(value)) => vec![Some(value)],
        Some(Elements::Pack(pack)) => pack
            .elems
            .iter()
            .map(|element| match element {
                Element::Expr(value) => Some(&**value),
                Element::Empty(_) => None,
            })
            .collect(),
    })
}

/// Joins two operands by the `SPACE` operator: `f x`.
fn adjacency(left: Expr, space: Token![SPACE], right: Expr) -> Expr {
    Expr::from(ExprBinary {
        left: Box::new(left),
        op: BinaryOperator::Space(space),
        right: Box::new(right),
    })
}

/// Whether an operation can receive a value through an installed method.
fn is_operation(expression: &Expr) -> bool {
    matches!(
        expression,
        Expr::Binary(_) | Expr::Field(_) | Expr::Prefix(_) | Expr::Postfix(_)
    )
}

/// Whether an expression may stand left of `=`, or of `:=` when `local`.
pub(crate) fn is_assignment_target(expression: &Expr, local: bool) -> bool {
    match expression {
        Expr::Symbol(_) => true,
        Expr::New(_) => local,
        expression => is_operation(expression) || is_parallel_target(expression),
    }
}

/// Whether an expression is a collection of parallel assignment targets.
fn is_parallel_target(expression: &Expr) -> bool {
    collection_values(expression).is_some_and(|values| {
        values.into_iter().flatten().all(|value| {
            matches!(value, Expr::Symbol(_))
                || is_operation(value)
                || matches!(value, Expr::Assign(value) if matches!(value.op, AssignOperator::Evaluated(_)))
                || is_parallel_target(value)
        })
    })
}

/// The parameters of a function whose left operand of `->` is `expression`.
pub(crate) fn function_params(expression: Expr) -> Option<FunctionParams> {
    if let Expr::Symbol(symbol) = expression {
        return Some(FunctionParams::Variadic(symbol));
    }
    let (delimiter, elements) = collection_contents(expression)?;
    let params = match elements {
        None => None,
        Some(Elements::Single(value)) => match *value {
            Expr::Symbol(symbol) => Some(Params::Single(symbol)),
            _ => return None,
        },
        Some(Elements::Pack(pack)) => {
            let symbols = pack
                .elems
                .try_map(
                    |element| match element {
                        Element::Expr(value) => match *value {
                            Expr::Symbol(symbol) => Ok(symbol),
                            _ => Err(()),
                        },
                        Element::Empty(_) => Err(()),
                    },
                    Ok,
                )
                .ok()?;
            Some(Params::Pack(ParamsPack::new(symbols)))
        }
    };
    Some(FunctionParams::List(ParamList::new(Delimited::new(
        delimiter, params,
    ))))
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
        "symbol" | "global" | "local" | "threadLocal" => UnaryAction::Quote,
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
    use super::{NativeCellParser, NativeCellState, NativeParseError};
    use crate::cell_stream::frame_recovering;
    use crate::{NativeRealtimeParser, Spanned, lex_str, parse_native, parse_native_recovering};

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

    #[test]
    fn quote_modifiers_reject_string_literals_with_a_located_syntax_error() {
        for modifier in ["symbol", "local", "global", "threadLocal", "threadVariable"] {
            for literal in ["\"a\"", "///a///"] {
                let source = format!("{modifier} {literal}");
                for result in [
                    parse_native(&source),
                    parse_native_recovering(&source).map(|recovered| recovered.value),
                ] {
                    let error = result.unwrap_err();
                    assert!(
                        matches!(error, NativeParseError::Unexpected { .. }),
                        "{source}: {error}"
                    );
                    assert_eq!(error.span().text(), Some(literal), "{source}");
                    assert_eq!(
                        error.to_string(),
                        format!(
                            "expected a quoted token other than a string literal, found `{literal}`"
                        ),
                        "{source}",
                    );
                }
                let mut parser = NativeCellParser::new();
                assert_eq!(
                    parser.push_line(&source),
                    NativeCellState::Invalid,
                    "{source}"
                );

                #[cfg(feature = "tree-sitter")]
                {
                    assert!(crate::parse_file(&source).is_err(), "{source}");
                    assert!(crate::parse_file_recovering(&source).is_err(), "{source}");
                }
            }
        }
    }

    #[test]
    fn invalid_quoted_string_preserves_realtime_cells_and_comments() {
        let source = "a = 1\nsymbol -- reason\n\"a\"\nb = 2";
        let parser = NativeRealtimeParser::new(source);
        assert_eq!(parser.state(), NativeCellState::Invalid);
        assert_eq!(parser.cells().len(), 3);
        let cells = parser.cells();
        assert_eq!(cells[0].state(), NativeCellState::Complete);
        assert_eq!(cells[1].state(), NativeCellState::Invalid);
        assert_eq!(cells[2].state(), NativeCellState::Complete);
        let error = cells[1].parse_error().unwrap();
        assert_eq!(error.span().text(), Some("\"a\""));
        assert_eq!(cells[1].tokens().to_string(), "symbol -- reason\n\"a\"");
    }

    #[test]
    fn quote_modifiers_continue_to_accept_non_string_tokens() {
        for modifier in ["symbol", "local", "global", "threadLocal", "threadVariable"] {
            for token in ["a", "if", "symbol", "+", ";", "(", ")", "1", "1.5"] {
                let source = format!("{modifier} {token}");
                assert!(parse_native(&source).is_ok(), "{source}");
            }
        }
    }
}
