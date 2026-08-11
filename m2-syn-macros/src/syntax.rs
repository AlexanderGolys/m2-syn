use crate::utils::err_ret;

use std::collections::{BTreeMap, BTreeSet};

use proc_macro::TokenStream;
use proc_macro2::{Ident, Span, TokenStream as TokenStream2, TokenTree};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{
    Attribute, Error, Field, Fields, Index, Item, ItemEnum, ItemStruct, LitStr, Member, Meta,
    PathArguments, Result, Token, Type, TypePath, Visibility, braced, bracketed, parse_macro_input,
};

mod traversal;

use traversal::Traversal;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SyntaxTypeName(String);

impl From<&Ident> for SyntaxTypeName {
    fn from(value: &Ident) -> Self {
        Self(value.to_string())
    }
}

type ConversionStep = (Ident, Ident);
type ConversionPath = Vec<ConversionStep>;
type ConversionGraph = BTreeMap<(SyntaxTypeName, SyntaxTypeName), Vec<ConversionPath>>;

pub fn expand(input: TokenStream) -> TokenStream {
    let syntax = parse_macro_input!(input as Syntax);
    match syntax.expand() {
        Ok(expansion) => expansion.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

struct Syntax {
    tokens: Vec<TokenDefinition>,
    structs: Vec<StructDefinition>,
    enums: Vec<EnumDefinition>,
}

impl Parse for Syntax {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let tokens_keyword: Ident = input.parse()?;
        if tokens_keyword != "tokens" {
            err_ret!(tokens_keyword.span(), "expected `tokens`");
        }

        let token_content;
        braced!(token_content in input);

        let mut tokens = Vec::new();
        while !token_content.is_empty() {
            tokens.push(token_content.parse()?);
            if token_content.peek(Token![,]) {
                token_content.parse::<Token![,]>()?;
            }
        }

        let mut structs = Vec::new();
        let mut enums = Vec::new();
        while !input.is_empty() {
            match input.parse::<Item>()? {
                Item::Struct(item) => structs.push(StructDefinition::new(item)?),
                Item::Enum(item) => enums.push(EnumDefinition::new(item)?),
                item => {
                    return Err(Error::new_spanned(
                        item,
                        "syntax declarations support only structs and enums",
                    ));
                }
            }
        }

        Ok(Self {
            tokens,
            structs,
            enums,
        })
    }
}

struct TokenDefinition {
    name: Ident,
    pattern: TokenStream2,
    spelling: String,
}

impl Parse for TokenDefinition {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let name = input.parse()?;
        let pattern_content;
        bracketed!(pattern_content in input);
        let pattern: TokenStream2 = pattern_content.parse()?;
        let spelling = token_spelling(&pattern)?;
        Ok(Self {
            name,
            pattern,
            spelling,
        })
    }
}

fn token_spelling(pattern: &TokenStream2) -> Result<String> {
    let trees = pattern.clone().into_iter().collect::<Vec<_>>();
    if let [TokenTree::Literal(literal)] = trees.as_slice()
        && let Ok(literal) = syn::parse_str::<LitStr>(&literal.to_string())
    {
        return Ok(literal.value());
    }

    let mut spelling = String::new();
    for tree in trees {
        match tree {
            TokenTree::Ident(identifier) => spelling.push_str(&identifier.to_string()),
            TokenTree::Punct(punctuation) => spelling.push(punctuation.as_char()),
            TokenTree::Literal(literal) => spelling.push_str(&literal.to_string()),
            TokenTree::Group(group) => {
                return Err(Error::new(
                    group.span(),
                    "delimiters must be written as string literals",
                ));
            }
        }
    }

    if spelling.is_empty() {
        Err(Error::new(
            Span::call_site(),
            "token spelling cannot be empty",
        ))
    } else {
        Ok(spelling)
    }
}

struct StructDefinition {
    attrs: Vec<Attribute>,
    visibility: Visibility,
    name: Ident,
    kind: String,
    fields: StructFields,
}

enum StructFields {
    Leaf,
    Product {
        style: ProductStyle,
        fields: Vec<FieldDefinition>,
    },
}

#[derive(Clone, Copy)]
enum ProductStyle {
    Named,
    Unnamed,
}

impl StructDefinition {
    fn new(mut item: ItemStruct) -> Result<Self> {
        reject_generics(&item.generics, &item.ident)?;
        let syntax = take_syntax_attribute(&mut item.attrs)?;
        let kind = syntax
            .kind
            .unwrap_or_else(|| to_snake_case(&item.ident.to_string()));
        let fields = match item.fields {
            Fields::Unit => StructFields::Leaf,
            Fields::Named(fields) => StructFields::Product {
                style: ProductStyle::Named,
                fields: fields
                    .named
                    .into_iter()
                    .enumerate()
                    .map(|(index, field)| FieldDefinition::new(field, index))
                    .collect::<Result<_>>()?,
            },
            Fields::Unnamed(fields) => StructFields::Product {
                style: ProductStyle::Unnamed,
                fields: fields
                    .unnamed
                    .into_iter()
                    .enumerate()
                    .map(|(index, field)| FieldDefinition::new(field, index))
                    .collect::<Result<_>>()?,
            },
        };
        Ok(Self {
            attrs: item.attrs,
            visibility: item.vis,
            name: item.ident,
            kind,
            fields,
        })
    }
}

