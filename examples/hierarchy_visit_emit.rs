use m2_syn::visit::Visit;
use m2_syn::{
    AnyCell, Cell, ParseError, SourceId, Symbol, SyntaxKind, ToTokens, parse_tokens, quote_m2,
};

fn require_cell<T: Cell>(cell: &T) -> SyntaxKind {
    cell.kind()
}

#[derive(Default)]
struct SymbolCollector {
    symbols: Vec<String>,
}

impl<'ast> Visit<'ast> for SymbolCollector {
    fn visit_symbol(&mut self, symbol: &'ast Symbol) {
        self.symbols.push(symbol.text.clone());
    }
}

fn main() -> Result<(), ParseError> {
    let quoted = quote_m2! {
        left + right
    };
    let source_file = parse_tokens(&quoted, SourceId(1))?;
    let AnyCell::ExpressionCell(cell) = &source_file.elements[0] else {
        panic!("quoted expression did not reconstruct as an ExpressionCell");
    };
    assert_eq!(require_cell(cell), SyntaxKind::ExpressionCell);

    let mut collector = SymbolCollector::default();
    collector.visit_source_file(&source_file);

    println!("hierarchy: Cell -> ExpressionCell -> AnyCell -> SourceFile");
    println!("visited: {}", collector.symbols.join(", "));
    print!("generated: {}", source_file.to_m2());
    Ok(())
}
