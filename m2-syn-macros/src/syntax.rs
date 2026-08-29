#[path = "syntax/grammar.rs"]
mod grammar;
#[path = "syntax/traversal.rs"]
mod traversal;

use crate::tokens::{OperatorKind, TokenDefinitions};
use proc_macro2::{Ident, TokenStream};
use quote::{ToTokens, format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::parse::{Parse, ParseStream};
use syn::{Error, Member, Result, parse2};
use traversal::Traversal;

struct Syntax {
    tokens: TokenDefinitions,
    structs: Vec<StructDefinition>,
    enums: Vec<EnumDefinition>,
    declarations: Vec<TokenStream>,
}

#[derive(Clone, Copy)]
pub(crate) struct SyntaxFeatures {
    pretty: bool,
    walk: bool,
}

impl SyntaxFeatures {
    pub(crate) const CORE: Self = Self {
        pretty: false,
        walk: false,
    };
    pub(crate) const PRETTY: Self = Self {
        pretty: true,
        walk: false,
    };
    pub(crate) const WALK: Self = Self {
        pretty: false,
        walk: true,
    };
    pub(crate) const PRETTY_WALK: Self = Self {
        pretty: true,
        walk: true,
    };
}

/// Whole-declaration facts, derived once and shared by every expansion stage.
///
/// Each fact needs the complete declaration to compute — which spelling a name
/// resolves to, whether a type is stored inline, how a variant reaches its
/// enum — so no helper can derive one from the fragment it is expanding.
/// Passing the context rather than the individual answers is what lets helpers
/// live in whichever file suits them.
pub(crate) struct SyntaxContext<'syntax> {
    tokens: &'syntax TokenDefinitions,
    features: SyntaxFeatures,
    /// Types whose declarations request indirection at inline use sites.
    boxed: BTreeSet<SyntaxTypeName>,
    /// How each declared name is written in generated code, which is
    /// `Token![…]` for a spelled token and the bare name otherwise.
    declarations: BTreeMap<SyntaxTypeName, TokenStream>,
    /// CST node kinds more than one struct migrates from, where the kind
    /// alone cannot say which type a node is.
    shared_kinds: BTreeSet<String>,
}

impl<'syntax> SyntaxContext<'syntax> {
    fn new(syntax: &'syntax Syntax, features: SyntaxFeatures) -> Self {
        let tokens = &syntax.tokens;
        let declarations = syntax
            .all_types()
            .map(|name| (SyntaxTypeName::from(name), type_reference(tokens, name)))
            .collect();
        let mut claims: BTreeMap<String, usize> = BTreeMap::new();
        for definition in &syntax.structs {
            *claims.entry(definition.cst_kind()).or_default() += 1;
        }
        Self {
            tokens,
            features,
            boxed: syntax
                .structs
                .iter()
                .filter(|definition| definition.boxed)
                .map(|definition| (&definition.name).into())
                .chain(
                    syntax
                        .enums
                        .iter()
                        .filter(|definition| definition.boxed)
                        .map(|definition| (&definition.name).into()),
                )
                .collect(),
            declarations,
            shared_kinds: claims
                .into_iter()
                .filter(|(_, claimants)| *claimants > 1)
                .map(|(kind, _)| kind)
                .collect(),
        }
    }

    /// Whether a node's kind is claimed by several structs, so that matching
    /// has to inspect its children to tell them apart.
    fn kind_is_shared(&self, kind: &str) -> bool {
        self.shared_kinds.contains(kind)
    }

    pub(crate) fn tokens(&self) -> &TokenDefinitions {
        self.tokens
    }

    fn pretty(&self) -> bool {
        self.features.pretty
    }

    pub(crate) fn is_boxed(&self, name: &Ident) -> bool {
        self.boxed.contains(&name.into())
    }

    pub(crate) fn type_reference(&self, name: &Ident) -> TokenStream {
        type_reference(self.tokens, name)
    }

    fn declaration(&self, name: &SyntaxTypeName) -> Option<&TokenStream> {
        self.declarations.get(name)
    }
}

