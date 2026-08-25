//! Direct construction of the typed graph from a flat token stream.
//!
//! The native precedence engine owns parsing decisions only. Token storage,
//! cursor position, trivia skipping, and newline detection belong to
//! [`ParseStream`], shared with generated [`Parse`]
//! implementations.

use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::lexer::{canonical_word_spelling, split_cells};
use crate::nodes::*;
use crate::parse::{Lookahead, SignificantToken};
use crate::{
    Delimited, DelimiterKind, DelimiterToken, DelimiterValue, LexError, LiteralKind, Parse,
    ParseStream, Parser, Punctuated, Recovered, Span, Spanned, Token, TokenParseError, TokenStream,
    TokenTree, lex_str, lex_str_recovering,
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
            span: token.span(),
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

    fn parse_stream(&mut self, tokens: TokenStream) -> Result<SourceFile, Self::Error> {
        parse_cells(tokens)
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
    NativeParser.parse_stream(tokens)
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
    let syntax = NativeParser.parse_stream(tokens)?;
    Ok(Recovered::new(syntax, diagnostics))
}

#[derive(Debug, Spanned)]
/// An error produced by the native lexer or precedence parser.
pub enum NativeParseError {
    /// Tokenization failed before parsing began.
    Lex(LexError),
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
                span: operator.span(),
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
        semicolon: Option<Span>,
        eoc_span: Span,
    ) -> Result<Option<crate::nodes::Cell>, NativeParseError> {
        self.skip_trivia();
        if matches!(self.peek(), Lookahead::End(_)) {
            return match semicolon {
                None => Ok(None),
                Some(span) => Err(NativeParseError::Unexpected {
                    found: ";".into(),
                    expected: "a statement before `;`",
                    span,
                }),
            };
        }

        let (components, _) = self.parse_components()?;

        if let Lookahead::Token(token) = self.peek() {
            return Err(unexpected!(token.token, "the end of the cell"));
        }

        let eoc = Eoc::new("", eoc_span);
        let cell =
            crate::nodes::Cell::new(ExprPack::new(components), semicolon.map(Token![;]), eoc);
        Ok(Some(cell))
    }

    fn parse_components(&mut self) -> Result<(Punctuated<Component>, bool), NativeParseError> {
        let first = if self.at_punctuation(",") {
            Component::Empty(Empty::new("", empty_span_at(&self.peek().span())))
        } else {
            self.parse_required(PREC_COMMA)?.into()
        };
        let mut elements = Punctuated::from_single(first);
        let mut has_comma = false;

        while let Some(comma) = self.take_punctuation(",") {
            has_comma = true;
            let next = if self.at_end() || self.at_punctuation(";") || self.at_punctuation(",") {
                Component::Empty(Empty::new("", empty_span_at(&self.peek().span())))
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
            UnaryAction::Atom => lower_atom(token.token)?,
            UnaryAction::Prefix => {
                let strength = info.unary_strength.expect("prefix operators have U");
                let operand = self.parse_required(level.max(strength))?;
                let span = token.token.span();
                let spelling = token.token.description();
                let operator = PrefixOperator::from_token_tree(token.token).ok_or({
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
                lower_prefix_statement(token.token, operand)
            }
            UnaryAction::Control => {
                if is_token!(&token.token, if) {
                    self.parse_if_statement(token.token)?
                } else if is_token!(&token.token, for) {
                    self.parse_for_loop(token.token)?
                } else if is_token!(&token.token, while) {
                    self.parse_while_loop(token.token)?
                } else if is_token!(&token.token, try) {
                    self.parse_try_statement(token.token)?
                } else if is_token!(&token.token, new) {
                    self.parse_new_statement(token.token)?
                } else {
                    unreachable!("control actions are assigned only to control keywords")
                }
            }
            UnaryAction::Debug => self.parse_debug_clause(token.token)?,
            UnaryAction::Delimiter => parse_delimited(token.token)?,
            UnaryAction::Quote => {
                let operator = token.token.description();
                let quoted = match self.consume() {
                    Lookahead::Token(quoted) => quoted.token,
                    Lookahead::End(span) => {
                        return Err(NativeParseError::MissingOperand { operator, span });
                    }
                };
                let specifier = if is_token!(&token.token, symbol) {
                    QuoteSpecifier::Symbol(typed_token(token.token))
                } else if is_token!(&token.token, local) {
                    QuoteSpecifier::Local(typed_token(token.token))
                } else if is_token!(&token.token, global) {
                    QuoteSpecifier::Global(typed_token(token.token))
                } else if is_token!(&token.token, threadVariable) {
                    QuoteSpecifier::ThreadVariable(typed_token(token.token))
                } else if is_token!(&token.token, threadLocal) {
                    QuoteSpecifier::ThreadLocal(typed_token(token.token))
                } else {
                    unreachable!("quote actions are assigned only to quote specifiers")
                };
                let quoted_span = quoted.span();
                let Some(quoted_text) = quoted.spelling().map(str::to_owned) else {
                    return Err(unexpected!(quoted, "a quoted token"));
                };
                let quoted = Symbol::new(quoted_text, quoted_span);
                QuoteExpression::new(specifier, quoted).into()
            }
            UnaryAction::Error => return Err(unexpected!(token.token, "an expression")),
        };
        self.accumulate(result, level)
    }

    fn parse_if_statement(&mut self, keyword: TokenTree) -> Result<Expr, NativeParseError> {
        let condition = self.required_clause_expression(PREC_CONTROL, &keyword)?;
        let then_keyword = self.consume_exact("then")?;
        let then_value = self.required_clause_expression(PREC_CONTROL, &then_keyword)?;
        let then_clause = ThenClause::new(typed_token(then_keyword), then_value);
        let else_clause = self
            .take_exact("else")
            .map(|else_keyword| {
                let value = self.required_clause_expression(PREC_CONTROL, &else_keyword)?;
                Ok::<_, NativeParseError>(ElseClause::new(typed_token(else_keyword), value))
            })
            .transpose()?;
        Ok(IfStatement::new(typed_token(keyword), condition, then_clause, else_clause).into())
    }

    fn parse_for_loop(&mut self, keyword: TokenTree) -> Result<Expr, NativeParseError> {
        let variable = self.consume_symbol("a loop variable")?;

        let iteration_domain = if let Some(token) = self.take_exact("in") {
            let collection = self.required_clause_expression(PREC_LOOP_CLAUSE, &token)?;
            Some(IterIn::new(typed_token(token), collection).into())
        } else {
            let lower_bound = self
                .take_exact("from")
                .map(|token| {
                    let value = self.required_clause_expression(PREC_LOOP_CLAUSE, &token)?;
                    Ok::<_, NativeParseError>(RangeBoundFrom::new(typed_token(token), value))
                })
                .transpose()?;
            let upper_bound = self
                .take_exact("to")
                .map(|token| {
                    let value = self.required_clause_expression(PREC_LOOP_CLAUSE, &token)?;
                    Ok::<_, NativeParseError>(RangeBoundTo::new(typed_token(token), value))
                })
                .transpose()?;
            (lower_bound.is_some() || upper_bound.is_some())
                .then(|| IterRange::new(lower_bound, upper_bound).into())
        };

        let when_condition = if let Some(token) = self.take_exact("when") {
            let condition = self.required_clause_expression(PREC_LOOP_CLAUSE, &token)?;
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

    fn parse_while_loop(&mut self, keyword: TokenTree) -> Result<Expr, NativeParseError> {
        let condition = self.required_clause_expression(PREC_CONTROL, &keyword)?;
        let body = self.parse_loop_body()?;
        Ok(WhileLoop::new(typed_token(keyword), condition, body).into())
    }

    fn parse_loop_body(&mut self) -> Result<LoopBody, NativeParseError> {
        let (list_step, do_step) = if let Some(token) = self.take_exact("list") {
            let listed_value = self.required_clause_expression(PREC_CONTROL, &token)?;
            let list_step = Some(ListStep::new(typed_token(token), listed_value));
            if let Some(token) = self.take_exact("do") {
                let ignored_value = self.required_clause_expression(PREC_CONTROL, &token)?;
                (
                    list_step,
                    Some(DoStep::new(typed_token(token), ignored_value)),
                )
            } else {
                (list_step, None)
            }
        } else if let Some(token) = self.take_exact("do") {
            let ignored_value = self.required_clause_expression(PREC_CONTROL, &token)?;
            (None, Some(DoStep::new(typed_token(token), ignored_value)))
        } else {
            return Err(self.expected_at_lookahead("`list` or `do`"));
        };

        Ok(LoopBody::new(list_step, do_step))
    }

    fn parse_try_statement(&mut self, keyword: TokenTree) -> Result<Expr, NativeParseError> {
        let value = self.required_clause_expression(PREC_CONTROL, &keyword)?;
        let then_clause = self
            .take_exact("then")
            .map(|then_keyword| {
                let value = self.required_clause_expression(PREC_CONTROL, &then_keyword)?;
                Ok::<_, NativeParseError>(ThenClause::new(typed_token(then_keyword), value))
            })
            .transpose()?;

        let fallback = if let Some(else_keyword) = self.take_exact("else") {
            let value = self.required_clause_expression(PREC_CONTROL, &else_keyword)?;
            Some(TryFallback::ElseClause(ElseClause::new(
                typed_token(else_keyword),
                value,
            )))
        } else if let Some(except_keyword) = self.take_exact("except") {
            let exception = self.consume_symbol("an exception symbol")?;
            let do_keyword = self.consume_exact("do")?;
            let value = self.required_clause_expression(PREC_CONTROL, &do_keyword)?;
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

    fn parse_new_statement(&mut self, keyword: TokenTree) -> Result<Expr, NativeParseError> {
        let class = self.required_clause_expression(PREC_LOOP_CLAUSE, &keyword)?;
        let parent_class = if let Some(token) = self.take_exact("of") {
            let parent = self.required_clause_expression(PREC_LOOP_CLAUSE, &token)?;
            Some(OfClass::new(typed_token(token), parent))
        } else {
            None
        };
        let instance = if let Some(token) = self.take_exact("from") {
            let instance = self.required_clause_expression(PREC_LOOP_CLAUSE, &token)?;
            Some(FromInstance::new(typed_token(token), instance))
        } else {
            None
        };

        Ok(NewStatement::new(typed_token(keyword), class, parent_class, instance).into())
    }

    fn parse_debug_clause(&mut self, keyword: TokenTree) -> Result<Expr, NativeParseError> {
        let spelling = parsing_spelling(&keyword).to_owned();
        if matches!(
            spelling.as_str(),
            "time" | "timing" | "elapsedTime" | "elapsedTiming" | "profile"
        ) {
            let value = self
                .parse_required(PREC_CONTROL)
                .map_err(|error| error.missing_operand_for(&keyword))?;
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
                    .map_err(|error| error.missing_operand_for(&keyword))?,
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
                span: keyword.span(),
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
                let span = token.token.span();
                let text = token.token.description();
                Ok(Symbol::new(text, span))
            }
            Lookahead::Token(token) => Err(unexpected!(token.token, expected)),
            Lookahead::End(span) => Err(NativeParseError::unexpected_end(expected, span)),
        }
    }

    fn consume_exact(&mut self, spelling: &'static str) -> Result<TokenTree, NativeParseError> {
        match self.consume() {
            Lookahead::Token(token) if parsing_spelling(&token.token) == spelling => {
                Ok(token.token)
            }
            Lookahead::Token(token) => Err(unexpected!(token.token, spelling)),
            Lookahead::End(span) => Err(NativeParseError::unexpected_end(spelling, span)),
        }
    }

    fn take_exact(&mut self, spelling: &'static str) -> Option<TokenTree> {
        if matches!(
            self.peek(),
            Lookahead::Token(token)
                if parsing_spelling(&token.token) == spelling
        ) {
            let Lookahead::Token(token) = self.consume() else {
                unreachable!("the token just peeked is consumable")
            };
            Some(token.token)
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
            let info = parse_info(&next.token);
            if info.precedence <= level {
                return Ok(left);
            }
            let Lookahead::Token(operator) = self.consume() else {
                unreachable!("the token just peeked is consumable")
            };
            left = if let Some(strength) = info.binary_strength {
                let right = self
                    .parse_required(strength)
                    .map_err(|error| error.missing_operand_for(&operator.token))?;
                lower_binary(left, operator.token, right)?
            } else if info.postfix {
                let span = operator.token.span();
                let spelling = operator.token.description();
                let operator = PostfixOperator::from_token_tree(operator.token).ok_or(
                    NativeParseError::Unsupported {
                        syntax: spelling,
                        span,
                    },
                )?;
                PostfixExpr::new(left, operator).into()
            } else if !matches!(info.unary, UnaryAction::Error) {
                if operator.crossed_newline {
                    return Err(NativeParseError::NewlineInApplication {
                        span: operator.token.span(),
                    });
                }
                let adjacency_span = operator
                    .token
                    .span()
                    .start()
                    .map_or_else(Span::detached, Span::from_offset);
                let right = self.parse_consumed(operator, PREC_APPLICATION_RIGHT)?;
                AdjacentExpr::new(left, Adj::new("", adjacency_span), right).into()
            } else {
                return Err(unexpected!(operator.token, "an operator"));
            };
        }
    }

    fn only_expression(&self, components: Punctuated<Component>) -> Result<Expr, NativeParseError> {
        let mut components = components.into_iter();
        match (components.next(), components.next()) {
            (Some(Component::Expr(value)), None) => Ok(*value),
            (Some(value), _) => Err(NativeParseError::Unexpected {
                found: "a sequence".into(),
                expected: "one expression",
                span: value.span(),
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

    fn take_punctuation(&mut self, spelling: &str) -> Option<TokenTree> {
        if !self.at_punctuation(spelling) {
            return None;
        }
        let Lookahead::Token(token) = self.consume() else {
            unreachable!("the punctuation just peeked is consumable")
        };
        Some(token.token)
    }
}

fn parse_delimited(token: TokenTree) -> Result<Expr, NativeParseError> {
    let TokenTree::Group(group) = token else {
        return Err(unexpected!(token, "a delimited group"));
    };
    let kind = group.delim_kind();
    let delimiter_spans = group.delimiter().spans.clone();
    let mut contents = Engine::new(group.into_stream());
    let mut muted = Vec::new();
    let (elements, comma) = loop {
        let elements = contents.parse_optional_components()?;
        let Some(separator) = contents.take_punctuation(";") else {
            break elements.map_or((None, false), |(elements, comma)| (Some(elements), comma));
        };
        let Some((elements, _)) = elements else {
            return Err(unexpected!(separator, "a statement before `;`"));
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
            value.delimiter = <Delimiter![[]] as DelimiterToken>::new(delimiter_spans);
            value.into()
        }
        DelimiterKind::Brace => {
            let mut value = List::new(muted, elements.map(ExprPack::new));
            value.delimiter = <Delimiter![{}] as DelimiterToken>::new(delimiter_spans);
            value.into()
        }
        DelimiterKind::AngleBar => {
            let mut value = AngleBarList::new(muted, elements.map(ExprPack::new));
            value.delimiter = <Delimiter![< | | >] as DelimiterToken>::new(delimiter_spans);
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
                value.delimiter = <Delimiter![()] as DelimiterToken>::new(delimiter_spans);
                return Ok(value.into());
            };
            value.delimiter = <Delimiter![()] as DelimiterToken>::new(delimiter_spans);
            value.into()
        }
        DelimiterKind::Parenthesis => {
            let value = elements
                .map(|elements| contents.only_expression(elements))
                .transpose()?;
            let mut value = Holder::new(Vec::new(), value);
            value.delimiter = <Delimiter![()] as DelimiterToken>::new(delimiter_spans);
            value.into()
        }
    };
    Ok(expression)
}

fn lower_prefix_statement(keyword: TokenTree, value: Option<Expr>) -> Expr {
    if is_token!(&keyword, break) {
        BreakExpr::new(typed_token(keyword), value).into()
    } else if is_token!(&keyword, continue) {
        ContinueExpr::new(typed_token(keyword), value).into()
    } else if is_token!(&keyword, return) {
        ReturnExpr::new(typed_token(keyword), value).into()
    } else {
        let value = value.expect("catch, throw, and trap require a value");
        if is_token!(&keyword, catch) {
            Catch::new(typed_token(keyword), value).into()
        } else if is_token!(&keyword, throw) {
            Throw::new(typed_token(keyword), value).into()
        } else if is_token!(&keyword, trap) {
            Trap::new(typed_token(keyword), value).into()
        } else {
            unreachable!("statement actions are assigned only to statement keywords")
        }
    }
}

fn lower_atom(token: TokenTree) -> Result<Expr, NativeParseError> {
    match token {
        TokenTree::Ident(token) => Ok(Symbol::new(token.text(), token.span()).into()),
        TokenTree::Literal(token) => {
            let span = token.span();
            Ok(match token.kind {
                LiteralKind::Integer => IntegerLiteral::new(token.text(), span).into(),
                LiteralKind::Float => FloatLiteral::new(token.text(), span).into(),
                LiteralKind::String => lower_string_literal(token.text(), span),
                LiteralKind::RawString => lower_raw_string_literal(token.text(), span),
            })
        }
        token => Err(unexpected!(token, "an expression")),
    }
}

fn lower_binary(left: Expr, operator: TokenTree, right: Expr) -> Result<Expr, NativeParseError> {
    let span = operator.span();
    let spelling = operator
        .spelling()
        .expect("binary actions are assigned only to spelled tokens");
    match spelling {
        "=" => lower_assignment(left, operator, right, false),
        ":=" => lower_assignment(left, operator, right, true),
        "<-" => Ok(EvaluatedAssignment::new(left, typed_token(operator), right).into()),
        "=>" => Ok(OptionExpression::new(left, typed_token(operator), right).into()),
        "->" => {
            let parameters = lambda_parameters(left)
                .ok_or(NativeParseError::InvalidLambdaParameters { span })?;
            Ok(LambdaExpression::new(parameters, typed_token(operator), right).into())
        }
        _ => {
            let spelling = spelling.to_owned();
            let operator =
                BinaryOperator::from_token_tree(operator).ok_or(NativeParseError::Unsupported {
                    syntax: spelling,
                    span,
                })?;
            Ok(BinaryExpr::new(left, operator, right).into())
        }
    }
}

fn lower_assignment(
    left: Expr,
    operator: TokenTree,
    right: Expr,
    local: bool,
) -> Result<Expr, NativeParseError> {
    let span = operator.span();
    match (local, left) {
        (false, Expr::Symbol(variable)) => {
            Ok(GlobalBinding::new(variable, typed_token(operator), right).into())
        }
        (true, Expr::Symbol(variable)) => {
            Ok(LocalBinding::new(variable, typed_token(operator), right).into())
        }
        (false, Expr::OperatorExpr(argument_expr)) => {
            Ok(OperatorAssignment::new(argument_expr, typed_token(operator), right).into())
        }
        (true, Expr::OperatorExpr(OperatorExpr::AdjacentExpr(application))) => {
            let (typical_value, function) = installation_value(right);
            Ok(MethodInstallation::new(
                *application.left_operand,
                *application.right_operand,
                typed_token(operator),
                typical_value,
                function,
            )
            .into())
        }
        (true, Expr::OperatorExpr(domain_expr)) => {
            let (typical_value, function) = installation_value(right);
            Ok(OperatorInstallation::new(
                domain_expr,
                typed_token(operator),
                typical_value,
                function,
            )
            .into())
        }
        (true, Expr::NewStatement(new_statement)) => {
            Ok(NewInstallation::new(new_statement, typed_token(operator), right).into())
        }
        (local, left) => {
            let argument_pack =
                assignment_pack(left).ok_or(NativeParseError::InvalidAssignmentTarget { span })?;
            if local {
                Ok(
                    LocalParallelAssignment::new(argument_pack, typed_token(operator), right)
                        .into(),
                )
            } else {
                Ok(
                    GlobalParallelAssignment::new(argument_pack, typed_token(operator), right)
                        .into(),
                )
            }
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

fn parse_cells(tokens: TokenStream) -> Result<SourceFile, NativeParseError> {
    let eof_span = ParseStream::new(tokens.clone()).eof_span();
    let mut elements = Vec::new();
    for cell in split_cells(tokens) {
        if let Some(element) = Engine::new(cell.stream).parse_cell(cell.semicolon, cell.eoc)? {
            elements.push(element);
        }
    }
    Ok(SourceFile::new(elements, Eof::new("", eof_span)))
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
                span: token.token.span(),
            }),
        }
    }
}

impl Parse for SourceFile {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        parse_cells(input.by_ref().collect()).map_err(token_parse_error)
    }
}

fn token_parse_error(error: NativeParseError) -> TokenParseError {
    let span = error.span();
    TokenParseError::Syntax {
        message: error.to_string(),
        span,
    }
}

fn parsing_spelling(token: &TokenTree) -> &str {
    token.spelling().map_or("", canonical_word_spelling)
}

fn typed_token<T: Token>(token: TokenTree) -> T {
    let spelling = token.description();
    T::from_token_tree(token)
        .unwrap_or_else(|| panic!("`{spelling}` cannot be lowered to `{}`", T::SPELLING))
}

fn empty_span_at(lookahead: &Span) -> Span {
    lookahead.subspan(0, 0)
}

fn lower_string_literal(text: &str, span: Span) -> Expr {
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
    StringLiteral::new(elements, span).into()
}

fn lower_raw_string_literal(text: &str, span: Span) -> Expr {
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
    RawStringLiteral::new(elements, span).into()
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
    use super::split_cells;
    use crate::lex_str;

    /// Cell boundaries are decided here, from the flat stream, using the same
    /// operator/clause/quote state the grammar needs. Verified against M2
    /// v1.26.05 for each case.
    #[test]
    fn cells_use_operator_clause_and_delimiter_state() {
        fn cells(source: &str) -> usize {
            split_cells(lex_str(source).unwrap()).len()
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
}