struct FieldDefinition {
    attrs: Vec<Attribute>,
    visibility: Visibility,
    member: Member,
    binding: Ident,
    source: FieldSource,
    shape: TypeShape,
}

enum FieldSource {
    Named(String),
    Unfielded,
}

impl FieldDefinition {
    fn new(mut field: Field, index: usize) -> Result<Self> {
        let syntax = take_syntax_attribute(&mut field.attrs)?;
        let member = field
            .ident
            .clone()
            .map(Member::Named)
            .unwrap_or_else(|| Member::Unnamed(Index::from(index)));
        let binding = field
            .ident
            .clone()
            .unwrap_or_else(|| format_ident!("field_{index}"));
        let source = if syntax.unfielded {
            FieldSource::Unfielded
        } else if let Some(field) = syntax.field {
            FieldSource::Named(field)
        } else if let Member::Named(name) = &member
            && !name.to_string().starts_with('_')
        {
            FieldSource::Named(name.to_string().trim_start_matches("r#").to_owned())
        } else {
            FieldSource::Unfielded
        };
        Ok(Self {
            attrs: field.attrs,
            visibility: field.vis,
            member,
            binding,
            source,
            shape: TypeShape::parse(field.ty)?,
        })
    }
}

struct EnumDefinition {
    attrs: Vec<Attribute>,
    visibility: Visibility,
    name: Ident,
    variants: Vec<VariantDefinition>,
}

struct VariantDefinition {
    attrs: Vec<Attribute>,
    name: Ident,
    shape: TypeShape,
}

impl EnumDefinition {
    fn new(mut item: ItemEnum) -> Result<Self> {
        reject_generics(&item.generics, &item.ident)?;
        take_syntax_attribute(&mut item.attrs)?;
        let variants = item
            .variants
            .into_iter()
            .map(|mut variant| {
                if variant.discriminant.is_some() {
                    return Err(Error::new_spanned(
                        variant,
                        "syntax coproduct variants cannot have discriminants",
                    ));
                }
                take_syntax_attribute(&mut variant.attrs)?;
                let mut fields = match variant.fields {
                    Fields::Unnamed(fields) if fields.unnamed.len() == 1 => fields.unnamed,
                    fields => {
                        return Err(Error::new_spanned(
                            fields,
                            "syntax coproduct variants must contain exactly one unnamed value",
                        ));
                    }
                };
                let field = fields.pop().expect("one variant field");
                let shape = TypeShape::parse(field.ty)?;
                if !matches!(shape, TypeShape::Base(_, _)) {
                    return Err(Error::new(
                        variant.ident.span(),
                        "syntax coproduct variants must directly contain a declared syntax type",
                    ));
                }
                Ok(VariantDefinition {
                    attrs: variant.attrs,
                    name: variant.ident,
                    shape,
                })
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            attrs: item.attrs,
            visibility: item.vis,
            name: item.ident,
            variants,
        })
    }
}

#[derive(Default)]
struct SyntaxAttribute {
    kind: Option<String>,
    field: Option<String>,
    unfielded: bool,
}

impl Parse for SyntaxAttribute {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let values = Punctuated::<Meta, Token![,]>::parse_terminated(input)?;
        let mut result = Self::default();
        for value in values {
            match value {
                Meta::Path(path) if path.is_ident("unfielded") => result.unfielded = true,
                Meta::NameValue(value) if value.path.is_ident("kind") => {
                    result.kind = Some(literal_string(value.value, "kind")?);
                }
                Meta::NameValue(value) if value.path.is_ident("field") => {
                    result.field = Some(literal_string(value.value, "field")?);
                }
                value => {
                    return Err(Error::new_spanned(
                        value,
                        "supported syntax options are `kind = \"...\"`, `field = \"...\"`, and `unfielded`",
                    ));
                }
            }
        }
        Ok(result)
    }
}

fn literal_string(expression: syn::Expr, option: &str) -> Result<String> {
    match expression {
        syn::Expr::Lit(expression) => match expression.lit {
            syn::Lit::Str(value) => Ok(value.value()),
            literal => Err(Error::new_spanned(
                literal,
                format!("`{option}` must be a string literal"),
            )),
        },
        expression => Err(Error::new_spanned(
            expression,
            format!("`{option}` must be a string literal"),
        )),
    }
}

fn take_syntax_attribute(attributes: &mut Vec<Attribute>) -> Result<SyntaxAttribute> {
    let mut result = SyntaxAttribute::default();
    let mut retained = Vec::new();
    for attribute in attributes.drain(..) {
        if attribute.path().is_ident("syntax") {
            let parsed = attribute.parse_args::<SyntaxAttribute>()?;
            if parsed.kind.is_some() {
                result.kind = parsed.kind;
            }
            if parsed.field.is_some() {
                result.field = parsed.field;
            }
            result.unfielded |= parsed.unfielded;
        } else {
            retained.push(attribute);
        }
    }
    *attributes = retained;
    Ok(result)
}

