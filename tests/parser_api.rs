use std::borrow::Cow;
use std::convert::Infallible;
use std::vec::IntoIter;

use m2_syn::{
    CSTNodeClassLabel, Eof, Expr, ExternalCstChild, ExternalCstNode, Lex, NativeParser, Parse,
    ParseStream, Parser, SourceFile, Span, Spanned, Symbol, Token, TokenStream, TreeSitterParser,
    lex_str, parse_quote_m2, parse_with, quote_m2, reconstruct,
};

struct EmptyFileParser;

impl Parser for EmptyFileParser {
    type Error = Infallible;

    fn parse_stream(&mut self, _tokens: TokenStream) -> Result<SourceFile, Self::Error> {
        Ok(SourceFile::new(Vec::new(), Eof::new("", Span::detached())))
    }
}

struct SymbolParser;

impl Parser<Symbol> for SymbolParser {
    type Error = Infallible;

    fn parse_stream(&mut self, tokens: TokenStream) -> Result<Symbol, Self::Error> {
        Ok(Symbol::new(tokens.to_string(), Span::detached()))
    }
}

#[test]
fn external_parsers_can_produce_files_or_more_specific_targets() {
    let file = parse_with(&mut EmptyFileParser, lex_str("").unwrap()).unwrap();
    let symbol: Symbol = parse_with(&mut SymbolParser, lex_str("example").unwrap()).unwrap();

    assert!(file.cells.is_empty());
    assert_eq!(symbol.text(), "example");
}

#[test]
fn both_built_in_parsers_use_the_same_generic_entry_point() {
    let tokens = lex_str("left + right").unwrap();
    let tree_sitter_file: SourceFile =
        parse_with(&mut TreeSitterParser::new().unwrap(), tokens.clone()).unwrap();
    let native_file: SourceFile = parse_with(&mut NativeParser, tokens).unwrap();

    assert_eq!(tree_sitter_file.cells.len(), 1);
    assert_eq!(native_file.cells.len(), 1);
}

#[test]
fn token_parse_stream_supports_speculative_parsing() {
    let mut input = ParseStream::new(quote_m2!(+ *));
    let mut fork = input.fork();
    let _: Token![+] = Parse::parse(&mut fork).unwrap();

    input.advance_to(&fork);
    let _: Token![*] = Parse::parse(&mut input).unwrap();
    assert!(input.is_eof());
}

#[test]
fn generated_expression_and_source_roots_parse_quoted_tokens() {
    let expression: Expr = parse_quote_m2!(left + right);
    let file: SourceFile = parse_quote_m2!(left; right);

    assert_eq!(expression.to_m2(), "left + right");
    assert_eq!(file.to_m2(), "left;\nright");
}

#[test]
fn parsing_a_source_file_from_tokens_preserves_their_spans() {
    let file: SourceFile = m2_syn::parse2(quote_m2!(left)).unwrap();

    assert_eq!(file.span().bounds(), Some((0, 4)));
    assert_eq!(file.span().text(), Some("left"));
}

#[derive(Clone)]
struct SymbolCst;

impl ExternalCstNode for SymbolCst {
    type Children<'syntax> = IntoIter<ExternalCstChild<Self>>;

    fn identity(&self) -> CSTNodeClassLabel<'_> {
        CSTNodeClassLabel::new("symbol", true)
    }

    fn children(&self) -> Self::Children<'_> {
        Vec::new().into_iter()
    }

    fn text(&self) -> Cow<'_, str> {
        Cow::Borrowed("from_cst")
    }
}

impl Spanned for SymbolCst {
    fn span(&self) -> Span {
        Span::detached()
    }
}

#[test]
fn parser_specific_csts_can_use_generated_reconstruction() {
    let symbol: Symbol = reconstruct(SymbolCst).unwrap();

    assert_eq!(symbol.text(), "from_cst");
}
