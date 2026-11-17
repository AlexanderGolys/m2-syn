//! Procedural macros backing the `m2-syn` typed syntax API.
#![warn(missing_docs)]

extern crate proc_macro;

mod quote_m2;
mod roles;
mod syntax;
mod tokens;

use std::collections::BTreeSet;
use syntax::Traversal;

use proc_macro2::{TokenStream, TokenTree};
use quote::{ToTokens, format_ident, quote};
use syn::{Data, DeriveInput, Error, Fields, Member, Result, Type, parse_macro_input, parse_quote};

fn base_crate() -> TokenStream {
    quote!(::m2_syn)
}

fn expand(
    input: proc_macro::TokenStream,
    expansion: fn(TokenStream) -> Result<TokenStream>,
) -> proc_macro::TokenStream {
    expansion(input.into())
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Declares the token table: one typed token per spelling, the parser metadata
/// of each, and the `PrefixOperator`, `BinaryOperator`, `PostfixOperator`, and
/// `AugmentedOperator` enums that its roles define.
#[proc_macro]
pub fn tokens(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand(input, syntax::tokens)
}

/// Derives the syntax behaviour of a type declared in `m2_syn::nodes`: its
/// constructor, printing, pretty tree, CST migration, and child walkers.
///
/// Fields are read by their Rust types; `delimiter`, `span` and a symbol's
/// qualifiers are storage. `#[syntax(leaf = …)]` marks a terminal,
/// `#[syntax(literal = …)]` a string literal printed as one token, and
/// `#[syntax(cell_stream)]` a node parsed from framed cells.
#[proc_macro_derive(Syntax, attributes(syntax))]
pub fn derive_syntax(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    syntax::derive_syntax(&input)
        .map(|syntax| {
            let walk = expand_syntax_walk(&input);
            quote!(#walk #syntax)
        })
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Generates the `visit`, `visit_mut`, and `fold` traversals over the listed
/// syntax types.
#[proc_macro]
pub fn traversal(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand(input, syntax::traversal)
}

/// Resolves a token spelling to the type `tokens!` generates for it.
///
/// This is what `m2_syn::Token![…]` forwards to. The mapping is pure
/// computation over the concatenated M2 spelling, so whitespace and the
/// boundaries between Rust tokens and string fragments have no effect. It
/// needs no access to the schema and nothing has to be generated for it.
#[proc_macro]
pub fn token(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let base = base_crate();
    match tokens::token_type_name(&input.into()) {
        Ok(name) => quote!(#base::nodes::#name).into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Constructs an M2 token stream with recursive Rust interpolation.
///
/// `$(expression)` emits any value implementing `m2_syn::Lex`.
/// `$[pattern in iterator] { template }` repeats a recursively quoted template
/// with an explicit Rust `for` loop.
#[proc_macro]
pub fn quote_m2(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    quote_m2::expand(input.into()).into()
}

/// Quotes M2 syntax and parses it as the type inferred at the call site.
#[proc_macro]
pub fn parse_quote_m2(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = TokenStream::from(input);
    let base = base_crate();
    quote!({
        let tokens = #base::quote_m2!(#input);
        #base::parse2(tokens).expect("parse_quote_m2! input should parse")
    })
    .into()
}

/// Derives `m2_syn::Spanned` by joining the spans of a value's fields.
#[proc_macro_derive(Spanned)]
pub fn derive_spanned(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match expand_spanned(&input) {
        Ok(expansion) => expansion.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

/// Generates the child-walking implementation for one expanded syntax type.
#[doc(hidden)]
#[proc_macro_derive(__SyntaxWalk)]
pub fn derive_syntax_walk(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_syntax_walk(&input).into()
}

fn is_syntax_storage(member: &Member) -> bool {
    matches!(
        member,
        Member::Named(name)
            if matches!(
                name.to_string().as_str(),
                    "span" | "ident" | "raw" | "delimiter"
            )
    )
}

fn walk_struct_fields(fields: &Fields, mutable: bool) -> TokenStream {
    let calls = fields
        .members()
        .filter(|member| !is_syntax_storage(member))
        .map(|member| {
            if mutable {
                quote!(__walk::VisitValueMut::visit_value_mut(&mut self.#member, visitor);)
            } else {
                quote!(__walk::VisitValue::visit_value(&self.#member, visitor);)
            }
        });
    // A node storing its own source is spelled by its own text: a terminal.
    let storage_hooks = if mutable {
        walk_delimiter_mut(fields)
    } else {
        (has_member(fields, "span") || has_member(fields, "ident"))
            .then(|| quote!(visitor.visit_terminal(self);))
            .into_token_stream()
    };
    quote!(#(#calls)* #storage_hooks)
}

fn has_member(fields: &Fields, name: &str) -> bool {
    fields
        .members()
        .any(|member| matches!(&member, Member::Named(ident) if ident == name))
}

/// Hands a node's delimiter pair to the mutable visitor's hook.
fn walk_delimiter_mut(fields: &Fields) -> TokenStream {
    let base = base_crate();
    has_member(fields, "delimiter")
        .then(|| {
            quote! {
                let mut __m2_syn_delimiter = #base::DelimiterValue::delimiter(&self.delimiter);
                visitor.visit_delimiter_mut(&mut __m2_syn_delimiter);
                self.delimiter = #base::DelimiterValue::from_delimiter(__m2_syn_delimiter)
                    .expect("a delimiter visitor keeps the delimiter family");
            }
        })
        .into_token_stream()
}

fn walk_enum_fields(input: &DeriveInput, mutable: bool) -> TokenStream {
    let Data::Enum(data) = &input.data else {
        unreachable!("enum walking requires an enum")
    };
    let arms = data.variants.iter().map(|variant| {
        let name = &variant.ident;
        let members = variant.fields.members().collect::<Vec<_>>();
        let bindings = members
            .iter()
            .enumerate()
            .map(|(index, _)| format_ident!("__m2_syn_field_{index}"))
            .collect::<Vec<_>>();
        let selected = members
            .iter()
            .zip(&bindings)
            .filter(|(member, _)| !is_syntax_storage(member));
        let calls = selected.map(|(_, binding)| {
            if mutable {
                quote!(__walk::VisitValueMut::visit_value_mut(#binding, visitor);)
            } else {
                quote!(__walk::VisitValue::visit_value(#binding, visitor);)
            }
        });
        match &variant.fields {
            Fields::Named(_) => {
                let patterns = members
                    .iter()
                    .zip(&bindings)
                    .map(|(member, binding)| quote!(#member: #binding));
                quote!(Self::#name { #(#patterns),* } => { #(#calls)* })
            }
            Fields::Unnamed(_) => quote!(Self::#name(#(#bindings),*) => { #(#calls)* }),
            Fields::Unit => quote!(Self::#name => {}),
        }
    });
    quote!(match self { #(#arms,)* })
}

fn fold_struct(input: &DeriveInput) -> TokenStream {
    let Data::Struct(data) = &input.data else {
        unreachable!("struct folding requires a struct")
    };
    let name = &input.ident;
    let members = data.fields.members().collect::<Vec<_>>();
    let bindings = members
        .iter()
        .enumerate()
        .map(|(index, _)| format_ident!("__m2_syn_field_{index}"))
        .collect::<Vec<_>>();
    let values = members.iter().zip(&bindings).map(|(member, binding)| {
        if is_syntax_storage(member) {
            quote!(#binding)
        } else {
            quote!(__walk::FoldValue::fold_value(#binding, folder))
        }
    });
    match &data.fields {
        Fields::Named(_) => {
            let patterns = members
                .iter()
                .zip(&bindings)
                .map(|(member, binding)| quote!(#member: #binding));
            let fields = members
                .iter()
                .zip(values)
                .map(|(member, value)| quote!(#member: #value));
            quote! {
                let #name { #(#patterns),* } = self;
                #name { #(#fields),* }
            }
        }
        Fields::Unnamed(_) => quote! {
            let #name(#(#bindings),*) = self;
            #name(#(#values),*)
        },
        Fields::Unit => quote!(self),
    }
}

fn fold_enum(input: &DeriveInput) -> TokenStream {
    let Data::Enum(data) = &input.data else {
        unreachable!("enum folding requires an enum")
    };
    let arms = data.variants.iter().map(|variant| {
        let name = &variant.ident;
        let members = variant.fields.members().collect::<Vec<_>>();
        let bindings = members
            .iter()
            .enumerate()
            .map(|(index, _)| format_ident!("__m2_syn_field_{index}"))
            .collect::<Vec<_>>();
        let values = members.iter().zip(&bindings).map(|(member, binding)| {
            if is_syntax_storage(member) {
                quote!(#binding)
            } else {
                quote!(__walk::FoldValue::fold_value(#binding, folder))
            }
        });
        match &variant.fields {
            Fields::Named(_) => {
                let patterns = members
                    .iter()
                    .zip(&bindings)
                    .map(|(member, binding)| quote!(#member: #binding));
                let fields = members
                    .iter()
                    .zip(values)
                    .map(|(member, value)| quote!(#member: #value));
                quote!(Self::#name { #(#patterns),* } => Self::#name { #(#fields),* })
            }
            Fields::Unnamed(_) => {
                quote!(Self::#name(#(#bindings),*) => Self::#name(#(#values),*))
            }
            Fields::Unit => quote!(Self::#name => Self::#name),
        }
    });
    quote!(match self { #(#arms,)* })
}

fn expand_syntax_walk(input: &DeriveInput) -> TokenStream {
    let name = &input.ident;
    if !input.generics.params.is_empty() {
        return Error::new_spanned(input, "syntax walking requires a concrete syntax type")
            .to_compile_error();
    }
    let [visit_method, visit_mut_method, fold_method] =
        [Traversal::Visit, Traversal::VisitMut, Traversal::Fold]
            .map(|traversal| traversal.method(name));
    let (impl_generics, type_generics, where_clause) = input.generics.split_for_impl();
    let visit = match &input.data {
        Data::Struct(data) => walk_struct_fields(&data.fields, false),
        Data::Enum(_) => walk_enum_fields(input, false),
        Data::Union(_) => {
            return Error::new_spanned(input, "syntax walking does not support unions")
                .to_compile_error();
        }
    };
    let visit_mut = match &input.data {
        Data::Struct(data) => walk_struct_fields(&data.fields, true),
        Data::Enum(_) => walk_enum_fields(input, true),
        Data::Union(_) => unreachable!(),
    };
    let fold = match &input.data {
        Data::Struct(_) => fold_struct(input),
        Data::Enum(_) => fold_enum(input),
        Data::Union(_) => unreachable!(),
    };
    quote! {
        impl #impl_generics __walk::WalkChildren for #name #type_generics #where_clause {
            fn walk_children<'ast, V>(&'ast self, visitor: &mut V)
            where
                V: visit::Visit<'ast> + ?Sized,
            {
                #visit
            }
        }

        impl #impl_generics __walk::WalkChildrenMut for #name #type_generics #where_clause {
            fn walk_children_mut<V>(&mut self, visitor: &mut V)
            where
                V: visit_mut::VisitMut + ?Sized,
            {
                #visit_mut
            }
        }

        impl #impl_generics __walk::FoldChildren for #name #type_generics #where_clause {
            fn fold_children<F>(self, folder: &mut F) -> Self
            where
                F: fold::Fold + ?Sized,
            {
                #fold
            }
        }

        impl __walk::VisitValue for #name {
            fn visit_value<'ast, V>(&'ast self, visitor: &mut V)
            where
                V: visit::Visit<'ast> + ?Sized,
            {
                visitor.#visit_method(self);
            }
        }

        impl __walk::VisitValueMut for #name {
            fn visit_value_mut<V>(&mut self, visitor: &mut V)
            where
                V: visit_mut::VisitMut + ?Sized,
            {
                visitor.#visit_mut_method(self);
            }
        }

        impl __walk::FoldValue for #name {
            fn fold_value<F>(self, folder: &mut F) -> Self
            where
                F: fold::Fold + ?Sized,
            {
                folder.#fold_method(self)
            }
        }

    }
}

fn expand_spanned_members(members: impl IntoIterator<Item = (Member, TokenStream)>) -> TokenStream {
    let members = members.into_iter().collect::<Vec<_>>();
    let base = base_crate();
    let span = quote! {#base::Spanned::try_span};
    if members.is_empty() {
        return quote!(None);
    }
    if let Some((_, value)) = members
        .iter()
        .find(|(member, _)| matches!(member, Member::Named(ident) if ident == "span"))
    {
        quote!(#span(#value))
    } else {
        let values = members.iter().map(|(_, value)| value);
        quote! {
            #base::Span::join_all(
                [#(#base::Spanned::try_span(#values)),*]
            )
        }
    }
}

fn expand_spanned(input: &DeriveInput) -> Result<TokenStream> {
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
            let empty_fallback = data.variants.is_empty().then(|| quote!(_ => None));
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
            fn try_span(&self) -> Option<#span_ty> {
                #body
            }
        }
    })
}

fn type_mentions_parameter(ty: &impl ToTokens, parameters: &BTreeSet<String>) -> bool {
    fn stream_mentions_parameter(stream: TokenStream, parameters: &BTreeSet<String>) -> bool {
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
