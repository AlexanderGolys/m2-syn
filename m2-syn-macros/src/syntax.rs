#[path = "syntax/derive.rs"]
mod derive;

use crate::tokens::{OperatorKind, TokenDefinitions, token_type_name};
use proc_macro2::{Ident, Span, TokenStream};
use quote::{format_ident, quote};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{Error, Result, Token, parse2};

/// Expands `tokens!`: the token types and parser metadata of the table, the
/// operator enums its roles define, and the traversal of every token.
pub(crate) fn tokens(input: TokenStream) -> Result<TokenStream> {
    let tokens: TokenDefinitions = parse2(input)?;
    let table = tokens.expand();
    let operators = OperatorKind::ALL.into_iter().map(|kind| {
        let name = Ident::new(kind.category_name(), Span::call_site());
        expand_enum(&EnumDefinition::operator(name, kind, &tokens))
    });
    let walkers = tokens
        .iter()
        .map(|definition| expand_token_walk(&definition.name));
    Ok(quote!(#table #(#operators)* #(#walkers)*))
}

/// Expands `#[derive(Syntax)]`: the syntax behaviour of one declared type.
pub(crate) fn derive_syntax(input: &syn::DeriveInput) -> Result<TokenStream> {
    match derive::read(input)? {
        derive::Definition::Struct(definition) => expand_struct(&definition),
        derive::Definition::Enum(definition) => Ok(expand_enum(&definition)),
    }
}

/// Expands `traversal!`: the `visit`, `visit_mut`, and `fold` modules over the
/// listed syntax types.
pub(crate) fn traversal(input: TokenStream) -> Result<TokenStream> {
    let names = Punctuated::<Ident, Token![,]>::parse_terminated.parse2(input)?;
    let modules = Traversal::ALL.map(|traversal| traversal.expand(names.iter()));
    let support = expand_walk_support();
    Ok(quote!(#support #(#modules)*))
}

struct StructDefinition {
    name: Ident,
    fields: StructFields,
    wrapper: Option<Wrapper>,
    cst_kind: Option<String>,
    cell_stream: bool,
}

enum StructFields {
    Leaf(LeafKind),
    Product(Vec<FieldDefinition>),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LeafKind {
    /// Text kept as written: string contents and escapes.
    Text,
    /// An identifier with its package and index qualifiers: `Core$x`, `x$1`.
    Symbol,
    Empty,
    Float,
    Integer,
}

struct FieldDefinition {
    name: Ident,
    shape: Shape,
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

/// A delimiter family: `()`, `[]`, `{}`, or `<||>`.
#[derive(Clone, Copy)]
pub(crate) enum Delimiter {
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
    pub(crate) const ALL: [Self; 4] = [
        Self::Parenthesis,
        Self::Bracket,
        Self::Brace,
        Self::AngleBar,
    ];

    /// The family `Token![…]` names when written as a pair, in any spacing.
    pub(crate) fn named(pattern: &TokenStream) -> Result<Self> {
        Self::spelled(&crate::tokens::token_spelling(pattern)?)
            .ok_or_else(|| Error::new_spanned(pattern, "unknown delimiter pair"))
    }

    pub(crate) fn spelled(spelling: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|family| family.spelling() == spelling)
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Parenthesis => "Parenthesis",
            Self::Bracket => "Bracket",
            Self::Brace => "Brace",
            Self::AngleBar => "AngleBar",
        }
    }

    fn spelling(self) -> &'static str {
        match self {
            Self::Parenthesis => "()",
            Self::Bracket => "[]",
            Self::Brace => "{}",
            Self::AngleBar => "<||>",
        }
    }

    pub(crate) fn type_name(self) -> Ident {
        format_ident!("_{}Delimiter", self.name())
    }

    fn typed(self) -> TokenStream {
        let name = self.type_name();
        quote!(::m2_syn::nodes::#name)
    }
}

struct EnumDefinition {
    name: Ident,
    variants: Vec<VariantDefinition>,
    operator: Option<OperatorKind>,
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
        }
    }
}

/// A field's type as a stack of known functors over one syntax type, as in
/// `Option<Box<Expr>>`, which is `Optional(Boxed(Base(Expr)))`.
#[derive(Clone)]
enum Shape {
    Base(TokenStream, Ident),
    Boxed(Box<Self>),
    Delimited(Box<Self>),
    Optional(Box<Self>),
    /// A repetition, carrying the separator it prints between elements.
    Repeated(Box<Self>),
    Punctuated(Box<Self>),
}

impl Shape {
    fn token(ident: Ident, pattern: &TokenStream) -> Self {
        Self::Base(quote!(::m2_syn::Token![#pattern]), ident)
    }

    fn base_ident(&self) -> &Ident {
        match self {
            Self::Base(_, ident) => ident,
            Self::Boxed(inner)
            | Self::Delimited(inner)
            | Self::Optional(inner)
            | Self::Repeated(inner)
            | Self::Punctuated(inner) => inner.base_ident(),
        }
    }
}

fn expand_struct(definition: &StructDefinition) -> Result<TokenStream> {
    let name = &definition.name;
    let kind = definition.cst_kind();
    match &definition.fields {
        StructFields::Leaf(leaf) => {
            let output = format_ident!("output");
            let emit = match leaf {
                LeafKind::Empty => quote! {},
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
                LeafKind::Text => quote! {
                    output.extend([::m2_syn::IdentToken::from_span(
                        ::m2_syn::Spanned::expect_span(self),
                    )]);
                },
                LeafKind::Symbol => quote!(output.extend([self.ident.clone()]);),
            };
            let pretty_text = if matches!(leaf, LeafKind::Empty) {
                quote!("∅")
            } else {
                quote!(self.text())
            };
            let constructor = match leaf {
                LeafKind::Symbol => quote! {
                    Self {
                        ident: ::m2_syn::IdentToken::new(text.as_ref(), span),
                    }
                },
                LeafKind::Empty => quote! {
                    let _ = text;
                    Self { span }
                },
                _ => quote! {
                    let span = span.with_text(text.as_ref());
                    Self { span }
                },
            };
            let text_api = match leaf {
                LeafKind::Empty => quote! {},
                LeafKind::Symbol => quote! {
                    /// Returns the leaf spelling.
                    pub fn text(&self) -> &str {
                        self.ident.text()
                    }

                    /// The identifier token spelling this symbol.
                    pub fn ident(&self) -> &::m2_syn::IdentToken {
                        &self.ident
                    }

                    /// See [`::m2_syn::IdentToken::package_qualifier`].
                    pub fn package_qualifier(&self) -> Option<&::m2_syn::Span> {
                        self.ident.package_qualifier()
                    }

                    /// See [`::m2_syn::IdentToken::index_qualifier`].
                    pub fn index_qualifier(&self) -> Option<&::m2_syn::Span> {
                        self.ident.index_qualifier()
                    }

                    /// Replaces the spelling with a new immutable source snapshot.
                    pub fn set_text(&mut self, text: impl AsRef<str>) {
                        let span = ::m2_syn::Spanned::expect_span(&self.ident);
                        self.ident = ::m2_syn::IdentToken::new(text.as_ref(), span);
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
            let pretty_impl = quote! {
                impl ::m2_syn::PrettyTree for #name {
                    fn pretty_tree(&self) -> ::m2_syn::PrettyNode {
                        ::m2_syn::PrettyNode::token(
                            stringify!(#name),
                            #pretty_text,
                            ::m2_syn::Spanned::expect_span(self),
                        )
                    }
                }
            };
            Ok(quote! {

            impl #name {
                /// Creates a leaf from its text and source span.
                pub fn new(text: impl AsRef<str>, span: ::m2_syn::Span) -> Self {
                    #constructor
                }

                #text_api
            }

            impl ::m2_syn::Lex for #name {
                fn to_tokens(&self, #output: &mut ::m2_syn::TokenStream) {
                    #emit
                }
            }


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

        StructFields::Product(fields) => {
            // A kind spelled explicitly may be shared by several structs, so
            // the children have to tell them apart.
            let structural_matches = if definition.cst_kind.is_some() {
                fields
                    .iter()
                    .filter_map(FieldDefinition::required_match)
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let typed_delimiter = definition
                .wrapper
                .and_then(Wrapper::delimiter)
                .map(Delimiter::typed);
            let retains_span = matches!(definition.wrapper, Some(Wrapper::Literal(_)));
            let mut constructor_arguments = fields
                .iter()
                .map(|field| {
                    let binding = &field.name;
                    let ty = field.shape.ty();
                    quote! { #binding: #ty }
                })
                .collect::<Vec<_>>();
            if retains_span {
                constructor_arguments.push(quote!(span: ::m2_syn::Span));
            }
            let constructor_fields = fields.iter().map(|field| &field.name);
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
            let retained_span_member = retains_span.then(|| quote!(span,));
            let constructor = quote!({
                #constructor_delimiter
                Self { #(#constructor_fields,)* #delimiter_member #retained_span_member }
            });
            let span_fields = fields.iter().map(|field| {
                let binding = &field.name;
                quote! { ::m2_syn::Spanned::try_span(&#binding) }
            });
            let constructor_span = typed_delimiter
                .as_ref()
                .map(|_| quote!(let span = ::m2_syn::Span::join_all([#(#span_fields),*]);));
            let migrate_fields = fields
                .iter()
                .map(|field| {
                    field.migrate(fields.len() == 1 && matches!(field.shape, Shape::Delimited(_)))
                })
                .collect::<Result<Vec<_>>>()?;
            let migrated_fields = fields.iter().map(|field| &field.name);
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
            let migrated =
                quote!(Self { #(#migrated_fields,)* #delimiter_member #retained_span_member });
            let field_separator = definition.wrapper.map_or(" ", Wrapper::field_separator);
            let print_fields = fields
                .iter()
                .map(|field| field.to_tokens(field_separator))
                .collect::<Result<Vec<_>>>()?;
            let pretty_impl = {
                let pretty_fields = fields.iter().map(|field| {
                    let member = &field.name;
                    let label = member.to_string();
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
                            node
                        }
                    }
                }
            };
            let emit_contents = match definition.wrapper {
                Some(Wrapper::Literal(literal)) => {
                    let kind = match literal {
                        Literal::String => quote!(String),
                        Literal::RawString => quote!(RawString),
                    };
                    quote! {
                        output.extend([::m2_syn::LiteralToken::from_span(
                            ::m2_syn::LiteralKind::#kind,
                            ::m2_syn::Spanned::expect_span(self),
                        )]);
                    }
                }
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
            let cell_aware_source = definition.cell_stream.then(|| {
                quote! {
                    fn to_m2(&self) -> qf!(String) {
                        ::m2_syn::Tokenize::to_cell_stream(self)
                            .expect("typed source-file cells are complete")
                            .to_string()
                    }

                    /// Every comment of the file, including those no cell is
                    /// next to.
                    fn comments(&self) -> ::m2_syn::Comments {
                        ::m2_syn::Comments::within(&::m2_syn::Spanned::expect_span(self))
                    }
                }
            });
            Ok(quote! {

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
                        #emit_contents
                    }

                    #cell_aware_source
                }

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

/// One of the three generated traversals over the typed graph.
#[derive(Clone, Copy)]
pub(crate) enum Traversal {
    Visit,
    VisitMut,
    Fold,
}

impl Traversal {
    const ALL: [Self; 3] = [Self::Visit, Self::VisitMut, Self::Fold];

    /// The trait method, and free walker, that handles one syntax type.
    pub(crate) fn method(self, name: &Ident) -> Ident {
        let snake = to_snake_case(&name.to_string());
        match self {
            Self::Visit => format_ident!("visit_{snake}"),
            Self::VisitMut => format_ident!("visit_{snake}_mut"),
            Self::Fold => format_ident!("fold_{snake}"),
        }
    }

    fn expand<'a>(self, names: impl Iterator<Item = &'a Ident>) -> TokenStream {
        let (methods, walkers): (Vec<_>, Vec<_>) = names.map(|name| self.expand_type(name)).unzip();
        match self {
            Self::Visit => quote! {
                /// Shared-reference traversal over the typed graph.
                pub mod visit {
                    use super::*;

                    /// Per-node visiting rules, each defaulting to the free walker
                    /// of the same name.
                    pub trait Visit<'ast> {
                        #(#methods)*

                        /// Visits a token, leaf, or literal. The free walker of
                        /// every such node calls it.
                        fn visit_terminal(&mut self, _terminal: &'ast dyn ::m2_syn::Spanned) {}
                    }

                    #(#walkers)*
                }
            },
            Self::VisitMut => quote! {
                /// In-place traversal over the typed graph.
                pub mod visit_mut {
                    use super::*;

                    /// Per-node mutation rules, each defaulting to the free walker
                    /// of the same name.
                    pub trait VisitMut {
                        #(#methods)*

                        /// Visits the delimiter pair of a delimited node. The
                        /// family must be kept; spans may change.
                        fn visit_delimiter_mut(&mut self, _delimiter: &mut ::m2_syn::Delimiter) {}
                    }

                    #(#walkers)*
                }
            },
            Self::Fold => quote! {
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
            },
        }
    }

    fn expand_type(self, name: &Ident) -> (TokenStream, TokenStream) {
        let method = self.method(name);
        let doc = format!("Handles a `{name}` node.");
        let walker_doc = format!("Walks the children of a `{name}` node.");
        match self {
            Self::Visit => (
                quote! {
                    #[doc = #doc]
                    fn #method(&mut self, node: &'ast #name) {
                        self::#method(self, node);
                    }
                },
                quote! {
                    #[doc = #walker_doc]
                    pub fn #method<'ast, V: Visit<'ast> + ?Sized>(visitor: &mut V, node: &'ast #name) {
                        super::__walk::WalkChildren::walk_children(node, visitor);
                    }
                },
            ),
            Self::VisitMut => (
                quote! {
                    #[doc = #doc]
                    fn #method(&mut self, node: &mut #name) {
                        self::#method(self, node);
                    }
                },
                quote! {
                    #[doc = #walker_doc]
                    pub fn #method<V: VisitMut + ?Sized>(visitor: &mut V, node: &mut #name) {
                        super::__walk::WalkChildrenMut::walk_children_mut(node, visitor);
                    }
                },
            ),
            Self::Fold => (
                quote! {
                    #[doc = #doc]
                    fn #method(&mut self, node: #name) -> #name {
                        self::#method(self, node)
                    }
                },
                quote! {
                    #[doc = #walker_doc]
                    pub fn #method<F: Fold + ?Sized>(folder: &mut F, node: #name) -> #name {
                        super::__walk::FoldChildren::fold_children(node, folder)
                    }
                },
            ),
        }
    }
}

/// Tokens have no children: visiting one reports it as a terminal, and
/// folding one keeps it.
fn expand_token_walk(name: &Ident) -> TokenStream {
    quote! {
        impl __walk::WalkChildren for #name {
            fn walk_children<'ast, V>(&'ast self, _visitor: &mut V)
            where
                V: visit::Visit<'ast> + ?Sized,
            {}
        }

        impl __walk::WalkChildrenMut for #name {
            fn walk_children_mut<V>(&mut self, _visitor: &mut V)
            where
                V: visit_mut::VisitMut + ?Sized,
            {}
        }

        impl __walk::FoldChildren for #name {
            fn fold_children<F>(self, _folder: &mut F) -> Self
            where
                F: fold::Fold + ?Sized,
            {
                self
            }
        }

        impl __walk::VisitValue for #name {
            fn visit_value<'ast, V>(&'ast self, visitor: &mut V)
            where
                V: visit::Visit<'ast> + ?Sized,
            {
                visitor.visit_terminal(self);
            }
        }

        impl __walk::VisitValueMut for #name {
            fn visit_value_mut<V>(&mut self, _visitor: &mut V)
            where
                V: visit_mut::VisitMut + ?Sized,
            {}
        }

        impl __walk::FoldValue for #name {
            fn fold_value<F>(self, _folder: &mut F) -> Self
            where
                F: fold::Fold + ?Sized,
            {
                self
            }
        }
    }
}

fn expand_walk_support() -> TokenStream {
    quote! {
        #[doc(hidden)]
        pub(crate) mod __walk {
            use super::*;

            pub(crate) trait VisitValue {
                fn visit_value<'ast, V>(&'ast self, visitor: &mut V)
                where
                    V: super::visit::Visit<'ast> + ?Sized;
            }

            pub(crate) trait VisitValueMut {
                fn visit_value_mut<V>(&mut self, visitor: &mut V)
                where
                    V: super::visit_mut::VisitMut + ?Sized;
            }

            pub(crate) trait FoldValue: Sized {
                fn fold_value<F>(self, folder: &mut F) -> Self
                where
                    F: super::fold::Fold + ?Sized;
            }

            pub(crate) trait WalkChildren {
                fn walk_children<'ast, V>(&'ast self, visitor: &mut V)
                where
                    V: super::visit::Visit<'ast> + ?Sized;
            }

            pub(crate) trait WalkChildrenMut {
                fn walk_children_mut<V>(&mut self, visitor: &mut V)
                where
                    V: super::visit_mut::VisitMut + ?Sized;
            }

            pub(crate) trait FoldChildren: Sized {
                fn fold_children<F>(self, folder: &mut F) -> Self
                where
                    F: super::fold::Fold + ?Sized;
            }

            impl<T> VisitValue for Box<T>
            where
                T: VisitValue + ?Sized,
            {
                fn visit_value<'ast, V>(&'ast self, visitor: &mut V)
                where
                    V: super::visit::Visit<'ast> + ?Sized,
                {
                    T::visit_value(self, visitor);
                }
            }

            impl<T> VisitValueMut for Box<T>
            where
                T: VisitValueMut + ?Sized,
            {
                fn visit_value_mut<V>(&mut self, visitor: &mut V)
                where
                    V: super::visit_mut::VisitMut + ?Sized,
                {
                    T::visit_value_mut(self, visitor);
                }
            }

            impl<T> FoldValue for Box<T>
            where
                T: FoldValue,
            {
                fn fold_value<F>(self, folder: &mut F) -> Self
                where
                    F: super::fold::Fold + ?Sized,
                {
                    Box::new(T::fold_value(*self, folder))
                }
            }

            impl<T> VisitValue for Option<T>
            where
                T: VisitValue,
            {
                fn visit_value<'ast, V>(&'ast self, visitor: &mut V)
                where
                    V: super::visit::Visit<'ast> + ?Sized,
                {
                    if let Some(value) = self {
                        T::visit_value(value, visitor);
                    }
                }
            }

            impl<T> VisitValueMut for Option<T>
            where
                T: VisitValueMut,
            {
                fn visit_value_mut<V>(&mut self, visitor: &mut V)
                where
                    V: super::visit_mut::VisitMut + ?Sized,
                {
                    if let Some(value) = self {
                        T::visit_value_mut(value, visitor);
                    }
                }
            }

            impl<T> FoldValue for Option<T>
            where
                T: FoldValue,
            {
                fn fold_value<F>(self, folder: &mut F) -> Self
                where
                    F: super::fold::Fold + ?Sized,
                {
                    self.map(|value| T::fold_value(value, folder))
                }
            }

            impl<T> VisitValue for Vec<T>
            where
                T: VisitValue,
            {
                fn visit_value<'ast, V>(&'ast self, visitor: &mut V)
                where
                    V: super::visit::Visit<'ast> + ?Sized,
                {
                    for value in self {
                        T::visit_value(value, visitor);
                    }
                }
            }

            impl<T> VisitValueMut for Vec<T>
            where
                T: VisitValueMut,
            {
                fn visit_value_mut<V>(&mut self, visitor: &mut V)
                where
                    V: super::visit_mut::VisitMut + ?Sized,
                {
                    for value in self {
                        T::visit_value_mut(value, visitor);
                    }
                }
            }

            impl<T> FoldValue for Vec<T>
            where
                T: FoldValue,
            {
                fn fold_value<F>(self, folder: &mut F) -> Self
                where
                    F: super::fold::Fold + ?Sized,
                {
                    self.into_iter()
                        .map(|value| T::fold_value(value, folder))
                        .collect()
                }
            }

            impl<T, P> VisitValue for ::m2_syn::Punctuated<T, P>
            where
                T: VisitValue,
            {
                fn visit_value<'ast, V>(&'ast self, visitor: &mut V)
                where
                    V: super::visit::Visit<'ast> + ?Sized,
                {
                    for value in self.iter() {
                        T::visit_value(value, visitor);
                    }
                }
            }

            impl<T, P> VisitValueMut for ::m2_syn::Punctuated<T, P>
            where
                T: VisitValueMut,
                P: VisitValueMut,
            {
                fn visit_value_mut<V>(&mut self, visitor: &mut V)
                where
                    V: super::visit_mut::VisitMut + ?Sized,
                {
                    T::visit_value_mut(self.first_mut(), visitor);
                    for (separator, value) in self.pairs_mut() {
                        P::visit_value_mut(separator, visitor);
                        T::visit_value_mut(value, visitor);
                    }
                }
            }

            impl<T, P> FoldValue for ::m2_syn::Punctuated<T, P>
            where
                T: FoldValue,
            {
                fn fold_value<F>(self, folder: &mut F) -> Self
                where
                    F: super::fold::Fold + ?Sized,
                {
                    self.map_values(|value| T::fold_value(value, folder))
                }
            }

            impl<T, D> VisitValue for ::m2_syn::Delimited<T, D>
            where
                T: VisitValue,
            {
                fn visit_value<'ast, V>(&'ast self, visitor: &mut V)
                where
                    V: super::visit::Visit<'ast> + ?Sized,
                {
                    T::visit_value(&self.contents, visitor);
                }
            }

            impl<T, D> VisitValueMut for ::m2_syn::Delimited<T, D>
            where
                T: VisitValueMut,
                D: ::m2_syn::DelimiterValue,
            {
                fn visit_value_mut<V>(&mut self, visitor: &mut V)
                where
                    V: super::visit_mut::VisitMut + ?Sized,
                {
                    let mut delimiter = ::m2_syn::DelimiterValue::delimiter(&self.delimiter);
                    visitor.visit_delimiter_mut(&mut delimiter);
                    self.delimiter = ::m2_syn::DelimiterValue::from_delimiter(delimiter)
                        .expect("a delimiter visitor keeps the delimiter family");
                    T::visit_value_mut(&mut self.contents, visitor);
                }
            }

            impl<T, D> FoldValue for ::m2_syn::Delimited<T, D>
            where
                T: FoldValue,
            {
                fn fold_value<F>(self, folder: &mut F) -> Self
                where
                    F: super::fold::Fold + ?Sized,
                {
                    let (delimiter, contents) = self.into_parts();
                    ::m2_syn::Delimited::new(delimiter, T::fold_value(contents, folder))
                }
            }
        }
    }
}

impl FieldDefinition {
    fn required_match(&self) -> Option<TokenStream> {
        if !matches!(self.shape, Shape::Base(_, _)) {
            return None;
        }
        let ty = self.shape.ty();
        let field = self.name.to_string();
        Some(quote!(node.children().any(|child| {
            child.field == Some(#field) && <#ty as ::m2_syn::Migrate<N>>::matches(&child.node)
        })))
    }

    fn migrate(&self, delimited_newtype: bool) -> Result<TokenStream> {
        let binding = &self.name;
        if delimited_newtype {
            let Shape::Delimited(contents) = &self.shape else {
                unreachable!("only delimited newtypes take this migration path")
            };
            let contents = contents.migrate_from_cursor(quote!(__m2_syn_delimited_children))?;
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
            let field = binding.to_string();
            let contents = contents.migrate_from_cursor(quote!(__m2_syn_delimited_children))?;
            return Ok(quote! {
                let __m2_syn_delimited_node = children.required_field(#field)?;
                let mut __m2_syn_delimited_children =
                    ::m2_syn::ChildCursor::new(&__m2_syn_delimited_node);
                let __m2_syn_delimiter = __m2_syn_delimited_children.required_delimiter()?;
                let __m2_syn_contents = #contents;
                let #binding = ::m2_syn::Delimited::new(
                    __m2_syn_delimiter,
                    __m2_syn_contents,
                );
            });
        }
        let element = self.shape.element()?;
        let base_type = element.ty();
        let field = binding.to_string();
        let selected = match &self.shape {
            Shape::Base(_, _) | Shape::Boxed(_) => quote!(children.required_field(#field)?),
            Shape::Optional(inner) if !inner.is_sequence() => {
                quote!(children.optional_field(#field))
            }
            _ => quote!(children.repeated_field(#field)),
        };
        let stored_element = self.shape.storage_element();
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
    /// spaced away from it, which is what `;`, `,`, zero-width end-of-cell
    /// markers, and the usually implied `SPACE` do.
    fn is_attached(&self) -> bool {
        [
            quote!(;),
            quote!(,),
            quote!(SPACE),
            quote!(EOC),
            quote!(BOF),
            quote!(EOF),
        ]
        .iter()
        .any(|spelling| {
            token_type_name(spelling).is_ok_and(|name| name == *self.shape.base_ident())
        })
    }

    fn to_tokens(&self, field_separator: &'static str) -> Result<TokenStream> {
        let member = &self.name;
        self.shape.element()?;
        let write = if let Shape::Repeated(_) = self.shape {
            // `Option` and `Punctuated` carry their own `Lex` impls; a bare
            // `Vec` is the only functor that has to space its elements here.
            if field_separator.is_empty() {
                quote! {
                    for value in &self.#member {
                        ::m2_syn::Lex::to_tokens(value, &mut field_output);
                    }
                }
            } else {
                let push_separator = push_source_separator(&quote!(field_output), " ");
                quote! {
                    for (index, value) in self.#member.iter().enumerate() {
                        if index != 0 {
                            #push_separator
                        }
                        ::m2_syn::Lex::to_tokens(value, &mut field_output);
                    }
                }
            }
        } else {
            quote! { ::m2_syn::Lex::to_tokens(&self.#member, &mut field_output); }
        };
        let separator =
            (!self.is_attached() && !field_separator.is_empty()).then_some(field_separator);
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
        "" | " " => quote! {},
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
    /// The syntax element held by this field's functor stack.
    fn element(&self) -> Result<&Self> {
        match self {
            Self::Base(_, _) => Ok(self),
            Self::Boxed(inner)
            | Self::Delimited(inner)
            | Self::Optional(inner)
            | Self::Repeated(inner)
            | Self::Punctuated(inner) => inner.element(),
        }
    }

    fn is_sequence(&self) -> bool {
        match self {
            Self::Repeated(_) | Self::Punctuated(_) => true,
            Self::Boxed(inner) | Self::Optional(inner) => inner.is_sequence(),
            Self::Delimited(_) => false,
            Self::Base(_, _) => false,
        }
    }

    fn storage_element(&self) -> &Self {
        match self {
            Self::Optional(inner)
            | Self::Repeated(inner)
            | Self::Punctuated(inner)
            | Self::Delimited(inner) => inner.storage_element(),
            Self::Base(_, _) | Self::Boxed(_) => self,
        }
    }
}

impl Shape {
    fn migrate_from_cursor(&self, cursor: TokenStream) -> Result<TokenStream> {
        match self {
            Self::Base(_, _) => {
                let ty = self.ty();
                let stored = self.clone();
                let value = stored.wrap_value(quote!(
                    <#ty as ::m2_syn::Migrate<N>>::migrate(
                        #cursor.required_matching::<#ty>()?
                    )?
                ));
                Ok(value)
            }
            Self::Boxed(inner) => {
                let value = inner.migrate_from_cursor(cursor)?;
                Ok(quote!(<qf!(Box<_>)>::new(#value)))
            }
            Self::Optional(inner) if matches!(inner.as_ref(), Self::Base(_, _)) => {
                let ty = inner.ty();
                let stored = inner.clone();
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
            Self::Repeated(inner) if matches!(inner.as_ref(), Self::Base(_, _)) => {
                let ty = inner.ty();
                let stored = inner.clone();
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
                let stored = inner.clone();
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
                let stored = element.clone();
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
            Self::Repeated(inner) => {
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
            | Self::Repeated(_)
            | Self::Punctuated(_) => value,
        }
    }
}

fn expand_enum(definition: &EnumDefinition) -> TokenStream {
    let name = &definition.name;
    let variants = definition.variants.iter().map(|variant| {
        let name = &variant.name;
        let ty = variant.shape.ty();
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
        let stored = variant.shape.clone();
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
    let pretty_impl = {
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
    };
    let spelling_api = definition.operator.map(|_| {
        let from_token_arms = definition.variants.iter().map(|variant| {
            let variant_name = &variant.name;
            let ty = variant.shape.ty();
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
            let ty = variant.shape.ty();
            quote!(Self::#variant_name(_) => <#ty as ::m2_syn::Token>::SPELLING)
        });
        quote! {
            impl #name {
                /// Converts a matching raw operator token into this category.
                pub fn from_token_tree(token: ::m2_syn::TokenTree) -> qf!(Option<Self>) {
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
    let declaration = definition.operator.map(|_| {
        quote! {
            #[derive(::m2_syn::__SyntaxWalk)]
            #[derive(Debug, ::m2_syn::Spanned)]
            #[doc = concat!("The typed `", stringify!(#name), "` syntax category.")]
            pub enum #name {
                #(#variants,)*
            }
        }
    });
    let conversions = expand_conversions(definition);
    quote! {
        #declaration
        #conversions

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

/// `From` each variant payload that no other variant shares, wrapping it in
/// the variant's box when it has one.
fn expand_conversions(definition: &EnumDefinition) -> TokenStream {
    let name = &definition.name;
    let payload = |variant: &VariantDefinition| match &variant.shape {
        Shape::Base(ty, _) => Some((ty.clone(), quote!(value))),
        Shape::Boxed(inner) => Some((inner.ty(), quote!(<qf!(Box<_>)>::new(value)))),
        _ => None,
    };
    let payloads = definition
        .variants
        .iter()
        .filter_map(payload)
        .collect::<Vec<_>>();
    let unique = |ty: &TokenStream| {
        payloads
            .iter()
            .filter(|(other, _)| other.to_string() == ty.to_string())
            .count()
            == 1
    };
    let impls = definition.variants.iter().filter_map(|variant| {
        let (ty, value) = payload(variant).filter(|(ty, _)| unique(ty))?;
        let variant = &variant.name;
        Some(quote! {
            impl From<#ty> for #name {
                fn from(value: #ty) -> Self {
                    Self::#variant(#value)
                }
            }
        })
    });
    quote!(#(#impls)*)
}

pub(crate) fn to_snake_case(name: &str) -> String {
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
