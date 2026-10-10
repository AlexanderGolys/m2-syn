//! Native source-to-token conversion.
//!
//! This module owns byte classification, maximal munch, delimiter balancing,
//! and top-level cell splitting. Its output uses only the shared raw atoms from
//! `token_stream`; it never constructs a second lexer-specific token model.
//! Spans retain source identity and byte ranges, while lexical atoms retain the
//! spellings needed for parser-independent token processing.

use std::error::Error;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::mem::take;
use std::sync::Arc;

use crate::{
    Delimiter, DelimiterKind, DoubleSpan, GENERATED_OPERATOR_SPELLINGS,
    GENERATED_POSTFIX_OPERATOR_SPELLINGS, GENERATED_PUNCTUATION_SPELLINGS,
    GENERATED_WORD_TOKEN_SPELLINGS, IdentToken, LiteralKind, LiteralToken, Offset, Punct,
    Recovered, Source, Span, Spanned, TokenGroup, TokenStream, TokenTree, Trivia, TriviaKind,
};

/// The MAX GROUP DEPTH constant.
const MAX_GROUP_DEPTH: usize = 256;

/// Collapses an explicitly `Core$`-qualified identifier back to its bare
/// spelling when that spelling is one of the hardcoded word-token aliases.
pub(crate) fn canonical_word_spelling(spelling: &str) -> &str {
    let Some(word) = spelling.strip_prefix("Core$") else {
        return spelling;
    };
    if GENERATED_WORD_TOKEN_SPELLINGS.contains(&word) {
        word
    } else {
        spelling
    }
}

#[doc(hidden)]
pub fn __qualified_identifier_spans(spelling: &str, span: &Span) -> (Option<Span>, Option<Span>) {
    let components = spelling.match_indices('$').collect::<Vec<_>>();
    let subspan = |start: usize, end: usize| span.subspan(start, end);
    match components.as_slice() {
        [(separator, _)] => {
            let left = &spelling[..*separator];
            let right_start = separator + 1;
            let right = &spelling[right_start..];
            if is_bare_symbol_name(left) && is_bare_symbol_name(right) {
                (Some(subspan(0, *separator)), None)
            } else if is_bare_symbol_name(left) && is_decimal_index(right) {
                (None, Some(subspan(right_start, spelling.len())))
            } else {
                (None, None)
            }
        }
        [(package_separator, _), (index_separator, _)] => {
            let package = &spelling[..*package_separator];
            let name_start = package_separator + 1;
            let name = &spelling[name_start..*index_separator];
            let index_start = index_separator + 1;
            let index = &spelling[index_start..];
            if is_bare_symbol_name(package) && is_bare_symbol_name(name) && is_decimal_index(index)
            {
                (
                    Some(subspan(0, *package_separator)),
                    Some(subspan(index_start, spelling.len())),
                )
            } else {
                (None, None)
            }
        }
        _ => (None, None),
    }
}

/// Returns whether the value is bare symbol name.
fn is_bare_symbol_name(spelling: &str) -> bool {
    let Some(first) = spelling.as_bytes().first().copied() else {
        return false;
    };
    is_identifier_start(first)
        && spelling
            .as_bytes()
            .iter()
            .copied()
            .all(|byte| byte != b'$' && is_identifier_continue(byte))
}

/// Returns whether the value is decimal index.
fn is_decimal_index(spelling: &str) -> bool {
    !spelling.is_empty() && spelling.bytes().all(|byte| byte.is_ascii_digit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// The class of a tokenization failure.
pub enum LexErrorKind {
    /// A byte cannot begin any M2 token.
    InvalidCharacter,
    /// A string contains an invalid escape sequence.
    InvalidEscape,
    /// A numeric spelling is malformed.
    InvalidNumber,
    /// The byte input is not valid UTF-8.
    InvalidUtf8,
    /// Delimiter nesting exceeded the defensive parser limit.
    NestingLimitExceeded,
    /// A closing delimiter has no matching opener.
    UnexpectedClosingDelimiter,
    /// Input ended before a delimited group closed.
    UnterminatedGroup,
    /// Input ended inside a string literal.
    UnterminatedString,
    /// Input ended inside a raw string literal.
    UnterminatedRawString,
    /// Input ended inside a block comment.
    UnterminatedBlockComment,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Spanned)]
/// A tokenization failure and the source span that caused it.
pub struct LexError {
    /// The kind.
    kind: LexErrorKind,
    /// The delimiter.
    delimiter: Option<DelimiterKind>,
    /// The span.
    span: Span,
}

impl LexError {
    /// Returns the error class.
    pub fn kind(&self) -> LexErrorKind {
        self.kind
    }

    /// Returns the unclosed expression family for a missing closer.
    pub fn unclosed_expression(&self) -> Option<&'static str> {
        match self.kind {
            LexErrorKind::UnterminatedGroup => {
                self.delimiter.as_ref().map(DelimiterKind::expression_name)
            }
            LexErrorKind::UnterminatedString => Some("string literal"),
            LexErrorKind::UnterminatedRawString => Some("raw string literal"),
            _ => None,
        }
    }

    /// Returns the token spelling required to complete this error.
    pub fn expected(&self) -> Option<&'static str> {
        match self.kind {
            LexErrorKind::UnterminatedGroup => self.delimiter.map(DelimiterKind::closing),
            LexErrorKind::UnterminatedString => Some("\""),
            LexErrorKind::UnterminatedRawString => Some("///"),
            LexErrorKind::UnterminatedBlockComment => Some("*-"),
            _ => None,
        }
    }

    /// Returns a readable description of what terminated this error.
    pub fn found(&self) -> Option<&'static str> {
        matches!(
            self.kind,
            LexErrorKind::UnterminatedGroup
                | LexErrorKind::UnterminatedString
                | LexErrorKind::UnterminatedRawString
                | LexErrorKind::UnterminatedBlockComment
        )
        .then_some("end of input")
    }
}

impl Display for LexError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FmtResult {
        let message = match self.kind {
            LexErrorKind::InvalidCharacter => "invalid character",
            LexErrorKind::InvalidEscape => "invalid string escape",
            LexErrorKind::InvalidNumber => "invalid number literal",
            LexErrorKind::InvalidUtf8 => "source is not valid UTF-8",
            LexErrorKind::NestingLimitExceeded => "delimiter nesting limit exceeded",
            LexErrorKind::UnexpectedClosingDelimiter => "unexpected closing delimiter",
            LexErrorKind::UnterminatedGroup => {
                if let Some(delimiter) = self.delimiter {
                    return write!(
                        formatter,
                        "unclosed {}: expected `{}`, found end of input",
                        delimiter.expression_name(),
                        delimiter.closing(),
                    );
                }
                "unterminated delimited group"
            }
            LexErrorKind::UnterminatedString => {
                "unclosed string literal: expected `\"`, found end of input"
            }
            LexErrorKind::UnterminatedRawString => {
                "unclosed raw string literal: expected `///`, found end of input"
            }
            LexErrorKind::UnterminatedBlockComment => "unterminated block comment",
        };
        formatter.write_str(message)
    }
}

