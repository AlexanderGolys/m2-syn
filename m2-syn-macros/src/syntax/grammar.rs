use super::{
    Delimiter, EnumDefinition, FieldDefinition, FieldSource, LeafKind, Literal, Member,
    OperatorKind, ProductStyle, Shape, StructDefinition, StructFields, TokenDefinitions,
    VariantDefinition, Wrapper, to_snake_case,
};
use proc_macro2::{Ident, TokenStream};
use quote::format_ident;
use syn::parse::ParseStream;
use syn::{Attribute, Error, Result, Token, Visibility, braced, bracketed, parenthesized};

struct Declaration {
    name: Ident,
    expression: GrammarExpr,
    boxed: bool,
}

struct GrammarField {
    name: Option<Ident>,
    expression: GrammarExpr,
}

enum GrammarExpr {
    Leaf(LeafKind),
    Named(Ident),
    Token { name: Ident, pattern: TokenStream },
    Product(Vec<GrammarField>, ProductStyle),
    Sum(Vec<GrammarField>, Option<OperatorKind>),
    Repeated(Box<Self>, &'static str),
    Optional(Box<Self>),
    Punctuated(Box<Self>),
    Boxed(Box<Self>),
    Delimited(Box<Self>),
    Positional(Box<Self>),
    Wrapped(Wrapper, Vec<GrammarField>),
    Node(Ident, Vec<GrammarField>),
}

pub(super) fn parse(
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
) -> Result<(Vec<StructDefinition>, Vec<EnumDefinition>)> {
    let mut declarations = Vec::new();
    while !input.is_empty() {
        let fork = input.fork();
        let _ = fork.call(Attribute::parse_outer);
        if fork.peek(Token![struct]) {
            let declaration = parse_struct_declaration(input, tokens, &mut declarations)?;
            declarations.push(declaration);
        } else if fork.peek(Token![enum]) {
            let declaration = parse_enum_declaration(input, tokens, &mut declarations)?;
            declarations.push(declaration);
        } else {
            return Err(input.error("expected a syntax `struct` or `enum`"));
        }
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
    declarations: &mut Vec<Declaration>,
) -> Result<Declaration> {
    let attrs = input.call(Attribute::parse_outer)?;
    input.parse::<Visibility>()?;
    input.parse::<Token![struct]>()?;
    let boxed_before_name = parse_box_marker(input)?;
    let name: Ident = input.parse()?;
    let boxed = boxed_before_name || parse_box_marker(input)?;

    if input.peek(Token![;]) {
        input.parse::<Token![;]>()?;
        let leaf = attrs
            .iter()
            .find(|attribute| attribute.path().is_ident("leaf"))
            .map(|attribute| {
                let kind: Ident = attribute.parse_args()?;
                match kind.to_string().as_str() {
                    "empty" => Ok(LeafKind::Empty),
                    "eoc" => Ok(LeafKind::Eoc),
                    "float" => Ok(LeafKind::Float),
                    "integer" => Ok(LeafKind::Integer),
                    "qualified" => Ok(LeafKind::QualifiedIdent),
                    _ => Err(Error::new(kind.span(), "unknown leaf kind")),
                }
            })
            .transpose()?
            .unwrap_or(LeafKind::Ident);
        return Ok(Declaration {
            name,
            expression: GrammarExpr::Leaf(leaf),
            boxed,
        });
    }

    let expression = if input.peek(syn::token::Brace) {
        let content;
        braced!(content in input);
        GrammarExpr::Product(
            parse_field_list(&content, tokens, declarations)?,
            ProductStyle::Named,
        )
    } else if input.peek(syn::token::Paren) {
        let content;
        parenthesized!(content in input);
        let fields = parse_tuple_field_list(&content, tokens, declarations)?;
        if input.peek(Token![;]) {
            input.parse::<Token![;]>()?;
        }
        GrammarExpr::Product(fields, ProductStyle::Tuple)
    } else {
        return Err(input.error("expected `;`, `{ ... }`, or `( ... )` after struct name"));
    };
    let expression = wrap_struct_attributes(expression, &attrs)?;
    Ok(Declaration {
        name,
        expression,
        boxed,
    })
}

fn parse_field_list(
    content: ParseStream<'_>,
    tokens: &TokenDefinitions,
    declarations: &mut Vec<Declaration>,
) -> Result<Vec<GrammarField>> {
    let mut fields = Vec::new();
    while !content.is_empty() {
        fields.push(parse_field(content, tokens, declarations)?);
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        } else if !content.is_empty() {
            return Err(content.error("expected `,` between fields"));
        }
    }
    Ok(fields)
}

fn parse_tuple_field_list(
    content: ParseStream<'_>,
    tokens: &TokenDefinitions,
    declarations: &mut Vec<Declaration>,
) -> Result<Vec<GrammarField>> {
    let mut fields = Vec::new();
    while !content.is_empty() {
        fields.push(GrammarField {
            name: None,
            expression: parse_field_type(content, tokens, declarations)?,
        });
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        } else if !content.is_empty() {
            return Err(content.error("expected `,` between tuple fields"));
        }
    }
    Ok(fields)
}