fn reject_generics(generics: &syn::Generics, name: &Ident) -> Result<()> {
    if generics.params.is_empty() && generics.where_clause.is_none() {
        Ok(())
    } else {
        Err(Error::new(
            name.span(),
            "generated syntax nodes do not support generics",
        ))
    }
}

#[derive(Clone)]
enum TypeShape {
    Base(TypePath, Ident),
    Boxed(Box<Self>),
    Optional(Box<Self>),
    Repeated(Box<Self>),
}

impl TypeShape {
    fn parse(ty: Type) -> Result<Self> {
        let Type::Path(path) = ty else {
            return Err(Error::new_spanned(
                ty,
                "syntax fields support only named types, Box, Option, and Vec",
            ));
        };
        if path.qself.is_some() {
            return Err(Error::new_spanned(
                path,
                "qualified self types are not supported in syntax fields",
            ));
        }
        let segment = path
            .path
            .segments
            .last()
            .ok_or_else(|| Error::new_spanned(&path, "expected a type name"))?;
        let wrapper = match segment.ident.to_string().as_str() {
            "Box" => Some(0),
            "Option" => Some(1),
            "Vec" => Some(2),
            _ => None,
        };
        let Some(wrapper) = wrapper else {
            if !matches!(segment.arguments, PathArguments::None) {
                return Err(Error::new_spanned(
                    &segment.arguments,
                    "generic syntax field types must be Box, Option, or Vec",
                ));
            }
            return Ok(Self::Base(path.clone(), segment.ident.clone()));
        };
        let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
            return Err(Error::new_spanned(
                &segment.arguments,
                "container syntax types require one type argument",
            ));
        };
        if arguments.args.len() != 1 {
            return Err(Error::new_spanned(
                arguments,
                "container syntax types require one type argument",
            ));
        }
        let syn::GenericArgument::Type(inner) = arguments.args.first().expect("one argument")
        else {
            return Err(Error::new_spanned(
                arguments,
                "container syntax types require a type argument",
            ));
        };
        let inner = Box::new(Self::parse(inner.clone())?);
        Ok(match wrapper {
            0 => Self::Boxed(inner),
            1 => Self::Optional(inner),
            2 => Self::Repeated(inner),
            _ => unreachable!(),
        })
    }

    fn base_ident(&self) -> &Ident {
        match self {
            Self::Base(_, ident) => ident,
            Self::Boxed(inner) | Self::Optional(inner) | Self::Repeated(inner) => {
                inner.base_ident()
            }
        }
    }
}

impl Syntax {
    fn expand(&self) -> Result<TokenStream2> {
        let names = self.declared_names()?;
        self.validate_references(&names)?;
        let token_like = self.token_like_types();
        let tokens = self.expand_tokens();
        let structs = self
            .structs
            .iter()
            .map(|definition| self.expand_struct(definition, &token_like))
            .collect::<Result<Vec<_>>>()?;
        let enums = self.enums.iter().map(expand_enum).collect::<Vec<_>>();
        let conversions = self.expand_conversions()?;
        let node_kinds = self.expand_node_kinds();
        let visits = self.expand_visit(&token_like);
        let visit_muts = self.expand_visit_mut(&token_like);
        let folds = self.expand_fold(&token_like);

        Ok(quote! {
            #node_kinds
            #tokens
            #(#structs)*
            #(#enums)*
            #conversions
            #visits
            #visit_muts
            #folds
        })
    }

    fn declared_names(&self) -> Result<BTreeSet<SyntaxTypeName>> {
        let mut names = BTreeSet::new();
        for name in self
            .tokens
            .iter()
            .map(|definition| &definition.name)
            .chain(self.structs.iter().map(|definition| &definition.name))
            .chain(self.enums.iter().map(|definition| &definition.name))
        {
            if !names.insert(name.into()) {
                return Err(Error::new(name.span(), "duplicate syntax type name"));
            }
        }
        Ok(names)
    }

    fn validate_references(&self, names: &BTreeSet<SyntaxTypeName>) -> Result<()> {
        for shape in
            self.structs
                .iter()
                .filter_map(|definition| match &definition.fields {
                    StructFields::Product { fields, .. } => {
                        Some(fields.iter().map(|field| &field.shape))
                    }
                    StructFields::Leaf => None,
                })
                .flatten()
                .chain(self.enums.iter().flat_map(|definition| {
                    definition.variants.iter().map(|variant| &variant.shape)
                }))
        {
            let ident = shape.base_ident();
            if !names.contains(&ident.into()) {
                return Err(Error::new(
                    ident.span(),
                    "syntax fields must refer to a type declared in the same syntax block",
                ));
            }
        }
        Ok(())
    }

    fn token_like_types(&self) -> BTreeSet<SyntaxTypeName> {
        let mut token_like = self
            .tokens
            .iter()
            .map(|definition| (&definition.name).into())
            .collect::<BTreeSet<_>>();
        loop {
            let before = token_like.len();
            for definition in &self.enums {
                if definition
                    .variants
                    .iter()
                    .all(|variant| token_like.contains(&variant.shape.base_ident().into()))
                {
                    token_like.insert((&definition.name).into());
                }
            }
            if token_like.len() == before {
                return token_like;
            }
        }
    }

