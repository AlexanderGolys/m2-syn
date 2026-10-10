//! Procedural macros backing the `m2-syn` typed syntax API.
#![warn(missing_docs)]

extern crate proc_macro;

mod quote_m2;
mod roles;
mod syntax;
mod tokens;

use std::collections::BTreeSet;

use proc_macro2::{TokenStream, TokenTree};
use quote::{ToTokens, format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{
    Data, DeriveInput, Error, Fields, Member, Result, Token, Type, parse_macro_input, parse_quote,
};

fn base_crate() -> TokenStream {
    quote!(::m2_syn)
}

fn expand_syntax(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let expansion = match syntax::generate(input.into()) {
        Ok(expansion) => expansion,
        Err(error) => return error.into_compile_error().into(),
    };
    expansion.into()
}

/// Declares typed syntax nodes and token atoms.
#[proc_macro]
pub fn syntax(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expand_syntax(input)
}

/// Resolves a token spelling to the type `syntax!` generates for it.
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

struct SyntaxTokenWalk {
    name: syn::Ident,
    ty: Type,
}

impl Parse for SyntaxTokenWalk {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let name = input.parse()?;
        input.parse::<Token![,]>()?;
        let ty = input.parse()?;
        Ok(Self { name, ty })
    }
}

/// Generates the leaf-walking implementation for one syntax token type.
#[doc(hidden)]
#[proc_macro]
pub fn __syntax_token_walk(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let SyntaxTokenWalk { name, ty } = parse_macro_input!(input as SyntaxTokenWalk);
    expand_syntax_token_walk(&name, &ty).into()
}

fn expand_syntax_token_walk(name: &syn::Ident, ty: &Type) -> TokenStream {
    let snake = syntax::to_snake_case(&name.to_string());
    let visit_method = format_ident!("visit_{snake}");
    let visit_mut_method = format_ident!("visit_{snake}_mut");
    let fold_method = format_ident!("fold_{snake}");
    quote! {
        impl __walk::WalkChildren for #ty {
            fn walk_children<'ast, V>(&'ast self, _visitor: &mut V)
            where
                V: visit::Visit<'ast> + ?Sized,
            {}
        }

        impl __walk::WalkChildrenMut for #ty {
            fn walk_children_mut<V>(&mut self, visitor: &mut V)
            where
                V: visit_mut::VisitMut + ?Sized,
            {
                visitor.visit_terminal_mut(self);
            }
        }

        impl __walk::FoldChildren for #ty {
            fn fold_children<F>(self, _folder: &mut F) -> Self
            where
                F: fold::Fold + ?Sized,
            {
                self
            }
        }

        impl __walk::VisitValue for #ty {
            fn visit_value<'ast, V>(&'ast self, visitor: &mut V)
            where
                V: visit::Visit<'ast> + ?Sized,
            {
                visitor.#visit_method(self);
            }
        }

        impl __walk::VisitValueMut for #ty {
            fn visit_value_mut<V>(&mut self, visitor: &mut V)
            where
                V: visit_mut::VisitMut + ?Sized,
            {
                visitor.#visit_mut_method(self);
            }
        }

        impl __walk::FoldValue for #ty {
            fn fold_value<F>(self, folder: &mut F) -> Self
            where
                F: fold::Fold + ?Sized,
            {
                folder.#fold_method(self)
            }
        }

    }
}

struct SyntaxWalkFunction {
    traversal: syn::Ident,
    name: syn::Ident,
    ty: Type,
}

impl Parse for SyntaxWalkFunction {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let traversal = input.parse()?;
        input.parse::<Token![,]>()?;
        let name = input.parse()?;
        input.parse::<Token![,]>()?;
        let ty = input.parse()?;
        Ok(Self {
            traversal,
            name,
            ty,
        })
    }
}

/// Generates one free function inside a traversal module.
#[doc(hidden)]
#[proc_macro]
pub fn __syntax_walk_function(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as SyntaxWalkFunction);
    expand_syntax_walk_function(&input).into()
}

fn expand_syntax_walk_function(input: &SyntaxWalkFunction) -> TokenStream {
    let name = &input.name;
    let ty = &input.ty;
    let snake = syntax::to_snake_case(&name.to_string());
    match input.traversal.to_string().as_str() {
        "visit" => {
            let method = format_ident!("visit_{snake}");
            quote! {
                #[doc = concat!("Walks the children of a `", stringify!(#name), "` node.")]
                pub fn #method<'ast, V>(visitor: &mut V, node: &'ast #ty)
                where
                    V: Visit<'ast> + ?Sized,
                {
                    super::__walk::WalkChildren::walk_children(node, visitor);
                }
            }
        }
        "visit_mut" => {
            let method = format_ident!("visit_{snake}_mut");
            quote! {
                #[doc = concat!("Walks the mutable children of a `", stringify!(#name), "` node.")]
                pub fn #method<V>(visitor: &mut V, node: &mut #ty)
                where
                    V: VisitMut + ?Sized,
                {
                    super::__walk::WalkChildrenMut::walk_children_mut(node, visitor);
                }
            }
        }
        "fold" => {
            let method = format_ident!("fold_{snake}");
            quote! {
                #[doc = concat!("Folds the children of a `", stringify!(#name), "` node.")]
                pub fn #method<F>(folder: &mut F, node: #ty) -> #ty
                where
                    F: Fold + ?Sized,
                {
                    super::__walk::FoldChildren::fold_children(node, folder)
                }
            }
        }
        _ => Error::new_spanned(&input.traversal, "expected `visit`, `visit_mut`, or `fold`")
            .to_compile_error(),
    }
}

fn is_syntax_storage(member: &Member) -> bool {
    matches!(
        member,
        Member::Named(name)
            if matches!(
                name.to_string().as_str(),
                "span"
                    | "raw"
                    | "package_qualifier"
                    | "index_qualifier"
                    | "leading_trivia"
                    | "trailing_trivia"
                    | "delimiter"
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
    let storage_hooks = mutable.then(|| walk_trivia_storage_mut(fields));
    quote!(#(#calls)* #storage_hooks)
}

/// Hands the trivia a node stores itself to the mutable visitor's hooks.
fn walk_trivia_storage_mut(fields: &Fields) -> TokenStream {
    let has_member = |name: &str| {
        fields
            .members()
            .any(|member| matches!(&member, Member::Named(ident) if ident == name))
    };
    let base = base_crate();
    let terminal = has_member("trailing_trivia").then(|| quote!(visitor.visit_terminal_mut(self);));
    let delimiter = has_member("delimiter").then(|| {
        quote! {
            let mut __m2_syn_delimiter = #base::DelimiterValue::delimiter(&self.delimiter);
            visitor.visit_delimiter_mut(&mut __m2_syn_delimiter);
            self.delimiter = #base::DelimiterValue::from_delimiter(__m2_syn_delimiter)
                .expect("a delimiter visitor keeps the delimiter family");
        }
    });
    quote!(#terminal #delimiter)
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
    let snake = syntax::to_snake_case(&name.to_string());
    let visit_method = format_ident!("visit_{snake}");
    let visit_mut_method = format_ident!("visit_{snake}_mut");
    let fold_method = format_ident!("fold_{snake}");
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
