#[path = "syntax/grammar.rs"]
mod grammar;
#[path = "syntax/traversal.rs"]
mod traversal;

use crate::tokens::{OperatorKind, TokenDefinitions};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::parse::{Parse, ParseStream};
use syn::{Attribute, Error, Member, Result, Token, Visibility};
use traversal::Traversal;

struct Syntax {
    tokens: TokenDefinitions,
    structs: Vec<StructDefinition>,
    enums: Vec<EnumDefinition>,
}

pub struct GeneratedSyntax {
    tokens: TokenStream,
    nodes: TokenStream,
    visit: TokenStream,
    visit_mut: TokenStream,
    fold: TokenStream,
}

impl GeneratedSyntax {
    #[allow(dead_code)]
    pub fn files(self) -> [(&'static str, TokenStream); 5] {
        [
            ("tokens.rs", self.tokens),
            ("nodes.rs", self.nodes),
            ("visit.rs", self.visit),
            ("visit_mut.rs", self.visit_mut),
            ("fold.rs", self.fold),
        ]
    }

    #[allow(dead_code)]
    pub fn combined(self) -> TokenStream {
        let Self {
            tokens,
            nodes,
            visit,
            visit_mut,
            fold,
        } = self;
        quote! {
            #tokens
            #nodes
            #visit
            #visit_mut
            #fold
        }
    }
}

pub fn generate(input: TokenStream) -> Result<GeneratedSyntax> {
    syn::parse2::<Syntax>(input)?.expand()
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SyntaxTypeName(String);

impl From<&Ident> for SyntaxTypeName {
    fn from(value: &Ident) -> Self {
        Self(value.to_string())
    }
}

type ConversionGraph = BTreeMap<(SyntaxTypeName, SyntaxTypeName), Vec<Vec<(Ident, Ident)>>>;

struct StructDefinition {
    attrs: Vec<Attribute>,
    visibility: Visibility,
    name: Ident,
    fields: StructFields,
    delimiter: Option<DelimiterKind>,
    cst_kind: Option<String>,
}

enum StructFields {
    Leaf,
    Product { fields: Vec<FieldDefinition> },
}

struct FieldDefinition {
    attrs: Vec<Attribute>,
    visibility: Visibility,
    member: Member,
    binding: Ident,
    source: FieldSource,
    shape: TypeShape,
    repeated_separator: &'static str,
    attached: bool,
}

enum FieldSource {
    Named(String),
    Unfielded,
}

#[derive(Clone, Copy)]
enum DelimiterKind {
    Parenthesis,
    Bracket,
    Brace,
    AngleBar,
    String,
    RawString,
}

impl DelimiterKind {
    fn field_separator(self) -> &'static str {
        match self {
            Self::String | Self::RawString => "",
            Self::Parenthesis | Self::Bracket | Self::Brace | Self::AngleBar => " ",
        }
    }

    fn typed(self) -> Option<TokenStream> {
        match self {
            Self::Parenthesis => Some(quote!(Delimiter![()])),
            Self::Bracket => Some(quote!(Delimiter![[]])),
            Self::Brace => Some(quote!(Delimiter![{}])),
            Self::AngleBar => Some(quote!(Delimiter![<| |>])),
            Self::String | Self::RawString => None,
        }
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
    fn operator(kind: OperatorKind, tokens: &TokenDefinitions) -> Self {
        let variants = tokens
            .operator_variants(kind)
            .map(|token| VariantDefinition {
                attrs: Vec::new(),
                name: token.name.clone(),
                shape: TypeShape::token(token.name.clone(), token.pattern.clone()),
            })
            .collect();
        Self {
            attrs: Vec::new(),
            visibility: syn::parse_quote!(pub),
            name: kind.enum_name(),
            variants,
        }
    }
}

#[derive(Clone)]
enum TypeShape {
    Base(TokenStream, Ident),
    Optional(Box<Self>),
    Repeated(Box<Self>),
}

impl TypeShape {
    fn base(ident: Ident) -> Self {
        Self::Base(quote!(#ident), ident)
    }

    fn token(ident: Ident, pattern: TokenStream) -> Self {
        Self::Base(quote!(Token![#pattern]), ident)
    }

    fn base_ident(&self) -> &Ident {
        match self {
            Self::Base(_, ident) => ident,
            Self::Optional(inner) | Self::Repeated(inner) => inner.base_ident(),
        }
    }
}

impl Parse for Syntax {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let tokens: TokenDefinitions = input.parse()?;
        let mut enums = [
            OperatorKind::Prefix,
            OperatorKind::Binary,
            OperatorKind::Postfix,
        ]
        .into_iter()
        .map(|kind| EnumDefinition::operator(kind, &tokens))
        .collect::<Vec<_>>();
        let (structs, grammar_enums) = grammar::parse(input, &tokens)?;
        enums.extend(grammar_enums);

        Ok(Self {
            tokens,
            structs,
            enums,
        })
    }
}
impl Syntax {
    fn expand(&self) -> Result<GeneratedSyntax> {
        let names = self.declared_names()?;
        self.validate_references(&names)?;
        let token_like = self.token_like_types();
        let tokens = self.tokens.expand();
        let structs = self
            .structs
            .iter()
            .map(|definition| self.expand_struct(definition, &token_like))
            .collect::<Result<Vec<_>>>()?;
        let enums = self
            .enums
            .iter()
            .map(|definition| expand_enum(definition, &self.tokens))
            .collect::<Vec<_>>();
        let conversions = self.expand_conversions()?;
        let visits = self.expand_visit(&token_like);
        let visit_muts = self.expand_visit_mut(&token_like);
        let folds = self.expand_fold(&token_like);

        Ok(GeneratedSyntax {
            tokens,
            nodes: quote! {
                #(#structs)*
                #(#enums)*
                #conversions
            },
            visit: visits,
            visit_mut: visit_muts,
            fold: folds,
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

    fn expand_struct(
        &self,
        definition: &StructDefinition,
        token_like: &BTreeSet<SyntaxTypeName>,
    ) -> Result<TokenStream> {
        let attrs = &definition.attrs;
        let visibility = &definition.visibility;
        let name = &definition.name;
        let kind = definition
            .cst_kind
            .clone()
            .unwrap_or_else(|| to_snake_case(&name.to_string()));
        match &definition.fields {
            StructFields::Leaf => {
                let emit = match name.to_string().as_str() {
                    "FloatLiteral" => quote! {
                        output.push_literal(::m2_syn::Literal::new(
                            ::m2_syn::LiteralKind::Float,
                            self.text.clone(),
                            ::m2_syn::Spanned::span(self),
                        ));
                    },
                    "IntegerLiteral" => quote! {
                        output.push_literal(::m2_syn::Literal::new(
                            ::m2_syn::LiteralKind::Integer,
                            self.text.clone(),
                            ::m2_syn::Spanned::span(self),
                        ));
                    },
                    "BlockComment" => quote! {
                        output.push_trivia(::m2_syn::Trivia::new(
                            ::m2_syn::TriviaKind::BlockComment,
                            &self.text,
                            ::m2_syn::Spanned::span(self),
                        ));
                    },
                    "LineComment" => quote! {
                        output.push_trivia(::m2_syn::Trivia::new(
                            ::m2_syn::TriviaKind::LineComment,
                            &self.text,
                            ::m2_syn::Spanned::span(self),
                        ));
                    },
                    _ => quote! {
                        output.push_ident(::m2_syn::IdentToken::new(
                            &self.text,
                            ::m2_syn::Spanned::span(self),
                        ));
                    },
                };
                Ok(quote! {

                #(#attrs)*
                #[derive(Debug, ::m2_syn::Spanned)]
                #visibility struct #name {
                    pub text: String,
                    span: ::m2_syn::Span,
                }

                impl #name {
                    pub fn new(text: impl Into<String>, span: ::m2_syn::Span) -> Self {
                        Self { text: text.into(), span }
                    }
                }

                impl ::m2_syn::ToTokens for #name {
                    fn to_tokens(&self, output: &mut ::m2_syn::TokenStream) {
                        #emit
                    }
                }

                impl<N> ::m2_syn::Reconstruct<N> for #name
                where
                    N: ::m2_syn::ExternalCstNode,
                {
                    fn matches(node: &N) -> bool {
                        node.identity().matches(#kind, true)
                    }

                    fn reconstruct(node: N) -> Result<Self, ::m2_syn::ReconstructError> {
                        if !<Self as ::m2_syn::Reconstruct<N>>::matches(&node) {
                            return Err(::m2_syn::ReconstructError::wrong_node(
                                #kind,
                                true,
                                node.identity(),
                            ));
                        }
                        Ok(Self::new(node.text(), node.span()))
                    }
                }

                })
            }

            StructFields::Product { fields } => {
                let structural_matches = if definition.cst_kind.is_some() {
                    fields
                        .iter()
                        .filter_map(FieldDefinition::required_match)
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let expanded_fields = fields.iter().map(|field| {
                    let attrs = &field.attrs;
                    let visibility = &field.visibility;
                    let member = &field.member;
                    let stored = field.shape.stored_type(token_like, false);
                    quote! { #(#attrs)* #visibility #member: #stored }
                });
                let typed_delimiter = definition.delimiter.and_then(DelimiterKind::typed);
                let delimiter_field = typed_delimiter
                    .as_ref()
                    .map(|ty| quote!(pub delimiter: #ty,));
                let struct_definition = quote! {
                    #visibility struct #name {
                        #(#expanded_fields,)*
                        #delimiter_field
                    }
                };
                let constructor_arguments = fields.iter().map(|field| {
                    let binding = &field.binding;
                    let ty = field.shape.source_type();
                    quote! { #binding: #ty }
                });
                let constructor_conversions = fields
                    .iter()
                    .filter_map(|field| {
                        let binding = &field.binding;
                        let stored = field.shape.stored_shape(token_like, false);
                        if field.shape.conversion_is_identity(&stored) {
                            None
                        } else {
                            let value = field.shape.convert_value(&stored, quote!(#binding));
                            Some(quote! { let #binding = #value; })
                        }
                    })
                    .collect::<Vec<_>>();
                let constructor_fields = fields.iter().map(|field| &field.binding);
                let constructor_delimiter = typed_delimiter.as_ref().map(|ty| {
                    quote!(let delimiter = <#ty as ::m2_syn::DelimiterToken>::new(
                        ::m2_syn::DoubleSpan::new(span, span),
                    );)
                });
                let delimiter_member = typed_delimiter.as_ref().map(|_| quote!(delimiter,));
                let constructor_value = if constructor_delimiter.is_some() {
                    quote!({
                        #constructor_delimiter
                        Self { #(#constructor_fields,)* #delimiter_member }
                    })
                } else {
                    quote!(Self { #(#constructor_fields,)* })
                };
                let constructor = if constructor_conversions.is_empty() {
                    constructor_value
                } else {
                    quote!({
                        #(#constructor_conversions)*
                        #constructor_value
                    })
                };
                let span_fields = fields.iter().map(|field| {
                    let binding = &field.binding;
                    quote! { ::m2_syn::Spanned::span(&#binding) }
                });
                let constructor_span = typed_delimiter
                    .as_ref()
                    .map(|_| quote!(let span = ::m2_syn::Span::join_all([#(#span_fields),*]);));
                let reconstruct_fields = fields
                    .iter()
                    .map(|field| field.reconstruct(token_like))
                    .collect::<Result<Vec<_>>>()?;
                let reconstructed_fields = fields.iter().map(|field| &field.binding);
                let reconstruct_delimiter = typed_delimiter.as_ref().map(|ty| {
                    quote!(let delimiter = <#ty as ::m2_syn::DelimiterToken>::new(
                        ::m2_syn::DoubleSpan::new(span, span),
                    );)
                });
                let reconstruct_span = typed_delimiter
                    .as_ref()
                    .map(|_| quote!(let span = node.span();));
                let reconstructed = if reconstruct_delimiter.is_some() {
                    quote!({
                        #reconstruct_delimiter
                        Self { #(#reconstructed_fields,)* #delimiter_member }
                    })
                } else {
                    quote!(Self { #(#reconstructed_fields,)* })
                };
                let field_separator = definition
                    .delimiter
                    .map_or(" ", DelimiterKind::field_separator);
                let print_fields = fields
                    .iter()
                    .map(|field| field.to_tokens(field_separator))
                    .collect::<Result<Vec<_>>>()?;
                let emit_contents = match definition.delimiter {
                    Some(DelimiterKind::String) => quote! {
                        output.push_literal(::m2_syn::Literal::new(
                            ::m2_syn::LiteralKind::String,
                            ::std::format!("\"{}\"", contents),
                            ::m2_syn::Spanned::span(self),
                        ));
                    },
                    Some(DelimiterKind::RawString) => quote! {
                        output.push_literal(::m2_syn::Literal::new(
                            ::m2_syn::LiteralKind::RawString,
                            ::std::format!("///{}///", contents),
                            ::m2_syn::Spanned::span(self),
                        ));
                    },
                    Some(_) => quote! {
                        ::m2_syn::DelimiterToken::surround(
                            &self.delimiter,
                            output,
                            contents,
                        );
                    },
                    None => quote! {
                        ::m2_syn::ToTokens::to_tokens(&contents, output);
                    },
                };
                Ok(quote! {
                    #(#attrs)*
                    #[derive(Debug, ::m2_syn::Spanned)]
                    #struct_definition

                    impl #name {
                        pub fn new(#(#constructor_arguments),*) -> Self {
                            #constructor_span
                            #constructor
                        }
                    }
                    impl ::m2_syn::ToTokens for #name {
                        fn to_tokens(&self, output: &mut ::m2_syn::TokenStream) {
                            let mut contents = ::m2_syn::TokenStream::new();
                            #(#print_fields)*
                            #emit_contents
                        }
                    }

                    impl<N> ::m2_syn::Reconstruct<N> for #name
                    where
                        N: ::m2_syn::ExternalCstNode,
                    {
                        fn matches(node: &N) -> bool {
                            node.identity().matches(#kind, true)
                                #(&& #structural_matches)*
                        }

                        fn reconstruct(node: N) -> Result<Self, ::m2_syn::ReconstructError> {
                            if !<Self as ::m2_syn::Reconstruct<N>>::matches(&node) {
                                return Err(::m2_syn::ReconstructError::wrong_node(
                                    #kind,
                                    true,
                                    node.identity(),
                                ));
                            }
                            #reconstruct_span
                            let mut children = ::m2_syn::ChildCursor::new(&node);
                            #(#reconstruct_fields)*
                            Ok(#reconstructed)
                        }
                    }
                })
            }
        }
    }

    fn expand_conversions(&self) -> Result<TokenStream> {
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

        let declarations = self.declaration_types();
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

    fn declaration_types(&self) -> BTreeMap<SyntaxTypeName, TokenStream> {
        self.all_types()
            .map(|name| (SyntaxTypeName::from(name), self.type_reference(name)))
            .collect()
    }

    fn type_reference(&self, name: &Ident) -> TokenStream {
        if let Some(pattern) = self.tokens.pattern(name) {
            quote!(Token![#pattern])
        } else {
            quote!(#name)
        }
    }

    fn expand_visit(&self, token_like: &BTreeSet<SyntaxTypeName>) -> TokenStream {
        let methods = self.all_types().map(|name| {
            let method = format_ident!("visit_{}", to_snake_case(&name.to_string()));
            let ty = self.type_reference(name);
            quote! {
                fn #method(&mut self, node: &'ast #ty) {
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

    fn expand_visit_mut(&self, token_like: &BTreeSet<SyntaxTypeName>) -> TokenStream {
        let methods = self.all_types().map(|name| {
            let method = format_ident!("visit_{}_mut", to_snake_case(&name.to_string()));
            let ty = self.type_reference(name);
            quote! {
                fn #method(&mut self, node: &mut #ty) {
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

    fn expand_fold(&self, token_like: &BTreeSet<SyntaxTypeName>) -> TokenStream {
        let methods = self.all_types().map(|name| {
            let method = format_ident!("fold_{}", to_snake_case(&name.to_string()));
            let ty = self.type_reference(name);
            quote! {
                fn #method(&mut self, node: #ty) -> #ty {
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
    ) -> Vec<TokenStream> {
        let tokens = self.tokens.iter().map(|definition| {
            let ty = self.type_reference(&definition.name);
            traversal.empty_walker(&definition.name, ty)
        });
        let structs = self.structs.iter().map(|definition| {
            let name = &definition.name;
            match &definition.fields {
                StructFields::Leaf => traversal.empty_walker(name, quote!(#name)),
                StructFields::Product { fields } => traversal.struct_walker(
                    name,
                    fields,
                    token_like,
                    definition
                        .delimiter
                        .and_then(DelimiterKind::typed)
                        .is_some(),
                ),
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
    fn required_match(&self) -> Option<TokenStream> {
        let (Cardinality::Required, element) = self.shape.cardinality().ok()? else {
            return None;
        };
        let ty = element.source_type();
        let field_match = match &self.source {
            FieldSource::Named(field) => quote!(child.field == Some(#field)),
            FieldSource::Unfielded => quote!(child.field.is_none()),
        };
        Some(quote!(node.children().any(|child| {
            #field_match && <#ty as ::m2_syn::Reconstruct<N>>::matches(&child.node)
        })))
    }

    fn reconstruct(&self, token_like: &BTreeSet<SyntaxTypeName>) -> Result<TokenStream> {
        let binding = &self.binding;
        let (cardinality, element) = self.shape.cardinality()?;
        let base_type = element.source_type();
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
                quote!(children.required_matching::<#base_type>()?)
            }
            (FieldSource::Unfielded, Cardinality::Optional) => {
                quote!(children.optional_matching::<#base_type>())
            }
            (FieldSource::Unfielded, Cardinality::Repeated) => {
                quote!(children.repeated_matching::<#base_type>())
            }
        };
        let stored_element = element.stored_shape(token_like, cardinality == Cardinality::Repeated);
        Ok(match cardinality {
            Cardinality::Required => {
                let reconstructed = quote!(
                    <#base_type as ::m2_syn::Reconstruct<N>>::reconstruct(#selected)?
                );
                let value = stored_element.wrap_value(reconstructed);
                quote!(let #binding = #value;)
            }
            Cardinality::Optional => {
                let value = stored_element.wrap_value(quote!(value));
                quote!(let #binding = #selected
                    .map(|node| {
                        let value = <#base_type as ::m2_syn::Reconstruct<N>>::reconstruct(node)?;
                        Ok::<_, ::m2_syn::ReconstructError>(#value)
                    })
                    .transpose()?;)
            }
            Cardinality::Repeated => {
                let value = stored_element.wrap_value(quote!(value));
                quote!(let #binding = #selected
                    .into_iter()
                    .map(|node| {
                        let value = <#base_type as ::m2_syn::Reconstruct<N>>::reconstruct(node)?;
                        Ok::<_, ::m2_syn::ReconstructError>(#value)
                    })
                    .collect::<Result<::std::vec::Vec<_>, _>>()?;)
            }
        })
    }

    fn to_tokens(&self, field_separator: &'static str) -> Result<TokenStream> {
        let member = &self.member;
        let (cardinality, _) = self.shape.cardinality()?;
        let write = match cardinality {
            Cardinality::Required | Cardinality::Optional => quote! {
                ::m2_syn::ToTokens::to_tokens(&self.#member, &mut field_output);
            },
            Cardinality::Repeated => {
                let separator = if field_separator.is_empty() && self.repeated_separator == " " {
                    ""
                } else {
                    self.repeated_separator
                };
                if separator.is_empty() {
                    quote! {
                        for value in &self.#member {
                            ::m2_syn::ToTokens::to_tokens(value, &mut field_output);
                        }
                    }
                } else {
                    let push_separator = push_detached_separator(quote!(field_output), separator);
                    quote! {
                        for (index, value) in self.#member.iter().enumerate() {
                            if index != 0 {
                                #push_separator
                            }
                            ::m2_syn::ToTokens::to_tokens(value, &mut field_output);
                        }
                    }
                }
            }
        };
        let separator = (!self.attached && !field_separator.is_empty()).then_some(field_separator);
        let separate = separator.map(|separator| {
            let push_separator = push_detached_separator(quote!(contents), separator);
            if separator == " " {
                quote! {
                    if !contents.is_empty()
                        && !contents.ends_with_whitespace()
                        && !field_output.starts_with_whitespace()
                    {
                        #push_separator
                    }
                }
            } else {
                quote! {
                    if !contents.is_empty() {
                        #push_separator
                    }
                }
            }
        });
        Ok(quote! {
            {
                let mut field_output = ::m2_syn::TokenStream::new();
                #write
                if !field_output.is_empty() {
                    #separate
                    ::m2_syn::ToTokens::to_tokens(&field_output, &mut contents);
                }
            }
        })
    }
}

fn push_detached_separator(output: TokenStream, separator: &str) -> TokenStream {
    match separator {
        "" => quote! {},
        " " => quote! {
            #output.push_trivia(::m2_syn::Trivia::new(
                ::m2_syn::TriviaKind::Whitespace,
                " ",
                ::m2_syn::Span::detached(),
            ));
        },
        "\n" => quote! {
            #output.push_trivia(::m2_syn::Trivia::new(
                ::m2_syn::TriviaKind::Whitespace,
                "\n",
                ::m2_syn::Span::detached(),
            ));
        },
        ", " => quote! {
            #output.push_punct(::m2_syn::Punct::new(
                ",",
                ::m2_syn::Span::detached(),
            ));
            #output.push_trivia(::m2_syn::Trivia::new(
                ::m2_syn::TriviaKind::Whitespace,
                " ",
                ::m2_syn::Span::detached(),
            ));
        },
        separator => panic!("unsupported generated separator `{separator}`"),
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
    Base(TokenStream, Ident),
    Boxed(Box<Self>),
    Optional(Box<Self>),
    Repeated(Box<Self>),
}

impl TypeShape {
    fn source_type(&self) -> TokenStream {
        match self {
            Self::Base(path, _) => quote!(#path),
            Self::Optional(inner) => {
                let inner = inner.source_type();
                quote!(::std::option::Option<#inner>)
            }
            Self::Repeated(inner) => {
                let inner = inner.source_type();
                quote!(::std::vec::Vec<#inner>)
            }
        }
    }

    fn stored_type(&self, token_like: &BTreeSet<SyntaxTypeName>, indirect: bool) -> TokenStream {
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
            Self::Optional(inner) => {
                StoredShape::Optional(Box::new(inner.stored_shape(token_like, indirect)))
            }
            Self::Repeated(inner) => {
                StoredShape::Repeated(Box::new(inner.stored_shape(token_like, true)))
            }
        }
    }

    fn convert_value(&self, stored: &StoredShape, value: TokenStream) -> TokenStream {
        if self.conversion_is_identity(stored) {
            return value;
        }
        match (self, stored) {
            (Self::Base(_, _), StoredShape::Boxed(inner))
                if matches!(inner.as_ref(), StoredShape::Base(_, _)) =>
            {
                quote!(::std::boxed::Box::new(#value))
            }
            (Self::Optional(source), StoredShape::Optional(target)) => {
                if matches!(
                    (source.as_ref(), target.as_ref()),
                    (
                        Self::Base(_, _),
                        StoredShape::Boxed(inner)
                    ) if matches!(inner.as_ref(), StoredShape::Base(_, _))
                ) {
                    quote!(#value.map(::std::boxed::Box::new))
                } else {
                    let converted = source.convert_value(target, quote!(value));
                    quote!(#value.map(|value| #converted))
                }
            }
            (Self::Repeated(source), StoredShape::Repeated(target)) => {
                let converted = source.convert_value(target, quote!(value));
                quote!({
                    let values = #value;
                    let mut converted_values = ::std::vec::Vec::with_capacity(values.len());
                    for value in values {
                        converted_values.push(#converted);
                    }
                    converted_values
                })
            }
            _ => quote!(#value),
        }
    }

    fn conversion_is_identity(&self, stored: &StoredShape) -> bool {
        match (self, stored) {
            (Self::Base(_, _), StoredShape::Base(_, _)) => true,
            (Self::Optional(source), StoredShape::Optional(target))
            | (Self::Repeated(source), StoredShape::Repeated(target)) => {
                source.conversion_is_identity(target)
            }
            _ => false,
        }
    }

    fn cardinality(&self) -> Result<(Cardinality, &Self)> {
        match self {
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
    fn ty(&self) -> TokenStream {
        match self {
            Self::Base(path, _) => quote!(#path),
            Self::Boxed(inner) => {
                let inner = inner.ty();
                quote!(::std::boxed::Box<#inner>)
            }
            Self::Optional(inner) => {
                let inner = inner.ty();
                quote!(::std::option::Option<#inner>)
            }
            Self::Repeated(inner) => {
                let inner = inner.ty();
                quote!(::std::vec::Vec<#inner>)
            }
        }
    }

    fn wrap_value(&self, value: TokenStream) -> TokenStream {
        match self {
            Self::Base(_, _) => value,
            Self::Boxed(inner) => {
                let value = inner.wrap_value(value);
                quote!(::std::boxed::Box::new(#value))
            }
            Self::Optional(_) | Self::Repeated(_) => value,
        }
    }

    fn visit(&self, value: TokenStream) -> TokenStream {
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

    fn visit_mut(&self, value: TokenStream) -> TokenStream {
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

    fn fold(&self, value: TokenStream) -> TokenStream {
        match self {
            Self::Base(_, ident) => {
                let method = format_ident!("fold_{}", to_snake_case(&ident.to_string()));
                quote!(folder.#method(#value))
            }
            Self::Boxed(inner) => {
                let folded = inner.fold(quote!(*#value));
                quote!(::std::boxed::Box::new(#folded))
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

fn expand_enum(definition: &EnumDefinition, tokens: &TokenDefinitions) -> TokenStream {
    let attrs = &definition.attrs;
    let visibility = &definition.visibility;
    let name = &definition.name;
    let variants = definition.variants.iter().map(|variant| {
        let attrs = &variant.attrs;
        let name = &variant.name;
        let ty = variant.shape.source_type();
        quote! { #(#attrs)* #name(#ty) }
    });
    let token_arms = definition.variants.iter().map(|variant| {
        let variant = &variant.name;
        quote!(Self::#variant(node) => ::m2_syn::ToTokens::to_tokens(node, output))
    });
    let matches = definition
        .variants
        .iter()
        .map(|variant| {
            let base = variant.shape.source_type();
            quote!(<#base as ::m2_syn::Reconstruct<N>>::matches(node))
        })
        .reduce(|left, right| quote!(#left || #right))
        .unwrap_or_else(|| quote!(false));
    let reconstruct_arms = definition.variants.iter().map(|variant| {
        let variant_name = &variant.name;
        let base = variant.shape.source_type();
        quote! {
            if <#base as ::m2_syn::Reconstruct<N>>::matches(&node) {
                return Ok(Self::#variant_name(
                    <#base as ::m2_syn::Reconstruct<N>>::reconstruct(node)?
                ));
            }
        }
    });
    let borrowed_fallback = definition
        .variants
        .is_empty()
        .then(|| quote!(_ => unreachable!("empty generated syntax category")));
    let spelling_api = [
        OperatorKind::Prefix,
        OperatorKind::Binary,
        OperatorKind::Postfix,
    ]
    .into_iter()
    .find(|kind| kind.enum_name() == *name)
    .map(|_| {
        let from_spelling_arms = definition.variants.iter().map(|variant| {
            let variant_name = &variant.name;
            let ty = variant.shape.source_type();
            let spelling = tokens
                .spelling(variant.shape.base_ident())
                .expect("operator variants are generated from tokens");
            quote!(#spelling => Some(Self::#variant_name(#ty(span))))
        });
        let from_token_arms = definition.variants.iter().map(|variant| {
            let variant_name = &variant.name;
            let ty = variant.shape.source_type();
            quote! {
                if <#ty as ::m2_syn::Token>::matches_token_tree(&token) {
                    return Some(Self::#variant_name(
                        <#ty as ::m2_syn::Token>::from_token_tree(token)?
                    ));
                }
            }
        });
        let spelling_arms = definition.variants.iter().map(|variant| {
            let variant_name = &variant.name;
            let spelling = tokens
                .spelling(variant.shape.base_ident())
                .expect("operator variants are generated from tokens");
            quote!(Self::#variant_name(_) => #spelling)
        });
        quote! {
            impl #name {
                pub fn from_spelling(
                    spelling: &str,
                    span: ::m2_syn::Span,
                ) -> ::std::option::Option<Self> {
                    match spelling {
                        #(#from_spelling_arms,)*
                        _ => None,
                    }
                }

                pub fn from_token_tree(token: ::m2_syn::TokenTree) -> ::std::option::Option<Self> {
                    #(#from_token_arms)*
                    None
                }

                pub fn spelling(&self) -> &'static str {
                    match self {
                        #(#spelling_arms,)*
                        #borrowed_fallback
                    }
                }
            }
        }
    });
    quote! {
        #(#attrs)*
        #[derive(Debug, ::m2_syn::Spanned)]
        #visibility enum #name {
            #(#variants,)*
        }

        impl ::m2_syn::ToTokens for #name {
            fn to_tokens(&self, output: &mut ::m2_syn::TokenStream) {
                match self { #(#token_arms,)* #borrowed_fallback }
            }
        }

        impl<N> ::m2_syn::Reconstruct<N> for #name
        where
            N: ::m2_syn::ExternalCstNode,
        {
            fn matches(node: &N) -> bool {
                #matches
            }

            fn reconstruct(node: N) -> Result<Self, ::m2_syn::ReconstructError> {
                #(#reconstruct_arms)*
                Err(::m2_syn::ReconstructError::wrong_category(
                    stringify!(#name),
                    node.identity(),
                ))
            }
        }

        #spelling_api
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

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::parse2;

    #[test]
    fn staged_tokens_feed_one_lookup_macro_and_operator_enums() {
        let syntax: Syntax = parse2(quote! {
            tokens {
                [+] {pref, bin, aug}
                [!] {post}
            }
            keywords: { [if] }
            markers: {}
            punct: { [,] }

            Leaf ::= leaf
        })
        .unwrap();
        let expansion = syntax.expand().unwrap().combined();
        parse2::<syn::File>(expansion.clone()).unwrap();
        let expansion = expansion.to_string();

        assert_eq!(expansion.matches("macro_rules ! Token").count(), 1);
        assert!(expansion.contains("enum PrefixOperator"));
        assert!(expansion.contains("enum BinaryOperator"));
        assert!(expansion.contains("enum PostfixOperator"));
        assert!(expansion.contains("AddEql"));
        assert!(expansion.contains("IfKeyword"));
        assert!(expansion.contains("Cma"));
    }
}
