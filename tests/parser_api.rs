use std::borrow::Cow;
use std::convert::Infallible;

use m2_syn::{
    CellStream, CstChild, CstNode, NativeParser, NodeIdentity, Parse, SourceFile, SourceId, Span,
    Symbol, TreeSitterParser, lex_str, parse_with, reconstruct,
};

struct EmptyFileParser;

impl Parse for EmptyFileParser {
    type Error = Infallible;

    fn parse(&mut self, _tokens: CellStream) -> Result<SourceFile, Self::Error> {
        Ok(SourceFile::new(Vec::new()))
    }
}

struct SymbolParser;

impl Parse<Symbol> for SymbolParser {
    type Error = Infallible;

    fn parse(&mut self, tokens: CellStream) -> Result<Symbol, Self::Error> {
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
