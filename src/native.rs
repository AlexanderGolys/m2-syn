//! Direct construction of the typed graph from [`CellStream`](crate::CellStream).
//!
//! The native precedence engine owns parsing decisions only. Token storage,
//! cursor position, trivia skipping, and newline detection belong to
//! [`ParseStream`](crate::ParseStream), shared with generated [`Parse`](crate::Parse)
//! implementations.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::option::Option;

use crate::nodes::canonical_keyword_spelling;
use crate::parse::{Lookahead, SignificantToken};
use crate::{
    AngleBarList, AnyCell, Array, Assignment, AssignmentExpr, BinaryAssignment, BinaryExpression,
    BinaryInstallation, BinaryOperator, BindingPack, BreakStatement, CatchStatement, CellStream,
    Collection, Component, ContinueStatement, DebugClause, DebugKeyword, ElseClause,
    EmptyComponent, EvaluatedAssignment, ExceptClause, Expr, ExpressionCell, FloatLiteral, ForLoop,
    IfStatement, IntegerLiteral, IterationRange, LambdaExpression, LambdaParameters, LexError,
    List, LiteralKind, LocalAssignment, LocalStructuredBinding, LoopBody, MutedCell, MutedGroup,
    NakedSequence, NewStatement, OperatorExpr, OptionExpression, ParenthesizedExpression,
    ParseStream, Parser, PostfixAssignment, PostfixExpression, PostfixInstallation,
    PostfixOperator, PrefixAssignment, PrefixExpression, PrefixInstallation, PrefixOperator,
    QuoteExpression, QuoteSpecifier, RawStringContent, RawStringElement, RawStringLiteral,
    ReturnStatement, Sequence, SequenceCell, SourceFile, SourceId, Span, Spanned, StringContent,
    StringElement, StringLiteral, StructuredBinding, Symbol, ThenClause, ThrowStatement,
    TokenStream, TokenTree, TrapStatement, TryFallback, TryStatement, WhileLoop, lex_str,
};

const PREC_CLOSER: u8 = 6;
const PREC_SEMICOLON: u8 = 8;
const PREC_COMMA: u8 = 10;
const PREC_CONTROL: u8 = 12;
const PREC_LOOP_CLAUSE: u8 = 16;
const PREC_SPACE: u8 = 62;

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

    fn parse(&mut self, tokens: CellStream) -> Result<SourceFile, Self::Error> {
        parse_cells(tokens)
    }
}

pub fn parse_native(source: &str, source_id: SourceId) -> Result<SourceFile, NativeParseError> {
    let tokens = lex_str(source, source_id).map_err(NativeParseError::Lex)?;
    NativeParser.parse(tokens)
}

#[derive(Debug)]
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

impl Spanned for NativeParseError {
    fn span(&self) -> Span {
        match self {
            Self::Lex(error) => error.span(),
            Self::Unexpected { span, .. }
            | Self::MissingOperand { span, .. }
            | Self::NewlineInApplication { span }
            | Self::InvalidAssignmentTarget { span }
            | Self::InvalidLambdaParameters { span }
            | Self::Unsupported { span, .. } => *span,
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
    Statement(PrefixStatementKind),
    If,
    For,
    While,
    Try,
    New,
    Debug,
    Delimiter,
    Quote,
    Error,
}

#[derive(Debug, Clone, Copy)]
/// Parser-local selector for the prefix-statement routine.
///
/// Each variant corresponds to a generated `Token![..]` atom. The enum exists
/// only to share operand parsing before the raw token is refined to its typed
/// token and placed in the resulting CST node.
enum PrefixStatementKind {
    Break,
    Continue,
    Return,
    Catch,
    Throw,
    Trap,
}

impl PrefixStatementKind {
    fn value_is_optional(self) -> bool {
        matches!(self, Self::Break | Self::Continue | Self::Return)
    }
}

#[derive(Debug, Clone, Copy)]
enum BinaryAction {
    Adjacent,
    Binary,
    Postfix,
    Error,
}

#[derive(Debug, Clone, Copy)]
struct ParseInfo {
    precedence: u8,
    binary_strength: Option<u8>,
    unary_strength: Option<u8>,
    unary: UnaryAction,
    binary: BinaryAction,
}

impl ParseInfo {
    const fn atom(precedence: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: None,
            unary: UnaryAction::Atom,
            binary: BinaryAction::Adjacent,
        }
    }

    const fn binary(precedence: u8, strength: u8) -> Self {
        Self {
            precedence,
            binary_strength: Some(strength),
            unary_strength: None,
            unary: UnaryAction::Error,
            binary: BinaryAction::Binary,
        }
    }

    const fn prefix_binary(precedence: u8, binary_strength: u8, unary_strength: u8) -> Self {
        Self {
            precedence,
            binary_strength: Some(binary_strength),
            unary_strength: Some(unary_strength),
            unary: UnaryAction::Prefix,
            binary: BinaryAction::Binary,
        }
    }

    const fn prefix(precedence: u8, unary_strength: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: Some(unary_strength),
            unary: UnaryAction::Prefix,
            binary: BinaryAction::Adjacent,
        }
    }

