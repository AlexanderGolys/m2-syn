extern crate proc_macro;

mod quote_m2;
mod syntax;
mod tokens;

use proc_macro::TokenStream;
use std::collections::BTreeSet;

use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use quote::{ToTokens, format_ident, quote};
use syn::{Data, DeriveInput, Error, Fields, Member, Result, Type, parse_macro_input, parse_quote};

#[proc_macro]
pub fn syntax(input: TokenStream) -> TokenStream {
    match syntax::generate(input.into()) {
        Ok(expansion) => expansion.combined().into(),
        Err(error) => error.into_compile_error().into(),
    }
}

#[proc_macro]
/// Constructs an M2 token stream with recursive Rust interpolation.
///
/// `$(expression)` emits any value implementing `m2_syn::ToTokens`.
/// `$[pattern in iterator] { template }` repeats a recursively quoted template
/// with an explicit Rust `for` loop.
pub fn quote_m2(input: TokenStream) -> TokenStream {
    quote_m2::expand(input)
}

#[proc_macro]
/// Quotes M2 syntax and parses it as the type inferred at the call site.
pub fn parse_quote_m2(input: TokenStream) -> TokenStream {
    let input = TokenStream2::from(input);
    quote!({
        let tokens = ::m2_syn::quote_m2!(#input);
        ::m2_syn::parse1(tokens).expect("parse_quote_m2! input should parse")
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
    let mut generics = input.generics.clone();
    let type_parameters = input
        .generics
        .type_params()
        .map(|parameter| parameter.ident.to_string())
        .collect::<BTreeSet<_>>();
    for ty in spanned_field_types(input)
        .into_iter()
        .filter(|ty| type_mentions_parameter(ty, &type_parameters))
    {
        generics
            .make_where_clause()
            .predicates
            .push(parse_quote!(#ty: ::m2_syn::Spanned));
    }
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
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

fn type_mentions_parameter(ty: &Type, parameters: &BTreeSet<String>) -> bool {
    fn stream_mentions_parameter(stream: TokenStream2, parameters: &BTreeSet<String>) -> bool {
        stream.into_iter().any(|token| match token {
            TokenTree::Ident(ident) => parameters.contains(&ident.to_string()),
            TokenTree::Group(group) => stream_mentions_parameter(group.stream(), parameters),
            TokenTree::Punct(_) | TokenTree::Literal(_) => false,
        })
    }

    stream_mentions_parameter(ty.to_token_stream(), parameters)
}

fn spanned_field_types(input: &DeriveInput) -> Vec<&Type> {
    fn selected(fields: &Fields) -> Vec<&Type> {
        fields
            .iter()
            .find(|field| field.ident.as_ref().is_some_and(|ident| ident == "span"))
            .map_or_else(
                || fields.iter().map(|field| &field.ty).collect(),
                |field| vec![&field.ty],
            )
    }

    match &input.data {
        Data::Struct(data) => selected(&data.fields),
        Data::Enum(data) => data
            .variants
            .iter()
            .flat_map(|variant| selected(&variant.fields))
            .collect(),
        Data::Union(_) => Vec::new(),
    }
}