impl Error for LexError {}

/// Lexes bytes into one source-spanned stream of token trees.
///
/// Malformed source is always reported as [`LexError`]; byte contents do not
/// cause this function to panic. A user-provided iterator may still panic from
/// its own [`Iterator::next`] implementation.
///
/// # Errors
///
/// Returns [`LexError`] for malformed UTF-8, literals, comments, delimiters,
/// or characters that Macaulay2 cannot tokenize.
pub fn lex<I>(bytes: I) -> Result<TokenStream, LexError>
where
    I: IntoIterator<Item = u8>,
{
    lexer_from_bytes(bytes)?.lex()
}

/// Lexes bytes while retaining groups whose closing delimiter is missing.
///
/// Every missing closer becomes an unbounded closing boundary.
/// The corresponding [`LexErrorKind::UnterminatedGroup`] diagnostic spans from
/// that group's opening delimiter through end of input. Other lexical failures
/// remain fatal.
///
/// # Errors
///
/// Returns [`LexError`] for malformed UTF-8, literals, comments, unmatched
/// closing delimiters, invalid characters, or excessive delimiter nesting.
pub fn lex_recovering<I>(bytes: I) -> Result<Recovered<TokenStream, LexError>, LexError>
where
    I: IntoIterator<Item = u8>,
{
    lexer_from_bytes(bytes)?.lex_recovering()
}

/// Lexes M2 source text into its source-spanned token tree.
///
/// # Errors
///
/// Returns [`LexError`] when the source contains malformed lexical syntax.
pub fn lex_str(source: &str) -> Result<TokenStream, LexError> {
    NativeLexer::new(Arc::<str>::from(source)).lex()
}

/// Lexes source text while retaining unclosed groups and string literals.
///
/// # Errors
///
/// Returns [`LexError`] for malformed lexical syntax other than missing group
/// or string-literal closers, which are returned in
/// [`Recovered::diagnostics`].
pub fn lex_str_recovering(source: &str) -> Result<Recovered<TokenStream, LexError>, LexError> {
    NativeLexer::new(Arc::<str>::from(source)).lex_recovering()
}

/// Lexes one statically spelled fragment emitted by [`crate::quote_m2!`].
///
/// Procedural-macro input arrives as Rust token trees, so the macro first joins
/// each uninterrupted static run into M2 source text. Sending that text through
/// the real lexer keeps quoted literals, identifier rules, and maximal
/// punctuation exactly aligned with ordinary source parsing.
///
/// # Panics
///
/// Panics when a static fragment accepted by Rust's tokenization is not valid
/// M2 lexical syntax. Interpolate a pre-built [`TokenStream`] when its source
/// spelling cannot be represented directly in Rust macro input.
#[doc(hidden)]
pub fn __quote_fragment(source: &str) -> TokenStream {
    NativeLexer::new(Arc::<str>::from(source))
        .lex()
        .unwrap_or_else(|error| panic!("invalid static M2 quote fragment: {error}"))
}

/// Handles lexer from bytes.
fn lexer_from_bytes<I>(bytes: I) -> Result<NativeLexer, LexError>
where
    I: IntoIterator<Item = u8>,
{
    let bytes = bytes.into_iter().collect::<Vec<_>>();
    let len = bytes.len();
    let source = String::from_utf8(bytes).map_err(|_| LexError {
        kind: LexErrorKind::InvalidUtf8,
        delimiter: None,
        span: Span::ranged(Source::new(""), 0..len),
    })?;
    Ok(NativeLexer::new(Arc::<str>::from(source)))
}

/// Internal state for NativeLexer.
struct NativeLexer {
    /// The source.
    source: Arc<str>,
    /// The source span.
    source_span: Span,
    /// The offset.
    offset: Offset,
}

impl NativeLexer {
    /// Creates a new value.
    fn new(source: Arc<str>) -> Self {
        let source_span = Span::ranged(Source::new(Arc::clone(&source)), 0..source.len());
        Self {
            source,
            source_span,
            offset: 0,
        }
    }

    /// Handles lex.
    fn lex(self) -> Result<TokenStream, LexError> {
        let recovered = self.lex_recovering()?;
        match recovered.diagnostics.into_iter().next() {
            Some(error) => Err(error),
            None => Ok(recovered.value),
        }
    }

