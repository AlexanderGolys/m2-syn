use m2_syn::nodes::Symbol;
use m2_syn::visit::Visit;
use m2_syn::{Lex, ParseError, parse_tokens, quote_m2};

#[derive(Default)]
struct SymbolCollector {
    symbols: Vec<String>,
}

impl<'ast> Visit<'ast> for SymbolCollector {
    fn visit_symbol(&mut self, symbol: &'ast Symbol) {
        self.symbols.push(symbol.text().to_owned());
    }
}

fn main() -> Result<(), ParseError> {
    let quoted = quote_m2! {
        left + right 2
    };
    let source_file = parse_tokens(&quoted)?;
    let _cell = &source_file.cells[0];

    let mut collector = SymbolCollector::default();
    collector.visit_source_file(&source_file);

    println!("hierarchy: Component -> ExprPack -> Cell -> SourceFile");
    println!("visited: {}", collector.symbols.join(", "));
    print!("generated: {}", source_file.to_m2());
    Ok(())
}
