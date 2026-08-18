//! Direct construction of the typed graph from [`CellStream`](crate::CellStream).
//!
//! The native precedence engine owns parsing decisions only. Token storage,
//! cursor position, trivia skipping, and newline detection belong to
//! [`ParseStream`](crate::ParseStream), shared with generated [`Parse`](crate::Parse)
//! implementations.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::option::Option;

use crate::lexer::canonical_keyword_spelling;
use crate::parse::{Lookahead, SignificantToken};
use crate::{
    AngleBarList, AnyCell, Array, Assignment, AssignmentExpr, BinaryAssignment, BinaryExpression,
    BinaryInstallation, BinaryOperator, BindingPack, BreakStatement, CatchStatement, CellStream,
    Collection, Component, ContinueStatement, DebugClause, DebugKeyword, DelimiterKind, ElseClause,
    EmptyComponent, EvaluatedAssignment, ExceptClause, Expr, ExpressionCell, FloatLiteral, ForLoop,
    IfStatement, IntegerLiteral, IterationRange, LambdaExpression, LambdaParameters, LexError,
    List, LiteralKind, LocalAssignment, LocalStructuredBinding, LoopBody, MutedCell, MutedGroup,
    NakedSequence, NewStatement, OperatorExpr, OptionExpression, PREC_APPLICATION,
    PREC_APPLICATION_RIGHT, PREC_CLOSER, PREC_COLLECTION, PREC_COMMA, PREC_CONTROL,
    PREC_LOOP_CLAUSE, PREC_QUOTE, PREC_SEMICOLON, ParenthesizedExpression, Parse, ParseStream,
    Parser, PostfixAssignment, PostfixExpression, PostfixInstallation, PostfixOperator,
    PrefixAssignment, PrefixExpression, PrefixInstallation, PrefixOperator, Punctuated,
    QuoteExpression, QuoteSpecifier, RawStringContent, RawStringElement, RawStringLiteral,
    ReturnStatement, Sequence, SequenceCell, SourceFile, SourceId, Span, Spanned, StringContent,
    StringElement, StringLiteral, StructuredBinding, Symbol, ThenClause, ThrowStatement,
    TokenParseError, TokenStream, TokenTree, TrapStatement, TryFallback, TryStatement, WhileLoop,
    lex_str,
};

macro_rules! is_token {
    ($token:expr, $($pattern:tt)*) => {
        <Token![$($pattern)*] as crate::Token>::matches_token_tree($token)
    };
}

/// The built-in parser implemented directly from Macaulay2's `P/B/U` table.
#[derive(Debug, Default)]
pub struct NativeParser;

impl NativeParser {
    pub fn new() -> Self {
        Self
    }
}

impl Parser for NativeParser {
    type Error = NativeParseError;

    fn parse_cells(&mut self, tokens: CellStream) -> Result<SourceFile, Self::Error> {
        parse_cells(tokens)
    }
}

pub fn parse_native(source: &str, source_id: SourceId) -> Result<SourceFile, NativeParseError> {
    let tokens = lex_str(source, source_id).map_err(NativeParseError::Lex)?;
    NativeParser.parse_cells(tokens)
}

#[derive(Debug, Spanned)]
pub enum NativeParseError {
    Lex(LexError),
    Unexpected {
        found: String,
        expected: &'static str,
        span: Span,
    },
    MissingOperand {
        operator: String,
        span: Span,
    },
    NewlineInApplication {
        span: Span,
    },
    InvalidAssignmentTarget {
        span: Span,
    },
    InvalidLambdaParameters {
        span: Span,
    },
    Unsupported {
        syntax: String,
        span: Span,
    },
}