    /// Lexes recovering.
    fn lex_recovering(mut self) -> Result<Recovered<TokenStream, LexError>, LexError> {
        let mut output = TokenStream::new();
        let mut groups = Vec::<OpenGroup>::new();
        let mut diagnostics = Vec::new();
        // A quote specifier takes the following token literally, so that token
        // is not interpreted as structure. This is the one grammar fact the
        // lexer needs: without it `symbol (` opens a group and `symbol ;` ends
        // a cell, neither of which M2 does.
        let mut quote_next = false;
        loop {
            let Some(first) = self.peek(0) else {
                output.set_source(Arc::clone(self.source_span.source()));
                while let Some(group) = groups.pop() {
                    diagnostics.push(LexError {
                        kind: LexErrorKind::UnterminatedGroup,
                        delimiter: Some(group.kind),
                        span: self.span_from(group.opening_start),
                    });
                }
                return Ok(Recovered::new(output, diagnostics));
            };

            if first == b'\n' || (first == b'\r' && self.byte_after_current() == Some(b'\n')) {
                self.lex_line_break(&mut output)?;
            } else if first == b'\r' {
                self.lex_carriage_return(&mut output)?;
            } else if matches!(first, b' ' | b'\t') {
                self.lex_whitespace(&mut output)?;
            } else if let Some(width) = self.math_operator_width() {
                self.lex_math_operator(width, &mut output)?;
                quote_next = false;
            } else if is_identifier_start(first) {
                let quoted = take(&mut quote_next);
                self.lex_identifier(&mut output)?;
                quote_next = !quoted && last_token_quotes_next(&output);
            } else if first.is_ascii_digit()
                || (first == b'.'
                    && self
                        .byte_after_current()
                        .is_some_and(|byte| byte.is_ascii_digit()))
            {
                self.lex_number(&mut output)?;
                quote_next = false;
            } else if first == b'"' {
                match self.lex_string(&mut output) {
                    Ok(()) => {}
                    Err(error) if error.kind == LexErrorKind::UnterminatedString => {
                        output.extend([TokenTree::Literal(LiteralToken::from_span(
                            LiteralKind::String,
                            error.span.clone(),
                        ))]);
                        diagnostics.push(error);
                    }
                    Err(error) => return Err(error),
                }
                quote_next = false;
            } else if let Some(fixed) = self.match_fixed() {
                let quoted = take(&mut quote_next);
                match fixed.kind {
                    // A quoted bracket is an ordinary token, not a group
                    // boundary; the parser reads it as the quoted symbol.
                    FixedKind::Open(_) | FixedKind::Close(_) if quoted => {
                        self.lex_punctuation(fixed.len, &mut output)?;
                    }
                    FixedKind::Punctuation => {
                        self.lex_punctuation(fixed.len, &mut output)?;
                    }
                    // A comment does not consume the pending quote: M2 reads
                    // `symbol -- note⏎x` as the quoted `x`.
                    FixedKind::LineComment => {
                        self.lex_line_comment(fixed.len, &mut output)?;
                        quote_next = quoted;
                    }
                    FixedKind::BlockComment => {
                        self.lex_block_comment(fixed.len, &mut output)?;
                        quote_next = quoted;
                    }
                    FixedKind::RawString => match self.lex_raw_string(fixed.len, &mut output) {
                        Ok(()) => {}
                        Err(error) if error.kind == LexErrorKind::UnterminatedRawString => {
                            output.extend([TokenTree::Literal(LiteralToken::from_span(
                                LiteralKind::RawString,
                                error.span.clone(),
                            ))]);
                            diagnostics.push(error);
                        }
                        Err(error) => return Err(error),
                    },
                    FixedKind::Open(kind) => {
                        if groups.len() == MAX_GROUP_DEPTH {
                            return Err(self.error(LexErrorKind::NestingLimitExceeded, self.offset));
                        }
                        let opening_start = self.offset;
                        self.take_exact(fixed.len, opening_start)?;
                        let opening = self.span_from(opening_start);
                        groups.push(OpenGroup {
                            kind,
                            opening_start,
                        });
                        output.extend([TokenGroup::new(
                            Delimiter::new(kind, DoubleSpan::unclosed(opening)),
                            TokenStream::new(),
                        )]);
                    }
                    FixedKind::Close(kind) => {
                        let closing_start = self.offset;
                        self.take_exact(fixed.len, closing_start)?;
                        let closing = self.span_from(closing_start);
                        let Some(group) = groups.pop() else {
                            return Err(
                                self.error(LexErrorKind::UnexpectedClosingDelimiter, closing_start)
                            );
                        };
                        if group.kind != kind {
                            return Err(
                                self.error(LexErrorKind::UnexpectedClosingDelimiter, closing_start)
                            );
                        }
                        assert!(
                            output.complete_last_group(kind, closing),
                            "the open-group stack and token stream must agree"
                        );
                    }
                }
            } else {
                let start = self.offset;
                self.next();
                return Err(self.error(LexErrorKind::InvalidCharacter, start));
            }
        }
    }

    /// Lexes line break.
    fn lex_line_break(&mut self, _output: &mut TokenStream) -> Result<(), LexError> {
        let start = self.offset;
        let first = self.next_or_error(LexErrorKind::InvalidCharacter, start)?;
        let mut bytes = vec![first];
        if first == b'\r' && self.peek(0) == Some(b'\n') {
            bytes.push(self.next_or_error(LexErrorKind::InvalidCharacter, start)?);
        }
        let _ = bytes;
        Ok(())
    }

    /// Lexes carriage return.
    fn lex_carriage_return(&mut self, _output: &mut TokenStream) -> Result<(), LexError> {
        let start = self.offset;
        let byte = self.next_or_error(LexErrorKind::InvalidCharacter, start)?;
        let _ = byte;
        Ok(())
    }

    /// Lexes whitespace.
    fn lex_whitespace(&mut self, _output: &mut TokenStream) -> Result<(), LexError> {
        let bytes = self.take_while(|byte| matches!(byte, b' ' | b'\t'));
        let _ = bytes;
        Ok(())
    }

    /// Lexes identifier.
    fn lex_identifier(&mut self, output: &mut TokenStream) -> Result<(), LexError> {
        let start = self.offset;
        let mut bytes = vec![self.next_or_error(LexErrorKind::InvalidUtf8, start)?];
        while let Some(byte) = self.peek(0) {
            if !is_identifier_continue(byte) || self.math_operator_width().is_some() {
                break;
            }
            bytes.push(self.next_or_error(LexErrorKind::InvalidUtf8, start)?);
        }
        let _ = bytes;
        output.extend([TokenTree::Ident(IdentToken::from_span(
            self.span_from(start),
        ))]);
        Ok(())
    }

    /// Lexes number.
    fn lex_number(&mut self, output: &mut TokenStream) -> Result<(), LexError> {
        let start = self.offset;
        let mut bytes = Vec::new();
        let mut decimal = true;

        if self.peek(0) == Some(b'0') {
            bytes.push(self.next_or_error(LexErrorKind::InvalidNumber, start)?);
            if let Some(radix) = self.number_radix() {
                decimal = false;
                bytes.push(self.next_or_error(LexErrorKind::InvalidNumber, start)?);
                bytes.extend(self.take_while(|byte| radix.is_digit(byte)));
            } else {
                bytes.extend(self.take_while(|byte| byte.is_ascii_digit()));
            }
        } else {
            bytes.extend(self.take_while(|byte| byte.is_ascii_digit()));
        }

        let next = self.peek(0);
        let begins_float = decimal
            && (next == Some(b'.') && self.byte_after_current() != Some(b'.')
                || matches!(next, Some(b'p' | b'e' | b'E')));
        let kind = if begins_float {
            if self.peek(0) == Some(b'.') {
                bytes.push(self.next_or_error(LexErrorKind::InvalidNumber, start)?);
                bytes.extend(self.take_while(|byte| byte.is_ascii_digit()));
            }
            if self.peek(0) == Some(b'p') {
                bytes.push(self.next_or_error(LexErrorKind::InvalidNumber, start)?);
                if !self.peek(0).is_some_and(|byte| byte.is_ascii_digit()) {
                    return Err(self.error(LexErrorKind::InvalidNumber, start));
                }
                bytes.extend(self.take_while(|byte| byte.is_ascii_digit()));
            }
            if matches!(self.peek(0), Some(b'e' | b'E')) {
                bytes.push(self.next_or_error(LexErrorKind::InvalidNumber, start)?);
                if matches!(self.peek(0), Some(b'+' | b'-')) {
                    bytes.push(self.next_or_error(LexErrorKind::InvalidNumber, start)?);
                }
                if !self.peek(0).is_some_and(|byte| byte.is_ascii_digit()) {
                    return Err(self.error(LexErrorKind::InvalidNumber, start));
                }
                bytes.extend(self.take_while(|byte| byte.is_ascii_digit()));
            }
            LiteralKind::Float
        } else {
            LiteralKind::Integer
        };

        let _ = bytes;
        output.extend([TokenTree::Literal(LiteralToken::from_span(
            kind,
            self.span_from(start),
        ))]);
        Ok(())
    }