/// `name: (_) Type` marks a field addressed positionally in the CST rather
/// than by a field name. Plain `name: Type` fields are looked up by CST field
/// name, which is the common case.
fn parse_field(
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
    declarations: &mut Vec<Declaration>,
) -> Result<GrammarField> {
    let name: Ident = input.parse()?;
    input.parse::<Token![:]>()?;

    let mut positional = false;
    if input.peek(syn::token::Paren) {
        let marker;
        parenthesized!(marker in input);
        marker.parse::<Token![_]>()?;
        positional = true;
        if !marker.is_empty() {
            return Err(marker.error("expected `_`"));
        }
    }

    let mut expression = parse_field_type(input, tokens, declarations)?;
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
fn parse_field_type(
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
    declarations: &mut Vec<Declaration>,
) -> Result<GrammarExpr> {
    let mut expression = parse_type_expression(input, tokens, declarations)?;
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
    declarations: &mut Vec<Declaration>,
) -> Result<Declaration> {
    let attrs = input.call(Attribute::parse_outer)?;
    input.parse::<Visibility>()?;
    input.parse::<Token![enum]>()?;
    let boxed_before_name = parse_box_marker(input)?;
    let name: Ident = input.parse()?;
    let boxed = boxed_before_name || parse_box_marker(input)?;

    let content;
    braced!(content in input);
    let mut variants = Vec::new();
    while !content.is_empty() {
        variants.push(GrammarField {
            name: None,
            expression: parse_type_expression(&content, tokens, declarations)?,
        });
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        } else if !content.is_empty() {
            return Err(content.error("expected `,` between variants"));
        }
    }
    let operator = attrs
        .iter()
        .find(|attribute| attribute.path().is_ident("operator"))
        .map(|attribute| {
            let role: Ident = attribute.parse_args()?;
            match role.to_string().as_str() {
                "prefix" => Ok(OperatorKind::Prefix),
                "binary" => Ok(OperatorKind::Binary),
                "postfix" => Ok(OperatorKind::Postfix),
                _ => Err(Error::new(role.span(), "unknown operator role")),
            }
        })
        .transpose()?;
    Ok(Declaration {
        name,
        expression: GrammarExpr::Sum(variants, operator),
        boxed,
    })
}

fn parse_type_expression(
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
    declarations: &mut Vec<Declaration>,
) -> Result<GrammarExpr> {
    let attrs = input.call(Attribute::parse_outer)?;
    let name: Ident = input.parse()?;

    if name == "Token" && input.peek(Token![!]) {
        if !attrs.is_empty() {
            return Err(Error::new(
                name.span(),
                "token references cannot have attributes",
            ));
        }
        input.parse::<Token![!]>()?;
        let pattern;
        bracketed!(pattern in input);
        let pattern: TokenStream = pattern.parse()?;
        return Ok(GrammarExpr::Token {
            name: tokens.resolve(&pattern)?,
            pattern,
        });
    }

    if input.peek(Token![<]) {
        if !attrs.is_empty() {
            return Err(Error::new(
                name.span(),
                "type wrappers cannot have attributes",
            ));
        }
        input.parse::<Token![<]>()?;
        let inner = parse_field_type(input, tokens, declarations)?;
        input.parse::<Token![>]>()?;
        return match name.to_string().as_str() {
            "Vec" => Ok(GrammarExpr::Repeated(Box::new(inner), " ")),
            "Lines" => Ok(GrammarExpr::Repeated(Box::new(inner), "\n")),
            "Punctuated" => Ok(GrammarExpr::Punctuated(Box::new(inner))),
            "Box" => Ok(GrammarExpr::Boxed(Box::new(inner))),
            "Delimited" => Ok(GrammarExpr::Delimited(Box::new(inner))),
            _ => Err(Error::new(name.span(), "unsupported syntax type wrapper")),
        };
    }

    let boxed = parse_box_marker(input)?;
    let body = if input.peek(syn::token::Brace) {
        let content;
        braced!(content in input);
        Some(GrammarExpr::Product(
            parse_field_list(&content, tokens, declarations)?,
            ProductStyle::Named,
        ))
    } else if input.peek(syn::token::Paren) {
        let content;
        parenthesized!(content in input);
        Some(GrammarExpr::Product(
            parse_tuple_field_list(&content, tokens, declarations)?,
            ProductStyle::Tuple,
        ))
    } else {
        None
    };

    if let Some(body) = body {
        let body = wrap_struct_attributes(body, &attrs)?;
        declarations.push(Declaration {
            name: name.clone(),
            expression: body,
            boxed,
        });
        Ok(GrammarExpr::Named(name))
    } else if attrs.is_empty() && !boxed {
        Ok(GrammarExpr::Named(name))
    } else {
        Err(Error::new(
            name.span(),
            "attributes and `*` annotate an inline type declaration, not a reference",
        ))
    }
}

