//! Procedural macros backing the `m2-syn` typed syntax API.
#![warn(missing_docs)]

extern crate proc_macro;

mod quote_m2;
mod roles;
mod syntax;
mod tokens;

use proc_macro::TokenStream;
use std::collections::BTreeSet;

use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use quote::{ToTokens, format_ident, quote};
use syn::{Data, DeriveInput, Error, Fields, Member, Result, Type, parse_macro_input, parse_quote};

fn base_crate() -> TokenStream2 {
    quote!(::m2_syn)
}

/// Declares typed syntax nodes, token atoms, and traversal implementations.
#[proc_macro]
pub fn syntax(input: TokenStream) -> TokenStream {
    let expansion = match syntax::generate(input.into()) {
        Ok(expansion) => expansion,
        Err(error) => return error.into_compile_error().into(),
    };
    expansion.into()
}

/// Resolves a token spelling to the type `syntax!` generates for it.
///
/// This is what `m2_syn::Token![…]` forwards to. The mapping is pure
/// computation over the spelling, so it needs no access to the schema and
/// nothing has to be generated for it.
#[proc_macro]
pub fn token(input: TokenStream) -> TokenStream {
    let base = base_crate();
    match tokens::token_type_name(&input.into()) {
        Ok(name) => quote!(#base::#name).into(),
        Err(error) => error.into_compile_error().into(),
    }
}
/// Constructs an M2 token stream with recursive Rust interpolation.
///
/// `$(expression)` emits any value implementing `m2_syn::Lex`.
/// `$[pattern in iterator] { template }` repeats a recursively quoted template
/// with an explicit Rust `for` loop.
#[proc_macro]
pub fn quote_m2(input: TokenStream) -> TokenStream {
    quote_m2::expand(input)
}

/// Quotes M2 syntax and parses it as the type inferred at the call site.
#[proc_macro]
pub fn parse_quote_m2(input: TokenStream) -> TokenStream {
    let input = TokenStream2::from(input);
    let base = base_crate();
    quote!({
        let tokens = #base::quote_m2!(#input);
        #base::parse2(tokens).expect("parse_quote_m2! input should parse")
    })
    .into()
}

/// Derives `m2_syn::Spanned` by joining the spans of a value's fields.
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
    let base = base_crate();
    let span = quote! {#base::Spanned::span};
    if let Some((_, value)) = members
        .iter()
        .find(|(member, _)| matches!(member, Member::Named(ident) if ident == "span"))
    {
        quote!(#span(#value))
    } else {
        let values = members.iter().map(|(_, value)| value);
        quote! {
            #base::Span::join_all(
                [#(#base::Spanned::span(#values)),*]
            )
        }
    }
}

fn expand_spanned(input: &DeriveInput) -> Result<TokenStream2> {
    let name = &input.ident;
    let base = base_crate();
    let span_ty = quote! {#base::Span};
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
            .push(parse_quote!(#ty: #base::Spanned));
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
            #base::Spanned for #name #type_generics #where_clause
        {
            fn span(&self) -> #span_ty {
                #body
            }
        }
    })
}

fn type_mentions_parameter(ty: &impl ToTokens, parameters: &BTreeSet<String>) -> bool {
    fn stream_mentions_parameter(stream: TokenStream2, parameters: &BTreeSet<String>) -> bool {
        stream.into_iter().any(|token| match token {
            TokenTree::Ident(ident) => parameters.contains(&ident.to_string()),
            TokenTree::Group(group) => stream_mentions_parameter(group.stream(), parameters),
            _ => false,
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