    /// Handles number radix.
    fn number_radix(&mut self) -> Option<NumberRadix> {
        let marker = self.peek(0)?;
        let digit = self.peek(1);
        match marker {
            b'b' | b'B' if digit.is_some_and(|byte| NumberRadix::Binary.is_digit(byte)) => {
                Some(NumberRadix::Binary)
            }
            b'o' | b'O' if digit.is_some_and(|byte| NumberRadix::Octal.is_digit(byte)) => {
                Some(NumberRadix::Octal)
            }
            b'x' | b'X' if digit.is_some_and(|byte| NumberRadix::Hexadecimal.is_digit(byte)) => {
                Some(NumberRadix::Hexadecimal)
            }
            _ => None,
        }
    }

    /// Lexes string.
    fn lex_string(&mut self, output: &mut TokenStream) -> Result<(), LexError> {
        let start = self.offset;
        let mut bytes = vec![self.next_or_error(LexErrorKind::UnterminatedString, start)?];

        loop {
            match self.peek(0) {
                None => return Err(self.error(LexErrorKind::UnterminatedString, start)),
                Some(b'"') => {
                    bytes.push(self.next_or_error(LexErrorKind::UnterminatedString, start)?);
                    let _ = bytes;
                    output.extend([TokenTree::Literal(LiteralToken::from_span(
                        LiteralKind::String,
                        self.span_from(start),
                    ))]);
                    return Ok(());
                }
                Some(b'\\') => {
                    let escape_start = self.offset;
                    bytes.push(self.next_or_error(LexErrorKind::UnterminatedString, start)?);
                    let Some(escaped) = self.peek(0) else {
                        return Err(self.error(LexErrorKind::UnterminatedString, start));
                    };
                    bytes.push(self.next_or_error(LexErrorKind::UnterminatedString, start)?);
                    match escaped {
                        b'a' | b'b' | b'e' | b'E' | b'f' | b'r' | b't' | b'v' | b'n' | b'"'
                        | b'\\' => {}
                        b'0'..=b'7' => {
                            for _ in 0..2 {
                                if !matches!(self.peek(0), Some(b'0'..=b'7')) {
                                    break;
                                }
                                bytes.push(
                                    self.next_or_error(LexErrorKind::InvalidEscape, escape_start)?,
                                );
                            }
                        }
                        b'x' => self.take_hex_escape(&mut bytes, 2, escape_start)?,
                        b'u' => self.take_hex_escape(&mut bytes, 4, escape_start)?,
                        _ => return Err(self.error(LexErrorKind::InvalidEscape, escape_start)),
                    }
                }
                Some(_) => bytes.push(self.next_or_error(LexErrorKind::UnterminatedString, start)?),
            }
        }
    }

    /// Takes hex escape.
    fn take_hex_escape(
        &mut self,
        bytes: &mut Vec<u8>,
        digits: usize,
        start: Offset,
    ) -> Result<(), LexError> {
        for _ in 0..digits {
            if !self.peek(0).is_some_and(|byte| byte.is_ascii_hexdigit()) {
                return Err(self.error(LexErrorKind::InvalidEscape, start));
            }
            bytes.push(self.next_or_error(LexErrorKind::InvalidEscape, start)?);
        }
        Ok(())
    }

    /// Lexes raw string.
    fn lex_raw_string(
        &mut self,
        opener_len: usize,
        output: &mut TokenStream,
    ) -> Result<(), LexError> {
        let start = self.offset;
        let mut bytes = self.take_exact(opener_len, start)?;

        loop {
            if self.peek(0).is_none() {
                return Err(self.error(LexErrorKind::UnterminatedRawString, start));
            }
            if self.peek(0) == Some(b'/') {
                match self.slash_run_len() {
                    3 => {
                        bytes.extend(self.take_exact(3, start)?);
                        let _ = bytes;
                        output.extend([TokenTree::Literal(LiteralToken::from_span(
                            LiteralKind::RawString,
                            self.span_from(start),
                        ))]);
                        return Ok(());
                    }
                    0 => return Err(self.error(LexErrorKind::UnterminatedRawString, start)),
                    1 => {
                        bytes.push(self.next_or_error(LexErrorKind::UnterminatedRawString, start)?);
                    }
                    _ => bytes.extend(self.take_exact(2, start)?),
                }
            } else {
                bytes.push(self.next_or_error(LexErrorKind::UnterminatedRawString, start)?);
            }
        }
    }

    /// Lexes line comment.
    fn lex_line_comment(
        &mut self,
        opener_len: usize,
        output: &mut TokenStream,
    ) -> Result<(), LexError> {
        let start = self.offset;
        let mut bytes = self.take_exact(opener_len, start)?;
        loop {
            match self.peek(0) {
                None | Some(b'\n') => break,
                Some(b'\r') if self.byte_after_current() == Some(b'\n') => break,
                Some(_) => bytes.push(self.next_or_error(LexErrorKind::InvalidCharacter, start)?),
            }
        }
        let _ = bytes;
        output.push_line_comment(Trivia::from_span(
            TriviaKind::LineComment,
            self.span_from(start),
        ));
        Ok(())
    }

    /// Lexes block comment.
    fn lex_block_comment(
        &mut self,
        opener_len: usize,
        output: &mut TokenStream,
    ) -> Result<(), LexError> {
        let start = self.offset;
        let mut bytes = self.take_exact(opener_len, start)?;
        loop {
            if self.peek(0).is_none() {
                return Err(self.error(LexErrorKind::UnterminatedBlockComment, start));
            }
            if self.peek(0) == Some(b'*') && self.byte_after_current() == Some(b'-') {
                bytes.extend(self.take_exact(2, start)?);
                let _ = bytes;
                output.push_block_comment(Trivia::from_span(
                    TriviaKind::BlockComment,
                    self.span_from(start),
                ));
                return Ok(());
            }
            bytes.push(self.next_or_error(LexErrorKind::UnterminatedBlockComment, start)?);
        }
    }