    fn expand_node_kinds(&self) -> TokenStream2 {
        let variants = self
            .tokens
            .iter()
            .map(|definition| &definition.name)
            .chain(self.structs.iter().map(|definition| &definition.name));
        quote! {
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
            pub enum SyntaxKind {
                #(#variants,)*
            }
        }
    }

    fn expand_tokens(&self) -> TokenStream2 {
        let definitions = self.tokens.iter().map(|definition| {
            let name = &definition.name;
            let spelling = &definition.spelling;
            quote! {
                #[derive(Debug, Clone, Copy, PartialEq, Eq)]
                pub struct #name {
                    span: ::m2_syn::Span,
                }

                impl #name {
                    pub fn new(span: ::m2_syn::Span) -> Self {
                        Self { span }
                    }
                }

                impl ::m2_syn::Spanned for #name {
                    fn span(&self) -> ::m2_syn::Span {
                        self.span
                    }
                }

                impl ::m2_syn::AstNode for #name {
                    type Kind = SyntaxKind;

                    fn kind(&self) -> SyntaxKind {
                        SyntaxKind::#name
                    }
                }

                impl ::m2_syn::ConcreteNode for #name {
                    const NAME: &'static str = #spelling;
                    const NAMED: bool = false;
                }

                impl ::m2_syn::Token for #name {
                    const SPELLING: &'static str = #spelling;
                }

                impl ::m2_syn::ToTokens for #name {
                    fn to_tokens(&self, output: &mut ::m2_syn::TokenStream) {
                        let span = ::m2_syn::Spanned::span(self);
                        match <Self as ::m2_syn::Token>::SPELLING {
                            "EOC" => output.push_end_of_cell(span),
                            "EOF" => output.push_end_of_file(span),
                            spelling => output.push_text(spelling, span),
                        }
                    }
                }

                impl<N> ::m2_syn::Reconstruct<N> for #name
                where
                    N: ::m2_syn::CstNode,
                {
                    fn matches(node: &N) -> bool {
                        ::m2_syn::matches_concrete::<Self, N>(node)
                    }

                    fn reconstruct(node: N) -> Result<Self, ::m2_syn::ReconstructError> {
                        ::m2_syn::expect_concrete::<Self, N>(&node)?;
                        Ok(Self::new(node.span()))
                    }
                }
            }
        });
        let arms = self.tokens.iter().map(|definition| {
            let name = &definition.name;
            let pattern = &definition.pattern;
            quote! { [#pattern] => { $crate::#name }; }
        });
        quote! {
            #(#definitions)*

            #[macro_export]
            macro_rules! Token {
                #(#arms)*
            }
        }
    }

    fn expand_struct(
        &self,
        definition: &StructDefinition,
        token_like: &BTreeSet<SyntaxTypeName>,
    ) -> Result<TokenStream2> {
        let attrs = &definition.attrs;
        let visibility = &definition.visibility;
        let name = &definition.name;
        let kind = &definition.kind;
        let common = quote! {
            impl ::m2_syn::AstNode for #name {
                type Kind = SyntaxKind;

                fn kind(&self) -> SyntaxKind {
                    SyntaxKind::#name
                }
            }

            impl ::m2_syn::ConcreteNode for #name {
                const NAME: &'static str = #kind;
                const NAMED: bool = true;
            }
        };
        match &definition.fields {
            StructFields::Leaf => Ok(quote! {
                #(#attrs)*
                #[derive(Debug)]
                #visibility struct #name {
                    pub text: String,
                    span: ::m2_syn::Span,
                }

                impl #name {
                    pub fn new(text: impl Into<String>, span: ::m2_syn::Span) -> Self {
                        Self { text: text.into(), span }
                    }
                }

                impl ::m2_syn::Spanned for #name {
                    fn span(&self) -> ::m2_syn::Span {
                        self.span
                    }
                }

                impl ::m2_syn::ToTokens for #name {
                    fn to_tokens(&self, output: &mut ::m2_syn::TokenStream) {
                        output.push_text(&self.text, ::m2_syn::Spanned::span(self));
                    }
                }

                #common

                impl<N> ::m2_syn::Reconstruct<N> for #name
                where
                    N: ::m2_syn::CstNode,
                {
                    fn matches(node: &N) -> bool {
                        ::m2_syn::matches_concrete::<Self, N>(node)
                    }

                    fn reconstruct(node: N) -> Result<Self, ::m2_syn::ReconstructError> {
                        ::m2_syn::expect_concrete::<Self, N>(&node)?;
                        Ok(Self::new(node.text(), node.span()))
                    }
                }
            }),
            StructFields::Product { style, fields } => {
                let struct_definition = match style {
                    ProductStyle::Named => {
                        let expanded_fields = fields.iter().map(|field| {
                            let attrs = &field.attrs;
                            let visibility = &field.visibility;
                            let member = &field.member;
                            let stored = field.shape.stored_type(token_like, false);
                            quote! { #(#attrs)* #visibility #member: #stored }
                        });
                        quote! {
                            #visibility struct #name {
                                #(#expanded_fields,)*
                            }
                        }
                    }
                    ProductStyle::Unnamed => {
                        let expanded_fields = fields.iter().map(|field| {
                            let attrs = &field.attrs;
                            let visibility = &field.visibility;
                            let stored = field.shape.stored_type(token_like, false);
                            quote! { #(#attrs)* #visibility #stored }
                        });
                        quote! {
                            #visibility struct #name(#(#expanded_fields,)*);
                        }
                    }
                };
                let constructor_arguments = fields.iter().map(|field| {
                    let binding = &field.binding;
                    let ty = field.shape.source_type();
                    quote! { #binding: #ty }
                });
                let constructor = match style {
                    ProductStyle::Named => {
                        let fields = fields.iter().map(|field| {
                            let member = &field.member;
                            let binding = &field.binding;
                            let value =
                                field.shape.store_value(quote!(#binding), token_like, false);
                            quote! { #member: #value }
                        });
                        quote!(Self { #(#fields,)* })
                    }
                    ProductStyle::Unnamed => {
                        let fields = fields.iter().map(|field| {
                            let binding = &field.binding;
                            field.shape.store_value(quote!(#binding), token_like, false)
                        });
                        quote!(Self(#(#fields,)*))
                    }
                };
                let span_fields = fields.iter().map(|field| {
                    let member = &field.member;
                    quote! { ::m2_syn::Spanned::span(&self.#member) }
                });
                let reconstruct_fields = fields
                    .iter()
                    .map(|field| field.reconstruct(token_like))
                    .collect::<Result<Vec<_>>>()?;
                let reconstructed = match style {
                    ProductStyle::Named => {
                        let fields = fields.iter().map(|field| {
                            let member = &field.member;
                            let binding = &field.binding;
                            quote!(#member: #binding)
                        });
                        quote!(Self { #(#fields,)* })
                    }
                    ProductStyle::Unnamed => {
                        let fields = fields.iter().map(|field| &field.binding);
                        quote!(Self(#(#fields,)*))
                    }
                };
                Ok(quote! {
                    #(#attrs)*
                    #[derive(Debug)]
                    #struct_definition

                    impl #name {
                        pub fn new(#(#constructor_arguments),*) -> Self {
                            #constructor
                        }
                    }
                   impl ::m2_syn::Spanned for #name {
                        fn span(&self) -> ::m2_syn::Span {
                            ::m2_syn::Span::join_all([#(#span_fields),*])
                        }
                    }

                    #common

                    impl<N> ::m2_syn::Reconstruct<N> for #name
                    where
                        N: ::m2_syn::CstNode,
                    {
                        fn matches(node: &N) -> bool {
                            ::m2_syn::matches_concrete::<Self, N>(node)
                        }

                        fn reconstruct(node: N) -> Result<Self, ::m2_syn::ReconstructError> {
                            ::m2_syn::expect_concrete::<Self, N>(&node)?;
                            let mut children = ::m2_syn::ChildCursor::new(&node);
                            #(#reconstruct_fields)*
                            Ok(#reconstructed)
                        }
                    }
                })
            }
        }
    }

    fn expand_conversions(&self) -> Result<TokenStream2> {
        let mut paths = ConversionGraph::new();
        for definition in &self.enums {
            for variant in &definition.variants {
                let source = variant.shape.base_ident().into();
                paths
                    .entry((source, (&definition.name).into()))
                    .or_default()
                    .push(vec![(definition.name.clone(), variant.name.clone())]);
            }
        }

        loop {
            let snapshot = paths.clone();
            let mut changed = false;
            for ((source, middle), source_paths) in &snapshot {
                for ((candidate, target), target_paths) in &snapshot {
                    if middle != candidate || source == target {
                        continue;
                    }
                    let entry = paths.entry((source.clone(), target.clone())).or_default();
                    for source_path in source_paths {
                        for target_path in target_paths {
                            let combined = source_path
                                .iter()
                                .cloned()
                                .chain(target_path.iter().cloned())
                                .collect::<Vec<_>>();
                            if combined.len() > self.enums.len() {
                                continue;
                            }
                            if !entry.contains(&combined) {
                                entry.push(combined);
                                changed = true;
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        let declarations = self.declaration_idents();
        let implementations = paths.into_iter().filter_map(|((source, target), paths)| {
            if paths.len() != 1 {
                return None;
            }
            let source = declarations.get(&source)?;
            let target = declarations.get(&target)?;
            let wrappers = &paths[0];
            let value = wrappers.iter().fold(
                quote!(value),
                |value, (owner, variant)| quote!(#owner::#variant(#value)),
            );
            Some(quote! {
                impl From<#source> for #target {
                    fn from(value: #source) -> Self {
                        #value
                    }
                }
            })
        });
        Ok(quote! { #(#implementations)* })
    }

    fn declaration_idents(&self) -> BTreeMap<SyntaxTypeName, Ident> {
        self.tokens
            .iter()
            .map(|definition| definition.name.clone())
            .chain(
                self.structs
                    .iter()
                    .map(|definition| definition.name.clone()),
            )
            .chain(self.enums.iter().map(|definition| definition.name.clone()))
            .map(|ident| (SyntaxTypeName::from(&ident), ident))
            .collect()
    }

    fn expand_visit(&self, token_like: &BTreeSet<SyntaxTypeName>) -> TokenStream2 {
        let methods = self.all_types().map(|name| {
            let method = format_ident!("visit_{}", to_snake_case(&name.to_string()));
            quote! {
                fn #method(&mut self, node: &'ast #name) {
                    $crate::visit::#method(self, node);
                }
            }
        });
        let walkers = self.expand_walkers(Traversal::Visit, token_like);
        quote! {
            #[doc(hidden)]
            #[macro_export]
            macro_rules! __m2_syn_visit_methods {
                () => { #(#methods)* };
            }

            #[doc(hidden)]
            #[macro_export]
            macro_rules! __m2_syn_visit_walkers {
                () => { #(#walkers)* };
            }
        }
    }

    fn expand_visit_mut(&self, token_like: &BTreeSet<SyntaxTypeName>) -> TokenStream2 {
        let methods = self.all_types().map(|name| {
            let method = format_ident!("visit_{}_mut", to_snake_case(&name.to_string()));
            quote! {
                fn #method(&mut self, node: &mut #name) {
                    $crate::visit_mut::#method(self, node);
                }
            }
        });
        let walkers = self.expand_walkers(Traversal::VisitMut, token_like);
        quote! {
            #[doc(hidden)]
            #[macro_export]
            macro_rules! __m2_syn_visit_mut_methods {
                () => { #(#methods)* };
            }

            #[doc(hidden)]
            #[macro_export]
            macro_rules! __m2_syn_visit_mut_walkers {
                () => { #(#walkers)* };
            }
        }
    }

    fn expand_fold(&self, token_like: &BTreeSet<SyntaxTypeName>) -> TokenStream2 {
        let methods = self.all_types().map(|name| {
            let method = format_ident!("fold_{}", to_snake_case(&name.to_string()));
            quote! {
                fn #method(&mut self, node: #name) -> #name {
                    $crate::fold::#method(self, node)
                }
            }
        });
        let walkers = self.expand_walkers(Traversal::Fold, token_like);
        quote! {
            #[doc(hidden)]
            #[macro_export]
            macro_rules! __m2_syn_fold_methods {
                () => { #(#methods)* };
            }

            #[doc(hidden)]
            #[macro_export]
            macro_rules! __m2_syn_fold_walkers {
                () => { #(#walkers)* };
            }
        }
    }

    fn all_types(&self) -> impl Iterator<Item = &Ident> {
        self.tokens
            .iter()
            .map(|definition| &definition.name)
            .chain(self.structs.iter().map(|definition| &definition.name))
            .chain(self.enums.iter().map(|definition| &definition.name))
    }

    fn expand_walkers(
        &self,
        traversal: Traversal,
        token_like: &BTreeSet<SyntaxTypeName>,
    ) -> Vec<TokenStream2> {
        let tokens = self
            .tokens
            .iter()
            .map(|definition| traversal.empty_walker(&definition.name));
        let structs = self.structs.iter().map(|definition| {
            let name = &definition.name;
            match &definition.fields {
                StructFields::Leaf => traversal.empty_walker(name),
                StructFields::Product { style, fields } => {
                    traversal.struct_walker(name, *style, fields, token_like)
                }
            }
        });
        let enums = self
            .enums
            .iter()
            .map(|definition| traversal.enum_walker(definition));
        tokens.chain(structs).chain(enums).collect()
    }
}

impl FieldDefinition {
    fn reconstruct(&self, token_like: &BTreeSet<SyntaxTypeName>) -> Result<TokenStream2> {
        let binding = &self.binding;
        let (cardinality, element) = self.shape.cardinality()?;
        let base = element.base_ident();
        let selected = match (&self.source, cardinality) {
            (FieldSource::Named(field), Cardinality::Required) => {
                quote!(children.required_field(#field)?)
            }
            (FieldSource::Named(field), Cardinality::Optional) => {
                quote!(children.optional_field(#field))
            }
            (FieldSource::Named(field), Cardinality::Repeated) => {
                quote!(children.repeated_field(#field))
            }
            (FieldSource::Unfielded, Cardinality::Required) => {
                quote!(children.required_matching::<#base>()?)
            }
            (FieldSource::Unfielded, Cardinality::Optional) => {
                quote!(children.optional_matching::<#base>())
            }
            (FieldSource::Unfielded, Cardinality::Repeated) => {
                quote!(children.repeated_matching::<#base>())
            }
        };
        let stored_element = element.stored_shape(token_like, cardinality == Cardinality::Repeated);
        Ok(match cardinality {
            Cardinality::Required => {
                let reconstructed = quote!(
                    <#base as ::m2_syn::Reconstruct<N>>::reconstruct(#selected)?
                );
                let value = stored_element.wrap_value(reconstructed);
                quote!(let #binding = #value;)
            }
            Cardinality::Optional => {
                let value = stored_element.wrap_value(quote!(value));
                quote!(let #binding = #selected
                    .map(|node| <#base as ::m2_syn::Reconstruct<N>>::reconstruct(node)
                        .map(|value| #value))
                    .transpose()?;)
            }
            Cardinality::Repeated => {
                let value = stored_element.wrap_value(quote!(value));
                quote!(let #binding = #selected
                    .into_iter()
                    .map(|node| <#base as ::m2_syn::Reconstruct<N>>::reconstruct(node)
                        .map(|value| #value))
                    .collect::<Result<Vec<_>, _>>()?;)
            }
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cardinality {
    Required,
    Optional,
    Repeated,
}

#[derive(Clone)]
enum StoredShape {
    Base(TypePath, Ident),
    Boxed(Box<Self>),
    Optional(Box<Self>),
    Repeated(Box<Self>),
}

impl TypeShape {
    fn source_type(&self) -> TokenStream2 {
        match self {
            Self::Base(path, _) => quote!(#path),
            Self::Boxed(inner) => {
                let inner = inner.source_type();
                quote!(Box<#inner>)
            }
            Self::Optional(inner) => {
                let inner = inner.source_type();
                quote!(Option<#inner>)
            }
            Self::Repeated(inner) => {
                let inner = inner.source_type();
                quote!(Vec<#inner>)
            }
        }
    }

    fn stored_type(&self, token_like: &BTreeSet<SyntaxTypeName>, indirect: bool) -> TokenStream2 {
        self.stored_shape(token_like, indirect).ty()
    }

    fn stored_shape(&self, token_like: &BTreeSet<SyntaxTypeName>, indirect: bool) -> StoredShape {
        match self {
            Self::Base(path, ident) => {
                let base = StoredShape::Base(path.clone(), ident.clone());
                if indirect || token_like.contains(&ident.into()) {
                    base
                } else {
                    StoredShape::Boxed(Box::new(base))
                }
            }
            Self::Boxed(inner) => {
                StoredShape::Boxed(Box::new(inner.stored_shape(token_like, true)))
            }
            Self::Optional(inner) => {
                StoredShape::Optional(Box::new(inner.stored_shape(token_like, indirect)))
            }
            Self::Repeated(inner) => {
                StoredShape::Repeated(Box::new(inner.stored_shape(token_like, true)))
            }
        }
    }

    fn store_value(
        &self,
        value: TokenStream2,
        token_like: &BTreeSet<SyntaxTypeName>,
        indirect: bool,
    ) -> TokenStream2 {
        let stored = self.stored_shape(token_like, indirect);
        self.convert_value(&stored, value)
    }

    fn convert_value(&self, stored: &StoredShape, value: TokenStream2) -> TokenStream2 {
        match (self, stored) {
            (Self::Base(_, _), StoredShape::Base(_, _)) => value,
            (Self::Base(_, _), StoredShape::Boxed(inner))
                if matches!(inner.as_ref(), StoredShape::Base(_, _)) =>
            {
                quote!(Box::new(#value))
            }
            (Self::Boxed(source), StoredShape::Boxed(target)) => {
                let converted = source.convert_value(target, quote!(*value));
                quote!(Box::new({ let value = #value; #converted }))
            }
            (Self::Optional(source), StoredShape::Optional(target)) => {
                let converted = source.convert_value(target, quote!(value));
                quote!(#value.map(|value| #converted))
            }
            (Self::Repeated(source), StoredShape::Repeated(target)) => {
                let converted = source.convert_value(target, quote!(value));
                quote!(#value.into_iter().map(|value| #converted).collect())
            }
            _ => quote!(#value),
        }
    }

    fn cardinality(&self) -> Result<(Cardinality, &Self)> {
        match self {
            Self::Boxed(inner) => inner.cardinality(),
            Self::Optional(inner)
                if !matches!(inner.as_ref(), Self::Optional(_) | Self::Repeated(_)) =>
            {
                Ok((Cardinality::Optional, inner))
            }
            Self::Repeated(inner)
                if !matches!(inner.as_ref(), Self::Optional(_) | Self::Repeated(_)) =>
            {
                Ok((Cardinality::Repeated, inner))
            }
            Self::Optional(_) | Self::Repeated(_) => Err(Error::new(
                self.base_ident().span(),
                "nested Option and Vec fields need an explicit reconstruction strategy",
            )),
            Self::Base(_, _) => Ok((Cardinality::Required, self)),
        }
    }
}

impl StoredShape {
    fn ty(&self) -> TokenStream2 {
        match self {
            Self::Base(path, _) => quote!(#path),
            Self::Boxed(inner) => {
                let inner = inner.ty();
                quote!(Box<#inner>)
            }
            Self::Optional(inner) => {
                let inner = inner.ty();
                quote!(Option<#inner>)
            }
            Self::Repeated(inner) => {
                let inner = inner.ty();
                quote!(Vec<#inner>)
            }
        }
    }

    fn wrap_value(&self, value: TokenStream2) -> TokenStream2 {
        match self {
            Self::Base(_, _) => value,
            Self::Boxed(inner) => {
                let value = inner.wrap_value(value);
                quote!(Box::new(#value))
            }
            Self::Optional(_) | Self::Repeated(_) => value,
        }
    }

    fn visit(&self, value: TokenStream2) -> TokenStream2 {
        match self {
            Self::Base(_, ident) => {
                let method = format_ident!("visit_{}", to_snake_case(&ident.to_string()));
                quote!(visitor.#method(#value);)
            }
            Self::Boxed(inner) => inner.visit(quote!((#value).as_ref())),
            Self::Optional(inner) => {
                let visit = inner.visit(quote!(value));
                quote!(if let Some(value) = (#value).as_ref() { #visit })
            }
            Self::Repeated(inner) => {
                let visit = inner.visit(quote!(value));
                quote!(for value in #value { #visit })
            }
        }
    }

    fn visit_mut(&self, value: TokenStream2) -> TokenStream2 {
        match self {
            Self::Base(_, ident) => {
                let method = format_ident!("visit_{}_mut", to_snake_case(&ident.to_string()));
                quote!(visitor.#method(#value);)
            }
            Self::Boxed(inner) => inner.visit_mut(quote!((#value).as_mut())),
            Self::Optional(inner) => {
                let visit = inner.visit_mut(quote!(value));
                quote!(if let Some(value) = (#value).as_mut() { #visit })
            }
            Self::Repeated(inner) => {
                let visit = inner.visit_mut(quote!(value));
                quote!(for value in #value { #visit })
            }
        }
    }

    fn fold(&self, value: TokenStream2) -> TokenStream2 {
        match self {
            Self::Base(_, ident) => {
                let method = format_ident!("fold_{}", to_snake_case(&ident.to_string()));
                quote!(folder.#method(#value))
            }
            Self::Boxed(inner) => {
                let folded = inner.fold(quote!(*#value));
                quote!(Box::new(#folded))
            }
            Self::Optional(inner) => {
                let folded = inner.fold(quote!(value));
                quote!(#value.map(|value| #folded))
            }
            Self::Repeated(inner) => {
                let folded = inner.fold(quote!(value));
                quote!(#value.into_iter().map(|value| #folded).collect())
            }
        }
    }
}

fn expand_enum(definition: &EnumDefinition) -> TokenStream2 {
    let attrs = &definition.attrs;
    let visibility = &definition.visibility;
    let name = &definition.name;
    let variants = definition.variants.iter().map(|variant| {
        let attrs = &variant.attrs;
        let name = &variant.name;
        let ty = variant.shape.source_type();
        quote! { #(#attrs)* #name(#ty) }
    });
    let span_arms = definition.variants.iter().map(|variant| {
        let variant = &variant.name;
        quote!(Self::#variant(node) => ::m2_syn::Spanned::span(node))
    });
    let kind_arms = definition.variants.iter().map(|variant| {
        let variant = &variant.name;
        quote!(Self::#variant(node) => ::m2_syn::AstNode::kind(node))
    });
    let token_arms = definition.variants.iter().map(|variant| {
        let variant = &variant.name;
        quote!(Self::#variant(node) => ::m2_syn::ToTokens::to_tokens(node, output))
    });
    let match_checks = definition.variants.iter().map(|variant| {
        let base = variant.shape.base_ident();
        quote!(<#base as ::m2_syn::Reconstruct<N>>::matches(node))
    });
    let reconstruct_arms = definition.variants.iter().map(|variant| {
        let variant_name = &variant.name;
        let base = variant.shape.base_ident();
        quote! {
            if <#base as ::m2_syn::Reconstruct<N>>::matches(&node) {
                return Ok(Self::#variant_name(
                    <#base as ::m2_syn::Reconstruct<N>>::reconstruct(node)?
                ));
            }
        }
    });
    quote! {
        #(#attrs)*
        #[derive(Debug)]
        #visibility enum #name {
            #(#variants,)*
        }

        impl ::m2_syn::Spanned for #name {
            fn span(&self) -> ::m2_syn::Span {
                match self { #(#span_arms,)* }
            }
        }

        impl ::m2_syn::AstNode for #name {
            type Kind = SyntaxKind;

            fn kind(&self) -> SyntaxKind {
                match self { #(#kind_arms,)* }
            }
        }

        impl ::m2_syn::ToTokens for #name {
            fn to_tokens(&self, output: &mut ::m2_syn::TokenStream) {
                match self { #(#token_arms,)* }
            }
        }

        impl<N> ::m2_syn::Reconstruct<N> for #name
        where
            N: ::m2_syn::CstNode,
        {
            fn matches(node: &N) -> bool {
                false #(|| #match_checks)*
            }

            fn reconstruct(node: N) -> Result<Self, ::m2_syn::ReconstructError> {
                #(#reconstruct_arms)*
                Err(::m2_syn::ReconstructError::wrong_category(
                    stringify!(#name),
                    node.identity(),
                ))
            }
        }
    }
}

fn to_snake_case(name: &str) -> String {
    let characters = name.chars().collect::<Vec<_>>();
    let mut result = String::new();
    for (index, character) in characters.iter().copied().enumerate() {
        if character.is_uppercase()
            && index > 0
            && (characters[index - 1].is_lowercase()
                || characters
                    .get(index + 1)
                    .is_some_and(|next| next.is_lowercase()))
        {
            result.push('_');
        }
        result.extend(character.to_lowercase());
    }
    result
}