    const fn statement(kind: PrefixStatementKind) -> Self {
        Self {
            precedence: PREC_SPACE,
            binary_strength: None,
            unary_strength: Some(PREC_CONTROL),
            unary: UnaryAction::Statement(kind),
            binary: BinaryAction::Adjacent,
        }
    }

    const fn control(unary: UnaryAction, unary_strength: u8) -> Self {
        Self {
            precedence: PREC_SPACE,
            binary_strength: None,
            unary_strength: Some(unary_strength),
            unary,
            binary: BinaryAction::Adjacent,
        }
    }

    const fn postfix(precedence: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: None,
            unary: UnaryAction::Error,
            binary: BinaryAction::Postfix,
        }
    }

    const fn delimiter(precedence: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: Some(PREC_CLOSER),
            unary: UnaryAction::Delimiter,
            binary: BinaryAction::Adjacent,
        }
    }

    const fn stop(precedence: u8) -> Self {
        Self {
            precedence,
            binary_strength: None,
            unary_strength: None,
            unary: UnaryAction::Error,
            binary: BinaryAction::Error,
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
                crate::DelimiterKind::Empty => Ok(None),
                crate::DelimiterKind::Semicolon => Err(NativeParseError::Unexpected {
                    found: ";".into(),
                    expected: "a statement before `;`",
                    span: delimiter.span2.span_close,
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
            crate::DelimiterKind::Semicolon => AnyCell::MutedCell(MutedCell::new(
                components,
                Token![;](delimiter.span2.span_close),
            )),
            crate::DelimiterKind::Empty if comma => {
                AnyCell::SequenceCell(SequenceCell::new(NakedSequence::new(components)))
            }
            crate::DelimiterKind::Empty => {
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

    fn parse_components(&mut self) -> Result<(Vec<Component>, bool), NativeParseError> {
        let mut elements = Vec::new();
        let mut has_comma = false;
        let mut needs_component = true;

        loop {
            if let Some(comma) = self.take_punctuation(",") {
                if needs_component {
                    elements.push(Component::EmptyComponent(EmptyComponent::new(
                        "",
                        comma.span(),
                    )));
                }
                has_comma = true;
                needs_component = true;
                continue;
            }

            if self.at_end() || self.at_punctuation(";") {
                if has_comma && needs_component {
                    elements.push(Component::EmptyComponent(EmptyComponent::new(
                        "",
                        self.peek().span(),
                    )));
                }
                break;
            }

            if !needs_component {
                break;
            }
            elements.push(Component::Expr(self.parse_required(PREC_COMMA)?));
            needs_component = false;
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
            UnaryAction::Statement(kind) => {
                let strength = info
                    .unary_strength
                    .expect("prefix statements have a unary binding strength");
                let operand = if kind.value_is_optional() {
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
                self.lower_prefix_statement(kind, token.token, operand)
            }
            UnaryAction::If => self.parse_if_statement(token.token)?,
            UnaryAction::For => self.parse_for_loop(token.token)?,
            UnaryAction::While => self.parse_while_loop(token.token)?,
            UnaryAction::Try => self.parse_try_statement(token.token)?,
            UnaryAction::New => self.parse_new_statement(token.token)?,
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
                let specifier_spelling = parsing_spelling(&token.token).to_owned();
                let specifier = match specifier_spelling.as_str() {
                    "symbol" => QuoteSpecifier::Symbol(typed_token(token.token)),
                    "local" => QuoteSpecifier::Local(typed_token(token.token)),
                    "global" => QuoteSpecifier::Global(typed_token(token.token)),
                    "threadVariable" => QuoteSpecifier::ThreadVariable(typed_token(token.token)),
                    "threadLocal" => QuoteSpecifier::ThreadLocal(typed_token(token.token)),
                    _ => unreachable!("quote actions are assigned to quote specifiers"),
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

    fn lower_prefix_statement(
        &self,
        kind: PrefixStatementKind,
        keyword: TokenTree,
        value: Option<Expr>,
    ) -> Expr {
        match kind {
            PrefixStatementKind::Break => BreakStatement::new(typed_token(keyword), value).into(),
            PrefixStatementKind::Continue => {
                ContinueStatement::new(typed_token(keyword), value).into()
            }
            PrefixStatementKind::Return => ReturnStatement::new(typed_token(keyword), value).into(),
            PrefixStatementKind::Catch => CatchStatement::new(
                typed_token(keyword),
                value.expect("catch statements require a value"),
            )
            .into(),
            PrefixStatementKind::Throw => ThrowStatement::new(
                typed_token(keyword),
                value.expect("throw statements require a value"),
            )
            .into(),
            PrefixStatementKind::Trap => TrapStatement::new(
                typed_token(keyword),
                value.expect("trap statements require a value"),
            )
            .into(),
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
            left = match info.binary {
                BinaryAction::Adjacent => {
                    if operator.crossed_newline {
                        return Err(NativeParseError::NewlineInApplication {
                            span: operator.token.span(),
                        });
                    }
                    let adjacency_span = operator.leading_span;
                    let right = self.parse_consumed(operator, PREC_SPACE - 1)?;
                    BinaryExpression::new(
                        left,
                        BinaryOperator::from(Token![SPACE](adjacency_span)),
                        right,
                    )
                    .into()
                }
                BinaryAction::Binary => {
                    let strength = info.binary_strength.expect("binary operators have B");
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
                }
                BinaryAction::Postfix => {
                    let span = operator.token.span();
                    let spelling = token_description(&operator.token);
                    let operator = PostfixOperator::from_token_tree(operator.token).ok_or(
                        NativeParseError::Unsupported {
                            syntax: spelling,
                            span,
                        },
                    )?;
                    PostfixExpression::new(left, operator).into()
                }
                BinaryAction::Error => {
                    return Err(self.unexpected(operator.token, "an operator"));
                }
            };
        }
    }

    fn parse_delimited(&mut self, token: TokenTree) -> Result<Expr, NativeParseError> {
        let TokenTree::Group(group) = token else {
            return Err(self.unexpected(token, "a delimited group"));
        };
        let kind = group.delim_kind();
        let delimiter_span = group.double_span();
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
            crate::DelimiterKind::Empty | crate::DelimiterKind::Semicolon => {
                return Err(NativeParseError::Unsupported {
                    syntax: format!("{} cell delimiter in expression position", kind),
                    span: delimiter_span.span(),
                });
            }
            crate::DelimiterKind::Bracket => {
                let mut value = Array::new(muted, elements);
                value.delimiter = <Delimiter![[]] as crate::DelimiterToken>::new(delimiter_span);
                value.into()
            }
            crate::DelimiterKind::Brace => {
                let mut value = List::new(muted, elements);
                value.delimiter = <Delimiter![{}] as crate::DelimiterToken>::new(delimiter_span);
                value.into()
            }
            crate::DelimiterKind::AngleBar => {
                let mut value = AngleBarList::new(muted, elements);
                value.delimiter =
                    <Delimiter![< | | >] as crate::DelimiterToken>::new(delimiter_span);
                value.into()
            }
            crate::DelimiterKind::Parenthesis if !muted.is_empty() || comma => {
                let mut value = Sequence::new(muted, elements);
                value.delimiter = <Delimiter![()] as crate::DelimiterToken>::new(delimiter_span);
                value.into()
            }
            crate::DelimiterKind::Parenthesis => {
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

    fn only_expression(&self, mut components: Vec<Component>) -> Result<Expr, NativeParseError> {
        match components.pop() {
            Some(Component::Expr(value)) if components.is_empty() => Ok(value),
            Some(value) => Err(NativeParseError::Unexpected {
                found: "a sequence".into(),
                expected: "one expression",
                span: value.span(),
            }),
            None => Err(NativeParseError::MissingOperand {
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
            crate::DelimiterKind::Empty | crate::DelimiterKind::Semicolon => {
                ParseInfo::stop(PREC_CLOSER)
            }
            crate::DelimiterKind::Bracket | crate::DelimiterKind::AngleBar => {
                ParseInfo::delimiter(56)
            }
            crate::DelimiterKind::Parenthesis | crate::DelimiterKind::Brace => {
                ParseInfo::delimiter(PREC_SPACE)
            }
        };
    }

    let text = parsing_spelling(token);
    match text {
        ";" => ParseInfo::stop(PREC_SEMICOLON),
        "," => ParseInfo::stop(PREC_COMMA),
        "else" | "then" | "do" | "list" | "except" => ParseInfo::stop(PREC_CONTROL),
        "=" | ":=" | "<-" | "->" | "=>" | ">>" => ParseInfo::binary(14, 13),
        "when" | "of" | "in" | "from" | "to" => ParseInfo::stop(PREC_LOOP_CLAUSE),
        "<<" => ParseInfo::prefix_binary(18, 18, 18),
        "|-" => ParseInfo::prefix_binary(20, 20, 20),
        "===>" => ParseInfo::binary(22, 21),
        "<===" => ParseInfo::prefix_binary(22, 21, 22),
        "<==>" => ParseInfo::binary(24, 23),
        "==>" => ParseInfo::binary(26, 25),
        "<==" => ParseInfo::prefix_binary(26, 25, 26),
        "or" => ParseInfo::binary(28, 27),
        "??" => ParseInfo::prefix_binary(28, 27, 28),
        "xor" => ParseInfo::binary(30, 29),
        "and" => ParseInfo::binary(32, 31),
        "not" => ParseInfo::prefix(34, 34),
        "<" | ">" | "<=" | ">=" | "?" | "~" => ParseInfo::prefix_binary(36, 35, 36),
        "===" | "==" | "=!=" | "!=" => ParseInfo::binary(36, 35),
        "||" => ParseInfo::binary(38, 38),
        ":" => ParseInfo::binary(40, 39),
        "|" => ParseInfo::binary(42, 42),
        "^^" => ParseInfo::binary(44, 44),
        "&" => ParseInfo::binary(46, 46),
        ".." | "..<" => ParseInfo::binary(48, 48),
        "-" | "+" => ParseInfo::prefix_binary(50, 50, 50),
        "++" => ParseInfo::binary(50, 50),
        "·" => ParseInfo::binary(52, 52),
        "**" | "⊠" | "⧢" => ParseInfo::binary(54, 54),
        "\\" | "\\\\" => ParseInfo::binary(58, 57),
        "*" => ParseInfo::prefix_binary(58, 58, 58),
        "/" | "%" | "//" => ParseInfo::binary(58, 58),
        "@" => ParseInfo::binary(60, 59),
        "SPACE" => ParseInfo::binary(PREC_SPACE, PREC_SPACE - 1),
        "break" => ParseInfo::statement(PrefixStatementKind::Break),
        "continue" => ParseInfo::statement(PrefixStatementKind::Continue),
        "return" => ParseInfo::statement(PrefixStatementKind::Return),
        "catch" => ParseInfo::statement(PrefixStatementKind::Catch),
        "throw" => ParseInfo::statement(PrefixStatementKind::Throw),
        "trap" => ParseInfo::statement(PrefixStatementKind::Trap),
        "if" => ParseInfo::control(UnaryAction::If, PREC_CONTROL),
        "for" => ParseInfo::control(UnaryAction::For, PREC_LOOP_CLAUSE),
        "while" => ParseInfo::control(UnaryAction::While, PREC_CONTROL),
        "try" => ParseInfo::control(UnaryAction::Try, PREC_CONTROL),
        "TEST" | "time" | "timing" | "elapsedTime" | "elapsedTiming" | "breakpoint" | "profile"
        | "shield" | "step" | "finish" => ParseInfo::control(UnaryAction::Debug, PREC_CONTROL),
        "new" => ParseInfo::control(UnaryAction::New, PREC_LOOP_CLAUSE),
        "(*)" => ParseInfo::postfix(64),
        "@@" | "@@?" => ParseInfo::binary(66, 66),
        "^~" | "_~" | "_*" | "^*" => ParseInfo::postfix(68),
        "^" | "^>" | "^>=" | "^<" | "^<=" | "^**" | "|_" | "_" | "_>" | "_>=" | "_<" | "_<="
        | "#?" | "." | ".?" => ParseInfo::binary(70, 70),
        "#" => ParseInfo::prefix_binary(70, 70, PREC_SPACE - 1),
        "!" | "^!" | "_!" => ParseInfo::postfix(72),
        "symbol" | "global" | "threadVariable" | "threadLocal" | "local" => ParseInfo {
            precedence: PREC_SPACE,
            binary_strength: None,
            unary_strength: Some(74),
            unary: UnaryAction::Quote,
            binary: BinaryAction::Adjacent,
        },
        spelling if is_augmented_assignment(spelling) => ParseInfo::binary(14, 13),
        _ => match token {
            TokenTree::Ident(_) | TokenTree::Literal(_) => ParseInfo::atom(PREC_SPACE),
            _ => ParseInfo::stop(PREC_SPACE),
        },
    }
}

fn is_augmented_assignment(spelling: &str) -> bool {
    matches!(
        spelling,
        "%=" | "&="
            | "*="
            | "**="
            | "+="
            | "++="
            | "-="
            | "..="
            | "..<="
            | "/="
            | "//="
            | "<==>="
            | "===>="
            | "==>="
            | ">>="
            | "??="
            | "@="
            | "@@="
            | "@@?="
            | "\\="
            | "\\\\="
            | "^="
            | "^**="
            | "^^="
            | "_="
            | "|="
            | "|-="
            | "|_="
            | "||="
            | "~="
            | "·="
            | "⊠="
            | "⧢="
    )
}