    /// Handles match fixed.
    fn match_fixed(&mut self) -> Option<FixedMatch> {
        fixed_spellings()
            .filter(|(spelling, _)| {
                spelling
                    .bytes()
                    .enumerate()
                    .all(|(distance, expected)| self.peek(distance) == Some(expected))
            })
            .max_by_key(|(spelling, _)| spelling.len())
            .map(|(spelling, kind)| FixedMatch {
                len: spelling.len(),
                kind,
            })
    }

    /// Handles math operator width.
    fn math_operator_width(&mut self) -> Option<usize> {
        let first = self.peek(0)?;
        if !matches!(first, 0xc2 | 0xc3 | 0xe2) {
            return None;
        }
        self.peek(1)
            .filter(|second| is_math_operator_pair(first, *second))
            .and_then(|_| utf8_char_width(first))
    }

    /// Lexes math operator.
    fn lex_math_operator(
        &mut self,
        width: usize,
        output: &mut TokenStream,
    ) -> Result<(), LexError> {
        let start = self.offset;
        let mut bytes = self.take_exact(width, start)?;
        if self.peek(0) == Some(b'=') {
            bytes.push(self.next_or_error(LexErrorKind::InvalidUtf8, start)?);
        }
        self.emit_punctuation(bytes, start, output)
    }

    /// Lexes punctuation.
    fn lex_punctuation(&mut self, len: usize, output: &mut TokenStream) -> Result<(), LexError> {
        let start = self.offset;
        let bytes = self.take_exact(len, start)?;
        self.emit_punctuation(bytes, start, output)
    }

    /// Emits punctuation.
    fn emit_punctuation(
        &self,
        bytes: Vec<u8>,
        start: Offset,
        output: &mut TokenStream,
    ) -> Result<(), LexError> {
        let _ = bytes;
        output.extend([TokenTree::Punct(Punct::from_span(self.span_from(start)))]);
        Ok(())
    }

    /// Handles byte after current.
    fn byte_after_current(&mut self) -> Option<u8> {
        self.peek(1)
    }

    /// Handles slash run len.
    fn slash_run_len(&mut self) -> usize {
        let mut len = 0;
        while len < 4 && self.peek(len) == Some(b'/') {
            len += 1;
        }
        len
    }

    /// Takes while.
    fn take_while(&mut self, predicate: impl Fn(u8) -> bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        while self.peek(0).is_some_and(&predicate) {
            let Some(byte) = self.next() else {
                break;
            };
            bytes.push(byte);
        }
        bytes
    }

