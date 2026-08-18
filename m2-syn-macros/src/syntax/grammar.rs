use super::*;
use syn::{GenericArgument, PathArguments, Type, TypeMacro, braced, parenthesized};

struct Declaration {
    name: Ident,
    expression: GrammarExpr,
}

struct GrammarField {
    name: Option<Ident>,
    expression: GrammarExpr,
}

enum GrammarExpr {
    Leaf,
    Named(Ident),
    Token { name: Ident, pattern: TokenStream },
    Product(Vec<GrammarField>),
    Sum(Vec<GrammarField>),
    Repeated(Box<Self>),
    Lines(Box<Self>),
    Optional(Box<Self>),
    Punctuated(Box<Self>),
    Positional(Box<Self>),
    Delimited(DelimiterKind, Vec<GrammarField>),
    Node(Ident, Vec<GrammarField>),
}

pub(super) fn parse(
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
) -> Result<(Vec<StructDefinition>, Vec<EnumDefinition>)> {
    let mut declarations = Vec::new();
    while !input.is_empty() {
        let item_kind = {
            let fork = input.fork();
            let _ = fork.call(syn::Attribute::parse_outer);
            let _ = fork.parse::<Visibility>();
            if fork.peek(Token![struct]) {
                Some("struct")
            } else if fork.peek(Token![enum]) {
                Some("enum")
            } else {
                None
            }
        };
        if item_kind == Some("struct") {
            declarations.push(parse_struct_declaration(input, tokens)?);
            continue;
        }
        if item_kind == Some("enum") {
            declarations.push(parse_enum_declaration(input, tokens)?);
            continue;
        }
        return Err(input.error("expected a syntax `struct` or `enum`"));
    }

    Lowerer::new(tokens).lower(declarations)
}

/// Struct bodies are *not* parsed as `syn::Fields` — `(_)` and `X?` are not
/// valid Rust type syntax, so fields are parsed with a small dedicated
/// grammar instead of borrowing Rust's. Only the leading `#[cst(kind = ..)]`
/// / `#[delimiter(..)]` attributes and the `struct` keyword itself reuse real
/// Rust syntax, since those don't clash with anything below them.
fn parse_struct_declaration(
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
) -> Result<Declaration> {
    let attrs = input.call(syn::Attribute::parse_outer)?;
    input.parse::<Visibility>()?;
    input.parse::<Token![struct]>()?;
    let name: Ident = input.parse()?;

    if input.peek(Token![;]) {
        input.parse::<Token![;]>()?;
        return Ok(Declaration {
            name,
            expression: GrammarExpr::Leaf,
        });
    }

    let content;
    braced!(content in input);
    let fields = parse_field_list(&content, tokens)?;
    let expression = wrap_struct_attributes(GrammarExpr::Product(fields), &attrs)?;
    Ok(Declaration { name, expression })
}

fn parse_field_list(
    content: ParseStream<'_>,
    tokens: &TokenDefinitions,
) -> Result<Vec<GrammarField>> {
    let mut fields = Vec::new();
    while !content.is_empty() {
        fields.push(parse_field(content, tokens)?);
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        } else if !content.is_empty() {
            return Err(content.error("expected `,` between fields"));
        }
    }
    Ok(fields)
}

/// `name: (_) Type` marks a field addressed positionally in the CST rather
/// than by a field name; `(_ lines)` additionally marks a `Vec<T>` field as
/// newline-separated. Plain `name: Type` fields are looked up by CST field
/// name, which is the common case.
fn parse_field(input: ParseStream<'_>, tokens: &TokenDefinitions) -> Result<GrammarField> {
    let name: Ident = input.parse()?;
    input.parse::<Token![:]>()?;

    let mut positional = false;
    let mut lines = false;
    if input.peek(syn::token::Paren) {
        let marker;
        parenthesized!(marker in input);
        marker.parse::<Token![_]>()?;
        positional = true;
        if !marker.is_empty() {
            let tag: Ident = marker.parse()?;
            if tag != "lines" {
                return Err(Error::new(tag.span(), "expected `lines`"));
            }
            lines = true;
        }
        if !marker.is_empty() {
            return Err(marker.error("expected `_` or `_ lines`"));
        }
    }

    let mut expression = parse_field_type(input, tokens)?;
    if lines {
        expression = match expression {
            GrammarExpr::Repeated(inner) => GrammarExpr::Lines(inner),
            expression => {
                return Err(Error::new(
                    expression.span(),
                    "`(_ lines)` requires a `Vec<T>` field",
                ));
            }
        };
    }
    if positional {
        expression = GrammarExpr::Positional(Box::new(expression));
    }
    Ok(GrammarField {
        name: Some(name),
        expression,
    })
}

