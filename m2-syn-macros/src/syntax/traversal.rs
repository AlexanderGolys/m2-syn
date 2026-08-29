use super::{
    EnumDefinition, ProductStyle, Shape, StructDefinition, StructFields, SyntaxContext, Wrapper,
    to_snake_case,
};
use proc_macro2::{Ident, TokenStream};
use quote::{format_ident, quote};

#[derive(Clone, Copy)]
pub(super) enum Traversal {
    Visit,
    VisitMut,
    Fold,
}

/// How a lifting joins the code produced for a field's elements.
///
/// A functor decides *what* to walk — dereference a box, enter an `Option`,
/// loop over a sequence — and the ambient decides how the element code is
/// combined once walked. Every traversal is one of these two.
#[derive(Clone, Copy)]
enum Ambient {
    /// Statements executed for each element in order, reaching it through
    /// `access`.
    Sequence { access: Access },
    /// An expression rebuilding the field from each element's result.
    Rebuild,
}

/// Which reference a [`Ambient::Sequence`] traversal takes to an element.
#[derive(Clone, Copy)]
enum Access {
    Shared,
    Unique,
}

impl Access {
    /// Reborrows a field whose own type is `Box<T>` or `Option<T>`, so that the
    /// element inside is reached by reference rather than by value.
    fn reborrow(self, value: TokenStream) -> TokenStream {
        match self {
            Self::Shared => quote!((#value).as_ref()),
            Self::Unique => quote!((#value).as_mut()),
        }
    }
}

impl Traversal {
    fn ambient(self) -> Ambient {
        match self {
            Self::Visit => Ambient::Sequence {
                access: Access::Shared,
            },
            Self::VisitMut => Ambient::Sequence {
                access: Access::Unique,
            },
            Self::Fold => Ambient::Rebuild,
        }
    }

    /// The per-node action at a leaf: the dispatch back into the trait.
    fn leaf(self, name: &Ident, value: TokenStream) -> TokenStream {
        let snake = to_snake_case(&name.to_string());
        match self {
            Self::Visit => {
                let method = format_ident!("visit_{snake}");
                quote!(visitor.#method(#value);)
            }
            Self::VisitMut => {
                let method = format_ident!("visit_{snake}_mut");
                quote!(visitor.#method(#value);)
            }
            Self::Fold => {
                let method = format_ident!("fold_{snake}");
                quote!(folder.#method(#value))
            }
        }
    }

    /// Lifts [`Self::leaf`] over the functors a field is wrapped in.
    ///
    /// This is the entire per-functor table. Each arm contributes only how to
    /// reach the elements; the ambient decides how their code is rejoined.
    /// Adding a functor adds an arm here and nowhere else.
    pub(super) fn lift(self, shape: &Shape, value: TokenStream) -> TokenStream {
        match (shape, self.ambient()) {
            (Shape::Base(_, name), _) => self.leaf(name, value),

            (Shape::Boxed(inner), Ambient::Sequence { access }) => {
                self.lift(inner, access.reborrow(value))
            }
            (Shape::Boxed(inner), Ambient::Rebuild) => {
                let element = self.lift(inner, quote!(*#value));
                quote!(<qf!(Box<_>)>::new(#element))
            }

            (Shape::Delimited(inner), Ambient::Sequence { access }) => {
                if let Shape::Optional(inner) = inner.as_ref() {
                    let element = self.lift(inner, quote!(value));
                    let contents = match access {
                        Access::Shared => quote!((#value).contents.as_ref()),
                        Access::Unique => quote!((#value).contents.as_mut()),
                    };
                    return quote!(if let Some(value) = #contents { #element });
                }
                let contents = match access {
                    Access::Shared => quote!(&(#value).contents),
                    Access::Unique => quote!(&mut (#value).contents),
                };
                self.lift(inner, contents)
            }
            (Shape::Delimited(inner), Ambient::Rebuild) => {
                let contents = self.lift(inner, quote!(#value.contents));
                quote!(::m2_syn::Delimited::new(#value.delimiter, #contents))
            }

            (Shape::Optional(inner), Ambient::Sequence { access }) => {
                let element = self.lift(inner, quote!(value));
                let value = access.reborrow(value);
                quote!(if let Some(value) = #value { #element })
            }
            (
                Shape::Repeated(inner, _) | Shape::Punctuated(inner),
                Ambient::Sequence { access },
            ) => {
                let element = self.lift(inner, quote!(value));
                let values = match access {
                    Access::Shared => quote!((#value).iter()),
                    Access::Unique => quote!((#value).iter_mut()),
                };
                quote!(for value in #values { #element })
            }

            (Shape::Optional(inner), Ambient::Rebuild) => {
                let element = self.lift(inner, quote!(value));
                quote!(#value.map(|value| #element))
            }
            (Shape::Punctuated(inner), Ambient::Rebuild) => {
                let element = self.lift(inner, quote!(value));
                quote!({
                    #value.map_values(|value| #element)
                })
            }
            (Shape::Repeated(inner, _), Ambient::Rebuild) => {
                let element = self.lift(inner, quote!(value));
                quote!(#value.into_iter().map(|value| #element).collect())
            }
        }
    }

    pub(super) fn empty_walker(self, name: &Ident, ty: TokenStream) -> TokenStream {
        let snake = to_snake_case(&name.to_string());
        match self {
            Self::Visit => {
                let method = format_ident!("visit_{snake}");
                quote! {
                    #[doc = concat!("Walks the children of a `", stringify!(#name), "` node.")]
                    pub fn #method<'ast, V>(_visitor: &mut V, _node: &'ast #ty)
                    where
                        V: Visit<'ast> + ?Sized,
                    {}
                }
            }
            Self::VisitMut => {
                let method = format_ident!("visit_{snake}_mut");
                quote! {
                    #[doc = concat!("Walks the mutable children of a `", stringify!(#name), "` node.")]
                    pub fn #method<V>(_visitor: &mut V, _node: &mut #ty)
                    where
                        V: VisitMut + ?Sized,
                    {}
                }
            }
            Self::Fold => {
                let method = format_ident!("fold_{snake}");
                quote! {
                    #[doc = concat!("Folds the children of a `", stringify!(#name), "` node.")]
                    pub fn #method<F>(_folder: &mut F, node: #ty) -> #ty
                    where
                        F: Fold + ?Sized,
                    {
                        node
                    }
                }
            }
        }
    }

    pub(super) fn struct_walker(
        self,
        definition: &StructDefinition,
        context: &SyntaxContext<'_>,
    ) -> TokenStream {
        let name = &definition.name;
        let StructFields::Product { fields, style } = &definition.fields else {
            unreachable!("only product structs have generated child walkers")
        };
        let has_typed_delimiter = definition.wrapper.and_then(Wrapper::delimiter).is_some();
        let has_retained_span = matches!(definition.wrapper, Some(Wrapper::Literal(_)));
        let has_comments = definition.comments;
        let snake = to_snake_case(&name.to_string());
        match self {
            Self::Visit => {
                let method = format_ident!("visit_{snake}");
                let statements = fields.iter().map(|field| {
                    let member = &field.member;
                    let stored = field.shape.stored_shape(context, false);
                    self.lift(&stored, quote!(&node.#member))
                });
                quote! {
                    #[doc = concat!("Walks the children of a `", stringify!(#name), "` node.")]
                    pub fn #method<'ast, V>(visitor: &mut V, node: &'ast #name)
                    where
                        V: Visit<'ast> + ?Sized,
                    {
                        #(#statements)*
                    }
                }
            }
            Self::VisitMut => {
                let method = format_ident!("visit_{snake}_mut");
                let statements = fields.iter().map(|field| {
                    let member = &field.member;
                    let stored = field.shape.stored_shape(context, false);
                    self.lift(&stored, quote!(&mut node.#member))
                });
                quote! {
                    #[doc = concat!("Walks the mutable children of a `", stringify!(#name), "` node.")]
                    pub fn #method<V>(visitor: &mut V, node: &mut #name)
                    where
                        V: VisitMut + ?Sized,
                    {
                        #(#statements)*
                    }
                }
            }
            Self::Fold => {
                let method = format_ident!("fold_{snake}");
                let named_fields = fields.iter().map(|field| {
                    let member = &field.member;
                    let stored = field.shape.stored_shape(context, false);
                    let folded = self.lift(&stored, quote!(node.#member));
                    quote!(#member: #folded)
                });
                let delimiter = has_typed_delimiter.then(|| quote!(delimiter: node.delimiter,));
                let span = has_retained_span.then(|| {
                    quote!(
                        span: node.span,
                        leading_trivia: node.leading_trivia,
                        trailing_trivia: node.trailing_trivia,
                    )
                });
                let comments = has_comments.then(|| quote!(comments: node.comments,));
                let folded = match *style {
                    ProductStyle::Named => {
                        quote!(#name { #(#named_fields,)* #delimiter #span #comments })
                    }
                    ProductStyle::Tuple => {
                        let tuple_fields = fields.iter().map(|field| {
                            let member = &field.member;
                            let stored = field.shape.stored_shape(context, false);
                            self.lift(&stored, quote!(node.#member))
                        });
                        quote!(#name(#(#tuple_fields,)*))
                    }
                };
                quote! {
                    #[doc = concat!("Folds the children of a `", stringify!(#name), "` node.")]
                    pub fn #method<F>(folder: &mut F, node: #name) -> #name
                    where
                        F: Fold + ?Sized,
                    {
                        #folded
                    }
                }
            }
        }
    }

    pub(super) fn enum_walker(
        self,
        definition: &EnumDefinition,
        context: &SyntaxContext<'_>,
    ) -> TokenStream {
        let name = &definition.name;
        let snake = to_snake_case(&name.to_string());
        let borrowed_fallback = definition
            .variants
            .is_empty()
            .then(|| quote!(_ => unreachable!("empty generated syntax category")));
        match self {
            Self::Visit => {
                let method = format_ident!("visit_{snake}");
                let arms = definition.variants.iter().map(|variant| {
                    let variant_name = &variant.name;
                    let statement =
                        self.lift(&variant.shape.stored_shape(context, false), quote!(node));
                    quote!(#name::#variant_name(node) => { #statement })
                });
                quote! {
                    #[doc = concat!("Walks the active `", stringify!(#name), "` alternative.")]
                    pub fn #method<'ast, V>(visitor: &mut V, node: &'ast #name)
                    where
                        V: Visit<'ast> + ?Sized,
                    {
                        match node { #(#arms,)* #borrowed_fallback }
                    }
                }
            }
            Self::VisitMut => {
                let method = format_ident!("visit_{snake}_mut");
                let arms = definition.variants.iter().map(|variant| {
                    let variant_name = &variant.name;
                    let statement =
                        self.lift(&variant.shape.stored_shape(context, false), quote!(node));
                    quote!(#name::#variant_name(node) => { #statement })
                });
                quote! {
                    #[doc = concat!("Walks the active mutable `", stringify!(#name), "` alternative.")]
                    pub fn #method<V>(visitor: &mut V, node: &mut #name)
                    where
                        V: VisitMut + ?Sized,
                    {
                        match node { #(#arms,)* #borrowed_fallback }
                    }
                }
            }
            Self::Fold => {
                let method = format_ident!("fold_{snake}");
                let arms = definition.variants.iter().map(|variant| {
                    let variant_name = &variant.name;
                    let folded =
                        self.lift(&variant.shape.stored_shape(context, false), quote!(node));
                    quote!(#name::#variant_name(node) => #name::#variant_name(#folded))
                });
                quote! {
                    #[doc = concat!("Folds the active `", stringify!(#name), "` alternative.")]
                    pub fn #method<F>(folder: &mut F, node: #name) -> #name
                    where
                        F: Fold + ?Sized,
                    {
                        match node { #(#arms,)* }
                    }
                }
            }
        }
    }
}