    /// Takes exact.
    fn take_exact(&mut self, len: usize, start: Offset) -> Result<Vec<u8>, LexError> {
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            let Some(byte) = self.next() else {
                return Err(self.error(LexErrorKind::InvalidUtf8, start));
            };
            bytes.push(byte);
        }
        Ok(bytes)
    }

    /// Handles next.
    fn next(&mut self) -> Option<u8> {
        let byte = self.peek(0)?;
        self.offset += 1;
        Some(byte)
    }

    /// Handles peek.
    fn peek(&self, distance: usize) -> Option<u8> {
        self.offset
            .checked_add(distance)
            .and_then(|offset| self.source.as_bytes().get(offset))
            .copied()
    }

    /// Handles next or error.
    fn next_or_error(&mut self, kind: LexErrorKind, start: Offset) -> Result<u8, LexError> {
        match self.next() {
            Some(byte) => Ok(byte),
            None => Err(self.error(kind, start)),
        }
    }

    /// Handles error.
    fn error(&self, kind: LexErrorKind, start: Offset) -> LexError {
        LexError {
            kind,
            delimiter: None,
            span: self.span_from(start),
        }
    }

    /// The span running from `start` to wherever the cursor has reached.
    fn span_from(&self, start: Offset) -> Span {
        self.source_span.subspan(start, self.offset)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Internal alternatives for RequiredClause.
enum RequiredClause {
    /// The Then alternative.
    Then,
    /// The LoopBody alternative.
    LoopBody,
    /// The ExceptBody alternative.
    ExceptBody,
}

/// Handles newline ends cell.
pub(crate) fn newline_ends_cell(stream: &TokenStream) -> bool {
    let mut required_clauses = Vec::new();
    let mut last_text = None;
    let mut quote_next = false;

    for tree in stream.iter() {
        let text = match tree {
            TokenTree::Ident(token) => Some(token.text()),
            TokenTree::Punct(token) => Some(token.text()),
            TokenTree::Literal(_) | TokenTree::Group(_) => {
                quote_next = false;
                last_text = None;
                None
            }
        };

        let Some(text) = text else {
            continue;
        };
        if quote_next {
            quote_next = false;
            last_text = None;
            continue;
        }
        let text = canonical_word_spelling(text);
        last_text = Some(text);
        if is_quote_specifier(text) {
            quote_next = true;
            continue;
        }
        match text {
            "if" => required_clauses.push(RequiredClause::Then),
            "for" | "while" => required_clauses.push(RequiredClause::LoopBody),
            "except" => required_clauses.push(RequiredClause::ExceptBody),
            "then" => {
                remove_last_clause(&mut required_clauses, RequiredClause::Then);
            }
            "list" => {
                remove_last_clause(&mut required_clauses, RequiredClause::LoopBody);
            }
            "do" if !remove_last_clause(&mut required_clauses, RequiredClause::ExceptBody) => {
                remove_last_clause(&mut required_clauses, RequiredClause::LoopBody);
            }
            _ => {}
        }
    }

    if !required_clauses.is_empty() {
        return false;
    }

    let Some(last) = last_text else {
        return true;
    };
    if keyword_requires_value(last) {
        return false;
    }
    !GENERATED_OPERATOR_SPELLINGS.contains(&last)
        || GENERATED_POSTFIX_OPERATOR_SPELLINGS.contains(&last)
}

/// Removes last clause.
fn remove_last_clause(clauses: &mut Vec<RequiredClause>, clause: RequiredClause) -> bool {
    let Some(position) = clauses.iter().rposition(|candidate| *candidate == clause) else {
        return false;
    };
    clauses.remove(position);
    true
}

/// Handles keyword requires value.
fn keyword_requires_value(keyword: &str) -> bool {
    matches!(
        keyword,
        "if" | "then"
            | "else"
            | "for"
            | "while"
            | "in"
            | "from"
            | "to"
            | "when"
            | "list"
            | "do"
            | "new"
            | "of"
            | "try"
            | "except"
            | "catch"
            | "throw"
            | "trap"
            | "shield"
            | "TEST"
            | "time"
            | "timing"
            | "breakpoint"
            | "elapsedTime"
            | "elapsedTiming"
            | "profile"
            | "symbol"
            | "local"
            | "global"
            | "threadVariable"
            | "threadLocal"
    )
}

/// Whether the token just lexed quotes the one after it.
fn last_token_quotes_next(stream: &TokenStream) -> bool {
    matches!(stream.last_innermost(), Some(TokenTree::Ident(token))
        if is_quote_specifier(canonical_word_spelling(token.text())))
}

/// Returns whether the value is quote specifier.
pub(crate) fn is_quote_specifier(keyword: &str) -> bool {
    matches!(
        keyword,
        "symbol" | "local" | "global" | "threadVariable" | "threadLocal"
    )
}

/// Handles punctuation would merge.
pub(crate) fn punctuation_would_merge(left: &str, right: &str) -> bool {
    let mut combined = String::with_capacity(left.len() + right.len());
    combined.push_str(left);
    combined.push_str(right);
    fixed_spellings()
        .any(|(spelling, _)| spelling.len() > left.len() && combined.starts_with(spelling))
        || (right.starts_with('=') && is_math_operator(left))
}

#[derive(Debug)]
/// Internal state for OpenGroup.
struct OpenGroup {
    /// The kind.
    kind: DelimiterKind,
    /// The opening start.
    opening_start: Offset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Internal alternatives for FixedKind.
enum FixedKind {
    /// The Punctuation alternative.
    Punctuation,
    /// The LineComment alternative.
    LineComment,
    /// The BlockComment alternative.
    BlockComment,
    /// The RawString alternative.
    RawString,
    /// The Open alternative.
    Open(DelimiterKind),
    /// The Close alternative.
    Close(DelimiterKind),
}

/// The FIXED SPELLINGS constant.
const FIXED_SPELLINGS: &[(&str, FixedKind)] = &[
    ("///", FixedKind::RawString),
    ("--", FixedKind::LineComment),
    ("-*", FixedKind::BlockComment),
    ("(", FixedKind::Open(DelimiterKind::Parenthesis)),
    (")", FixedKind::Close(DelimiterKind::Parenthesis)),
    ("[", FixedKind::Open(DelimiterKind::Bracket)),
    ("]", FixedKind::Close(DelimiterKind::Bracket)),
    ("{", FixedKind::Open(DelimiterKind::Brace)),
    ("}", FixedKind::Close(DelimiterKind::Brace)),
    ("<|", FixedKind::Open(DelimiterKind::AngleBar)),
    ("|>", FixedKind::Close(DelimiterKind::AngleBar)),
];

/// Handles fixed spellings.
fn fixed_spellings() -> impl Iterator<Item = (&'static str, FixedKind)> {
    FIXED_SPELLINGS.iter().copied().chain(
        GENERATED_PUNCTUATION_SPELLINGS
            .iter()
            .map(|spelling| (*spelling, FixedKind::Punctuation)),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Internal state for FixedMatch.
struct FixedMatch {
    /// The len.
    len: usize,
    /// The kind.
    kind: FixedKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Internal alternatives for NumberRadix.
enum NumberRadix {
    /// The Binary alternative.
    Binary,
    /// The Octal alternative.
    Octal,
    /// The Hexadecimal alternative.
    Hexadecimal,
}

impl NumberRadix {
    /// Returns whether the value is digit.
    fn is_digit(self, byte: u8) -> bool {
        match self {
            Self::Binary => matches!(byte, b'0' | b'1'),
            Self::Octal => matches!(byte, b'0'..=b'7'),
            Self::Hexadecimal => byte.is_ascii_hexdigit(),
        }
    }
}

/// Returns whether the value is identifier start.
fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte >= 128
}

/// Returns whether the value is identifier continue.
fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'$' | b'\'') || byte >= 128
}

/// Returns whether the value is math operator pair.
fn is_math_operator_pair(first: u8, second: u8) -> bool {
    matches!(
        (first, second),
        (0xc2, 0xa0..=0xbf)
            | (0xc3, 0x97 | 0xb7)
            | (0xe2, 0x86..=0x8f | 0x9f | 0xa4..=0xaf)
    )
}

/// Returns whether the value is math operator.
fn is_math_operator(spelling: &str) -> bool {
    let bytes = spelling.as_bytes();
    bytes
        .first()
        .zip(bytes.get(1))
        .is_some_and(|(first, second)| {
            is_math_operator_pair(*first, *second) && utf8_char_width(*first) == Some(bytes.len())
        })
}

/// Handles UTF-8 char width.
fn utf8_char_width(first: u8) -> Option<usize> {
    match first {
        0x00..=0x7f => Some(1),
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(source: &str) -> TokenStream {
        let stream = lex_str(source).unwrap();
        assert_eq!(stream.to_source(), source);
        stream
    }

    fn span_at(start: usize, text: &str) -> Span {
        let source = Source::new(format!("{}{}", " ".repeat(start), text));
        Span::ranged(source, start..start + text.len())
    }

    #[test]
    fn lexer_accepts_an_arbitrary_non_peekable_byte_iterator() {
        let bytes = vec![b'a', b'+', b'1'].into_iter();
        let stream = lex(bytes).unwrap();
        assert_eq!(stream.to_source(), "a+1");
    }

    #[test]
    fn every_short_byte_sequence_returns_without_panicking() {
        fn check(bytes: &[u8]) {
            let outcome = std::panic::catch_unwind(|| lex(bytes.iter().copied()));
            assert!(outcome.is_ok(), "lexer panicked for bytes {bytes:?}");
        }

        check(&[]);
        for first in u8::MIN..=u8::MAX {
            check(&[first]);
            for second in u8::MIN..=u8::MAX {
                check(&[first, second]);
            }
        }
    }

    #[test]
    fn invalid_utf8_is_a_lex_error() {
        assert_eq!(
            lex([0xf0, 0x80, 0x80]).unwrap_err().kind(),
            LexErrorKind::InvalidUtf8
        );
    }

    #[test]
    fn punctuation_is_returned_as_maximal_tokens() {
        let source = "<<|-1||>2";
        let stream = tokens(source);
        let punctuation = stream
            .iter()
            .filter_map(|tree| match tree {
                TokenTree::Punct(token) => Some(token.text()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let integers = stream
            .iter()
            .filter_map(|tree| match tree {
                TokenTree::Literal(token) if token.kind == LiteralKind::Integer => {
                    Some(token.text())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(punctuation, ["<<", "|-", "||", ">"]);
        assert_eq!(integers, ["1", "2"]);
    }

    #[test]
    fn comment_openers_only_win_at_the_current_position() {
        let ordinary = tokens("|--1");
        assert!(
            ordinary.iter().all(|tree| {
                tree.leading_trivia().is_empty() && tree.trailing_trivia().is_empty()
            })
        );

        let source = "-- comment\n-* block *-";
        let comments = tokens(source);
        assert!(comments.is_empty());
        assert!(matches!(
            comments.suffix_trivia(),
            [line, block]
                if line.kind() == TriviaKind::LineComment
                    && block.kind() == TriviaKind::BlockComment
        ));
    }

    #[test]
    fn leading_operator_runs_split_at_maximal_known_spellings() {
        let source = "***1";
        let stream = tokens(source);
        let trees = stream.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [
                first,
                second,
                TokenTree::Literal(integer),
            ] if first.spelling() == Some("**")
                && second.spelling() == Some("*")
                && integer.kind == LiteralKind::Integer
                && integer.text() == "1"
        ));
    }

    #[test]
    fn identifiers_follow_the_m2_character_classes() {
        let source = "Core$foo'12 1not not1";
        let qualified = tokens(source);
        let trees = qualified.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [
                TokenTree::Ident(qualified),
                TokenTree::Literal(one),
                TokenTree::Ident(not),
                TokenTree::Ident(not_one),
            ] if qualified.text() == "Core$foo'12"
                && one.text() == "1"
                && not.text() == "not"
                && not_one.text() == "not1"
        ));

        let source = "_name";
        let underscored = tokens(source);
        let trees = underscored.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [underscore, TokenTree::Ident(name)]
                if underscore.spelling() == Some("_") && name.text() == "name"
        ));

        let source = "éλ🐻";
        let unicode = tokens(source);
        assert!(matches!(
            unicode.iter().next(),
            Some(TokenTree::Ident(identifier)) if identifier.text() == "éλ🐻"
        ));

        let source = "α⊠β";
        let operated = tokens(source);
        let trees = operated.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [
                TokenTree::Ident(alpha),
                operator,
                TokenTree::Ident(beta),
            ] if alpha.text() == "α"
                && operator.spelling() == Some("⊠")
                && beta.text() == "β"
        ));
    }

    #[test]
    fn qualified_identifier_metadata_uses_subspans_without_splitting_the_atom() {
        let spelling = "Package$name$12";
        let stream = tokens(spelling);
        assert!(matches!(
            stream.iter().collect::<Vec<_>>().as_slice(),
            [TokenTree::Ident(_)]
        ));

        let symbol = crate::Symbol::new(spelling, span_at(100, spelling));
        assert_eq!(
            symbol.package_qualifier.as_ref().and_then(Span::bounds),
            Some((100, 107))
        );
        assert_eq!(
            symbol.index_qualifier.as_ref().and_then(Span::bounds),
            Some((113, 115))
        );
        assert!(matches!(
            crate::Lex::to_token_stream(&symbol)
                .iter()
                .collect::<Vec<_>>()
                .as_slice(),
            [TokenTree::Ident(identifier)] if identifier.text() == spelling
        ));

        let package = crate::Symbol::new("Package$name", span_at(20, "Package$name"));
        assert_eq!(
            package.package_qualifier.as_ref().and_then(Span::bounds),
            Some((20, 27))
        );
        assert_eq!(package.index_qualifier, None);

        let indexed = crate::Symbol::new("name$12", span_at(30, "name$12"));
        assert_eq!(indexed.package_qualifier, None);
        assert_eq!(
            indexed.index_qualifier.as_ref().and_then(Span::bounds),
            Some((35, 37))
        );

        for unqualified in ["name", "A$B$C", "A$$B", "A$", "A$1B", "Core$Core$not"] {
            let symbol = crate::Symbol::new(unqualified, Span::for_text(unqualified));
            assert_eq!(symbol.package_qualifier, None, "`{unqualified}`");
            assert_eq!(symbol.index_qualifier, None, "`{unqualified}`");
        }
    }

    #[test]
    fn radix_literals_stop_before_the_first_invalid_digit() {
        let source = "0b010201";
        let binary = tokens(source);
        let trees = binary.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [
                TokenTree::Literal(binary),
                TokenTree::Literal(decimal),
            ]
                if binary.text() == "0b010" && decimal.text() == "201"
        ));

        let source = "0o70789";
        let octal = tokens(source);
        let trees = octal.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [
                TokenTree::Literal(octal),
                TokenTree::Literal(decimal),
            ]
                if octal.text() == "0o707" && decimal.text() == "89"
        ));

        let source = "0b1E2";
        let binary_then_identifier = tokens(source);
        let trees = binary_then_identifier.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [
                TokenTree::Literal(binary),
                TokenTree::Ident(identifier),
            ]
                if binary.text() == "0b1" && identifier.text() == "E2"
        ));
    }

    #[test]
    fn spans_preserve_physical_separation_between_punctuation_tokens() {
        let source = "++x +\n+";
        let stream = tokens(source);
        let trees = stream.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [
                increment,
                TokenTree::Ident(identifier),
                first_plus,
                second_plus,
            ] if increment.spelling() == Some("++")
                && identifier.text() == "x"
                && first_plus.spelling() == Some("+")
                && second_plus.spelling() == Some("+")
        ));
        let first_plus = trees[2].expect_span();
        let second_plus = trees[3].expect_span();
        assert_eq!(
            &first_plus.source().text()[first_plus.end().unwrap()..second_plus.start().unwrap()],
            "\n"
        );
    }

    #[test]
    fn strings_and_raw_strings_are_literal_token_trees() {
        for (source, kind) in [
            (r#""a\n\u0101""#, LiteralKind::String),
            ("///left////right///", LiteralKind::RawString),
            ("///left//right/single///", LiteralKind::RawString),
        ] {
            let stream = tokens(source);
            assert!(matches!(
                stream.iter().next(),
                Some(TokenTree::Literal(literal))
                    if literal.kind == kind && literal.text() == source
            ));
        }
    }

    #[test]
    fn structural_delimiters_form_nested_groups_with_exact_spans() {
        let source = "(1, [2, <|3|>])";
        let stream = tokens(source);
        let mut trees = stream.into_iter();
        let Some(TokenTree::Group(parenthesized)) = trees.next() else {
            panic!("the source should be one parenthesized group");
        };
        assert!(trees.next().is_none());
        assert_eq!(parenthesized.delim_kind(), DelimiterKind::Parenthesis);

        let span = parenthesized.expect_span();
        assert_eq!(span.start().unwrap(), 0);
        assert_eq!(span.end().unwrap(), 15);
        assert_eq!(parenthesized.delimiter().opening_span().end().unwrap(), 1);
        assert_eq!(
            parenthesized
                .delimiter()
                .closing_span()
                .unwrap()
                .start()
                .unwrap(),
            14
        );

        let bracketed = parenthesized
            .stream()
            .iter()
            .find_map(|tree| match tree {
                TokenTree::Group(group) => Some(group),
                _ => None,
            })
            .unwrap();
        assert_eq!(bracketed.delim_kind(), DelimiterKind::Bracket);
        assert_eq!(bracketed.delimiter().opening_span().start().unwrap(), 4);
        assert_eq!(
            bracketed
                .delimiter()
                .closing_span()
                .unwrap()
                .start()
                .unwrap(),
            13
        );

        let angle_bar = bracketed
            .stream()
            .iter()
            .find_map(|tree| match tree {
                TokenTree::Group(group) => Some(group),
                _ => None,
            })
            .unwrap();
        assert_eq!(angle_bar.delim_kind(), DelimiterKind::AngleBar);
        let opening = angle_bar.delimiter().opening_span();
        let closing = angle_bar.delimiter().closing_span().unwrap();
        assert_eq!(opening.start().unwrap(), 8);
        assert_eq!(opening.end().unwrap(), 10);
        assert_eq!(closing.start().unwrap(), 11);
        assert_eq!(closing.end().unwrap(), 13);
    }

    #[test]
    fn graded_operator_wins_before_parenthesis_grouping() {
        let source = "(*)";
        let graded = tokens(source);
        let trees = graded.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [graded] if graded.spelling() == Some("(*)")
        ));

        let grouped = tokens("(*x)");
        assert!(matches!(grouped.iter().next(), Some(TokenTree::Group(_))));
    }

    #[test]
    fn malformed_source_returns_lex_errors() {
        for (source, kind) in [
            ("\"\\q\"", LexErrorKind::InvalidEscape),
            ("(1", LexErrorKind::UnterminatedGroup),
            (")", LexErrorKind::UnexpectedClosingDelimiter),
            ("(]", LexErrorKind::UnexpectedClosingDelimiter),
            ("///", LexErrorKind::UnterminatedRawString),
            ("-* unterminated", LexErrorKind::UnterminatedBlockComment),
        ] {
            assert_eq!(lex_str(source).unwrap_err().kind(), kind);
        }

        let nested = "(".repeat(MAX_GROUP_DEPTH + 1);
        assert_eq!(
            lex_str(&nested).unwrap_err().kind(),
            LexErrorKind::NestingLimitExceeded
        );
    }

    #[test]
    fn recovering_lexer_retains_unclosed_string_literals() {
        for (source, kind, expected) in [
            ("\"", LexErrorKind::UnterminatedString, "\""),
            ("\"unfinished", LexErrorKind::UnterminatedString, "\""),
            ("///", LexErrorKind::UnterminatedRawString, "///"),
            ("///unfinished", LexErrorKind::UnterminatedRawString, "///"),
        ] {
            let recovered = lex_str_recovering(source).unwrap();

            assert_eq!(recovered.value.to_string(), source);
            assert_eq!(recovered.diagnostics.len(), 1);
            assert_eq!(recovered.diagnostics[0].kind(), kind);
            assert_eq!(recovered.diagnostics[0].expected(), Some(expected));
            assert_eq!(recovered.diagnostics[0].found(), Some("end of input"));
        }
    }

    #[test]
    fn a_crlf_line_break_counts_as_the_two_bytes_it_is() {
        let source = "x\r\ny";
        let stream = tokens(source);
        let range = stream
            .iter()
            .find_map(|tree| match tree {
                TokenTree::Ident(identifier) if identifier.text() == "y" => {
                    Some(identifier.expect_span())
                }
                _ => None,
            })
            .unwrap();
        // `x` `\r` `\n` `y`, so `y` occupies byte 3 alone.
        assert_eq!(range.bounds(), Some((3, 4)));

        let error = lex_str("-* x\r\n").unwrap_err();
        assert_eq!(error.expect_span().end(), Some("-* x\r\n".len()));
    }

    #[test]
    fn identifier_classes_match_m2_byte_ctype() {
        for byte in [b'a', b'Z', 0x80, 0xff] {
            assert!(is_identifier_start(byte));
            assert!(is_identifier_continue(byte));
        }
        for byte in *b"09$'" {
            assert!(!is_identifier_start(byte));
            assert!(is_identifier_continue(byte));
        }
        for byte in *b"_ +" {
            assert!(!is_identifier_start(byte));
            assert!(!is_identifier_continue(byte));
        }
    }

    #[test]
    fn math_operator_prefixes_match_m2_ranges() {
        for pair in [
            (0xc2, 0xa0),
            (0xc2, 0xbf),
            (0xc3, 0x97),
            (0xc3, 0xb7),
            (0xe2, 0x86),
            (0xe2, 0x87),
            (0xe2, 0x88),
            (0xe2, 0x8f),
            (0xe2, 0x9f),
            (0xe2, 0xa4),
            (0xe2, 0xa7),
            (0xe2, 0xa8),
            (0xe2, 0xaf),
        ] {
            assert!(is_math_operator_pair(pair.0, pair.1));
        }
        for pair in [
            (0xc2, 0x9f),
            (0xc3, 0x96),
            (0xc3, 0x98),
            (0xe2, 0x85),
            (0xe2, 0x90),
            (0xe2, 0x9e),
            (0xe2, 0xa3),
            (0xe2, 0xb0),
        ] {
            assert!(!is_math_operator_pair(pair.0, pair.1));
        }
    }

    #[test]
    fn utf8_width_rejects_continuations_and_invalid_leads() {
        assert_eq!(utf8_char_width(b'a'), Some(1));
        assert_eq!(utf8_char_width(0xc2), Some(2));
        assert_eq!(utf8_char_width(0xe2), Some(3));
        assert_eq!(utf8_char_width(0xf0), Some(4));
        assert_eq!(utf8_char_width(0x80), None);
        assert_eq!(utf8_char_width(0xc0), None);
        assert_eq!(utf8_char_width(0xf5), None);
    }

    #[test]
    fn ordinary_strings_accept_exact_m2_escape_widths() {
        for source in [r#""\u0101""#, r#""\x4f""#, r#""\1\12\123""#, r#""\n\"\\""#] {
            let stream = tokens(source);
            assert!(matches!(
                stream.iter().next(),
                Some(TokenTree::Literal(literal))
                    if literal.kind == LiteralKind::String && literal.text() == source
            ));
        }
        for source in [r#""\u101""#, r#""\u010g""#, r#""\x4""#] {
            assert_eq!(
                lex_str(source).unwrap_err().kind(),
                LexErrorKind::InvalidEscape
            );
        }
    }
}