/// A field's type, with trailing `?` making it optional instead of the more
/// verbose `Option<T>`.
fn parse_field_type(input: ParseStream<'_>, tokens: &TokenDefinitions) -> Result<GrammarExpr> {
    let ty: Type = input.parse()?;
    let mut expression = parse_rust_type(&ty, tokens)?;
    if input.peek(Token![?]) {
        input.parse::<Token![?]>()?;
        expression = GrammarExpr::Optional(Box::new(expression));
    }
    Ok(expression)
}

/// A variant is just the syntax type it wraps — `ExpressionCell` or
/// `Token![step]` — never a separately-spelled variant name. The variant's
/// Rust name is always derivable from what it wraps (the type's own name, or
/// the token's capitalized name), so respelling it would only duplicate the
/// same fact; [`Lowerer::lower_sum`] fills it in.
fn parse_enum_declaration(
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
) -> Result<Declaration> {
    input.call(syn::Attribute::parse_outer)?;
    input.parse::<Visibility>()?;
    input.parse::<Token![enum]>()?;
    let name: Ident = input.parse()?;

    let content;
    braced!(content in input);
    let mut variants = Vec::new();
    while !content.is_empty() {
        let ty: Type = content.parse()?;
        variants.push(GrammarField {
            name: None,
            expression: parse_rust_type(&ty, tokens)?,
        });
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        } else if !content.is_empty() {
            return Err(content.error("expected `,` between variants"));
        }
    }
    Ok(Declaration {
        name,
        expression: GrammarExpr::Sum(variants),
    })
}

fn parse_rust_type(ty: &Type, tokens: &TokenDefinitions) -> Result<GrammarExpr> {
    match ty {
        Type::Path(ty) if ty.qself.is_none() && ty.path.segments.len() == 1 => {
            let segment = &ty.path.segments[0];
            let name = segment.ident.to_string();
            match (&*name, &segment.arguments) {
                ("Vec", PathArguments::AngleBracketed(arguments)) => Ok(GrammarExpr::Repeated(
                    Box::new(parse_single_type_argument(arguments, tokens)?),
                )),
                ("Punctuated", PathArguments::AngleBracketed(arguments)) => {
                    Ok(GrammarExpr::Punctuated(Box::new(
                        parse_single_type_argument(arguments, tokens)?,
                    )))
                }
                (_, PathArguments::None) => Ok(GrammarExpr::Named(segment.ident.clone())),
                _ => Err(Error::new_spanned(ty, "unsupported syntax field type")),
            }
        }
        Type::Macro(TypeMacro { mac, .. }) if mac.path.is_ident("Token") => {
            Ok(GrammarExpr::Token {
                name: tokens.resolve(&mac.tokens)?,
                pattern: mac.tokens.clone(),
            })
        }
        _ => Err(Error::new_spanned(ty, "unsupported syntax field type")),
    }
}

fn parse_single_type_argument(
    arguments: &syn::AngleBracketedGenericArguments,
    tokens: &TokenDefinitions,
) -> Result<GrammarExpr> {
    let mut values = arguments.args.iter();
    let Some(GenericArgument::Type(ty)) = values.next() else {
        return Err(Error::new_spanned(arguments, "expected one syntax type"));
    };
    if values.next().is_some() {
        return Err(Error::new_spanned(arguments, "expected one syntax type"));
    }
    parse_rust_type(ty, tokens)
}

