//! Native source-to-token conversion.
//!
//! This module owns byte classification, maximal munch, delimiter balancing,
//! and top-level cell splitting. Its output uses only the shared raw atoms from
//! `token_stream`; it never constructs a second lexer-specific token model.
//! The private byte cursor provides arbitrary finite lookahead while pulling
//! the underlying iterator lazily.

use std::collections::VecDeque;
use std::error::Error;
use std::fmt::{Display, Formatter, Result as FmtResult};

use crate::nodes::{
    GENERATED_KEYWORD_SPELLINGS, GENERATED_OPERATOR_SPELLINGS,
    GENERATED_POSTFIX_OPERATOR_SPELLINGS, GENERATED_PUNCTUATION_SPELLINGS,
};
use crate::token_stream::delim::Delimiter;
use crate::{
    CellBlock, CellStream, DelimiterKind, Group, IdentToken, Literal, LiteralKind, Punct, SourceId,
    Span, Spanned, TextPoint, TextRange, TokenStream, TokenTree, Trivia, TriviaKind,
};

const MAX_GROUP_DEPTH: usize = 256;

/// Collapses an explicitly `Core$`-qualified identifier back to its bare
/// spelling when that spelling is one of the generated keywords, matching
/// M2's `Core$if` / `if` equivalence.
pub(crate) fn canonical_keyword_spelling(spelling: &str) -> &str {
    let Some(keyword) = spelling.strip_prefix("Core$") else {
        return spelling;
    };
    if GENERATED_KEYWORD_SPELLINGS.contains(&keyword) {
        keyword
    } else {
        spelling
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LexErrorKind {
    InvalidCharacter,
    InvalidEscape,
    InvalidNumber,
    InvalidUtf8,
    NestingLimitExceeded,
    UnexpectedClosingDelimiter,
    UnterminatedGroup,
    UnterminatedString,
    UnterminatedRawString,
    UnterminatedBlockComment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Spanned)]
pub struct LexError {
    kind: LexErrorKind,
    span: Span,
}

impl LexError {
    pub fn kind(self) -> LexErrorKind {
        self.kind
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
            LexErrorKind::UnterminatedGroup => "unterminated delimited group",
            LexErrorKind::UnterminatedString => "unterminated string literal",
            LexErrorKind::UnterminatedRawString => "unterminated raw string literal",
            LexErrorKind::UnterminatedBlockComment => "unterminated block comment",
        };
        formatter.write_str(message)
    }
}

impl Error for LexError {}

/// Lexes bytes into source-spanned cell blocks containing token trees.
///
/// Malformed source is always reported as [`LexError`]; byte contents do not
/// cause this function to panic. A user-provided iterator may still panic from
/// its own [`Iterator::next`] implementation.
pub fn lex<I>(bytes: I, source_id: SourceId) -> Result<CellStream, LexError>
where
    I: IntoIterator<Item = u8>,
{
    NativeLexer::new(bytes.into_iter(), source_id).lex()
}

/// Lexes M2 source text into its source-spanned token tree.
pub fn lex_str(source: &str, source_id: SourceId) -> Result<CellStream, LexError> {
    lex(source.bytes(), source_id)
}

/// A lazy cursor with arbitrary finite lookahead over a byte iterator.
///
/// Looking ahead only pulls enough bytes to answer the requested projection;
/// advancing removes the first buffered byte. The lexer therefore never needs
/// to tentatively consume and then push bytes back into its input.
struct ByteCursor<I>
where
    I: Iterator<Item = u8>,
{
    iterator: I,
    lookahead: VecDeque<u8>,
}

impl<I> ByteCursor<I>
where
    I: Iterator<Item = u8>,
{
    fn new(iterator: I) -> Self {
        Self {
            iterator,
            lookahead: VecDeque::new(),
        }
    }

    fn peek(&mut self, distance: usize) -> Option<u8> {
        while self.lookahead.len() <= distance {
            self.lookahead.push_back(self.iterator.next()?);
        }
        self.lookahead.get(distance).copied()
    }
}

impl<I> Iterator for ByteCursor<I>
where
    I: Iterator<Item = u8>,
{
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        self.lookahead.pop_front().or_else(|| self.iterator.next())
    }
}

struct NativeLexer<I: Iterator<Item = u8>> {
    input: ByteCursor<I>,
    source_id: SourceId,
    point: TextPoint,
}