fn type_reference(tokens: &TokenDefinitions, name: &Ident) -> TokenStream {
    if let Some(pattern) = tokens.pattern(name) {
        quote!(::m2_syn::Token![#pattern])
    } else {
        quote!(#name)
    }
}

pub(crate) fn generate(input: TokenStream, features: SyntaxFeatures) -> Result<TokenStream> {
    parse2::<Syntax>(input)?.expand(features)
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SyntaxTypeName(String);

impl From<&Ident> for SyntaxTypeName {
    fn from(value: &Ident) -> Self {
        Self(value.to_string())
    }
}

type ConversionGraph = BTreeMap<(SyntaxTypeName, SyntaxTypeName), Vec<Vec<(Ident, Ident, bool)>>>;

struct StructDefinition {
    name: Ident,
    fields: StructFields,
    wrapper: Option<Wrapper>,
    cst_kind: Option<String>,
    boxed: bool,
    cell_stream: bool,
    comments: bool,
    explicit: bool,
}

enum StructFields {
    Leaf(LeafKind),
    Product {
        fields: Vec<FieldDefinition>,
        style: ProductStyle,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProductStyle {
    Named,
    Tuple,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LeafKind {
    Ident,
    QualifiedIdent,
    Adj,
    Empty,
    Eoc,
    Eof,
    Float,
    Integer,
}

struct FieldDefinition {
    member: Member,
    binding: Ident,
    source: FieldSource,
    shape: Shape,
}

enum FieldSource {
    Named(String),
    Positional,
}

impl StructDefinition {
    /// The CST node this struct migrates from, which is its own name in
    /// snake case unless the grammar spells it differently.
    fn cst_kind(&self) -> String {
        self.cst_kind
            .clone()
            .unwrap_or_else(|| to_snake_case(&self.name.to_string()))
    }
}

/// How a product's contents are wrapped when printed: enclosed by a delimiter
/// pair, or spelled as a string literal.
///
/// These are separate because only the first is a *delimiter* — a pair of real
/// tokens the node stores and reproduces. A literal has no such tokens; its
/// quoting is part of how the literal itself is spelled.
#[derive(Clone, Copy)]
enum Wrapper {
    Delimited(Delimiter),
    Literal(Literal),
}

#[derive(Clone, Copy)]
enum Delimiter {
    Parenthesis,
    Bracket,
    Brace,
    AngleBar,
}

#[derive(Clone, Copy)]
enum Literal {
    String,
    RawString,
}

impl Wrapper {
    /// What separates this product's fields when printed. A literal's parts
    /// are its own text, so nothing goes between them.
    fn field_separator(self) -> &'static str {
        match self {
            Self::Delimited(_) => " ",
            Self::Literal(_) => "",
        }
    }

    fn delimiter(self) -> Option<Delimiter> {
        match self {
            Self::Delimited(delimiter) => Some(delimiter),
            Self::Literal(_) => None,
        }
    }
}

impl Delimiter {
    fn typed(self) -> TokenStream {
        match self {
            Self::Parenthesis => quote!(::m2_syn::Delimiter![()]),
            Self::Bracket => quote!(::m2_syn::Delimiter![[]]),
            Self::Brace => quote!(::m2_syn::Delimiter![{}]),
            Self::AngleBar => quote!(::m2_syn::Delimiter![<| |>]),
        }
    }
}

struct EnumDefinition {
    name: Ident,
    variants: Vec<VariantDefinition>,
    operator: Option<OperatorKind>,
    boxed: bool,
    explicit: bool,
}

struct VariantDefinition {
    name: Ident,
    shape: Shape,
}

impl EnumDefinition {
    fn operator(name: Ident, kind: OperatorKind, tokens: &TokenDefinitions) -> Self {
        let variants = tokens
            .operator_variants(kind)
            .map(|token| VariantDefinition {
                name: tokens.variant_name(&token.name),
                shape: Shape::token(token.name.clone(), &token.pattern),
            })
            .collect();
        Self {
            name,
            variants,
            operator: Some(kind),
            boxed: false,
            explicit: false,
        }
    }
}

/// A field's type as a stack of known functors over one syntax type.
///
/// The grammar only ever produces `Base`, `Optional`, `Repeated` and
/// `Punctuated`; [`Self::stored_shape`] adds `Boxed` where a field would
/// otherwise make its owner unsized. Both are the same grammar, so one lifting
/// covers both.
#[derive(Clone)]
enum Shape {
    Base(TokenStream, Ident),
    Boxed(Box<Self>),
    Delimited(Box<Self>),
    Optional(Box<Self>),
    /// A repetition, carrying the separator it prints between elements.
    Repeated(Box<Self>, &'static str),
    Punctuated(Box<Self>),
}

impl Shape {
    fn base(ident: Ident) -> Self {
        Self::Base(quote!(#ident), ident)
    }

    fn token(ident: Ident, pattern: &TokenStream) -> Self {
        Self::Base(quote!(::m2_syn::Token![#pattern]), ident)
    }

    fn base_ident(&self) -> &Ident {
        match self {
            Self::Base(_, ident) => ident,
            Self::Boxed(inner)
            | Self::Delimited(inner)
            | Self::Optional(inner)
            | Self::Repeated(inner, _)
            | Self::Punctuated(inner) => inner.base_ident(),
        }
    }
}

impl Parse for Syntax {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let tokens: TokenDefinitions = input.parse()?;
        let (structs, mut enums, declarations) = grammar::parse(input, &tokens)?;
        complete_operator_categories(&structs, &mut enums, &tokens);

        Ok(Self {
            tokens,
            structs,
            enums,
            declarations,
        })
    }
}

fn complete_operator_categories(
    structs: &[StructDefinition],
    enums: &mut Vec<EnumDefinition>,
    tokens: &TokenDefinitions,
) {
    for kind in OperatorKind::ALL {
        let category = kind.category_name();
        if let Some(definition) = enums.iter_mut().find(|node| node.name == category) {
            if definition.variants.is_empty() && definition.operator.is_none() {
                *definition = EnumDefinition::operator(definition.name.clone(), kind, tokens);
            }
        } else if syntax_references(structs, enums, category) {
            enums.push(EnumDefinition::operator(
                Ident::new(category, proc_macro2::Span::call_site()),
                kind,
                tokens,
            ));
        }
    }
}

fn syntax_references(structs: &[StructDefinition], enums: &[EnumDefinition], name: &str) -> bool {
    structs
        .iter()
        .filter_map(|definition| match &definition.fields {
            StructFields::Leaf(_) => None,
            StructFields::Product { fields, .. } => Some(fields.iter()),
        })
        .flatten()
        .any(|field| field.shape.base_ident() == name)
        || enums.iter().any(|definition| {
            definition
                .variants
                .iter()
                .any(|variant| variant.shape.base_ident() == name)
        })
}

impl Syntax {
    fn expand(&self, features: SyntaxFeatures) -> Result<TokenStream> {
        let names = self.declared_names()?;
        self.validate_references(&names)?;
        let context = SyntaxContext::new(self, features);
        let tokens = self.tokens.expand(features.pretty);
        let declarations = &self.declarations;
        let structs = self
            .structs
            .iter()
            .map(|definition| Self::expand_struct(definition, &context))
            .collect::<Result<Vec<_>>>()?;
        let enums = self
            .enums
            .iter()
            .map(|definition| expand_enum(definition, &context))
            .collect::<Vec<_>>();
        let conversions = self.expand_conversions(&context);
        let visits = features.walk.then(|| self.expand_visit(&context));
        let visit_muts = features.walk.then(|| self.expand_visit_mut(&context));
        let folds = features.walk.then(|| self.expand_fold(&context));

        Ok(quote! {
            #tokens
            use ::m2_syn::qf;
            use ::m2_syn::{Delimited, Punctuated, Token};
            #(#declarations)*
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
                return Err(Error::new(
                    name.span(),
                    format!("duplicate syntax type name: {name}"),
                ));
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
                    StructFields::Leaf(_) => None,
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

    fn expand_struct(
        definition: &StructDefinition,
        context: &SyntaxContext<'_>,
    ) -> Result<TokenStream> {
        let name = &definition.name;
        let kind = definition.cst_kind();
        match &definition.fields {
            StructFields::Leaf(leaf) => {
                if definition.comments {
                    return Err(Error::new(
                        definition.name.span(),
                        "`#[comments]` requires a named product struct",
                    ));
                }
                let output = format_ident!("output");
                let emit = match leaf {
                    LeafKind::Empty => quote! {},
                    LeafKind::Adj => quote! {
                        output.extend([::m2_syn::TokenTree::Punct(
                            ::m2_syn::Punct::from_span(::m2_syn::Spanned::expect_span(self)),
                        )]);
                    },
                    LeafKind::Eoc | LeafKind::Eof => quote! {},
                    LeafKind::Float => quote! {
                        output.extend([::m2_syn::LiteralToken::from_span(
                            ::m2_syn::LiteralKind::Float,
                            ::m2_syn::Spanned::expect_span(self),
                        )]);
                    },
                    LeafKind::Integer => quote! {
                        output.extend([::m2_syn::LiteralToken::from_span(
                            ::m2_syn::LiteralKind::Integer,
                            ::m2_syn::Spanned::expect_span(self),
                        )]);
                    },
                    LeafKind::Ident | LeafKind::QualifiedIdent => quote! {
                        output.extend([::m2_syn::IdentToken::from_span(
                            ::m2_syn::Spanned::expect_span(self),
                        )]);
                    },
                };
                let pretty_text = if matches!(leaf, LeafKind::Empty) {
                    quote!("∅")
                } else if matches!(leaf, LeafKind::Adj | LeafKind::Eoc | LeafKind::Eof) {
                    let marker = match leaf {
                        LeafKind::Adj => "∅",
                        LeafKind::Eoc => "EOC",
                        LeafKind::Eof => "EOF",
                        _ => unreachable!(),
                    };
                    quote!(#marker)
                } else {
                    quote!(self.span.text().expect("textual syntax spans must resolve"))
                };
                let qualifier_fields = matches!(leaf, LeafKind::QualifiedIdent).then(|| {
                    quote! {
                        /// The package-name portion before the first `$`, when this is a package-qualified symbol.
                        pub package_qualifier: qf!(Option<::m2_syn::Span>),
                        /// The decimal index after the final `$`, when this is an indexed symbol.
                        pub index_qualifier: qf!(Option<::m2_syn::Span>),
                    }
                });
                let constructor = match leaf {
                    LeafKind::QualifiedIdent => quote! {
                        let text = text.as_ref();
                        let span = span.with_text(text);
                        let (package_qualifier, index_qualifier) =
                            ::m2_syn::__qualified_identifier_spans(text, &span);
                        Self {
                            package_qualifier,
                            index_qualifier,
                            span,
                            leading_trivia: ::m2_syn::LeadingTrivia::default(),
                            trailing_trivia: ::m2_syn::TrailingTrivia::default(),
                        }
                    },
                    LeafKind::Empty | LeafKind::Adj | LeafKind::Eoc | LeafKind::Eof => quote! {
                        let _ = text;
                        Self {
                            span,
                            leading_trivia: ::m2_syn::LeadingTrivia::default(),
                            trailing_trivia: ::m2_syn::TrailingTrivia::default(),
                        }
                    },
                    _ => quote! {
                        let span = span.with_text(text.as_ref());
                        Self {
                            span,
                            leading_trivia: ::m2_syn::LeadingTrivia::default(),
                            trailing_trivia: ::m2_syn::TrailingTrivia::default(),
                        }
                    },
                };
                let text_api = match leaf {
                    LeafKind::Empty | LeafKind::Adj | LeafKind::Eoc | LeafKind::Eof => quote! {},
                    LeafKind::QualifiedIdent => quote! {
                        /// Returns the leaf spelling.
                        pub fn text(&self) -> &str {
                            self.span.text().expect("textual syntax spans must resolve")
                        }

                        /// Replaces the spelling with a new immutable source snapshot.
                        pub fn set_text(&mut self, text: impl AsRef<str>) {
                            let text = text.as_ref();
                            self.span = self.span.clone().with_text(text);
                            (self.package_qualifier, self.index_qualifier) =
                                ::m2_syn::__qualified_identifier_spans(text, &self.span);
                        }
                    },
                    _ => quote! {
                        /// Returns the leaf spelling.
                        pub fn text(&self) -> &str {
                            self.span.text().expect("textual syntax spans must resolve")
                        }

                        /// Replaces the spelling with a new immutable source snapshot.
                        pub fn set_text(&mut self, text: impl AsRef<str>) {
                            self.span = self.span.clone().with_text(text.as_ref());
                        }
                    },
                };
                let derives = if matches!(leaf, LeafKind::Adj | LeafKind::Eoc | LeafKind::Eof) {
                    quote!(Debug, Clone, PartialEq, Eq, Hash)
                } else {
                    quote!(Debug, ::m2_syn::Spanned)
                };
                let declaration = (!definition.explicit).then(|| {
                    quote! {
                        #[derive(#derives)]
                        #[doc = concat!("A typed Macaulay2 `", #kind, "` syntax leaf.")]
                        pub struct #name {
                            #qualifier_fields
                            span: ::m2_syn::Span,
                            leading_trivia: ::m2_syn::LeadingTrivia,
                            trailing_trivia: ::m2_syn::TrailingTrivia,
                        }
                    }
                });
                let boundary_spanned =
                    matches!(leaf, LeafKind::Adj | LeafKind::Eoc | LeafKind::Eof).then(|| {
                        quote! {
                            impl ::m2_syn::Spanned for #name {
                                fn try_span(&self) -> qf!(Option<::m2_syn::Span>) {
                                    Some(self.span.clone())
                                }
                            }
                        }
                    });
                let pretty_impl = context.pretty().then(|| {
                    quote! {
                        impl ::m2_syn::PrettyTree for #name {
                            fn pretty_tree(&self) -> ::m2_syn::PrettyNode {
                                ::m2_syn::PrettyNode::token(
                                    stringify!(#name),
                                    #pretty_text,
                                    ::m2_syn::Spanned::expect_span(self),
                                )
                            }
                        }
                    }
                });
                Ok(quote! {
                #declaration

                impl #name {
                    /// Creates a leaf from its text and source span.
                    pub fn new(text: impl AsRef<str>, span: ::m2_syn::Span) -> Self {
                        #constructor
                    }

                    #text_api
                }

                impl ::m2_syn::Lex for #name {
                    fn to_tokens(&self, #output: &mut ::m2_syn::TokenStream) {
                        ::m2_syn::Lex::to_tokens(&self.leading_trivia, #output);
                        #emit
                        ::m2_syn::Lex::to_tokens(&self.trailing_trivia, #output);
                    }
                }

                impl ::m2_syn::Terminal for #name {
                    fn leading_trivia(&self) -> &::m2_syn::LeadingTrivia {
                        &self.leading_trivia
                    }

                    fn leading_trivia_mut(&mut self) -> &mut ::m2_syn::LeadingTrivia {
                        &mut self.leading_trivia
                    }

                    fn trailing_trivia(&self) -> &::m2_syn::TrailingTrivia {
                        &self.trailing_trivia
                    }

                    fn trailing_trivia_mut(&mut self) -> &mut ::m2_syn::TrailingTrivia {
                        &mut self.trailing_trivia
                    }
                }

                #boundary_spanned

                #pretty_impl

                impl<N> ::m2_syn::Migrate<N> for #name
                where
                    N: ::m2_syn::ExternalCstNode,
                {
                    fn matches(node: &N) -> bool {
                        node.identity().matches(#kind, true)
                    }

                    fn migrate(node: N) -> Result<Self, ::m2_syn::MigrateError> {
                        if !<Self as ::m2_syn::Migrate<N>>::matches(&node) {
                            return Err(::m2_syn::MigrateError::wrong_node(
                                #kind,
                                true,
                                node.identity(),
                            ));
                        }
                        Ok(Self::new(node.text(), node.expect_span()))
                    }
                }

                })
            }

            StructFields::Product { fields, style } => {
                if definition.comments && *style != ProductStyle::Named {
                    return Err(Error::new(
                        definition.name.span(),
                        "`#[comments]` requires a named product struct",
                    ));
                }
                let structural_matches = if context.kind_is_shared(&kind) {
                    fields
                        .iter()
                        .filter_map(FieldDefinition::required_match)
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let expanded_fields = fields.iter().map(|field| {
                    let member = &field.member;
                    let stored = field.shape.stored_type(context, false);
                    let member_name = member.to_token_stream().to_string();
                    let doc = format!("The `{member_name}` child of this syntax node.");
                    quote! {
                        #[doc = #doc]
                        pub #member: #stored
                    }
                });
                let typed_delimiter = definition
                    .wrapper
                    .and_then(Wrapper::delimiter)
                    .map(Delimiter::typed);
                let delimiter_field = typed_delimiter.as_ref().map(|ty| {
                    quote!(
                        /// The paired delimiter surrounding this node.
                        pub delimiter: #ty,
                    )
                });
                let comments_field = definition.comments.then(|| {
                    quote! {
                        /// Comments structurally owned by this syntax node.
                        pub comments: ::m2_syn::CommentTrivia,
                    }
                });
                let retains_span = matches!(definition.wrapper, Some(Wrapper::Literal(_)));
                let retained_span_field = retains_span.then(|| {
                    quote! {
                        span: ::m2_syn::Span,
                        leading_trivia: ::m2_syn::LeadingTrivia,
                        trailing_trivia: ::m2_syn::TrailingTrivia,
                    }
                });
                let struct_definition = match style {
                    ProductStyle::Named => quote! {
                        #[doc = concat!("A typed Macaulay2 `", #kind, "` syntax node.")]
                        pub struct #name {
                            #(#expanded_fields,)*
                            #delimiter_field
                            #retained_span_field
                            #comments_field
                        }
                    },
                    ProductStyle::Tuple => {
                        let fields = fields.iter().map(|field| {
                            let stored = field.shape.stored_type(context, false);
                            quote!(pub #stored)
                        });
                        quote! {
                            #[doc = concat!("A typed Macaulay2 `", #kind, "` syntax node.")]
                            pub struct #name(#(#fields,)*);
                        }
                    }
                };
                let declaration = (!definition.explicit).then(|| {
                    quote! {
                        #[derive(Debug, ::m2_syn::Spanned)]
                        #struct_definition
                    }
                });
                let mut constructor_arguments = fields
                    .iter()
                    .map(|field| {
                        let binding = &field.binding;
                        let ty = field.shape.ty();
                        quote! { #binding: #ty }
                    })
                    .collect::<Vec<_>>();
                if retains_span {
                    constructor_arguments.push(quote!(span: ::m2_syn::Span));
                }
                let constructor_conversions = fields
                    .iter()
                    .filter_map(|field| {
                        let binding = &field.binding;
                        let stored = field.shape.stored_shape(context, false);
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
                    quote! {
                        let spans = span.map_or_else(
                            || ::m2_syn::DoubleSpan::synthetic(
                                <#ty as ::m2_syn::DelimiterToken>::KIND,
                            ),
                            |span| ::m2_syn::DoubleSpan::new(span.clone(), span),
                        );
                        let delimiter = <#ty as ::m2_syn::DelimiterToken>::new(spans);
                    }
                });
                let delimiter_member = typed_delimiter.as_ref().map(|_| quote!(delimiter,));
                let comments_member = definition
                    .comments
                    .then(|| quote!(comments: ::m2_syn::CommentTrivia::default(),));
                let retained_span_member = retains_span.then(|| {
                    quote! {
                        span,
                        leading_trivia: ::m2_syn::LeadingTrivia::default(),
                        trailing_trivia: ::m2_syn::TrailingTrivia::default(),
                    }
                });
                let constructor_value = match style {
                    ProductStyle::Named if constructor_delimiter.is_some() => quote!({
                        #constructor_delimiter
                        Self { #(#constructor_fields,)* #delimiter_member #retained_span_member #comments_member }
                    }),
                    ProductStyle::Named => {
                        quote!(Self { #(#constructor_fields,)* #retained_span_member #comments_member })
                    }
                    ProductStyle::Tuple => quote!(Self(#(#constructor_fields,)*)),
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
                    quote! { ::m2_syn::Spanned::try_span(&#binding) }
                });
                let constructor_span = typed_delimiter
                    .as_ref()
                    .map(|_| quote!(let span = ::m2_syn::Span::join_all([#(#span_fields),*]);));
                let migrate_fields = fields
                    .iter()
                    .map(|field| {
                        field.migrate(
                            context,
                            fields.len() == 1 && matches!(field.shape, Shape::Delimited(_)),
                        )
                    })
                    .collect::<Result<Vec<_>>>()?;
                let migrated_fields = fields.iter().map(|field| &field.binding);
                let migrate_span =
                    retains_span.then(|| quote!(let span = ::m2_syn::Spanned::expect_span(&node);));
                let migrate_delimiter = typed_delimiter.as_ref().map(|ty| {
                    quote! {
                        let opening = children.required_anonymous(
                            <#ty as ::m2_syn::DelimiterToken>::KIND.opening(),
                        )?;
                        let closing = children.required_anonymous(
                            <#ty as ::m2_syn::DelimiterToken>::KIND.closing(),
                        )?;
                        let delimiter = <#ty as ::m2_syn::DelimiterToken>::new(
                            ::m2_syn::DoubleSpan::new(
                                ::m2_syn::Spanned::expect_span(&opening),
                                ::m2_syn::Spanned::expect_span(&closing),
                            ),
                        );
                    }
                });
                let migrated = match style {
                    ProductStyle::Named if migrate_delimiter.is_some() => {
                        quote!(Self { #(#migrated_fields,)* #delimiter_member #retained_span_member #comments_member })
                    }
                    ProductStyle::Named => {
                        quote!(Self { #(#migrated_fields,)* #retained_span_member #comments_member })
                    }
                    ProductStyle::Tuple => quote!(Self(#(#migrated_fields,)*)),
                };
                let field_separator = definition.wrapper.map_or(" ", Wrapper::field_separator);
                let print_fields = fields
                    .iter()
                    .map(|field| field.to_tokens(field_separator, context))
                    .collect::<Result<Vec<_>>>()?;
                let pretty_impl = context.pretty().then(|| {
                    let pretty_fields = fields.iter().map(|field| {
                        let member = &field.member;
                        let label = member.to_token_stream().to_string();
                        quote! {
                            node.push(#label, &self.#member);
                        }
                    });
                    let pretty_opening = typed_delimiter.as_ref().map(|_| {
                        quote! {
                            let __m2_syn_delimiter =
                                ::m2_syn::DelimiterValue::delimiter(&self.delimiter);
                            node.push_node(
                                "opening",
                                ::m2_syn::PrettyNode::delimiter_boundary(
                                    __m2_syn_delimiter.kind,
                                    true,
                                    __m2_syn_delimiter.opening_span(),
                                ),
                            );
                        }
                    });
                    let pretty_closing = typed_delimiter.as_ref().map(|_| {
                        quote! {
                            if let Some(__m2_syn_closing) =
                                __m2_syn_delimiter.closing_span()
                            {
                                node.push_node(
                                    "closing",
                                    ::m2_syn::PrettyNode::delimiter_boundary(
                                        __m2_syn_delimiter.kind,
                                        false,
                                        __m2_syn_closing,
                                    ),
                                );
                            }
                        }
                    });
                    let pretty_comments = definition
                        .comments
                        .then(|| quote!(node.push("comments", &self.comments);));
                    quote! {
                        impl ::m2_syn::PrettyTree for #name {
                            fn pretty_tree(&self) -> ::m2_syn::PrettyNode {
                                let mut node = ::m2_syn::PrettyNode::syntax(
                                    stringify!(#name),
                                    ::m2_syn::Spanned::try_span(self),
                                );
                                #pretty_opening
                                #(#pretty_fields)*
                                #pretty_closing
                                #pretty_comments
                                node
                            }
                        }
                    }
                });
                let emit_contents = match definition.wrapper {
                    Some(Wrapper::Literal(Literal::String)) => quote! {
                        ::m2_syn::Lex::to_tokens(&self.leading_trivia, output);
                        let span = ::m2_syn::Spanned::expect_span(self);
                        output.extend([::m2_syn::LiteralToken::from_span(
                            ::m2_syn::LiteralKind::String,
                            span,
                        )]);
                        ::m2_syn::Lex::to_tokens(&self.trailing_trivia, output);
                    },
                    Some(Wrapper::Literal(Literal::RawString)) => quote! {
                        ::m2_syn::Lex::to_tokens(&self.leading_trivia, output);
                        let span = ::m2_syn::Spanned::expect_span(self);
                        output.extend([::m2_syn::LiteralToken::from_span(
                            ::m2_syn::LiteralKind::RawString,
                            span,
                        )]);
                        ::m2_syn::Lex::to_tokens(&self.trailing_trivia, output);
                    },
                    Some(_) => quote! {
                        ::m2_syn::DelimiterValue::surround(
                            &self.delimiter,
                            output,
                            contents,
                        );
                    },
                    None => quote! {
                        ::m2_syn::Lex::to_tokens(&contents, output);
                    },
                };
                let apply_comments = definition
                    .comments
                    .then(|| quote!(self.comments.__apply_to(&mut contents);));
                let terminal_impl = retains_span.then(|| {
                    quote! {
                        impl ::m2_syn::Terminal for #name {
                            fn leading_trivia(&self) -> &::m2_syn::LeadingTrivia {
                                &self.leading_trivia
                            }

                            fn leading_trivia_mut(&mut self) -> &mut ::m2_syn::LeadingTrivia {
                                &mut self.leading_trivia
                            }

                            fn trailing_trivia(&self) -> &::m2_syn::TrailingTrivia {
                                &self.trailing_trivia
                            }

                            fn trailing_trivia_mut(&mut self) -> &mut ::m2_syn::TrailingTrivia {
                                &mut self.trailing_trivia
                            }
                        }
                    }
                });
                let cell_aware_source = definition.cell_stream.then(|| {
                    quote! {
                        fn to_m2(&self) -> qf!(String) {
                            ::m2_syn::Tokenize::to_cell_stream(self)
                                .expect("typed source-file cells are complete")
                                .to_string()
                        }
                    }
                });
                Ok(quote! {
                    #declaration

                    impl #name {
                        /// Creates this node from its typed children.
                        pub fn new(#(#constructor_arguments),*) -> Self {
                            #constructor_span
                            #constructor
                        }
                    }
                    impl ::m2_syn::Lex for #name {
                        fn to_tokens(&self, output: &mut ::m2_syn::TokenStream) {
                            let mut contents = ::m2_syn::TokenStream::new();
                            #(#print_fields)*
                            #apply_comments
                            #emit_contents
                        }

                        #cell_aware_source
                    }

                    #terminal_impl

                    #pretty_impl

                    impl<N> ::m2_syn::Migrate<N> for #name
                    where
                        N: ::m2_syn::ExternalCstNode,
                    {
                        fn matches(node: &N) -> bool {
                            node.identity().matches(#kind, true)
                                #(&& #structural_matches)*
                        }

                        fn migrate(node: N) -> Result<Self, ::m2_syn::MigrateError> {
                            if !<Self as ::m2_syn::Migrate<N>>::matches(&node) {
                                return Err(::m2_syn::MigrateError::wrong_node(
                                    #kind,
                                    true,
                                    node.identity(),
                                ));
                            }
                            #migrate_span
                            let mut children = ::m2_syn::ChildCursor::new(&node);
                            #migrate_delimiter
                            #(#migrate_fields)*
                            Ok(#migrated)
                        }
                    }
                })
            }
        }
    }

    fn expand_conversions(&self, context: &SyntaxContext<'_>) -> TokenStream {
        let mut paths = ConversionGraph::new();
        for definition in &self.enums {
            for variant in &definition.variants {
                let source = variant.shape.base_ident().into();
                paths
                    .entry((source, (&definition.name).into()))
                    .or_default()
                    .push(vec![(
                        definition.name.clone(),
                        variant.name.clone(),
                        matches!(variant.shape.stored_shape(context, false), Shape::Boxed(_)),
                    )]);
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

        let implementations = paths.into_iter().filter_map(|((source, target), paths)| {
            if paths.len() != 1 {
                return None;
            }
            let source = context.declaration(&source)?;
            let target = context.declaration(&target)?;
            let wrappers = &paths[0];
            let value = wrappers
                .iter()
                .fold(quote!(value), |value, (owner, variant, boxed)| {
                    let value = if *boxed {
                        quote!(<qf!(Box<_>)>::new(#value))
                    } else {
                        value
                    };
                    quote!(#owner::#variant(#value))
                });
            Some(quote! {
                impl From<#source> for #target {
                    fn from(value: #source) -> Self {
                        #value
                    }
                }
            })
        });
        quote! { #(#implementations)* }
    }

    fn expand_visit(&self, context: &SyntaxContext<'_>) -> TokenStream {
        let methods = self.all_types().map(|name| {
            let method = format_ident!("visit_{}", to_snake_case(&name.to_string()));
            let ty = context.type_reference(name);
            quote! {
                #[doc = concat!("Visits a `", stringify!(#name), "` node.")]
                fn #method(&mut self, node: &'ast #ty) {
                    self::#method(self, node);
                }
            }
        });
        let walkers = self.expand_walkers(Traversal::Visit, context);
        quote! {
            /// Shared-reference traversal over the typed graph.
            pub mod visit {
                use super::*;

                /// Per-node visiting rules, each defaulting to the free walker
                /// of the same name.
                pub trait Visit<'ast> {
                    #(#methods)*
                }

                #(#walkers)*
            }
        }
    }

    fn expand_visit_mut(&self, context: &SyntaxContext<'_>) -> TokenStream {
        let methods = self.all_types().map(|name| {
            let method = format_ident!("visit_{}_mut", to_snake_case(&name.to_string()));
            let ty = context.type_reference(name);
            quote! {
                #[doc = concat!("Visits a mutable `", stringify!(#name), "` node.")]
                fn #method(&mut self, node: &mut #ty) {
                    self::#method(self, node);
                }
            }
        });
        let walkers = self.expand_walkers(Traversal::VisitMut, context);
        quote! {
            /// In-place traversal over the typed graph.
            pub mod visit_mut {
                use super::*;

                /// Per-node mutation rules, each defaulting to the free walker
                /// of the same name.
                pub trait VisitMut {
                    #(#methods)*
                }

                #(#walkers)*
            }
        }
    }

    fn expand_fold(&self, context: &SyntaxContext<'_>) -> TokenStream {
        let methods = self.all_types().map(|name| {
            let method = format_ident!("fold_{}", to_snake_case(&name.to_string()));
            let ty = context.type_reference(name);
            quote! {
                #[doc = concat!("Folds a `", stringify!(#name), "` node.")]
                fn #method(&mut self, node: #ty) -> #ty {
                    self::#method(self, node)
                }
            }
        });
        let walkers = self.expand_walkers(Traversal::Fold, context);
        quote! {
            /// Owning rebuild of the typed graph.
            pub mod fold {
                use super::*;

                /// Per-node rebuilding rules, each defaulting to the free
                /// walker of the same name.
                pub trait Fold {
                    #(#methods)*
                }

                #(#walkers)*
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
        context: &SyntaxContext<'_>,
    ) -> Vec<TokenStream> {
        let tokens = self.tokens.iter().map(|definition| {
            let ty = context.type_reference(&definition.name);
            traversal.empty_walker(&definition.name, ty)
        });
        let structs = self.structs.iter().map(|definition| {
            let name = &definition.name;
            match &definition.fields {
                StructFields::Leaf(_) => traversal.empty_walker(name, quote!(#name)),
                StructFields::Product { .. } => traversal.struct_walker(definition, context),
            }
        });
        let enums = self
            .enums
            .iter()
            .map(|definition| traversal.enum_walker(definition, context));
        tokens.chain(structs).chain(enums).collect()
    }
}

impl FieldDefinition {
    fn required_match(&self) -> Option<TokenStream> {
        if !matches!(self.shape, Shape::Base(_, _)) {
            return None;
        }
        let ty = self.shape.ty();
        let field_match = match &self.source {
            FieldSource::Named(field) => quote!(child.field == Some(#field)),
            FieldSource::Positional => quote!(child.field.is_none()),
        };
        Some(quote!(node.children().any(|child| {
            #field_match && <#ty as ::m2_syn::Migrate<N>>::matches(&child.node)
        })))
    }

    fn migrate(&self, context: &SyntaxContext<'_>, delimited_newtype: bool) -> Result<TokenStream> {
        let binding = &self.binding;
        if delimited_newtype {
            let Shape::Delimited(contents) = &self.shape else {
                unreachable!("only delimited newtypes take this migration path")
            };
            let contents =
                contents.migrate_from_cursor(quote!(__m2_syn_delimited_children), context)?;
            return Ok(quote! {
                let mut __m2_syn_delimited_children = ::m2_syn::ChildCursor::new(&node);
                let __m2_syn_delimiter = __m2_syn_delimited_children.required_delimiter()?;
                let __m2_syn_contents = #contents;
                let #binding = ::m2_syn::Delimited::new(
                    __m2_syn_delimiter,
                    __m2_syn_contents,
                );
            });
        }
        if let Shape::Delimited(contents) = &self.shape {
            let (delimiter, cursor) = match &self.source {
                FieldSource::Named(field) => (
                    quote! {
                        let __m2_syn_delimited_node = children.required_field(#field)?;
                        let mut __m2_syn_delimited_children =
                            ::m2_syn::ChildCursor::new(&__m2_syn_delimited_node);
                        let __m2_syn_delimiter =
                            __m2_syn_delimited_children.required_delimiter()?;
                    },
                    quote!(__m2_syn_delimited_children),
                ),
                FieldSource::Positional => (
                    quote! {
                        let __m2_syn_delimiter = children.required_delimiter()?;
                    },
                    quote!(children),
                ),
            };
            let contents = contents.migrate_from_cursor(cursor, context)?;
            return Ok(quote! {
                #delimiter
                let __m2_syn_contents = #contents;
                let #binding = ::m2_syn::Delimited::new(
                    __m2_syn_delimiter,
                    __m2_syn_contents,
                );
            });
        }
        let element = self.shape.element()?;
        let base_type = element.ty();
        let selected = match (&self.source, &self.shape) {
            (FieldSource::Named(field), Shape::Base(_, _) | Shape::Boxed(_)) => {
                quote!(children.required_field(#field)?)
            }
            (FieldSource::Named(field), Shape::Optional(inner)) if !inner.is_sequence() => {
                quote!(children.optional_field(#field))
            }
            (FieldSource::Named(field), _) => quote!(children.repeated_field(#field)),
            (FieldSource::Positional, Shape::Base(_, _) | Shape::Boxed(_)) => {
                quote!(children.required_matching::<#base_type>()?)
            }
            (FieldSource::Positional, Shape::Optional(inner)) if !inner.is_sequence() => {
                quote!(children.optional_matching::<#base_type>())
            }
            (FieldSource::Positional, _) => quote!(children.repeated_matching::<#base_type>()),
        };
        let stored_shape = self.shape.stored_shape(context, false);
        let stored_element = stored_shape.storage_element();
        if matches!(self.shape, Shape::Base(_, _) | Shape::Boxed(_)) {
            let value = stored_element.wrap_value(quote!(
                <#base_type as ::m2_syn::Migrate<N>>::migrate(#selected)?
            ));
            return Ok(quote!(let #binding = #value;));
        }

        // The remaining functors all map the same fallible conversion over
        // their nodes; only how the results are gathered differs.
        let element_value = stored_element.wrap_value(quote!(value));
        let convert = quote!(|node| {
            let value = <#base_type as ::m2_syn::Migrate<N>>::migrate(node)?;
            Ok::<_, ::m2_syn::MigrateError>(#element_value)
        });
        if matches!(&self.shape, Shape::Optional(inner) if !inner.is_sequence()) {
            Ok(quote!(let #binding = #selected.map(#convert).transpose()?;))
        } else if matches!(&self.shape, Shape::Optional(inner) if matches!(inner.as_ref(), Shape::Punctuated(_)))
        {
            Ok(quote! {
                let __m2_syn_values = #selected;
                let #binding = children.optional_punctuated(__m2_syn_values, ",")?
                    .map(|values| values.try_map(
                        |node| (#convert)(node),
                        |punctuation| <::m2_syn::Token![,] as ::m2_syn::Migrate<N>>::migrate(
                            punctuation,
                        ),
                    ))
                    .transpose()?;
            })
        } else if matches!(self.shape, Shape::Punctuated(_)) {
            Ok(quote! {
                let __m2_syn_values = #selected;
                let #binding = children.punctuated(__m2_syn_values, ",")?.try_map(
                    |node| (#convert)(node),
                    |punctuation| <::m2_syn::Token![,] as ::m2_syn::Migrate<N>>::migrate(
                        punctuation,
                    ),
                )?;
            })
        } else {
            Ok(quote!(let #binding = #selected
                .into_iter()
                .map(#convert)
                .collect::<Result<qf!(Vec<_>), _>>()?;))
        }
    }

    /// Whether this field's token hugs whatever precedes it instead of being
    /// spaced away from it, which is what `;`, `,`, and zero-width end-of-cell
    /// markers do.
    fn is_attached(&self, context: &SyntaxContext<'_>) -> bool {
        matches!(
            self.shape.base_ident().to_string().as_str(),
            "Adj" | "EOC" | "EOF"
        ) || context
            .tokens()
            .spelling(self.shape.base_ident())
            .is_some_and(|spelling| matches!(spelling, ";" | ","))
    }

    fn to_tokens(
        &self,
        field_separator: &'static str,
        context: &SyntaxContext<'_>,
    ) -> Result<TokenStream> {
        let member = &self.member;
        self.shape.element()?;
        let write = if let Shape::Repeated(_, separator) = self.shape {
            // `Option` and `Punctuated` carry their own `Lex` impls; a bare
            // `Vec` is the only functor that has to interleave separators here.
            let separator = if field_separator.is_empty() && separator == " " {
                ""
            } else {
                separator
            };
            if separator.is_empty() {
                quote! {
                    for value in &self.#member {
                        ::m2_syn::Lex::to_tokens(value, &mut field_output);
                    }
                }
            } else {
                let push_separator = push_source_separator(&quote!(field_output), separator);
                if separator == "\n" {
                    quote! {
                        for (index, value) in self.#member.iter().enumerate() {
                            if index != 0 && !field_output.ends_with_whitespace() {
                                #push_separator
                            }
                            ::m2_syn::Lex::to_tokens(value, &mut field_output);
                        }
                    }
                } else {
                    quote! {
                        for (index, value) in self.#member.iter().enumerate() {
                            if index != 0 {
                                #push_separator
                            }
                            ::m2_syn::Lex::to_tokens(value, &mut field_output);
                        }
                    }
                }
            }
        } else {
            quote! { ::m2_syn::Lex::to_tokens(&self.#member, &mut field_output); }
        };
        let separator =
            (!self.is_attached(context) && !field_separator.is_empty()).then_some(field_separator);
        let separate = separator.map(|separator| {
            let push_separator = push_source_separator(&quote!(contents), separator);
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
                    ::m2_syn::Lex::to_tokens(&field_output, &mut contents);
                }
            }
        })
    }
}

fn push_source_separator(output: &TokenStream, separator: &str) -> TokenStream {
    match separator {
        "" | " " | "\n" => quote! {},
        ", " => quote! {
            #output.extend([::m2_syn::Punct::new(
                ",",
                ::m2_syn::Span::for_text(","),
            )]);
        },
        separator => panic!("unsupported generated separator `{separator}`"),
    }
}

impl Shape {
    fn stored_type(&self, context: &SyntaxContext<'_>, indirect: bool) -> TokenStream {
        self.stored_shape(context, indirect).ty()
    }

    fn stored_shape(&self, context: &SyntaxContext<'_>, indirect: bool) -> Shape {
        match self {
            // Already indirect, so its element needs no box of its own.
            Self::Boxed(inner) => Shape::Boxed(Box::new(inner.stored_shape(context, true))),
            Self::Base(path, ident) => {
                let base = Shape::Base(path.clone(), ident.clone());
                if !indirect && context.is_boxed(ident) {
                    Shape::Boxed(Box::new(base))
                } else {
                    base
                }
            }
            Self::Delimited(inner) => {
                Shape::Delimited(Box::new(inner.stored_shape(context, indirect)))
            }
            Self::Optional(inner) => {
                Shape::Optional(Box::new(inner.stored_shape(context, indirect)))
            }
            Self::Repeated(inner, separator) => {
                Shape::Repeated(Box::new(inner.stored_shape(context, true)), separator)
            }
            Self::Punctuated(inner) => {
                // The mandatory first element is stored inline. Unlike `Vec`,
                // `Punctuated` therefore does not itself break recursive type
                // cycles; only an already-indirect outer context may do that.
                Shape::Punctuated(Box::new(inner.stored_shape(context, indirect)))
            }
        }
    }

    fn convert_value(&self, stored: &Shape, value: TokenStream) -> TokenStream {
        if self.conversion_is_identity(stored) {
            return value;
        }
        match (self, stored) {
            (Self::Base(_, _), Shape::Boxed(inner))
                if matches!(inner.as_ref(), Shape::Base(_, _)) =>
            {
                quote!(<qf!(Box<_>)>::new(#value))
            }
            (Self::Optional(source), Shape::Optional(target)) => {
                if matches!(
                    (source.as_ref(), target.as_ref()),
                    (
                        Self::Base(_, _),
                        Shape::Boxed(inner)
                    ) if matches!(inner.as_ref(), Shape::Base(_, _))
                ) {
                    quote!(#value.map(<qf!(Box<_>)>::new))
                } else {
                    let converted = source.convert_value(target, quote!(value));
                    quote!(#value.map(|value| #converted))
                }
            }
            (Self::Delimited(source), Shape::Delimited(target)) => {
                let converted = source.convert_value(target, quote!(contents));
                quote!({
                    let ::m2_syn::Delimited { delimiter, contents } = #value;
                    ::m2_syn::Delimited::new(delimiter, #converted)
                })
            }
            (Self::Repeated(source, _), Shape::Repeated(target, _)) => {
                let converted = source.convert_value(target, quote!(value));
                quote!({
                    let values = #value;
                    let mut converted_values = <qf!(Vec<_>)>::with_capacity(values.len());
                    for value in values {
                        converted_values.push(#converted);
                    }
                    converted_values
                })
            }
            (Self::Punctuated(source), Shape::Punctuated(target)) => {
                let converted = source.convert_value(target, quote!(value));
                quote!({
                    #value.map_values(|value| #converted)
                })
            }
            _ => quote!(#value),
        }
    }

    fn conversion_is_identity(&self, stored: &Shape) -> bool {
        match (self, stored) {
            (Self::Base(_, _), Shape::Base(_, _)) => true,
            (Self::Optional(source), Shape::Optional(target))
            | (Self::Delimited(source), Shape::Delimited(target))
            | (Self::Repeated(source, _), Shape::Repeated(target, _))
            | (Self::Punctuated(source), Shape::Punctuated(target)) => {
                source.conversion_is_identity(target)
            }
            _ => false,
        }
    }

    /// The syntax element held by this field's functor stack.
    fn element(&self) -> Result<&Self> {
        match self {
            Self::Base(_, _) => Ok(self),
            Self::Boxed(inner)
            | Self::Delimited(inner)
            | Self::Optional(inner)
            | Self::Repeated(inner, _)
            | Self::Punctuated(inner) => inner.element(),
        }
    }

    fn is_sequence(&self) -> bool {
        match self {
            Self::Repeated(_, _) | Self::Punctuated(_) => true,
            Self::Boxed(inner) | Self::Optional(inner) => inner.is_sequence(),
            Self::Delimited(_) => false,
            Self::Base(_, _) => false,
        }
    }

    fn storage_element(&self) -> &Self {
        match self {
            Self::Optional(inner)
            | Self::Repeated(inner, _)
            | Self::Punctuated(inner)
            | Self::Delimited(inner) => inner.storage_element(),
            Self::Base(_, _) | Self::Boxed(_) => self,
        }
    }
}

impl Shape {
    fn migrate_from_cursor(
        &self,
        cursor: TokenStream,
        context: &SyntaxContext<'_>,
    ) -> Result<TokenStream> {
        match self {
            Self::Base(_, _) => {
                let ty = self.ty();
                let stored = self.stored_shape(context, false);
                let value = stored.wrap_value(quote!(
                    <#ty as ::m2_syn::Migrate<N>>::migrate(
                        #cursor.required_matching::<#ty>()?
                    )?
                ));
                Ok(value)
            }
            Self::Boxed(inner) => {
                let value = inner.migrate_from_cursor(cursor, context)?;
                Ok(quote!(<qf!(Box<_>)>::new(#value)))
            }
            Self::Optional(inner) if matches!(inner.as_ref(), Self::Base(_, _)) => {
                let ty = inner.ty();
                let stored = inner.stored_shape(context, false);
                let value = stored.wrap_value(quote!(value));
                Ok(quote!(
                    #cursor.optional_matching::<#ty>()
                        .map(|node| {
                            let value = <#ty as ::m2_syn::Migrate<N>>::migrate(node)?;
                            Ok::<_, ::m2_syn::MigrateError>(#value)
                        })
                        .transpose()?
                ))
            }
            Self::Repeated(inner, _) if matches!(inner.as_ref(), Self::Base(_, _)) => {
                let ty = inner.ty();
                let stored = inner.stored_shape(context, true);
                let value = stored.wrap_value(quote!(value));
                Ok(quote!(
                    #cursor.repeated_matching::<#ty>()
                        .into_iter()
                        .map(|node| {
                            let value = <#ty as ::m2_syn::Migrate<N>>::migrate(node)?;
                            Ok::<_, ::m2_syn::MigrateError>(#value)
                        })
                        .collect::<Result<qf!(Vec<_>), _>>()?
                ))
            }
            Self::Punctuated(inner) if matches!(inner.as_ref(), Self::Base(_, _)) => {
                let ty = inner.ty();
                let stored = inner.stored_shape(context, false);
                let first = stored.wrap_value(quote!(
                    <#ty as ::m2_syn::Migrate<N>>::migrate(__m2_syn_first)?
                ));
                Ok(quote!({
                    let __m2_syn_values = #cursor.repeated_matching::<#ty>();
                    #cursor.punctuated(__m2_syn_values, ",")?.try_map(
                        |__m2_syn_first| Ok::<_, ::m2_syn::MigrateError>(#first),
                        |punctuation| <::m2_syn::Token![,] as ::m2_syn::Migrate<N>>::migrate(
                            punctuation,
                        ),
                    )?
                }))
            }
            Self::Optional(inner) if matches!(inner.as_ref(), Self::Punctuated(_)) => {
                let Self::Punctuated(element) = inner.as_ref() else {
                    unreachable!()
                };
                let Self::Base(_, _) = element.as_ref() else {
                    return Err(Error::new(
                        element.base_ident().span(),
                        "punctuated migration currently requires a plain syntax type",
                    ));
                };
                let ty = element.ty();
                let stored = element.stored_shape(context, false);
                let first = stored.wrap_value(quote!(
                    <#ty as ::m2_syn::Migrate<N>>::migrate(__m2_syn_first)?
                ));
                Ok(quote!({
                    let __m2_syn_values = #cursor.repeated_matching::<#ty>();
                    #cursor.optional_punctuated(__m2_syn_values, ",")?
                        .map(|values| values.try_map(
                            |__m2_syn_first| Ok::<_, ::m2_syn::MigrateError>(#first),
                            |punctuation| <::m2_syn::Token![,] as ::m2_syn::Migrate<N>>::migrate(
                                punctuation,
                            ),
                        ))
                        .transpose()?
                }))
            }
            _ => Err(Error::new(
                self.base_ident().span(),
                "unsupported delimited contents shape",
            )),
        }
    }

    fn ty(&self) -> TokenStream {
        match self {
            Self::Base(path, _) => quote!(#path),
            Self::Boxed(inner) => {
                let inner = inner.ty();
                quote!(qf!(Box<#inner>))
            }
            Self::Delimited(inner) => {
                let inner = inner.ty();
                quote!(::m2_syn::Delimited<#inner>)
            }
            Self::Optional(inner) => {
                let inner = inner.ty();
                quote!(qf!(Option<#inner>))
            }
            Self::Repeated(inner, _) => {
                let inner = inner.ty();
                quote!(qf!(Vec<#inner>))
            }
            Self::Punctuated(inner) => {
                let inner = inner.ty();
                quote!(::m2_syn::Punctuated<#inner>)
            }
        }
    }

    fn wrap_value(&self, value: TokenStream) -> TokenStream {
        match self {
            Self::Boxed(inner) => {
                let value = inner.wrap_value(value);
                quote!(<qf!(Box<_>)>::new(#value))
            }
            Self::Base(_, _)
            | Self::Delimited(_)
            | Self::Optional(_)
            | Self::Repeated(_, _)
            | Self::Punctuated(_) => value,
        }
    }
}

fn expand_enum(definition: &EnumDefinition, context: &SyntaxContext<'_>) -> TokenStream {
    let tokens = context.tokens();
    let name = &definition.name;
    let variants = definition.variants.iter().map(|variant| {
        let name = &variant.name;
        let ty = variant.shape.stored_type(context, false);
        let doc = format!("The `{name}` syntax alternative.");
        quote! {
            #[doc = #doc]
            #name(#ty)
        }
    });
    let token_arms = definition.variants.iter().map(|variant| {
        let variant = &variant.name;
        quote!(Self::#variant(node) => ::m2_syn::Lex::to_tokens(node, output))
    });
    let matches = definition
        .variants
        .iter()
        .map(|variant| {
            let base = variant
                .shape
                .element()
                .expect("syntax enum variants always contain one element")
                .ty();
            quote!(<#base as ::m2_syn::Migrate<N>>::matches(node))
        })
        .reduce(|left, right| quote!(#left || #right))
        .unwrap_or_else(|| quote!(false));
    let migrate_arms = definition.variants.iter().map(|variant| {
        let variant_name = &variant.name;
        let base = variant
            .shape
            .element()
            .expect("syntax enum variants always contain one element")
            .ty();
        let stored = variant.shape.stored_shape(context, false);
        let value = stored.wrap_value(quote!(
            <#base as ::m2_syn::Migrate<N>>::migrate(node)?
        ));
        quote! {
            if <#base as ::m2_syn::Migrate<N>>::matches(&node) {
                return Ok(Self::#variant_name(#value));
            }
        }
    });
    let borrowed_fallback = definition
        .variants
        .is_empty()
        .then(|| quote!(_ => unreachable!("empty generated syntax category")));
    let pretty_impl = context.pretty().then(|| {
        let pretty_arms = definition.variants.iter().map(|variant| {
            let variant = &variant.name;
            quote! {
                Self::#variant(value) => {
                    ::m2_syn::PrettyNode::variant(
                        stringify!(#name),
                        stringify!(#variant),
                        ::m2_syn::Spanned::try_span(self),
                        ::m2_syn::PrettyTree::pretty_tree(value),
                    )
                }
            }
        });
        quote! {
            impl ::m2_syn::PrettyTree for #name {
                fn pretty_tree(&self) -> ::m2_syn::PrettyNode {
                    match self { #(#pretty_arms,)* #borrowed_fallback }
                }
            }
        }
    });
    let spelling_api = definition.operator.map(|_| {
        let from_token_arms = definition.variants.iter().map(|variant| {
            let variant_name = &variant.name;
            let ty = variant.shape.stored_type(context, false);
            quote! {
                if <#ty as ::m2_syn::Token>::matches_token_tree(&token) {
                    return Some(Self::#variant_name(
                        <#ty as ::m2_syn::Token>::from_token_tree_with_trivia(
                            token,
                            trailing_trivia,
                        )?
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
                /// Converts a matching raw operator token into this category.
                pub fn from_token_tree(token: ::m2_syn::TokenTree) -> qf!(Option<Self>) {
                    Self::from_token_tree_with_trivia(
                        token,
                        ::m2_syn::TrailingTrivia::default(),
                    )
                }

                /// Converts a matching raw operator and its following trivia into this category.
                pub fn from_token_tree_with_trivia(
                    token: ::m2_syn::TokenTree,
                    trailing_trivia: ::m2_syn::TrailingTrivia,
                ) -> qf!(Option<Self>) {
                    #(#from_token_arms)*
                    None
                }

                /// Returns the operator's canonical source spelling.
                pub fn spelling(&self) -> &'static str {
                    match self {
                        #(#spelling_arms,)*
                        #borrowed_fallback
                    }
                }
            }
        }
    });
    let declaration = (!definition.explicit).then(|| {
        quote! {
            #[derive(Debug, ::m2_syn::Spanned)]
            #[doc = concat!("The typed `", stringify!(#name), "` syntax category.")]
            pub enum #name {
                #(#variants,)*
            }
        }
    });
    quote! {
        #declaration

        impl ::m2_syn::Lex for #name {
            fn to_tokens(&self, output: &mut ::m2_syn::TokenStream) {
                match self { #(#token_arms,)* #borrowed_fallback }
            }
        }

        #pretty_impl

        impl<N> ::m2_syn::Migrate<N> for #name
        where
            N: ::m2_syn::ExternalCstNode,
        {
            fn matches(node: &N) -> bool {
                #matches
            }

            fn migrate(node: N) -> Result<Self, ::m2_syn::MigrateError> {
                #(#migrate_arms)*
                Err(::m2_syn::MigrateError::wrong_category(
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
    fn tokens_feed_one_lookup_macro_and_operator_enums() {
        let syntax: Syntax = parse2(quote! {
            tokens {
                [=] { infix } (14, R)
                [+] { pref, bin, aug } (50, L)
                [!] { post } (72)
                [if] (62, 12)
                [,]
            }

            struct Leaf;
            struct UsesOperators {
                prefix: PrefixOperator,
                binary: BinaryOperator,
                postfix: PostfixOperator,
                augmented: AugmentedOperator,
            }
        })
        .unwrap();
        let expansion = syntax.expand(SyntaxFeatures::PRETTY_WALK).unwrap();
        parse2::<syn::File>(expansion.clone()).unwrap();
        let expansion = expansion.to_string();

        // The expansion is items only: `Token!` and `Delimiter!` are written
        // by hand in `m2-syn/src/schema/macros.rs`, and every other lookup is a plain
        // function so that rust-analyzer can resolve the call sites.
        assert_eq!(expansion.matches("macro_rules !").count(), 0);
        assert_eq!(expansion.matches("TOKEN_PARSE_RULES").count(), 1);
        assert!(expansion.contains("TokenParseRule :: PrefixBinary"));
        assert!(expansion.contains("TokenParseRule :: Postfix"));
        assert!(expansion.contains("pub mod visit"));
        assert!(expansion.contains("pub mod visit_mut"));
        assert!(expansion.contains("pub mod fold"));
        assert!(expansion.contains("PrettyTree"));
        assert!(expansion.contains("enum PrefixOperator"));
        assert!(expansion.contains("enum BinaryOperator"));
        assert!(expansion.contains("enum PostfixOperator"));
        assert!(expansion.contains("enum AugmentedOperator"));
        assert!(expansion.contains("AddEql"));
        assert!(expansion.contains("_If"));
        assert!(expansion.contains("Cma"));
    }
}