fn wrap_struct_attributes(
    mut expression: GrammarExpr,
    attrs: &[syn::Attribute],
) -> Result<GrammarExpr> {
    for attribute in attrs {
        if attribute.path().is_ident("cst") {
            let parser = |input: ParseStream<'_>| {
                let key: Ident = input.parse()?;
                input.parse::<Token![=]>()?;
                let kind: Ident = input.parse()?;
                if key != "kind" || !input.is_empty() {
                    return Err(input.error("expected `#[cst(kind = node_kind)]`"));
                }
                Ok(kind)
            };
            expression = GrammarExpr::Node(attribute.parse_args_with(parser)?, fields(expression)?);
        } else if attribute.path().is_ident("delimiter") {
            let delimiter: Ident = attribute.parse_args()?;
            let delimiter = match delimiter.to_string().as_str() {
                "parenthesis" => DelimiterKind::Parenthesis,
                "bracket" => DelimiterKind::Bracket,
                "brace" => DelimiterKind::Brace,
                "angle_bar" => DelimiterKind::AngleBar,
                "string" => DelimiterKind::String,
                "raw_string" => DelimiterKind::RawString,
                _ => return Err(Error::new(delimiter.span(), "unknown delimiter kind")),
            };
            expression = GrammarExpr::Delimited(delimiter, fields(expression)?);
        }
    }
    Ok(expression)
}

fn fields(expression: GrammarExpr) -> Result<Vec<GrammarField>> {
    match expression {
        GrammarExpr::Product(fields) => Ok(fields),
        _ => Err(Error::new(
            expression.span(),
            "CST and delimiter attributes require a struct with named fields",
        )),
    }
}

struct Lowerer<'tokens> {
    tokens: &'tokens TokenDefinitions,
    structs: Vec<StructDefinition>,
    enums: Vec<EnumDefinition>,
}

impl<'tokens> Lowerer<'tokens> {
    fn new(tokens: &'tokens TokenDefinitions) -> Self {
        Self {
            tokens,
            structs: Vec::new(),
            enums: Vec::new(),
        }
    }

    fn lower(
        mut self,
        declarations: Vec<Declaration>,
    ) -> Result<(Vec<StructDefinition>, Vec<EnumDefinition>)> {
        for declaration in declarations {
            self.lower_declaration(declaration.name, declaration.expression)?;
        }
        Ok((self.structs, self.enums))
    }

    fn lower_declaration(&mut self, name: Ident, expression: GrammarExpr) -> Result<()> {
        match expression {
            GrammarExpr::Leaf => self.push_struct(name, StructFields::Leaf, None),
            GrammarExpr::Product(fields) => {
                let fields = self.lower_fields(&name, fields)?;
                self.push_struct(name, StructFields::Product { fields }, None);
            }
            GrammarExpr::Delimited(delimiter, fields) => {
                let fields = self.lower_fields(&name, fields)?;
                self.push_struct(name, StructFields::Product { fields }, Some(delimiter));
            }
            GrammarExpr::Node(kind, fields) => {
                let fields = self.lower_fields(&name, fields)?;
                self.push_node(name, kind, fields);
            }
            GrammarExpr::Punctuated(inner) => {
                let fields = self.lower_fields(
                    &name,
                    vec![GrammarField {
                        name: None,
                        expression: GrammarExpr::Punctuated(inner),
                    }],
                )?;
                self.push_struct(name, StructFields::Product { fields }, None);
            }
            GrammarExpr::Sum(variants) => self.lower_sum(name, variants)?,
            expression => {
                return Err(Error::new(
                    name.span(),
                    format!(
                        "a top-level syntax declaration must be a leaf, product, sum, punctuated sequence, or delimited product; found {}",
                        expression.description(),
                    ),
                ));
            }
        }
        Ok(())
    }