impl<I> NativeLexer<I>
where
    I: Iterator<Item = u8>,
{
    fn new(bytes: I, source_id: SourceId) -> Self {
        Self {
            input: ByteCursor::new(bytes),
            source_id,
            point: TextPoint::new(0, 0, 0),
        }
    }

    fn lex(mut self) -> Result<CellStream, LexError> {
        let mut output = TokenStream::new();
        let mut cells = Vec::new();
        let mut cell_start = self.point;
        let mut groups = Vec::<OpenGroup>::new();
        loop {
            let Some(first) = self.input.peek(0) else {
                return match groups.last() {
                    Some(group) => {
                        Err(self.error(LexErrorKind::UnterminatedGroup, group.opening_start))
                    }
                    None => {
                        if !output.is_empty() {
                            let closing = self.span(self.point, self.point);
                            self.finish_cell(
                                &mut output,
                                &mut cells,
                                &mut cell_start,
                                DelimiterKind::Empty,
                                closing,
                            );
                        }
                        Ok(CellStream::new(cells, self.source_id))
                    }
                };
            };

            if first == b'\n' || (first == b'\r' && self.byte_after_current() == Some(b'\n')) {
                let top_level = groups.is_empty();
                self.lex_line_break(&mut output)?;
                if top_level && newline_ends_cell(&output) {
                    let closing = self.span(self.point, self.point);
                    self.finish_cell(
                        &mut output,
                        &mut cells,
                        &mut cell_start,
                        DelimiterKind::Empty,
                        closing,
                    );
                }
            } else if first == b'\r' {
                self.lex_carriage_return(&mut output)?;
            } else if matches!(first, b' ' | b'\t') {
                self.lex_whitespace(&mut output)?;
            } else if let Some(width) = self.math_operator_width() {
                self.lex_math_operator(width, &mut output)?;
            } else if is_identifier_start(first) {
                self.lex_identifier(&mut output)?;
            } else if first.is_ascii_digit()
                || (first == b'.'
                    && self
                        .byte_after_current()
                        .is_some_and(|byte| byte.is_ascii_digit()))
            {
                self.lex_number(&mut output)?;
            } else if first == b'"' {
                self.lex_string(&mut output)?;
            } else if let Some(punctuation) = self.take_punctuation_match() {
                match punctuation.kind {
                    PunctuationKind::Ordinary => {
                        let ends_cell = groups.is_empty() && punctuation.bytes.as_slice() == b";";
                        if ends_cell {
                            let closing_start = self.point;
                            self.commit_bytes(&punctuation.bytes);
                            let closing = self.span_from(closing_start);
                            self.finish_cell(
                                &mut output,
                                &mut cells,
                                &mut cell_start,
                                DelimiterKind::Semicolon,
                                closing,
                            );
                        } else {
                            self.push_punctuation(punctuation.bytes, &mut output)?;
                        }
                    }
                    PunctuationKind::LineComment => {
                        self.lex_line_comment(punctuation.bytes, &mut output)?;
                    }
                    PunctuationKind::BlockComment => {
                        let top_level = groups.is_empty();
                        let start_line = self.point.line;
                        self.lex_block_comment(punctuation.bytes, &mut output)?;
                        if top_level && self.point.line != start_line && newline_ends_cell(&output)
                        {
                            let closing = self.span(self.point, self.point);
                            self.finish_cell(
                                &mut output,
                                &mut cells,
                                &mut cell_start,
                                DelimiterKind::Empty,
                                closing,
                            );
                        }
                    }
                    PunctuationKind::RawString => {
                        self.lex_raw_string(punctuation.bytes, &mut output)?;
                    }
                    PunctuationKind::Open(kind) => {
                        if groups.len() == MAX_GROUP_DEPTH {
                            return Err(self.error(LexErrorKind::NestingLimitExceeded, self.point));
                        }
                        let opening_start = self.point;
                        self.commit_bytes(&punctuation.bytes);
                        let opening = self.span_from(opening_start);
                        groups.push(OpenGroup {
                            kind,
                            opening_start,
                            opening,
                            parent: std::mem::take(&mut output),
                        });
                    }
                    PunctuationKind::Close(kind) => {
                        let closing_start = self.point;
                        self.commit_bytes(&punctuation.bytes);
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
                        let contents = std::mem::replace(&mut output, group.parent);
                        output.push_group(Group::new(
                            Delimiter::new(kind, group.opening.join(closing)),
                            contents,
                        ));
                    }
                }
            } else {
                let start = self.point;
                self.next();
                return Err(self.error(LexErrorKind::InvalidCharacter, start));
            }
        }
    }

    fn lex_line_break(&mut self, output: &mut TokenStream) -> Result<(), LexError> {
        let start = self.point;
        let first = self.next_or_error(LexErrorKind::InvalidCharacter, start)?;
        let mut bytes = vec![first];
        if first == b'\r' && self.input.peek(0) == Some(b'\n') {
            bytes.push(self.next_or_error(LexErrorKind::InvalidCharacter, start)?);
        }
        let text = self.text(bytes, start)?;
        let span = self.span_from(start);
        output.push(TokenTree::Trivia(Trivia::new(
            TriviaKind::Whitespace,
            text,
            span,
        )));
        Ok(())
    }

    fn lex_carriage_return(&mut self, output: &mut TokenStream) -> Result<(), LexError> {
        let start = self.point;
        let byte = self.next_or_error(LexErrorKind::InvalidCharacter, start)?;
        let text = self.text(vec![byte], start)?;
        output.push(TokenTree::Trivia(Trivia::new(
            TriviaKind::Whitespace,
            text,
            self.span_from(start),
        )));
        Ok(())
    }

    fn lex_whitespace(&mut self, output: &mut TokenStream) -> Result<(), LexError> {
        let start = self.point;
        let bytes = self.take_while(|byte| matches!(byte, b' ' | b'\t'));
        let text = self.text(bytes, start)?;
        output.push(TokenTree::Trivia(Trivia::new(
            TriviaKind::Whitespace,
            text,
            self.span_from(start),
        )));
        Ok(())
    }

    fn lex_identifier(&mut self, output: &mut TokenStream) -> Result<(), LexError> {
        let start = self.point;
        let mut bytes = vec![self.next_or_error(LexErrorKind::InvalidUtf8, start)?];
        loop {
            let Some(byte) = self.input.peek(0) else {
                break;
            };
            if !is_identifier_continue(byte) || self.math_operator_width().is_some() {
                break;
            }
            bytes.push(self.next_or_error(LexErrorKind::InvalidUtf8, start)?);
        }
        let text = self.text(bytes, start)?;
        output.push(TokenTree::Ident(IdentToken::new(
            text,
            self.span_from(start),
        )));
        Ok(())
    }

    fn lex_number(&mut self, output: &mut TokenStream) -> Result<(), LexError> {
        let start = self.point;
        let mut bytes = Vec::new();
        let mut decimal = true;

        if self.input.peek(0) == Some(b'0') {
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

        let next = self.input.peek(0);
        let begins_float = decimal
            && (next == Some(b'.') && self.byte_after_current() != Some(b'.')
                || matches!(next, Some(b'p' | b'e' | b'E')));
        let kind = if begins_float {
            if self.input.peek(0) == Some(b'.') {
                bytes.push(self.next_or_error(LexErrorKind::InvalidNumber, start)?);
                bytes.extend(self.take_while(|byte| byte.is_ascii_digit()));
            }
            if self.input.peek(0) == Some(b'p') {
                bytes.push(self.next_or_error(LexErrorKind::InvalidNumber, start)?);
                if !self.input.peek(0).is_some_and(|byte| byte.is_ascii_digit()) {
                    return Err(self.error(LexErrorKind::InvalidNumber, start));
                }
                bytes.extend(self.take_while(|byte| byte.is_ascii_digit()));
            }
            if matches!(self.input.peek(0), Some(b'e' | b'E')) {
                bytes.push(self.next_or_error(LexErrorKind::InvalidNumber, start)?);
                if matches!(self.input.peek(0), Some(b'+' | b'-')) {
                    bytes.push(self.next_or_error(LexErrorKind::InvalidNumber, start)?);
                }
                if !self.input.peek(0).is_some_and(|byte| byte.is_ascii_digit()) {
                    return Err(self.error(LexErrorKind::InvalidNumber, start));
                }
                bytes.extend(self.take_while(|byte| byte.is_ascii_digit()));
            }
            LiteralKind::Float
        } else {
            LiteralKind::Integer
        };

        let text = self.text(bytes, start)?;
        output.push(TokenTree::Literal(Literal::new(
            kind,
            text,
            self.span_from(start),
        )));
        Ok(())
    }

    fn number_radix(&mut self) -> Option<NumberRadix> {
        let marker = self.input.peek(0)?;
        let digit = self.input.peek(1);
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

    fn lex_string(&mut self, output: &mut TokenStream) -> Result<(), LexError> {
        let start = self.point;
        let mut bytes = vec![self.next_or_error(LexErrorKind::UnterminatedString, start)?];

        loop {
            match self.input.peek(0) {
                None => return Err(self.error(LexErrorKind::UnterminatedString, start)),
                Some(b'"') => {
                    bytes.push(self.next_or_error(LexErrorKind::UnterminatedString, start)?);
                    let text = self.text(bytes, start)?;
                    output.push(TokenTree::Literal(Literal::new(
                        LiteralKind::String,
                        text,
                        self.span_from(start),
                    )));
                    return Ok(());
                }
                Some(b'\\') => {
                    let escape_start = self.point;
                    bytes.push(self.next_or_error(LexErrorKind::UnterminatedString, start)?);
                    let Some(escaped) = self.input.peek(0) else {
                        return Err(self.error(LexErrorKind::UnterminatedString, start));
                    };
                    bytes.push(self.next_or_error(LexErrorKind::UnterminatedString, start)?);
                    match escaped {
                        b'a' | b'b' | b'e' | b'E' | b'f' | b'r' | b't' | b'v' | b'n' | b'"'
                        | b'\\' => {}
                        b'0'..=b'7' => {
                            for _ in 0..2 {
                                if !matches!(self.input.peek(0), Some(b'0'..=b'7')) {
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

    fn take_hex_escape(
        &mut self,
        bytes: &mut Vec<u8>,
        digits: usize,
        start: TextPoint,
    ) -> Result<(), LexError> {
        for _ in 0..digits {
            if !self
                .input
                .peek(0)
                .is_some_and(|byte| byte.is_ascii_hexdigit())
            {
                return Err(self.error(LexErrorKind::InvalidEscape, start));
            }
            bytes.push(self.next_or_error(LexErrorKind::InvalidEscape, start)?);
        }
        Ok(())
    }

    fn lex_raw_string(
        &mut self,
        opener: Vec<u8>,
        output: &mut TokenStream,
    ) -> Result<(), LexError> {
        let start = self.point;
        self.commit_bytes(&opener);
        let mut bytes = opener;

        loop {
            if self.input.peek(0).is_none() {
                return Err(self.error(LexErrorKind::UnterminatedRawString, start));
            }
            if self.input.peek(0) == Some(b'/') {
                match self.slash_run_len() {
                    3 => {
                        bytes.extend(self.take_exact(3, start)?);
                        let text = self.text(bytes, start)?;
                        output.push(TokenTree::Literal(Literal::new(
                            LiteralKind::RawString,
                            text,
                            self.span_from(start),
                        )));
                        return Ok(());
                    }
                    0 => return Err(self.error(LexErrorKind::UnterminatedRawString, start)),
                    1 => {
                        bytes.push(self.next_or_error(LexErrorKind::UnterminatedRawString, start)?)
                    }
                    _ => bytes.extend(self.take_exact(2, start)?),
                }
            } else {
                bytes.push(self.next_or_error(LexErrorKind::UnterminatedRawString, start)?);
            }
        }
    }

    fn lex_line_comment(
        &mut self,
        opener: Vec<u8>,
        output: &mut TokenStream,
    ) -> Result<(), LexError> {
        let start = self.point;
        self.commit_bytes(&opener);
        let mut bytes = opener;
        loop {
            match self.input.peek(0) {
                None | Some(b'\n') => break,
                Some(b'\r') if self.byte_after_current() == Some(b'\n') => break,
                Some(_) => bytes.push(self.next_or_error(LexErrorKind::InvalidCharacter, start)?),
            }
        }
        let text = self.text(bytes, start)?;
        output.push(TokenTree::Trivia(Trivia::new(
            TriviaKind::LineComment,
            text,
            self.span_from(start),
        )));
        Ok(())
    }

    fn lex_block_comment(
        &mut self,
        opener: Vec<u8>,
        output: &mut TokenStream,
    ) -> Result<(), LexError> {
        let start = self.point;
        self.commit_bytes(&opener);
        let mut bytes = opener;
        loop {
            if self.input.peek(0).is_none() {
                return Err(self.error(LexErrorKind::UnterminatedBlockComment, start));
            }
            if self.input.peek(0) == Some(b'*') && self.byte_after_current() == Some(b'-') {
                bytes.extend(self.take_exact(2, start)?);
                let text = self.text(bytes, start)?;
                output.push(TokenTree::Trivia(Trivia::new(
                    TriviaKind::BlockComment,
                    text,
                    self.span_from(start),
                )));
                return Ok(());
            }
            bytes.push(self.next_or_error(LexErrorKind::UnterminatedBlockComment, start)?);
        }
    }

    fn take_punctuation_match(&mut self) -> Option<PunctuationMatch> {
        let mut candidates = Vec::new();
        for (spelling, kind) in [
            ("///", PunctuationKind::RawString),
            ("--", PunctuationKind::LineComment),
            ("-*", PunctuationKind::BlockComment),
            ("(", PunctuationKind::Open(DelimiterKind::Parenthesis)),
            (")", PunctuationKind::Close(DelimiterKind::Parenthesis)),
            ("[", PunctuationKind::Open(DelimiterKind::Bracket)),
            ("]", PunctuationKind::Close(DelimiterKind::Bracket)),
            ("{", PunctuationKind::Open(DelimiterKind::Brace)),
            ("}", PunctuationKind::Close(DelimiterKind::Brace)),
            ("<|", PunctuationKind::Open(DelimiterKind::AngleBar)),
            ("|>", PunctuationKind::Close(DelimiterKind::AngleBar)),
        ] {
            candidates.push((spelling.as_bytes(), kind));
        }
        candidates.extend(
            GENERATED_PUNCTUATION_SPELLINGS
                .iter()
                .map(|spelling| (spelling.as_bytes(), PunctuationKind::Ordinary)),
        );

        let mut accepted = None;
        let mut distance = 0;
        loop {
            let Some(next) = self.input.peek(distance) else {
                break;
            };
            if !candidates
                .iter()
                .any(|(spelling, _)| spelling.get(distance) == Some(&next))
            {
                break;
            }
            distance += 1;
            candidates.retain(|(spelling, _)| spelling.get(distance - 1) == Some(&next));
            for (spelling, kind) in &candidates {
                if spelling.len() == distance {
                    accepted = Some((distance, *kind));
                }
            }
        }

        let (accepted_len, kind) = accepted?;
        let bytes = (0..accepted_len)
            .map(|_| self.input.next().expect("accepted bytes were buffered"))
            .collect();
        Some(PunctuationMatch { bytes, kind })
    }

    fn math_operator_width(&mut self) -> Option<usize> {
        let first = self.input.peek(0)?;
        if !matches!(first, 0xc2 | 0xc3 | 0xe2) {
            return None;
        }
        self.input
            .peek(1)
            .filter(|second| is_math_operator_pair(first, *second))
            .and_then(|_| utf8_char_width(first))
    }

    fn lex_math_operator(
        &mut self,
        width: usize,
        output: &mut TokenStream,
    ) -> Result<(), LexError> {
        let start = self.point;
        let mut bytes = self.pull_exact(width, start)?;
        if self.input.peek(0) == Some(b'=') {
            bytes.push(self.pull_or_error(LexErrorKind::InvalidUtf8, start)?);
        }
        self.push_punctuation(bytes, output)
    }

    fn push_punctuation(
        &mut self,
        bytes: Vec<u8>,
        output: &mut TokenStream,
    ) -> Result<(), LexError> {
        let start = self.point;
        let text = self.text(bytes.clone(), start)?;
        self.commit_bytes(&bytes);
        output.push(TokenTree::Punct(Punct::new(text, self.span_from(start))));
        Ok(())
    }

    fn byte_after_current(&mut self) -> Option<u8> {
        self.input.peek(1)
    }

    fn slash_run_len(&mut self) -> usize {
        let mut len = 0;
        while len < 4 && self.input.peek(len) == Some(b'/') {
            len += 1;
        }
        len
    }

    fn take_while(&mut self, predicate: impl Fn(u8) -> bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        while self.input.peek(0).is_some_and(&predicate) {
            let Some(byte) = self.next() else {
                break;
            };
            bytes.push(byte);
        }
        bytes
    }

    fn take_exact(&mut self, len: usize, start: TextPoint) -> Result<Vec<u8>, LexError> {
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            let Some(byte) = self.next() else {
                return Err(self.error(LexErrorKind::InvalidUtf8, start));
            };
            bytes.push(byte);
        }
        Ok(bytes)
    }

    fn pull_exact(&mut self, len: usize, start: TextPoint) -> Result<Vec<u8>, LexError> {
        let mut bytes = Vec::with_capacity(len);
        for _ in 0..len {
            let Some(byte) = self.input.next() else {
                return Err(self.error(LexErrorKind::InvalidUtf8, start));
            };
            bytes.push(byte);
        }
        Ok(bytes)
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.input.next()?;
        let next = (byte == b'\r').then(|| self.input.peek(0)).flatten();
        self.advance(byte, next);
        Some(byte)
    }

    fn next_or_error(&mut self, kind: LexErrorKind, start: TextPoint) -> Result<u8, LexError> {
        match self.next() {
            Some(byte) => Ok(byte),
            None => Err(self.error(kind, start)),
        }
    }

    fn pull_or_error(&mut self, kind: LexErrorKind, start: TextPoint) -> Result<u8, LexError> {
        match self.input.next() {
            Some(byte) => Ok(byte),
            None => Err(self.error(kind, start)),
        }
    }

    fn commit_bytes(&mut self, bytes: &[u8]) {
        for (index, byte) in bytes.iter().copied().enumerate() {
            let next = (byte == b'\r')
                .then(|| bytes.get(index + 1).copied().or_else(|| self.input.peek(0)))
                .flatten();
            self.advance(byte, next);
        }
    }

    fn advance(&mut self, byte: u8, next: Option<u8>) {
        self.point.byte = self.point.byte.saturating_add(1);
        match byte {
            b'\r' if next == Some(b'\n') => {}
            b'\n' => {
                self.point.line = self.point.line.saturating_add(1);
                self.point.column = 0;
            }
            _ => self.point.column = self.point.column.saturating_add(1),
        }
    }

    fn text(&self, bytes: Vec<u8>, start: TextPoint) -> Result<String, LexError> {
        String::from_utf8(bytes).map_err(|_| self.error(LexErrorKind::InvalidUtf8, start))
    }

    fn error(&self, kind: LexErrorKind, start: TextPoint) -> LexError {
        LexError {
            kind,
            span: self.span_from(start),
        }
    }

    fn span_from(&self, start: TextPoint) -> Span {
        self.span(start, self.point)
    }

    fn span(&self, start: TextPoint, end: TextPoint) -> Span {
        Span::new(self.source_id, TextRange::new(start, end))
    }

    fn finish_cell(
        &self,
        output: &mut TokenStream,
        cells: &mut Vec<CellBlock>,
        cell_start: &mut TextPoint,
        kind: DelimiterKind,
        closing: Span,
    ) {
        let stream = std::mem::take(output);
        let opening = self.span(*cell_start, *cell_start);
        cells.push(CellBlock::new(
            Delimiter::new(kind, opening.join(closing)),
            stream,
        ));
        *cell_start = self.point;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequiredClause {
    Then,
    LoopBody,
    ExceptBody,
}

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
            TokenTree::Trivia(_) | TokenTree::Eof(_) => None,
        };

        let Some(text) = text else {
            continue;
        };
        if quote_next {
            quote_next = false;
            last_text = None;
            continue;
        }
        let text = canonical_keyword_spelling(text);
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
            "do" => {
                if !remove_last_clause(&mut required_clauses, RequiredClause::ExceptBody) {
                    remove_last_clause(&mut required_clauses, RequiredClause::LoopBody);
                }
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

fn remove_last_clause(clauses: &mut Vec<RequiredClause>, clause: RequiredClause) -> bool {
    let Some(position) = clauses.iter().rposition(|candidate| *candidate == clause) else {
        return false;
    };
    clauses.remove(position);
    true
}

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

fn is_quote_specifier(keyword: &str) -> bool {
    matches!(
        keyword,
        "symbol" | "local" | "global" | "threadVariable" | "threadLocal"
    )
}

#[derive(Debug)]
struct OpenGroup {
    kind: DelimiterKind,
    opening_start: TextPoint,
    opening: Span,
    parent: TokenStream,
}

// @##incorrect strings and comments are not punctuation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PunctuationKind {
    Ordinary,
    LineComment,
    BlockComment,
    RawString,
    Open(DelimiterKind),
    Close(DelimiterKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PunctuationMatch {
    bytes: Vec<u8>,
    kind: PunctuationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumberRadix {
    Binary,
    Octal,
    Hexadecimal,
}

impl NumberRadix {
    fn is_digit(self, byte: u8) -> bool {
        match self {
            Self::Binary => matches!(byte, b'0' | b'1'),
            Self::Octal => matches!(byte, b'0'..=b'7'),
            Self::Hexadecimal => byte.is_ascii_hexdigit(),
        }
    }
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte >= 128
}

fn is_identifier_continue(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'$' | b'\'') || byte >= 128
}

// @##unreadable_code
fn is_math_operator_pair(first: u8, second: u8) -> bool {
    let pair = u16::from(first) << 8 | u16::from(second);
    pair & 0xffe0 == 0xc2a0
        || matches!(pair, 0xc397 | 0xc3b7)
        || pair & 0xfffe == 0xe286
        || pair & 0xfff8 == 0xe288
        || pair == 0xe29f
        || pair & 0xfffc == 0xe2a4
        || pair & 0xfff8 == 0xe2a8
}

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
    use std::cell::Cell as CounterCell;

    use super::*;

    fn tokens(source: &str) -> TokenStream {
        let stream = lex_str(source, SourceId(11)).unwrap();
        assert_eq!(format!("{stream}"), source);
        let mut flattened = TokenStream::new();
        for cell in stream {
            flattened.extend(cell.into_stream());
        }
        flattened
    }

    #[test]
    fn byte_cursor_supports_lazy_arbitrary_finite_lookahead() {
        let reads = CounterCell::new(0);
        let iterator = [b'a', b'b', b'c'].into_iter().inspect(|_| {
            reads.set(reads.get() + 1);
        });
        let mut input = ByteCursor::new(iterator);

        assert_eq!(input.peek(0), Some(b'a'));
        assert_eq!(reads.get(), 1);
        assert_eq!(input.peek(2), Some(b'c'));
        assert_eq!(reads.get(), 3);
        assert_eq!(input.next(), Some(b'a'));
        assert_eq!(input.peek(0), Some(b'b'));
        assert_eq!(input.next(), Some(b'b'));
        assert_eq!(input.next(), Some(b'c'));
        assert_eq!(input.next(), None);
    }

    #[test]
    fn lexer_accepts_an_arbitrary_non_peekable_byte_iterator() {
        let bytes = vec![b'a', b'+', b'1'].into_iter();
        let stream = lex(bytes, SourceId(18)).unwrap();
        assert_eq!(format!("{stream}"), "a+1");
    }

    #[test]
    fn lexer_does_not_eagerly_consume_the_byte_iterator() {
        let bytes = std::iter::once(0).chain(std::iter::from_fn(|| {
            panic!("lexer read beyond the byte that already determines the error")
        }));
        assert_eq!(
            lex(bytes, SourceId(19)).unwrap_err().kind(),
            LexErrorKind::InvalidCharacter
        );
    }

    #[test]
    fn every_short_byte_sequence_returns_without_panicking() {
        fn check(bytes: &[u8]) {
            let outcome = std::panic::catch_unwind(|| lex(bytes.iter().copied(), SourceId(21)));
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
            lex([0xf0, 0x80, 0x80], SourceId(22)).unwrap_err().kind(),
            LexErrorKind::InvalidUtf8
        );
    }

    #[test]
    fn punctuation_is_returned_as_maximal_tokens() {
        let stream = tokens("<<|-1||>2");
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
            ordinary
                .iter()
                .all(|tree| !matches!(tree, TokenTree::Trivia(_)))
        );

        let comments = tokens("-- comment\n-* block *-");
        let trees = comments.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [
                TokenTree::Trivia(line),
                TokenTree::Trivia(newline),
                TokenTree::Trivia(block),
            ] if line.kind() == TriviaKind::LineComment
                && newline.kind() == TriviaKind::Whitespace
                && newline.contains_line_break()
                && block.kind() == TriviaKind::BlockComment
        ));
    }

    #[test]
    fn leading_operator_runs_split_at_maximal_known_spellings() {
        let stream = tokens("***1");
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
        let qualified = tokens("Core$foo'12 1not not1");
        let trees = qualified.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [
                TokenTree::Ident(qualified),
                TokenTree::Trivia(first_space),
                TokenTree::Literal(one),
                TokenTree::Ident(not),
                TokenTree::Trivia(second_space),
                TokenTree::Ident(not_one),
            ] if qualified.text() == "Core$foo'12"
                && first_space.kind() == TriviaKind::Whitespace
                && one.text() == "1"
                && not.text() == "not"
                && second_space.kind() == TriviaKind::Whitespace
                && not_one.text() == "not1"
        ));

        let underscored = tokens("_name");
        let trees = underscored.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [underscore, TokenTree::Ident(name)]
                if underscore.spelling() == Some("_") && name.text() == "name"
        ));

        let unicode = tokens("éλ🐻");
        assert!(matches!(
            unicode.iter().next(),
            Some(TokenTree::Ident(identifier)) if identifier.text() == "éλ🐻"
        ));

        let operated = tokens("α⊠β");
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
    fn radix_literals_stop_before_the_first_invalid_digit() {
        let binary = tokens("0b010201");
        let trees = binary.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [
                TokenTree::Literal(binary),
                TokenTree::Literal(decimal),
            ]
                if binary.text() == "0b010" && decimal.text() == "201"
        ));

        let octal = tokens("0o70789");
        let trees = octal.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [
                TokenTree::Literal(octal),
                TokenTree::Literal(decimal),
            ]
                if octal.text() == "0o707" && decimal.text() == "89"
        ));

        let binary_then_identifier = tokens("0b1E2");
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
    fn trivia_preserves_physical_separation_between_punctuation_tokens() {
        let stream = tokens("++x +\n+");
        let trees = stream.iter().collect::<Vec<_>>();
        assert!(matches!(
            trees.as_slice(),
            [
                increment,
                TokenTree::Ident(identifier),
                TokenTree::Trivia(space),
                first_plus,
                TokenTree::Trivia(newline),
                second_plus,
            ] if increment.spelling() == Some("++")
                && identifier.text() == "x"
                && space.kind() == TriviaKind::Whitespace
                && first_plus.spelling() == Some("+")
                && newline.kind() == TriviaKind::Whitespace
                && newline.contains_line_break()
                && second_plus.spelling() == Some("+")
        ));
    }

    #[test]
    fn cells_use_operator_clause_and_delimiter_state() {
        fn top_level_cell_count(source: &str) -> usize {
            lex_str(source, SourceId(11)).unwrap().iter().count()
        }

        assert_eq!(top_level_cell_count("1\n2"), 2);
        assert_eq!(top_level_cell_count("1 +\n2"), 1);
        assert_eq!(top_level_cell_count("not\n1"), 1);
        assert_eq!(top_level_cell_count("1!\n2"), 2);
        assert_eq!(top_level_cell_count("if condition\nthen value"), 1);
        assert_eq!(top_level_cell_count("if condition\nthen value\nnext"), 2);
        assert_eq!(top_level_cell_count("symbol if\nnext"), 2);
        assert_eq!(top_level_cell_count("symbol +\nnext"), 2);
        assert_eq!(top_level_cell_count("1 -* comment\n*- 2"), 2);
        assert_eq!(top_level_cell_count("1 + -* comment\n*- 2"), 1);
        assert_eq!(top_level_cell_count("1;2"), 2);
        assert_eq!(top_level_cell_count("(1\n2;3)"), 1);
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

        let span = parenthesized.span();
        assert_eq!(span.start_point().unwrap().byte, 0);
        assert_eq!(span.end_point().unwrap().byte, 15);
        assert_eq!(
            parenthesized
                .delimiter()
                .opening_span()
                .end_point()
                .unwrap()
                .byte,
            1
        );
        assert_eq!(
            parenthesized
                .delimiter()
                .closing_span()
                .start_point()
                .unwrap()
                .byte,
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
        assert_eq!(
            bracketed
                .delimiter()
                .opening_span()
                .start_point()
                .unwrap()
                .byte,
            4
        );
        assert_eq!(
            bracketed
                .delimiter()
                .closing_span()
                .start_point()
                .unwrap()
                .byte,
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
        let closing = angle_bar.delimiter().closing_span();
        assert_eq!(opening.start_point().unwrap().byte, 8);
        assert_eq!(opening.end_point().unwrap().byte, 10);
        assert_eq!(closing.start_point().unwrap().byte, 11);
        assert_eq!(closing.end_point().unwrap().byte, 13);
    }

    #[test]
    fn graded_operator_wins_before_parenthesis_grouping() {
        let graded = tokens("(*)");
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
            assert_eq!(lex_str(source, SourceId(11)).unwrap_err().kind(), kind);
        }

        let nested = "(".repeat(MAX_GROUP_DEPTH + 1);
        assert_eq!(
            lex_str(&nested, SourceId(11)).unwrap_err().kind(),
            LexErrorKind::NestingLimitExceeded
        );
    }

    #[test]
    fn spans_track_physical_lines() {
        let stream = tokens("x\r\ny");
        let range = stream
            .iter()
            .find_map(|tree| match tree {
                TokenTree::Ident(identifier) if identifier.text() == "y" => {
                    identifier.span().range().ok()
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(range.start(), Some(TextPoint::new(1, 0, 3)));
        assert_eq!(range.end(), Some(TextPoint::new(1, 1, 4)));

        let error = lex_str("-* x\r\n", SourceId(11)).unwrap_err();
        assert_eq!(error.span().end_point().unwrap(), TextPoint::new(1, 0, 6));
    }

    #[test]
    fn identifier_classes_match_m2_byte_ctype() {
        for byte in [b'a', b'Z', 0x80, 0xff] {
            assert!(is_identifier_start(byte));
            assert!(is_identifier_continue(byte));
        }
        for byte in [b'0', b'9', b'$', b'\''] {
            assert!(!is_identifier_start(byte));
            assert!(is_identifier_continue(byte));
        }
        for byte in [b'_', b' ', b'+'] {
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
                lex_str(source, SourceId(1)).unwrap_err().kind(),
                LexErrorKind::InvalidEscape
            );
        }
    }
}
