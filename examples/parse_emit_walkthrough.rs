//! Demonstrates parse emit walkthrough.

use m2_syn::{
    IdentToken, Lex, Parse, ParseStream, Span, Spanned, TokenParseError, TokenStream, TokenTree,
    parse2, quote_m2,
};

#[derive(Debug, Clone, PartialEq, Eq)]
/// State used by the Name example.
struct Name(IdentToken);

impl Parse for Name {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        let Some(token) = input.next_token() else {
            return Err(TokenParseError::UnexpectedEnd {
                expected: "identifier",
                span: Span::for_text(""),
            });
        };

        let span = token.span();
        match token {
            TokenTree::Ident(identifier) => Ok(Self(identifier)),
            token => Err(TokenParseError::UnexpectedToken {
                expected: "identifier",
                found: token.spelling().unwrap_or("group or trivia").to_owned(),
                span,
            }),
        }
    }
}

impl Lex for Name {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.0.to_tokens(output);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// State used by the Addition example.
struct Addition {
    /// The left.
    left: Name,
    /// The operator.
    operator: m2_syn::Token![+],
    /// The right.
    right: Name,
}

impl Parse for Addition {
    fn parse(input: &mut ParseStream) -> Result<Self, TokenParseError> {
        Ok(Self {
            left: Name::parse(input)?,
            operator: Parse::parse(input)?,
            right: Name::parse(input)?,
        })
    }
}

impl Lex for Addition {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.extend(self.left.to_token_stream());
        output.extend(self.operator.to_token_stream());
        output.extend(self.right.to_token_stream());
    }
}

fn main() -> Result<(), TokenParseError> {
    let quoted = quote_m2!(left + right);
    println!("1. quote_m2! produced:\n{quoted:#?}\n");

    let parsed: Addition = parse2(quoted)?;
    println!("2. Parse produced:\n{parsed:#?}\n");

    let emitted = parsed.to_token_stream();
    println!("3. Lex produced:\n{emitted:#?}\n");
    println!("4. Display merged those tokens as: {emitted}");

    let reparsed: Addition = parse2(emitted)?;
    assert_eq!(reparsed, parsed);

    Ok(())
}
