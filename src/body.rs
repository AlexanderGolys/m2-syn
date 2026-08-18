//! Value-returning bodies with semicolon-terminated discarded statements.

use crate::cst::Token;
use crate::{Parse, ParseStream, Spanned, ToTokens, TokenParseError, TokenStream};

type Semicolon = Token![;];

/// A statement whose value is discarded by a trailing semicolon.
#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
pub struct Terminated<S> {
    pub statement: S,
    pub semicolon: Semicolon,
}

impl<S> Terminated<S> {
    /// Creates a value-discarding terminated statement.
    pub fn new(statement: S, semicolon: Semicolon) -> Self {
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

impl<S: ToTokens> ToTokens for Terminated<S> {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.statement.to_tokens(output);
        self.semicolon.to_tokens(output);
    }
}

/// Statements discarded by semicolons followed by an optional returned tail.
///
/// This is analogous to a Rust block: every terminated statement is evaluated
/// for effects, while the unterminated tail supplies the body's value.
#[derive(Debug, Clone, PartialEq, Eq, Spanned)]
pub struct Body<S> {
    pub statements: Vec<Terminated<S>>,
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
                .is_some_and(<Semicolon as Token>::matches_token_tree)
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
        .filter(|token| <Semicolon as Token>::matches_token_tree(token))
    else {
        return Ok(());
    };
    Err(TokenParseError::UnexpectedToken {
        expected: "a statement before `;`",
        found: ";".to_owned(),
        span: token.span(),
    })
}

impl<S: ToTokens> ToTokens for Body<S> {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.statements.to_tokens(output);
        self.tail.to_tokens(output);
    }
}
