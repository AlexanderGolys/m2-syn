//! Value-returning bodies with semicolon-terminated discarded statements.

use crate::{Lex, Parse, ParseStream, Span, Spanned, Token, TokenParseError, TokenStream};

/// A statement whose value is discarded by a trailing semicolon.
pub struct Terminated<S> {
    /// The statement whose result is discarded.
    pub statement: S,
    /// The terminating semicolon.
    pub semicolon: Token![;],
}

impl<S: Spanned> Spanned for Terminated<S> {
    fn try_span(&self) -> Option<Span> {
        self.statement.try_span()?.join(self.semicolon.span())
    }
}

impl<S> Terminated<S> {
    /// Creates a value-discarding terminated statement.
    pub fn new(statement: S, semicolon: Token![;]) -> Self {
        Self {
            statement,
            semicolon,
        }
    }
}

impl<S: Parse> Parse for Terminated<S> {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        reject_empty_statement(input)?;
        Ok(Self::new(S::parse(input)?, Parse::parse(input)?))
    }
}

impl<S: Lex> Lex for Terminated<S> {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.statement.to_tokens(output);
        self.semicolon.to_tokens(output);
    }
}

/// Statements discarded by semicolons followed by an optional returned tail.
///
/// This is analogous to a Rust block: every terminated statement is evaluated
/// for effects, while the unterminated tail supplies the body's value.
#[derive(Spanned)]
pub struct Body<S> {
    /// Statements terminated by semicolons.
    pub statements: Vec<Terminated<S>>,
    /// The optional unterminated value returned by the body.
    pub tail: Option<S>,
}

impl<S> Body<S> {
    /// Creates a body from discarded statements and its returned tail.
    pub fn new(statements: Vec<Terminated<S>>, tail: Option<S>) -> Self {
        Self { statements, tail }
    }
}

impl<S: Parse> Parse for Body<S> {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let mut statements = Vec::new();
        while !input.is_eof() {
            reject_empty_statement(input)?;
            let statement = S::parse(input)?;
            if input
                .peek()
                .is_some_and(<Token![;] as Token>::matches_token_tree)
            {
                statements.push(Terminated::new(statement, Parse::parse(input)?));
            } else {
                return Ok(Self::new(statements, Some(statement)));
            }
        }
        Ok(Self::new(statements, None))
    }
}

fn reject_empty_statement(input: &ParseStream) -> Result<(), TokenParseError> {
    let Some(token) = input
        .peek()
        .filter(|token| <Token![;] as Token>::matches_token_tree(token))
    else {
        return Ok(());
    };
    Err(TokenParseError::unexpected(
        "a statement before `;`",
        ";",
        token.expect_span(),
    ))
}

impl<S: Lex> Lex for Body<S> {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.statements.to_tokens(output);
        self.tail.to_tokens(output);
    }
}
