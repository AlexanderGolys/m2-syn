use std::borrow::Cow;
use std::convert::Infallible;
use std::vec::IntoIter;

use m2_syn::{
    CSTNodeClassLabel, CellStream, Expr, ExternalCstChild, ExternalCstNode, NativeParser, Parse,
    ParseStream, Parser, SourceFile, SourceId, Span, Spanned, Symbol, ToTokens, Token,
    TreeSitterParser, lex_str, parse_quote_m2, parse_with, quote_m2, reconstruct,
};

struct EmptyFileParser;

impl Parser for EmptyFileParser {
    type Error = Infallible;

    fn parse_cells(&mut self, _tokens: CellStream) -> Result<SourceFile, Self::Error> {
        Ok(SourceFile::new(Vec::new()))
    }
}

struct SymbolParser;

impl Parser<Symbol> for SymbolParser {
    type Error = Infallible;

    fn parse_cells(&mut self, tokens: CellStream) -> Result<Symbol, Self::Error> {
        Ok(Symbol::new(tokens.to_string(), Span::detached()))
    }
}

#[test]
fn external_parsers_can_produce_files_or_more_specific_targets() {
    let file = parse_with(&mut EmptyFileParser, lex_str("", SourceId(1)).unwrap()).unwrap();
    let symbol: Symbol =
        parse_with(&mut SymbolParser, lex_str("example", SourceId(2)).unwrap()).unwrap();

    assert!(file.elements.is_empty());
    assert_eq!(symbol.text, "example");
}

#[test]
fn both_built_in_parsers_use_the_same_generic_entry_point() {
    let tokens = lex_str("left + right", SourceId(3)).unwrap();
    let tree_sitter_file: SourceFile =
        parse_with(&mut TreeSitterParser::new().unwrap(), tokens.clone()).unwrap();
    let native_file: SourceFile = parse_with(&mut NativeParser::new(), tokens).unwrap();

    assert_eq!(tree_sitter_file.elements.len(), 1);
    assert_eq!(native_file.elements.len(), 1);
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

    assert_eq!(expression.to_code(), "left + right");
    assert_eq!(file.to_code(), "left;\nright");
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

    assert_eq!(symbol.text, "from_cst");
}