    fn lower_sum(&mut self, name: Ident, variants: Vec<GrammarField>) -> Result<()> {
        let mut lowered = Vec::new();
        for variant in variants {
            let (variant_name, ty) = match variant.expression {
                GrammarExpr::Named(ty) => {
                    let variant_name = variant.name.unwrap_or_else(|| ty.clone());
                    (variant_name, ty)
                }
                GrammarExpr::Token { name: ty, pattern } => {
                    let variant_name = variant
                        .name
                        .unwrap_or_else(|| self.tokens.variant_name(&ty));
                    lowered.push(VariantDefinition {
                        attrs: Vec::new(),
                        name: variant_name,
                        shape: TypeShape::token(ty, pattern),
                    });
                    continue;
                }
                GrammarExpr::Product(fields) => {
                    let variant_name = required_variant_name(variant.name, &name)?;
                    let fields = self.lower_fields(&variant_name, fields)?;
                    self.push_struct(variant_name.clone(), StructFields::Product { fields }, None);
                    (variant_name.clone(), variant_name)
                }
                GrammarExpr::Delimited(delimiter, fields) => {
                    let variant_name = required_variant_name(variant.name, &name)?;
                    let fields = self.lower_fields(&variant_name, fields)?;
                    self.push_struct(
                        variant_name.clone(),
                        StructFields::Product { fields },
                        Some(delimiter),
                    );
                    (variant_name.clone(), variant_name)
                }
                GrammarExpr::Punctuated(inner) => {
                    let variant_name = required_variant_name(variant.name, &name)?;
                    let fields = self.lower_fields(
                        &variant_name,
                        vec![GrammarField {
                            name: None,
                            expression: GrammarExpr::Punctuated(inner),
                        }],
                    )?;
                    self.push_struct(variant_name.clone(), StructFields::Product { fields }, None);
                    (variant_name.clone(), variant_name)
                }
                expression => {
                    return Err(Error::new(
                        name.span(),
                        format!(
                            "sum variants must refer to a type or define an inline product; found {}",
                            expression.description(),
                        ),
                    ));
                }
            };
            lowered.push(VariantDefinition {
                attrs: Vec::new(),
                name: variant_name,
                shape: TypeShape::base(ty),
            });
        }
        self.enums.push(EnumDefinition {
            attrs: Vec::new(),
            visibility: syn::parse_quote!(pub),
            name,
            variants: lowered,
        });
        Ok(())
    }

    fn lower_fields(
        &self,
        owner: &Ident,
        fields: Vec<GrammarField>,
    ) -> Result<Vec<FieldDefinition>> {
        let mut lowered = Vec::new();
        for field in fields {
            self.lower_field(owner, field, false, false, &mut lowered)?;
        }
        Ok(lowered)
    }

    fn lower_field(
        &self,
        owner: &Ident,
        field: GrammarField,
        optional: bool,
        positional: bool,
        output: &mut Vec<FieldDefinition>,
    ) -> Result<()> {
        match field.expression {
            GrammarExpr::Positional(inner) => self.lower_field(
                owner,
                GrammarField {
                    name: field.name,
                    expression: *inner,
                },
                optional,
                true,
                output,
            ),
            GrammarExpr::Optional(inner) => self.lower_field(
                owner,
                GrammarField {
                    name: field.name,
                    expression: *inner,
                },
                true,
                positional,
                output,
            ),
            GrammarExpr::Product(fields) => {
                if field.name.is_some() {
                    return Err(Error::new(
                        owner.span(),
                        "a nested product cannot itself have a field name; name its contents instead",
                    ));
                }
                for nested in fields {
                    self.lower_field(owner, nested, optional, positional, output)?;
                }
                Ok(())
            }
            expression => {
                let index = output.len();
                let member = field.name.clone().unwrap_or_else(|| {
                    format_ident!("_{}_{}", to_snake_case(&owner.to_string()), index)
                });
                let source = if positional || field.name.is_none() {
                    FieldSource::Positional
                } else {
                    FieldSource::Named(
                        field
                            .name
                            .as_ref()
                            .expect("checked named field")
                            .to_string(),
                    )
                };
                let (mut shape, repeated_separator) = self.lower_field_shape(expression)?;
                if optional && !matches!(shape, TypeShape::Repeated(_) | TypeShape::Punctuated(_)) {
                    shape = TypeShape::Optional(Box::new(shape));
                }
                let base = shape.base_ident();
                let spelling = self.tokens.spelling(base);
                output.push(FieldDefinition {
                    attrs: Vec::new(),
                    visibility: syn::parse_quote!(pub),
                    member: Member::Named(member.clone()),
                    binding: member,
                    source,
                    shape,
                    repeated_separator,
                    attached: matches!(spelling, Some(";" | ",")),
                });
                Ok(())
            }
        }
    }