fn parse_box_marker(input: ParseStream<'_>) -> Result<bool> {
    if input.peek(Token![*]) {
        input.parse::<Token![*]>()?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn wrap_struct_attributes(mut expression: GrammarExpr, attrs: &[Attribute]) -> Result<GrammarExpr> {
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
            let name: Ident = attribute.parse_args()?;
            let delimiter = match name.to_string().as_str() {
                "parenthesis" => Delimiter::Parenthesis,
                "bracket" => Delimiter::Bracket,
                "brace" => Delimiter::Brace,
                "angle_bar" => Delimiter::AngleBar,
                _ => return Err(Error::new(name.span(), "unknown delimiter family")),
            };
            expression = GrammarExpr::Wrapped(Wrapper::Delimited(delimiter), fields(expression)?);
        } else if attribute.path().is_ident("literal") {
            let name: Ident = attribute.parse_args()?;
            let literal = match name.to_string().as_str() {
                "string" => Literal::String,
                "raw_string" => Literal::RawString,
                _ => return Err(Error::new(name.span(), "unknown literal kind")),
            };
            expression = GrammarExpr::Wrapped(Wrapper::Literal(literal), fields(expression)?);
        }
    }
    Ok(expression)
}

fn fields(expression: GrammarExpr) -> Result<Vec<GrammarField>> {
    match expression {
        GrammarExpr::Product(fields, ProductStyle::Named) => Ok(fields),
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
            self.lower_declaration(declaration.name, declaration.expression, declaration.boxed)?;
        }
        Ok((self.structs, self.enums))
    }

    fn lower_declaration(
        &mut self,
        name: Ident,
        expression: GrammarExpr,
        boxed: bool,
    ) -> Result<()> {
        match expression {
            GrammarExpr::Leaf(leaf) => {
                self.push_struct(name, StructFields::Leaf(leaf), None, boxed)
            }
            GrammarExpr::Product(fields, style) => {
                let fields = lower_fields(&name, fields, style)?;
                self.push_struct(name, StructFields::Product { fields, style }, None, boxed);
            }
            GrammarExpr::Wrapped(delimiter, fields) => {
                let fields = lower_fields(&name, fields, ProductStyle::Named)?;
                self.push_struct(
                    name,
                    StructFields::Product {
                        fields,
                        style: ProductStyle::Named,
                    },
                    Some(delimiter),
                    boxed,
                );
            }
            GrammarExpr::Node(kind, fields) => {
                let fields = lower_fields(&name, fields, ProductStyle::Named)?;
                self.push_node(name, kind.to_string(), fields, boxed);
            }
            GrammarExpr::Punctuated(inner) => {
                let fields = lower_fields(
                    &name,
                    vec![GrammarField {
                        name: None,
                        expression: GrammarExpr::Punctuated(inner),
                    }],
                    ProductStyle::Tuple,
                )?;
                self.push_struct(
                    name,
                    StructFields::Product {
                        fields,
                        style: ProductStyle::Tuple,
                    },
                    None,
                    boxed,
                );
            }
            GrammarExpr::Sum(variants, operator) => {
                self.lower_sum(name, variants, operator, boxed)?
            }
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

    fn lower_sum(
        &mut self,
        name: Ident,
        variants: Vec<GrammarField>,
        operator: Option<OperatorKind>,
        boxed: bool,
    ) -> Result<()> {
        if let Some(kind) = operator {
            if !variants.is_empty() {
                return Err(Error::new(
                    name.span(),
                    "operator enums derive their variants",
                ));
            }
            self.enums
                .push(EnumDefinition::operator(name, kind, self.tokens));
            return Ok(());
        }
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
                        name: variant_name,
                        shape: Shape::token(ty, &pattern),
                    });
                    continue;
                }
                GrammarExpr::Product(fields, style) => {
                    let variant_name = required_variant_name(variant.name, &name)?;
                    let fields = lower_fields(&variant_name, fields, style)?;
                    self.push_struct(
                        variant_name.clone(),
                        StructFields::Product { fields, style },
                        None,
                        false,
                    );
                    (variant_name.clone(), variant_name)
                }
                GrammarExpr::Wrapped(delimiter, fields) => {
                    let variant_name = required_variant_name(variant.name, &name)?;
                    let fields = lower_fields(&variant_name, fields, ProductStyle::Named)?;
                    self.push_struct(
                        variant_name.clone(),
                        StructFields::Product {
                            fields,
                            style: ProductStyle::Named,
                        },
                        Some(delimiter),
                        false,
                    );
                    (variant_name.clone(), variant_name)
                }
                GrammarExpr::Punctuated(inner) => {
                    let variant_name = required_variant_name(variant.name, &name)?;
                    let fields = lower_fields(
                        &variant_name,
                        vec![GrammarField {
                            name: None,
                            expression: GrammarExpr::Punctuated(inner),
                        }],
                        ProductStyle::Tuple,
                    )?;
                    self.push_struct(
                        variant_name.clone(),
                        StructFields::Product {
                            fields,
                            style: ProductStyle::Tuple,
                        },
                        None,
                        false,
                    );
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
                name: variant_name,
                shape: Shape::base(ty),
            });
        }
        self.enums.push(EnumDefinition {
            name,
            variants: lowered,
            operator: None,
            boxed,
        });
        Ok(())
    }
}

