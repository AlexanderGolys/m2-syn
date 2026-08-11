extern crate proc_macro;

mod quote_m2;
mod syntax;
mod utils;

use proc_macro::TokenStream;

#[proc_macro]
pub fn syntax(input: TokenStream) -> TokenStream {
    syntax::expand(input)
}

#[proc_macro]
pub fn quote_m2(input: TokenStream) -> TokenStream {
    quote_m2::expand(input)
}
