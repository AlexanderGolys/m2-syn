use std::borrow::Cow;
use std::convert::Infallible;

use m2_syn::{
    CstChild, CstNode, NodeIdentity, ParseInput, Parser, SourceFile, SourceId, Span, Symbol,
    TreeSitterParser, parse_with, reconstruct,
};

struct EmptyFileParser;

impl Parser for EmptyFileParser {
    type Error = Infallible;

    fn parse(&mut self, _input: ParseInput<'_>) -> Result<SourceFile, Self::Error> {
        Ok(SourceFile::new(Vec::new()))
    }
}

struct SymbolParser;

impl Parser<Symbol> for SymbolParser {
    type Error = Infallible;

    fn parse(&mut self, input: ParseInput<'_>) -> Result<Symbol, Self::Error> {
        Ok(Symbol::new(input.source, Span::detached()))
    }
}

#[test]
fn external_parsers_can_produce_files_or_more_specific_targets() {
    let file = parse_with(&mut EmptyFileParser, "", SourceId(1)).unwrap();
    let symbol: Symbol = parse_with(&mut SymbolParser, "example", SourceId(2)).unwrap();

    assert!(file.elements.is_empty());
    assert_eq!(symbol.text, "example");
}

#[test]
fn built_in_parser_uses_the_same_generic_entry_point() {
    let mut parser = TreeSitterParser::new().unwrap();
    let file: SourceFile = parse_with(&mut parser, "left + right", SourceId(3)).unwrap();

    assert_eq!(file.elements.len(), 1);
}

#[derive(Clone)]
struct SymbolCst;

impl CstNode for SymbolCst {
    type Children<'syntax> = std::vec::IntoIter<CstChild<Self>>;

    fn identity(&self) -> NodeIdentity<'_> {
        NodeIdentity::new("symbol", true)
    }

    fn children(&self) -> Self::Children<'_> {
        Vec::new().into_iter()
    }

    fn text(&self) -> Cow<'_, str> {
        Cow::Borrowed("from_cst")
    }

    fn span(&self) -> Span {
        Span::detached()
    }
}

#[test]
fn parser_specific_csts_can_use_generated_reconstruction() {
    let symbol: Symbol = reconstruct(SymbolCst).unwrap();

    assert_eq!(symbol.text, "from_cst");
}
