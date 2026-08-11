use super::*;

#[derive(Clone, Copy)]
pub(super) enum Traversal {
    Visit,
    VisitMut,
    Fold,
}

impl Traversal {
    pub(super) fn empty_walker(self, name: &Ident) -> TokenStream2 {
        let snake = to_snake_case(&name.to_string());
        match self {
            Self::Visit => {
                let method = format_ident!("visit_{snake}");
                quote! {
                    pub fn #method<'ast, V>(_visitor: &mut V, _node: &'ast #name)
                    where
                        V: Visit<'ast> + ?Sized,
                    {}
                }
            }
            Self::VisitMut => {
                let method = format_ident!("visit_{snake}_mut");
                quote! {
                    pub fn #method<V>(_visitor: &mut V, _node: &mut #name)
                    where
                        V: VisitMut + ?Sized,
                    {}
                }
            }
            Self::Fold => {
                let method = format_ident!("fold_{snake}");
                quote! {
                    pub fn #method<F>(_folder: &mut F, node: #name) -> #name
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
        name: &Ident,
        style: ProductStyle,
        fields: &[FieldDefinition],
        token_like: &BTreeSet<SyntaxTypeName>,
    ) -> TokenStream2 {
        let snake = to_snake_case(&name.to_string());
        match self {
            Self::Visit => {
                let method = format_ident!("visit_{snake}");
                let statements = fields.iter().map(|field| {
                    let member = &field.member;
                    field
                        .shape
                        .stored_shape(token_like, false)
                        .visit(quote!(&node.#member))
                });
                quote! {
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
                    field
                        .shape
                        .stored_shape(token_like, false)
                        .visit_mut(quote!(&mut node.#member))
                });
                quote! {
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
                let folded = match style {
                    ProductStyle::Named => {
                        let fields = fields.iter().map(|field| {
                            let member = &field.member;
                            let value = field
                                .shape
                                .stored_shape(token_like, false)
                                .fold(quote!(node.#member));
                            quote!(#member: #value)
                        });
                        quote!(#name { #(#fields,)* })
                    }
                    ProductStyle::Unnamed => {
                        let fields = fields.iter().map(|field| {
                            let member = &field.member;
                            field
                                .shape
                                .stored_shape(token_like, false)
                                .fold(quote!(node.#member))
                        });
                        quote!(#name(#(#fields,)*))
                    }
                };
                quote! {
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

    pub(super) fn enum_walker(self, definition: &EnumDefinition) -> TokenStream2 {
        let name = &definition.name;
        let snake = to_snake_case(&name.to_string());
        match self {
            Self::Visit => {
                let method = format_ident!("visit_{snake}");
                let arms = definition.variants.iter().map(|variant| {
                    let variant_name = &variant.name;
                    let statement = variant.shape.visit(quote!(node), true);
                    quote!(#name::#variant_name(node) => { #statement })
                });
                quote! {
                    pub fn #method<'ast, V>(visitor: &mut V, node: &'ast #name)
                    where
                        V: Visit<'ast> + ?Sized,
                    {
                        match node { #(#arms,)* }
                    }
                }
            }
            Self::VisitMut => {
                let method = format_ident!("visit_{snake}_mut");
                let arms = definition.variants.iter().map(|variant| {
                    let variant_name = &variant.name;
                    let statement = variant.shape.visit_mut(quote!(node), true);
                    quote!(#name::#variant_name(node) => { #statement })
                });
                quote! {
                    pub fn #method<V>(visitor: &mut V, node: &mut #name)
                    where
                        V: VisitMut + ?Sized,
                    {
                        match node { #(#arms,)* }
                    }
                }
            }
            Self::Fold => {
                let method = format_ident!("fold_{snake}");
                let arms = definition.variants.iter().map(|variant| {
                    let variant_name = &variant.name;
                    let folded = variant.shape.fold(quote!(node), true);
                    quote!(#name::#variant_name(node) => #name::#variant_name(#folded))
                });
                quote! {
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

impl TypeShape {
    fn visit(&self, value: TokenStream2, stored: bool) -> TokenStream2 {
        match self {
            Self::Base(_, ident) => {
                let method = format_ident!("visit_{}", to_snake_case(&ident.to_string()));
                if stored {
                    quote!(visitor.#method(#value);)
                } else {
                    quote!(visitor.#method((#value).as_ref());)
                }
            }
            Self::Boxed(inner) => inner.visit(quote!((#value).as_ref()), true),
            Self::Optional(inner) => {
                let visit = inner.visit(quote!(value), true);
                quote!(if let Some(value) = (#value).as_ref() { #visit })
            }
            Self::Repeated(inner) => {
                let visit = inner.visit(quote!(value), true);
                quote!(for value in #value { #visit })
            }
        }
    }

    fn visit_mut(&self, value: TokenStream2, stored: bool) -> TokenStream2 {
        match self {
            Self::Base(_, ident) => {
                let method = format_ident!("visit_{}_mut", to_snake_case(&ident.to_string()));
                if stored {
                    quote!(visitor.#method(#value);)
                } else {
                    quote!(visitor.#method((#value).as_mut());)
                }
            }
            Self::Boxed(inner) => inner.visit_mut(quote!((#value).as_mut()), true),
            Self::Optional(inner) => {
                let visit = inner.visit_mut(quote!(value), true);
                quote!(if let Some(value) = (#value).as_mut() { #visit })
            }
            Self::Repeated(inner) => {
                let visit = inner.visit_mut(quote!(value), true);
                quote!(for value in #value { #visit })
            }
        }
    }

    fn fold(&self, value: TokenStream2, stored: bool) -> TokenStream2 {
        match self {
            Self::Base(_, ident) => {
                let method = format_ident!("fold_{}", to_snake_case(&ident.to_string()));
                if stored {
                    quote!(folder.#method(#value))
                } else {
                    quote!(Box::new(folder.#method(*#value)))
                }
            }
            Self::Boxed(inner) => {
                let folded = inner.fold(quote!(*#value), true);
                quote!(Box::new(#folded))
            }
            Self::Optional(inner) => {
                let folded = inner.fold(quote!(value), true);
                quote!(#value.map(|value| #folded))
            }
            Self::Repeated(inner) => {
                let folded = inner.fold(quote!(value), true);
                quote!(#value.into_iter().map(|value| #folded).collect())
            }
        }
    }
}