impl Display for NativeParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Lex(error) => error.fmt(formatter),
            Self::Unexpected {
                found, expected, ..
            } => write!(formatter, "expected {expected}, found `{found}`"),
            Self::MissingOperand { operator, .. } => {
                write!(formatter, "operator `{operator}` requires an operand")
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

    const fn prefix(precedence: u8, unary_strength: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: Some(unary_strength),
            unary: UnaryAction::Prefix,
            postfix: false,
        }
    }

    const fn statement() -> Self {
        Self {
            precedence: PREC_APPLICATION,
            binary_strength: None,
            unary_strength: Some(PREC_CONTROL),
            unary: UnaryAction::Statement,
            postfix: false,
        }
    }

    const fn control(unary_strength: u8) -> Self {
        Self {
            precedence: PREC_APPLICATION,
            binary_strength: None,
            unary_strength: Some(unary_strength),
            unary: UnaryAction::Control,
            postfix: false,
        }
    }

    const fn debug() -> Self {
        Self {
            precedence: PREC_APPLICATION,
            binary_strength: None,
            unary_strength: Some(PREC_CONTROL),
            unary: UnaryAction::Debug,
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

    const fn quote(precedence: u8, unary_strength: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: Some(unary_strength),
            unary: UnaryAction::Quote,
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
        delimiter: crate::Delimiter,
    ) -> Result<Option<AnyCell>, NativeParseError> {
        self.skip_trivia();
        if matches!(self.peek(), Lookahead::End(_)) {
            return match delimiter.kind {
                DelimiterKind::Empty => Ok(None),
                DelimiterKind::Semicolon => Err(NativeParseError::Unexpected {
                    found: ";".into(),
                    expected: "a statement before `;`",
                    span: delimiter.closing_span(),
                }),
                _ => Err(NativeParseError::Unexpected {
                    found: delimiter.kind.to_string(),
                    expected: "an empty or semicolon cell delimiter",
                    span: delimiter.span(),
                }),
            };
        }

        let (components, comma) = self.parse_components()?;

        if let Lookahead::Token(token) = self.peek() {
            return Err(self.unexpected(token.token, "the end of the cell"));
        }

        let cell = match delimiter.kind {
            DelimiterKind::Semicolon => AnyCell::MutedCell(MutedCell::new(
                components,
                Token![;](delimiter.closing_span()),
            )),
            DelimiterKind::Empty if comma => {
                AnyCell::SequenceCell(SequenceCell::new(NakedSequence::new(components)))
            }
            DelimiterKind::Empty => {
                let value = self.only_expression(components)?;
                AnyCell::ExpressionCell(ExpressionCell::new(value))
            }
            _ => {
                return Err(NativeParseError::Unexpected {
                    found: delimiter.kind.to_string(),
                    expected: "an empty or semicolon cell delimiter",
                    span: delimiter.span(),
                });
            }
        };
        Ok(Some(cell))
    }

    fn parse_components(&mut self) -> Result<(Punctuated<Component>, bool), NativeParseError> {
        if self.at_end() || self.at_punctuation(";") {
            return Ok((Punctuated::new(), false));
        }

        let first = if self.at_punctuation(",") {
            Component::EmptyComponent(EmptyComponent::new("", self.peek().span()))
        } else {
            Component::Expr(self.parse_required(PREC_COMMA)?)
        };
        let mut elements = Punctuated::from_value(first);
        let mut has_comma = false;

        while let Some(comma) = self.take_punctuation(",") {
            has_comma = true;
            let next = if self.at_end() || self.at_punctuation(";") || self.at_punctuation(",") {
                Component::EmptyComponent(EmptyComponent::new("", self.peek().span()))
            } else {
                Component::Expr(self.parse_required(PREC_COMMA)?)
            };
            elements.push(typed_token(comma), next);
        }

        Ok((elements, has_comma))
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
            UnaryAction::Atom => self.lower_atom(token.token)?,
            UnaryAction::Prefix => {
                let strength = info.unary_strength.expect("prefix operators have U");
                let operand = self.parse_required(level.max(strength))?;
                let span = token.token.span();
                let spelling = token_description(&token.token);
                let operator = PrefixOperator::from_token_tree(token.token).ok_or({
                    NativeParseError::Unsupported {
                        syntax: spelling,
                        span,
                    }
                })?;
                PrefixExpression::new(operator, operand).into()
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
                        .map_err(|error| {
                            if matches!(error, NativeParseError::MissingOperand { .. }) {
                                NativeParseError::MissingOperand {
                                    operator: token_description(&token.token),
                                    span: token.token.span(),
                                }
                            } else {
                                error
                            }
                        })?
                        .into()
                };
                self.lower_prefix_statement(token.token, operand)
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
            UnaryAction::Delimiter => self.parse_delimited(token.token)?,
            UnaryAction::Quote => {
                let operator = token_description(&token.token);
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
                let Some(quoted_text) = token_spelling(&quoted).map(str::to_owned) else {
                    return Err(self.unexpected(quoted, "a quoted token"));
                };
                let quoted = Symbol::new(quoted_text, quoted_span);
                QuoteExpression::new(specifier, quoted).into()
            }
            UnaryAction::Error => return Err(self.unexpected(token.token, "an expression")),
        };
        self.accumulate(result, level)
    }

    fn lower_prefix_statement(&self, keyword: TokenTree, value: Option<Expr>) -> Expr {
        if is_token!(&keyword, break) {
            BreakStatement::new(typed_token(keyword), value).into()
        } else if is_token!(&keyword, continue) {
            ContinueStatement::new(typed_token(keyword), value).into()
        } else if is_token!(&keyword, return) {
            ReturnStatement::new(typed_token(keyword), value).into()
        } else if is_token!(&keyword, catch) {
            CatchStatement::new(
                typed_token(keyword),
                value.expect("catch statements require a value"),
            )
            .into()
        } else if is_token!(&keyword, throw) {
            ThrowStatement::new(
                typed_token(keyword),
                value.expect("throw statements require a value"),
            )
            .into()
        } else if is_token!(&keyword, trap) {
            TrapStatement::new(
                typed_token(keyword),
                value.expect("trap statements require a value"),
            )
            .into()
        } else {
            unreachable!("statement actions are assigned only to statement keywords")
        }
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

        let mut in_keyword = None;
        let mut iterated_collection = None;
        let mut from_keyword = None;
        let mut range_start = None;
        let mut to_keyword = None;
        let mut range_end = None;

        if let Some(token) = self.take_exact("in") {
            iterated_collection = Some(self.required_clause_expression(PREC_LOOP_CLAUSE, &token)?);
            in_keyword = Some(typed_token(token));
        } else {
            if let Some(token) = self.take_exact("from") {
                range_start = Some(self.required_clause_expression(PREC_LOOP_CLAUSE, &token)?);
                from_keyword = Some(typed_token(token));
            }
            if let Some(token) = self.take_exact("to") {
                range_end = Some(self.required_clause_expression(PREC_LOOP_CLAUSE, &token)?);
                to_keyword = Some(typed_token(token));
            }
        }

        let range =
            (in_keyword.is_some() || from_keyword.is_some() || to_keyword.is_some()).then(|| {
                IterationRange::new(
                    in_keyword,
                    iterated_collection,
                    from_keyword,
                    range_start,
                    to_keyword,
                    range_end,
                )
            });

        let (when_keyword, filter) = if let Some(token) = self.take_exact("when") {
            let filter = self.required_clause_expression(PREC_LOOP_CLAUSE, &token)?;
            (Some(typed_token(token)), Some(filter))
        } else {
            (None, None)
        };
        let body = self.parse_loop_body()?;

        Ok(ForLoop::new(
            typed_token(keyword),
            variable,
            range,
            when_keyword,
            filter,
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
        let mut list_keyword = None;
        let mut listed_value = None;
        let mut do_keyword = None;
        let mut ignored_value = None;

        if let Some(token) = self.take_exact("list") {
            listed_value = Some(self.required_clause_expression(PREC_CONTROL, &token)?);
            list_keyword = Some(typed_token(token));
            if let Some(token) = self.take_exact("do") {
                ignored_value = Some(self.required_clause_expression(PREC_CONTROL, &token)?);
                do_keyword = Some(typed_token(token));
            }
        } else if let Some(token) = self.take_exact("do") {
            ignored_value = Some(self.required_clause_expression(PREC_CONTROL, &token)?);
            do_keyword = Some(typed_token(token));
        } else {
            return Err(self.expected_at_lookahead("`list` or `do`"));
        }

        Ok(LoopBody::new(
            list_keyword,
            listed_value,
            do_keyword,
            ignored_value,
        ))
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
            Some(TryFallback::ExceptClause(ExceptClause::new(
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
        let (of_keyword, parent) = if let Some(token) = self.take_exact("of") {
            let parent = self.required_clause_expression(PREC_LOOP_CLAUSE, &token)?;
            (Some(typed_token(token)), Some(parent))
        } else {
            (None, None)
        };
        let (from_keyword, instance) = if let Some(token) = self.take_exact("from") {
            let instance = self.required_clause_expression(PREC_LOOP_CLAUSE, &token)?;
            (Some(typed_token(token)), Some(instance))
        } else {
            (None, None)
        };

        Ok(NewStatement::new(
            typed_token(keyword),
            class,
            of_keyword,
            parent,
            from_keyword,
            instance,
        )
        .into())
    }

    fn parse_debug_clause(&mut self, keyword: TokenTree) -> Result<Expr, NativeParseError> {
        let span = keyword.span();
        let spelling = parsing_spelling(&keyword).to_owned();
        let value_is_optional = matches!(spelling.as_str(), "step" | "finish");
        let value = if value_is_optional {
            self.parse_nullable(PREC_CONTROL)?
        } else {
            Some(self.parse_required(PREC_CONTROL).map_err(|error| {
                if matches!(error, NativeParseError::MissingOperand { .. }) {
                    NativeParseError::MissingOperand {
                        operator: token_description(&keyword),
                        span,
                    }
                } else {
                    error
                }
            })?)
        };
        let keyword = match spelling.as_str() {
            "step" => DebugKeyword::Step(typed_token(keyword)),
            "finish" => DebugKeyword::Finish(typed_token(keyword)),
            "shield" => DebugKeyword::Shield(typed_token(keyword)),
            "TEST" => DebugKeyword::Test(typed_token(keyword)),
            "time" => DebugKeyword::Time(typed_token(keyword)),
            "timing" => DebugKeyword::Timing(typed_token(keyword)),
            "breakpoint" => DebugKeyword::Breakpoint(typed_token(keyword)),
            "elapsedTime" => DebugKeyword::ElapsedTime(typed_token(keyword)),
            "elapsedTiming" => DebugKeyword::ElapsedTiming(typed_token(keyword)),
            "profile" => DebugKeyword::Profile(typed_token(keyword)),
            _ => unreachable!("debug actions are assigned only to debug keywords"),
        };
        Ok(DebugClause::new(keyword, value).into())
    }

    fn required_clause_expression(
        &mut self,
        level: u8,
        keyword: &TokenTree,
    ) -> Result<Expr, NativeParseError> {
        match self.parse_nullable(level)? {
            None => Err(NativeParseError::MissingOperand {
                operator: token_description(keyword),
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
                let text = token_description(&token.token);
                Ok(Symbol::new(text, span))
            }
            Lookahead::Token(token) => Err(self.unexpected(token.token, expected)),
            Lookahead::End(span) => Err(NativeParseError::Unexpected {
                found: "end of input".into(),
                expected,
                span,
            }),
        }
    }

    fn consume_exact(&mut self, spelling: &'static str) -> Result<TokenTree, NativeParseError> {
        match self.consume() {
            Lookahead::Token(token) if parsing_spelling(&token.token) == spelling => {
                Ok(token.token)
            }
            Lookahead::Token(token) => Err(self.unexpected(token.token, spelling)),
            Lookahead::End(span) => Err(NativeParseError::Unexpected {
                found: "end of input".into(),
                expected: spelling,
                span,
            }),
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
            Lookahead::Token(token) => self.unexpected(token.token, expected),
            Lookahead::End(span) => NativeParseError::Unexpected {
                found: "end of input".into(),
                expected,
                span,
            },
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
                let right = self.parse_required(strength).map_err(|error| {
                    if matches!(error, NativeParseError::MissingOperand { .. }) {
                        NativeParseError::MissingOperand {
                            operator: token_description(&operator.token),
                            span: operator.token.span(),
                        }
                    } else {
                        error
                    }
                })?;
                self.lower_binary(left, operator.token, right)?
            } else if info.postfix {
                let span = operator.token.span();
                let spelling = token_description(&operator.token);
                let operator = PostfixOperator::from_token_tree(operator.token).ok_or(
                    NativeParseError::Unsupported {
                        syntax: spelling,
                        span,
                    },
                )?;
                PostfixExpression::new(left, operator).into()
            } else if !matches!(info.unary, UnaryAction::Error) {
                if operator.crossed_newline {
                    return Err(NativeParseError::NewlineInApplication {
                        span: operator.token.span(),
                    });
                }
                let adjacency_span = operator.leading_span;
                let right = self.parse_consumed(operator, PREC_APPLICATION_RIGHT)?;
                BinaryExpression::new(
                    left,
                    BinaryOperator::from(Token![SPACE](adjacency_span)),
                    right,
                )
                .into()
            } else {
                return Err(self.unexpected(operator.token, "an operator"));
            };
        }
    }

    fn parse_delimited(&mut self, token: TokenTree) -> Result<Expr, NativeParseError> {
        let TokenTree::Group(group) = token else {
            return Err(self.unexpected(token, "a delimited group"));
        };
        let kind = group.delim_kind();
        let delimiter_span = group.span();
        let mut contents = Self::new(group.into_stream());
        let mut muted = Vec::new();
        let (elements, comma) = loop {
            let (elements, comma) = contents.parse_components()?;
            let Some(separator) = contents.take_punctuation(";") else {
                break (elements, comma);
            };
            if elements.is_empty() {
                return Err(contents.unexpected(separator, "a statement before `;`"));
            }
            muted.push(MutedGroup::new(elements, typed_token(separator)));
        };
        if let Lookahead::Token(token) = contents.peek() {
            return Err(contents.unexpected(token.token, "the end of the group"));
        }

        let expression: Expr = match kind {
            DelimiterKind::Empty | DelimiterKind::Semicolon => {
                return Err(NativeParseError::Unsupported {
                    syntax: format!("{} cell delimiter in expression position", kind),
                    span: delimiter_span.span(),
                });
            }
            DelimiterKind::Bracket => {
                let mut value = Array::new(muted, elements);
                value.delimiter = <Delimiter![[]] as crate::DelimiterToken>::new(delimiter_span);
                value.into()
            }
            DelimiterKind::Brace => {
                let mut value = List::new(muted, elements);
                value.delimiter = <Delimiter![{}] as crate::DelimiterToken>::new(delimiter_span);
                value.into()
            }
            DelimiterKind::AngleBar => {
                let mut value = AngleBarList::new(muted, elements);
                value.delimiter =
                    <Delimiter![< | | >] as crate::DelimiterToken>::new(delimiter_span);
                value.into()
            }
            DelimiterKind::Parenthesis if !muted.is_empty() || comma => {
                let mut value = Sequence::new(muted, elements);
                value.delimiter = <Delimiter![()] as crate::DelimiterToken>::new(delimiter_span);
                value.into()
            }
            DelimiterKind::Parenthesis => {
                let value = match elements.len() {
                    0 => None,
                    1 => Some(contents.only_expression(elements)?),
                    _ => unreachable!("multiple components imply a comma"),
                };
                let mut value = ParenthesizedExpression::new(Vec::new(), value);
                value.delimiter = <Delimiter![()] as crate::DelimiterToken>::new(delimiter_span);
                value.into()
            }
        };
        Ok(expression)
    }

    fn lower_atom(&mut self, token: TokenTree) -> Result<Expr, NativeParseError> {
        match token {
            TokenTree::Ident(token) => {
                let span = token.span();
                Ok(Symbol::new(token.text(), span).into())
            }
            TokenTree::Literal(token) => {
                let span = token.span();
                let text = token.text().to_owned();
                Ok(match token.kind {
                    LiteralKind::Integer => IntegerLiteral::new(text, span).into(),
                    LiteralKind::Float => FloatLiteral::new(text, span).into(),
                    LiteralKind::String => lower_string_literal(text, span),
                    LiteralKind::RawString => lower_raw_string_literal(text, span),
                })
            }
            token => Err(self.unexpected(token, "an expression")),
        }
    }

    fn lower_binary(
        &mut self,
        left: Expr,
        operator: TokenTree,
        right: Expr,
    ) -> Result<Expr, NativeParseError> {
        let span = operator.span();
        let spelling = token_spelling(&operator)
            .expect("binary actions are assigned only to spelled tokens")
            .to_owned();
        match spelling.as_str() {
            "=" => self.lower_assignment(left, operator, right, false),
            ":=" => self.lower_assignment(left, operator, right, true),
            "<-" => Ok(EvaluatedAssignment::new(left, typed_token(operator), right).into()),
            "=>" => Ok(OptionExpression::new(left, typed_token(operator), right).into()),
            "->" => {
                let parameters = lambda_parameters(left)
                    .ok_or(NativeParseError::InvalidLambdaParameters { span })?;
                Ok(LambdaExpression::new(parameters, typed_token(operator), right).into())
            }
            _ => {
                let operator = BinaryOperator::from_token_tree(operator).ok_or({
                    NativeParseError::Unsupported {
                        syntax: spelling,
                        span,
                    }
                })?;
                Ok(BinaryExpression::new(left, operator, right).into())
            }
        }
    }

    fn lower_assignment(
        &mut self,
        left: Expr,
        operator: TokenTree,
        right: Expr,
        local: bool,
    ) -> Result<Expr, NativeParseError> {
        let span = operator.span();
        let result =
            match (local, left) {
                (false, Expr::Symbol(left)) => {
                    AssignmentExpr::Assignment(Assignment::new(left, typed_token(operator), right))
                }
                (true, Expr::Symbol(left)) => AssignmentExpr::LocalAssignment(
                    LocalAssignment::new(left, typed_token(operator), right),
                ),
                (false, Expr::Collection(left)) => AssignmentExpr::StructuredBinding(
                    StructuredBinding::new(binding_pack(left), typed_token(operator), right),
                ),
                (true, Expr::Collection(left)) => AssignmentExpr::LocalStructuredBinding(
                    LocalStructuredBinding::new(binding_pack(left), typed_token(operator), right),
                ),
                (false, Expr::OperatorExpr(OperatorExpr::BinaryExpression(left))) => {
                    AssignmentExpr::BinaryAssignment(BinaryAssignment::new(
                        left,
                        typed_token(operator),
                        right,
                    ))
                }
                (true, Expr::OperatorExpr(OperatorExpr::BinaryExpression(left))) => {
                    AssignmentExpr::BinaryInstallation(BinaryInstallation::new(
                        left,
                        typed_token(operator),
                        right,
                    ))
                }
                (false, Expr::OperatorExpr(OperatorExpr::PrefixExpression(left))) => {
                    AssignmentExpr::PrefixAssignment(PrefixAssignment::new(
                        left,
                        typed_token(operator),
                        right,
                    ))
                }
                (true, Expr::OperatorExpr(OperatorExpr::PrefixExpression(left))) => {
                    AssignmentExpr::PrefixInstallation(PrefixInstallation::new(
                        left,
                        typed_token(operator),
                        right,
                    ))
                }
                (false, Expr::OperatorExpr(OperatorExpr::PostfixExpression(left))) => {
                    AssignmentExpr::PostfixAssignment(PostfixAssignment::new(
                        left,
                        typed_token(operator),
                        right,
                    ))
                }
                (true, Expr::OperatorExpr(OperatorExpr::PostfixExpression(left))) => {
                    AssignmentExpr::PostfixInstallation(PostfixInstallation::new(
                        left,
                        typed_token(operator),
                        right,
                    ))
                }
                _ => return Err(NativeParseError::InvalidAssignmentTarget { span }),
            };
        Ok(Expr::AssignmentExpr(result))
    }

    fn only_expression(&self, components: Punctuated<Component>) -> Result<Expr, NativeParseError> {
        let mut components = components.into_iter();
        match (components.next(), components.next()) {
            (Some(Component::Expr(value)), None) => Ok(value),
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

    fn unexpected(&self, token: TokenTree, expected: &'static str) -> NativeParseError {
        let span = token.span();
        NativeParseError::Unexpected {
            found: token_description(&token),
            expected,
            span,
        }
    }
}

fn parse_cells(cells: CellStream) -> Result<SourceFile, NativeParseError> {
    let mut elements = Vec::new();
    for cell in cells {
        let delimiter = *cell.delimiter();
        if let Some(element) = Engine::new(cell.into_stream()).parse_cell(delimiter)? {
            elements.push(element);
        }
    }
    Ok(SourceFile::new(elements))
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
                found: token_description(&token.token),
                span: token.token.span(),
            }),
        }
    }
}

impl Parse for SourceFile {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let source_id = input
            .peek()
            .and_then(|token| token.span().source().ok())
            .unwrap_or_else(SourceId::fresh);
        let source = input.by_ref().collect::<TokenStream>().to_string();
        parse_native(&source, source_id).map_err(token_parse_error)
    }
}

fn token_parse_error(error: NativeParseError) -> TokenParseError {
    let span = error.span();
    TokenParseError::Syntax {
        message: error.to_string(),
        span,
    }
}

fn token_spelling(token: &TokenTree) -> Option<&str> {
    token.spelling()
}

fn parsing_spelling(token: &TokenTree) -> &str {
    token_spelling(token)
        .map(canonical_keyword_spelling)
        .unwrap_or("")
}

fn token_description(token: &TokenTree) -> String {
    match token {
        TokenTree::Group(group) => group.delim_kind().to_string(),
        TokenTree::Trivia(token) => token.text().to_owned(),
        _ => token_spelling(token).unwrap_or("").to_owned(),
    }
}

fn typed_token<T: crate::cst::Token>(token: TokenTree) -> T {
    let spelling = token_description(&token);
    T::from_token_tree(token)
        .unwrap_or_else(|| panic!("`{spelling}` cannot be lowered to `{}`", T::SPELLING))
}

fn lower_string_literal(text: String, span: Span) -> Expr {
    let body = &text[1..text.len() - 1];
    let elements = (!body.is_empty())
        .then(|| StringElement::StringContent(StringContent::new(body, span)))
        .into_iter()
        .collect();
    StringLiteral::new(elements).into()
}

fn lower_raw_string_literal(text: String, span: Span) -> Expr {
    let body = &text[3..text.len() - 3];
    let elements = (!body.is_empty())
        .then(|| RawStringElement::RawStringContent(RawStringContent::new(body, span)))
        .into_iter()
        .collect();
    RawStringLiteral::new(elements).into()
}

fn binding_pack(collection: Collection) -> BindingPack {
    match collection {
        Collection::ParenthesizedExpression(value) => BindingPack::ParenthesizedExpression(value),
        Collection::Sequence(value) => BindingPack::Sequence(value),
        Collection::List(value) => BindingPack::List(value),
        Collection::Array(value) => BindingPack::Array(value),
        Collection::AngleBarList(value) => BindingPack::AngleBarList(value),
    }
}

fn lambda_parameters(expression: Expr) -> Option<LambdaParameters> {
    match expression {
        Expr::Symbol(value) => Some(LambdaParameters::Symbol(value)),
        Expr::Collection(Collection::ParenthesizedExpression(value)) => {
            Some(LambdaParameters::ParenthesizedExpression(value))
        }
        Expr::Collection(Collection::Sequence(value)) => Some(LambdaParameters::Sequence(value)),
        Expr::Collection(Collection::List(value)) => Some(LambdaParameters::List(value)),
        Expr::Collection(Collection::Array(value)) => Some(LambdaParameters::Array(value)),
        Expr::Collection(Collection::AngleBarList(value)) => {
            Some(LambdaParameters::AngleBarList(value))
        }
        _ => None,
    }
}

fn parse_info(token: &TokenTree) -> ParseInfo {
    if let TokenTree::Group(group) = token {
        return match group.delim_kind() {
            DelimiterKind::Empty | DelimiterKind::Semicolon => ParseInfo::stop(PREC_CLOSER),
            DelimiterKind::Bracket | DelimiterKind::AngleBar => {
                ParseInfo::delimiter(PREC_COLLECTION)
            }
            DelimiterKind::Parenthesis | DelimiterKind::Brace => {
                ParseInfo::delimiter(PREC_APPLICATION)
            }
        };
    }

    let text = parsing_spelling(token);
    if let Some(info) = __m2_syn_parse_info!(text, ParseInfo) {
        return info;
    }

    if is_token!(token, ;) {
        return ParseInfo::stop(PREC_SEMICOLON);
    }
    if is_token!(token, ,) {
        return ParseInfo::stop(PREC_COMMA);
    }
    if is_token!(token, else)
        || is_token!(token, then)
        || is_token!(token, do)
        || is_token!(token, list)
        || is_token!(token, except)
    {
        return ParseInfo::stop(PREC_CONTROL);
    }
    if is_token!(token, when)
        || is_token!(token, of)
        || is_token!(token, in)
        || is_token!(token, from)
        || is_token!(token, to)
    {
        return ParseInfo::stop(PREC_LOOP_CLAUSE);
    }

    if is_token!(token, break)
        || is_token!(token, continue)
        || is_token!(token, return)
        || is_token!(token, catch)
        || is_token!(token, throw)
        || is_token!(token, trap)
    {
        return ParseInfo::statement();
    }

    let control_strength = if is_token!(token, for) || is_token!(token, new) {
        Some(PREC_LOOP_CLAUSE)
    } else if is_token!(token, if) || is_token!(token, while) || is_token!(token, try) {
        Some(PREC_CONTROL)
    } else {
        None
    };
    if let Some(strength) = control_strength {
        return ParseInfo::control(strength);
    }

    if is_token!(token, TEST)
        || is_token!(token, time)
        || is_token!(token, timing)
        || is_token!(token, elapsedTime)
        || is_token!(token, elapsedTiming)
        || is_token!(token, breakpoint)
        || is_token!(token, profile)
        || is_token!(token, shield)
        || is_token!(token, step)
        || is_token!(token, finish)
    {
        return ParseInfo::debug();
    }

    if is_token!(token, symbol)
        || is_token!(token, global)
        || is_token!(token, threadVariable)
        || is_token!(token, threadLocal)
        || is_token!(token, local)
    {
        return ParseInfo::quote(PREC_APPLICATION, PREC_QUOTE);
    }

    match token {
        TokenTree::Ident(_) | TokenTree::Literal(_) => ParseInfo::atom(PREC_APPLICATION),
        _ => ParseInfo::stop(PREC_APPLICATION),
    }
}
