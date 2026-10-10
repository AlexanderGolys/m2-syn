//! Integration tests for parser api.

use std::borrow::Cow;
use std::convert::Infallible;
use std::vec::IntoIter;

use m2_syn::nodes::{BOF, EOF, Expr, SourceFile, Symbol};
use m2_syn::{
    CSTNodeClassLabel, CellStream, ExternalCstChild, ExternalCstNode, Lex, NativeParser, Parse,
    ParseStream, Parser, Span, Spanned, Token, Tokenize, TreeSitterParser, lex_str, migrate,
    parse_quote_m2, parse_with, quote_m2,
};

struct EmptyFileParser;

impl Parser for EmptyFileParser {
    type Error = Infallible;

    fn parse_cells(&mut self, _cells: CellStream) -> Result<SourceFile, Self::Error> {
        Ok(SourceFile::new(
            BOF::new("", Span::for_text("")),
            Vec::new(),
            EOF::new("", Span::for_text("")),
        ))
    }
}

struct SymbolParser;

impl Parser<Symbol> for SymbolParser {
    type Error = Infallible;

    fn parse_cells(&mut self, cells: CellStream) -> Result<Symbol, Self::Error> {
        Ok(Symbol::new(cells.to_string(), Span::for_text("")))
    }
}

#[test]
fn external_parsers_can_produce_files_or_more_specific_targets() {
    let file = parse_with(
        &mut EmptyFileParser,
        lex_str("").unwrap().to_cell_stream().unwrap(),
    )
    .unwrap();
    let symbol: Symbol = parse_with(
        &mut SymbolParser,
        lex_str("example").unwrap().to_cell_stream().unwrap(),
    )
    .unwrap();

    assert!(file.cells.is_empty());
    assert_eq!(symbol.text(), "example");
}

#[test]
fn both_built_in_parsers_use_the_same_generic_entry_point() {
    let tokens = lex_str("left + right").unwrap();
    let cells = tokens.to_cell_stream().unwrap();
    let tree_sitter_file: SourceFile =
        parse_with(&mut TreeSitterParser::new().unwrap(), cells.clone()).unwrap();
    let native_file: SourceFile = parse_with(&mut NativeParser, cells).unwrap();

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

    assert_eq!(expression.to_m2(), "left+right");
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
        Span::for_text("from_cst")
    }
}

#[test]
fn parser_specific_csts_can_use_generated_migration() {
    let symbol: Symbol = migrate(SymbolCst).unwrap();

    assert_eq!(symbol.text(), "from_cst");
}
