extern crate proc_macro;

mod quote_m2;
mod syntax;
mod tokens;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Error, Fields, Member, Result, parse_macro_input};

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

#[proc_macro]
pub fn parse_quote_m2(input: TokenStream) -> TokenStream {
    let input = TokenStream2::from(input);
    quote!({
        let tokens = ::m2_syn::quote_m2!(#input);
        ::m2_syn::parse2(tokens).expect("parse_quote_m2! input should parse")
    })
    .into()
}

#[proc_macro_derive(Spanned)]
pub fn derive_spanned(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_spanned(&input) {
        Ok(expansion) => expansion.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_spanned_members(
    members: impl IntoIterator<Item = (Member, TokenStream2)>,
) -> TokenStream2 {
    let members = members.into_iter().collect::<Vec<_>>();
    let span = quote! {::m2_syn::Spanned::span};
    if let Some((_, value)) = members
        .iter()
        .find(|(member, _)| matches!(member, Member::Named(ident) if ident == "span"))
    {
        quote!(#span(#value))
    } else {
        let values = members.iter().map(|(_, value)| value);
        quote! {
            ::m2_syn::Span::join_all(
                [#(::m2_syn::Spanned::span(#values)),*]
            )
        }
    }
}

fn expand_spanned(input: &DeriveInput) -> Result<TokenStream2> {
    let name = &input.ident;
    let span_ty = quote! {::m2_syn::Span};
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let body = match &input.data {
        Data::Struct(data) => {
            let members = data.fields.members().map(|member| {
                let value = quote!(&self.#member);
                (member, value)
            });
            expand_spanned_members(members)
        }
        Data::Enum(data) => {
            let arms = data
                .variants
                .iter()
                .map(|variant| {
                    let variant_name = &variant.ident;
                    let members = variant.fields.members().collect::<Vec<_>>();
                    let bindings = members
                        .iter()
                        .enumerate()
                        .map(|(index, _)| format_ident!("__m2_syn_field_{index}"))
                        .collect::<Vec<_>>();
                    let values = bindings.iter().map(|binding| quote!(#binding));
                    let span = expand_spanned_members(members.iter().cloned().zip(values));

                    match &variant.fields {
                        Fields::Named(_) => {
                            let fields = members
                                .iter()
                                .zip(&bindings)
                                .map(|(member, binding)| quote!(#member: #binding));
                            quote! {
                                Self::#variant_name { #(#fields),* } => #span
                            }
                        }
                        Fields::Unnamed(_) => quote! {
                            Self::#variant_name(#(#bindings),*) => #span
                        },
                        Fields::Unit => quote! {
                            Self::#variant_name => #span
                        },
                    }
                })
                .collect::<Vec<_>>();
            let empty_fallback = data
                .variants
                .is_empty()
                .then(|| quote!(_ => #span_ty::detached()));
            quote! {
                match self {
                    #(#arms,)*
                    #empty_fallback
                }
            }
        }
        Data::Union(_) => {
            return Err(Error::new_spanned(
                name,
                "Spanned cannot be derived for unions",
            ));
        }
    };

    Ok(quote! {
        impl #impl_generics
            ::m2_syn::Spanned for #name #type_generics #where_clause
        {
            fn span(&self) -> #span_ty {
                #body
            }
        }
    })
}
