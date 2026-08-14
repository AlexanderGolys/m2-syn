use super::*;
use syn::{braced, bracketed, parenthesized};

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
    Unfielded(Box<Self>),
    Delimited(DelimiterKind, Vec<GrammarField>),
    Node(Ident, Vec<GrammarField>),
}

pub(super) fn parse(
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
) -> Result<(Vec<StructDefinition>, Vec<EnumDefinition>)> {
    let mut declarations = Vec::new();
    while !input.is_empty() {
        let name: Ident = input.parse()?;
        input.parse::<Token![::]>()?;
        input.parse::<Token![=]>()?;
        declarations.push(Declaration {
            name,
            expression: parse_expression(input, tokens)?,
        });
    }

    Lowerer::new(tokens).lower(declarations)
}

fn parse_expression(input: ParseStream<'_>, tokens: &TokenDefinitions) -> Result<GrammarExpr> {
    let mut expression = if input.peek(syn::token::Paren) {
        let content;
        parenthesized!(content in input);
        GrammarExpr::Product(parse_fields(&content, tokens)?)
    } else if input.peek(syn::token::Brace) {
        let content;
        braced!(content in input);
        GrammarExpr::Sum(parse_fields(&content, tokens)?)
    } else if input.peek(syn::token::Bracket) {
        let content;
        bracketed!(content in input);
        GrammarExpr::Repeated(Box::new(parse_expression(&content, tokens)?))
    } else {
        let name: Ident = input.parse()?;
        match name.to_string().as_str() {
            "leaf" => GrammarExpr::Leaf,
            "Token" if input.peek(Token![!]) => {
                input.parse::<Token![!]>()?;
                let content;
                bracketed!(content in input);
                let pattern = content.parse()?;
                GrammarExpr::Token {
                    name: tokens.resolve(&pattern)?,
                    pattern,
                }
            }
            "unfielded" => GrammarExpr::Unfielded(Box::new(parse_expression(input, tokens)?)),
            function if input.peek(syn::token::Paren) => {
                let content;
                parenthesized!(content in input);
                match function {
                    "punct" => {
                        let inner = parse_expression(&content, tokens)?;
                        if !content.is_empty() {
                            return Err(content.error("`punct` accepts exactly one syntax type"));
                        }
                        GrammarExpr::Punctuated(Box::new(inner))
                    }
                    "lines" => {
                        let inner = parse_expression(&content, tokens)?;
                        if !content.is_empty() {
                            return Err(content.error("`lines` accepts exactly one syntax type"));
                        }
                        GrammarExpr::Lines(Box::new(inner))
                    }
                    "paren" | "bracket" | "brace" | "angle_bar" | "string" | "raw_string" => {
                        let delimiter = match function {
                            "paren" => DelimiterKind::Parenthesis,
                            "bracket" => DelimiterKind::Bracket,
                            "brace" => DelimiterKind::Brace,
                            "angle_bar" => DelimiterKind::AngleBar,
                            "string" => DelimiterKind::String,
                            "raw_string" => DelimiterKind::RawString,
                            _ => unreachable!(),
                        };
                        GrammarExpr::Delimited(delimiter, parse_fields(&content, tokens)?)
                    }
                    "node" => {
                        let kind = content.parse()?;
                        if !content.is_empty() {
                            content.parse::<Token![,]>()?;
                        }
                        GrammarExpr::Node(kind, parse_fields(&content, tokens)?)
                    }
                    _ => {
                        return Err(Error::new(
                            name.span(),
                            "expected `punct`, `lines`, `node`, `paren`, `bracket`, `brace`, `angle_bar`, `string`, or `raw_string`",
                        ));
                    }
                }
            }
            _ => GrammarExpr::Named(name),
        }
    };

    if input.peek(Token![?]) {
        input.parse::<Token![?]>()?;
        expression = GrammarExpr::Optional(Box::new(expression));
    }
    Ok(expression)
}

fn parse_fields(input: ParseStream<'_>, tokens: &TokenDefinitions) -> Result<Vec<GrammarField>> {
    let mut fields = Vec::new();
    while !input.is_empty() {
        let named = {
            let fork = input.fork();
            fork.parse::<Ident>().is_ok() && fork.peek(Token![:])
        };
        let name = if named {
            let name = input.parse()?;
            input.parse::<Token![:]>()?;
            Some(name)
        } else {
            None
        };
        fields.push(GrammarField {
            name,
            expression: parse_expression(input, tokens)?,
        });
        if input.is_empty() {
            break;
        }
        input.parse::<Token![,]>()?;
    }
    Ok(fields)
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
        unfielded: bool,
        output: &mut Vec<FieldDefinition>,
    ) -> Result<()> {
        match field.expression {
            GrammarExpr::Unfielded(inner) => self.lower_field(
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
                unfielded,
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
                    self.lower_field(owner, nested, optional, unfielded, output)?;
                }
                Ok(())
            }
            expression => {
                let index = output.len();
                let member = field.name.clone().unwrap_or_else(|| {
                    format_ident!("_{}_{}", to_snake_case(&owner.to_string()), index)
                });
                let source = if unfielded || field.name.is_none() {
                    FieldSource::Unfielded
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
                if optional && !matches!(shape, TypeShape::Repeated(_)) {
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
                Ok((TypeShape::Repeated(Box::new(inner)), ", "))
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
            Self::Unfielded(_) => "an unfielded value",
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
            | Self::Unfielded(inner) => inner.span(),
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
    fn parses_and_lowers_compact_products_sums_and_delimiters() {
        let syntax: Syntax = parse2(quote! {
            tokens { [=] {} }
            keywords: { [if] }
            markers: {}
            punct: { [,] }

            Leaf ::= leaf
            Pair ::= (left: Leaf, operator: Token![=], right: Leaf)
            Choice ::= {
                Pair,
                Wrapped: paren(values: punct(Leaf)),
            }
            Root ::= (values: unfielded [Choice])
        })
        .unwrap();

        assert!(syntax.structs.iter().any(|node| node.name == "Pair"));
        assert!(syntax.structs.iter().any(|node| node.name == "Wrapped"));
        assert!(syntax.enums.iter().any(|node| node.name == "Choice"));
        let expansion = syntax.expand().unwrap().combined();
        syn::parse2::<syn::File>(expansion).unwrap();
    }
}