fn lower_fields(
    owner: &Ident,
    fields: Vec<GrammarField>,
    style: ProductStyle,
) -> Result<Vec<FieldDefinition>> {
    let mut lowered = Vec::new();
    for field in fields {
        lower_field(owner, field, false, false, &mut lowered)?;
    }
    if style == ProductStyle::Tuple {
        for (index, field) in lowered.iter_mut().enumerate() {
            field.member = Member::Unnamed(syn::Index::from(index));
            field.source = FieldSource::Positional;
        }
    }
    Ok(lowered)
}

fn lower_field(
    owner: &Ident,
    field: GrammarField,
    optional: bool,
    positional: bool,
    output: &mut Vec<FieldDefinition>,
) -> Result<()> {
    match field.expression {
        GrammarExpr::Positional(inner) => lower_field(
            owner,
            GrammarField {
                name: field.name,
                expression: *inner,
            },
            optional,
            true,
            output,
        ),
        GrammarExpr::Optional(inner) => lower_field(
            owner,
            GrammarField {
                name: field.name,
                expression: *inner,
            },
            true,
            positional,
            output,
        ),
        GrammarExpr::Product(fields, _) => {
            if field.name.is_some() {
                return Err(Error::new(
                    owner.span(),
                    "a nested product cannot itself have a field name; name its contents instead",
                ));
            }
            for nested in fields {
                lower_field(owner, nested, optional, positional, output)?;
            }
            Ok(())
        }
        expression => {
            let index = output.len();
            let member = field.name.clone().unwrap_or_else(|| {
                format_ident!("_{}_{}", to_snake_case(&owner.to_string()), index)
            });
            let source = field
                .name
                .as_ref()
                .filter(|_| !positional)
                .map_or(FieldSource::Positional, |name| {
                    FieldSource::Named(name.to_string())
                });
            let mut shape = lower_field_shape(expression)?;
            if optional {
                shape = Shape::Optional(Box::new(shape));
            }
            output.push(FieldDefinition {
                member: Member::Named(member.clone()),
                binding: member,
                source,
                shape,
            });
            Ok(())
        }
    }
}