    fn lower_field_shape(&self, expression: GrammarExpr) -> Result<(TypeShape, &'static str)> {
        match expression {
            GrammarExpr::Named(name) => Ok((TypeShape::base(name), " ")),
            GrammarExpr::Token { name, pattern } => Ok((TypeShape::token(name, pattern), " ")),
            GrammarExpr::Repeated(inner) => {
                let (inner, _) = self.lower_field_shape(*inner)?;
                ensure_plain_element(&inner)?;
                Ok((TypeShape::Repeated(Box::new(inner)), " "))
            }
            GrammarExpr::Lines(inner) => {
                let (inner, _) = self.lower_field_shape(*inner)?;
                ensure_plain_element(&inner)?;
                Ok((TypeShape::Repeated(Box::new(inner)), "\n"))
            }
            GrammarExpr::Punctuated(inner) => {
                let (inner, _) = self.lower_field_shape(*inner)?;
                ensure_plain_element(&inner)?;
                Ok((TypeShape::Punctuated(Box::new(inner)), ""))
            }
            expression => Err(Error::new(
                expression.span(),
                format!(
                    "fields must contain a syntax type, token, repetition, or punctuated sequence; found {}",
                    expression.description(),
                ),
            )),
        }
    }

    fn push_struct(&mut self, name: Ident, fields: StructFields, delimiter: Option<DelimiterKind>) {
        self.structs.push(StructDefinition {
            attrs: Vec::new(),
            visibility: syn::parse_quote!(pub),
            name,
            fields,
            delimiter,
            cst_kind: None,
        });
    }

    fn push_node(&mut self, name: Ident, cst_kind: Ident, fields: Vec<FieldDefinition>) {
        self.structs.push(StructDefinition {
            attrs: Vec::new(),
            visibility: syn::parse_quote!(pub),
            name,
            fields: StructFields::Product { fields },
            delimiter: None,
            cst_kind: Some(cst_kind.to_string()),
        });
    }
}

impl GrammarExpr {
    fn description(&self) -> &'static str {
        match self {
            Self::Leaf => "a leaf",
            Self::Named(_) => "a type reference",
            Self::Token { .. } => "a token",
            Self::Product(_) => "a product",
            Self::Sum(_) => "a sum",
            Self::Repeated(_) => "a repetition",
            Self::Lines(_) => "a line-separated sequence",
            Self::Optional(_) => "an optional value",
            Self::Punctuated(_) => "a punctuated sequence",
            Self::Positional(_) => "a positional CST child",
            Self::Delimited(_, _) => "a delimited product",
            Self::Node(_, _) => "a concrete CST specialization",
        }
    }

    fn span(&self) -> proc_macro2::Span {
        match self {
            Self::Named(name) | Self::Token { name, .. } => name.span(),
            Self::Repeated(inner)
            | Self::Lines(inner)
            | Self::Optional(inner)
            | Self::Punctuated(inner)
            | Self::Positional(inner) => inner.span(),
            Self::Leaf
            | Self::Product(_)
            | Self::Sum(_)
            | Self::Delimited(_, _)
            | Self::Node(_, _) => proc_macro2::Span::call_site(),
        }
    }
}

fn required_variant_name(name: Option<Ident>, owner: &Ident) -> Result<Ident> {
    name.ok_or_else(|| {
        Error::new(
            owner.span(),
            "an inline sum variant needs a name before `:`",
        )
    })
}

fn ensure_plain_element(shape: &TypeShape) -> Result<()> {
    if matches!(shape, TypeShape::Base(_, _)) {
        Ok(())
    } else {
        Err(Error::new(
            shape.base_ident().span(),
            "repetition elements must be plain syntax types",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::parse2;

    #[test]
    fn parses_and_lowers_structs_enums_and_attributes() {
        let syntax: Syntax = parse2(quote! {
            precedence: {}
            augmented: (14, 13)
            tokens { [=] { infix } (14, 13, _) }
            keywords: { [if] }
            markers: {}
            punct: { [,] }

            struct Leaf;
            struct Pair { left: Leaf, operator: Token![=], right: Leaf }
            #[delimiter(parenthesis)]
            struct Wrapped {
                values: Punctuated<Leaf>,
            }
            enum Choice { Pair, Wrapped }
            struct Root {
                values: (_) Vec<Choice>,
            }
        })
        .unwrap();

        assert!(syntax.structs.iter().any(|node| node.name == "Pair"));
        assert!(syntax.structs.iter().any(|node| node.name == "Wrapped"));
        assert!(syntax.enums.iter().any(|node| node.name == "Choice"));
        let expansion = syntax.expand().unwrap().combined();
        syn::parse2::<syn::File>(expansion).unwrap();
    }
}
