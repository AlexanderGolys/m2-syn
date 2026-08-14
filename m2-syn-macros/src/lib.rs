extern crate proc_macro;

mod quote_m2;
mod syntax;
mod tokens;

use proc_macro::TokenStream;

#[proc_macro]
pub fn syntax(input: TokenStream) -> TokenStream {
    match syntax::generate(input.into()) {
        Ok(expansion) => expansion.combined().into(),
        Err(error) => error.into_compile_error().into(),
    }
}

#[proc_macro]
pub fn quote_m2(input: TokenStream) -> TokenStream {
    quote_m2::expand(input)
}