fn lower_field_shape(expression: GrammarExpr) -> Result<Shape> {
    match expression {
        GrammarExpr::Named(name) => Ok(Shape::base(name)),
        GrammarExpr::Token { name, pattern } => Ok(Shape::token(name, &pattern)),
        GrammarExpr::Optional(inner) => Ok(Shape::Optional(Box::new(lower_field_shape(*inner)?))),
        GrammarExpr::Repeated(inner, separator) => {
            let inner = lower_field_shape(*inner)?;
            ensure_plain_element(&inner)?;
            Ok(Shape::Repeated(Box::new(inner), separator))
        }
        GrammarExpr::Punctuated(inner) => {
            let inner = lower_field_shape(*inner)?;
            ensure_plain_element(&inner)?;
            Ok(Shape::Punctuated(Box::new(inner)))
        }
        GrammarExpr::Boxed(inner) => Ok(Shape::Boxed(Box::new(lower_field_shape(*inner)?))),
        GrammarExpr::Delimited(inner) => Ok(Shape::Delimited(Box::new(lower_field_shape(*inner)?))),
        expression => Err(Error::new(
            expression.span(),
            format!(
                "fields must contain a syntax type, token, repetition, or punctuated sequence; found {}",
                expression.description(),
            ),
        )),
    }
}

impl Lowerer<'_> {
    fn push_struct(
        &mut self,
        name: Ident,
        fields: StructFields,
        wrapper: Option<Wrapper>,
        boxed: bool,
    ) {
        self.structs.push(StructDefinition {
            name,
            fields,
            wrapper,
            cst_kind: None,
            boxed,
        });
    }

    fn push_node(
        &mut self,
        name: Ident,
        cst_kind: String,
        fields: Vec<FieldDefinition>,
        boxed: bool,
    ) {
        self.structs.push(StructDefinition {
            name,
            fields: StructFields::Product {
                fields,
                style: ProductStyle::Named,
            },
            wrapper: None,
            cst_kind: Some(cst_kind),
            boxed,
        });
    }
}

impl GrammarExpr {
    fn description(&self) -> &'static str {
        match self {
            Self::Leaf(_) => "a leaf",
            Self::Named(_) => "a type reference",
            Self::Token { .. } => "a token",
            Self::Product(_, _) => "a product",
            Self::Sum(_, _) => "a sum",
            Self::Repeated(_, _) => "a repetition",
            Self::Optional(_) => "an optional value",
            Self::Punctuated(_) => "a punctuated sequence",
            Self::Boxed(_) => "an explicitly boxed value",
            Self::Delimited(_) => "an arbitrarily delimited value",
            Self::Positional(_) => "a positional CST child",
            Self::Wrapped(_, _) => "a delimited product",
            Self::Node(_, _) => "a concrete CST specialization",
        }
    }

    fn span(&self) -> proc_macro2::Span {
        match self {
            Self::Named(name) | Self::Token { name, .. } => name.span(),
            Self::Repeated(inner, _)
            | Self::Optional(inner)
            | Self::Punctuated(inner)
            | Self::Boxed(inner)
            | Self::Delimited(inner)
            | Self::Positional(inner) => inner.span(),
            Self::Leaf(_)
            | Self::Product(_, _)
            | Self::Sum(_, _)
            | Self::Wrapped(_, _)
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

fn ensure_plain_element(shape: &Shape) -> Result<()> {
    if matches!(shape, Shape::Base(_, _)) {
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
    use crate::syntax::Syntax;
    use quote::quote;
    use syn::parse2;

    #[test]
    fn parses_and_lowers_structs_enums_and_attributes() {
        let syntax: Syntax = parse2(quote! {
            precedence: {}
            tokens {
                [=] { infix } (14, R)
                [if] (62, 12)
                [,]
            }

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
        let expansion = syntax.expand().unwrap();
        syn::parse2::<syn::File>(expansion).unwrap();
    }

    #[test]
    fn hoists_inline_products_and_boxes_marked_inline_references() {
        let syntax: Syntax = parse2(quote::quote! {
            precedence: {}
            tokens { [,] }

            struct Leaf;
            enum Expr* {
                Leaf,
                Application(Expr, Delimited<Punctuated<Expr>?>),
            }
            struct Root {
                named: Nested { value: Expr },
                tuple: Pair(Expr, Leaf),
            }
        })
        .unwrap();

        assert!(syntax.structs.iter().any(|node| node.name == "Application"));
        assert!(syntax.structs.iter().any(|node| node.name == "Nested"));
        assert!(syntax.structs.iter().any(|node| node.name == "Pair"));
        assert!(
            syntax
                .enums
                .iter()
                .any(|node| node.name == "Expr" && node.boxed)
        );

        let expansion = syntax.expand().unwrap();
        let rendered = expansion.to_string();
        assert!(rendered.contains("Box < Expr >"));
        assert!(rendered.contains("Delimited <"));
        syn::parse2::<syn::File>(expansion).unwrap();
    }
}
