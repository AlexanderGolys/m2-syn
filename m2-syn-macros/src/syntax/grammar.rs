use super::{
    Delimiter, EnumDefinition, FieldDefinition, FieldSource, LeafKind, Literal, Member,
    OperatorKind, ProductStyle, Shape, StructDefinition, StructFields, TokenDefinitions,
    VariantDefinition, Wrapper, to_snake_case,
};
use proc_macro2::{Group, Ident, TokenStream, TokenTree};
use quote::{format_ident, quote};
use syn::parse::{ParseStream, Parser};
use syn::{Attribute, Error, Result, Token, Visibility, braced, bracketed, parenthesized};

struct Declaration {
    name: Ident,
    expression: GrammarExpr,
    boxed: bool,
    cell_stream: bool,
    comments: bool,
    explicit: bool,
}

#[derive(Clone, Copy, Default)]
struct ExplicitDeclarationKind {
    leaf: Option<LeafKind>,
    operator: Option<OperatorKind>,
}

pub(super) struct ExplicitDeclaration {
    default_derives: TokenStream,
    body: TokenStream,
    generated_members: TokenStream,
    storage: TokenStream,
}

impl ExplicitDeclaration {
    pub(super) fn expand(&self) -> TokenStream {
        let Self {
            default_derives,
            body,
            generated_members,
            storage,
        } = self;
        quote! {
            ::these_macros_should_be_illegal::strutuct! {
                #[derive(::m2_syn::__SyntaxWalk)]
                #default_derives
                #body
                #generated_members
                #storage
            }
        }
    }
}

#[derive(Clone, Copy, Default)]
struct DefinitionFlags {
    boxed: bool,
    cell_stream: bool,
    comments: bool,
    explicit: bool,
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
) -> Result<(
    Vec<StructDefinition>,
    Vec<EnumDefinition>,
    Vec<ExplicitDeclaration>,
)> {
    let mut declarations = Vec::new();
    let mut explicit_declarations = Vec::new();
    while !input.is_empty() {
        let fork = input.fork();
        let _ = fork.call(Attribute::parse_outer);
        if fork.peek(Token![struct]) {
            let declaration = parse_struct_declaration(input, tokens, &mut declarations)?;
            declarations.push(declaration);
        } else if fork.peek(Token![enum]) {
            let declaration = parse_enum_declaration(input, tokens, &mut declarations)?;
            declarations.push(declaration);
        } else if begins_strutuct(&fork) {
            parse_strutuct(input, tokens, &mut declarations, &mut explicit_declarations)?;
        } else {
            return Err(input.error("expected a syntax `struct`, `enum`, or `strutuct!`"));
        }
    }
    let (structs, enums) = Lowerer::new(tokens).lower(declarations)?;
    Ok((structs, enums, explicit_declarations))
}

fn begins_strutuct(input: ParseStream<'_>) -> bool {
    input
        .parse::<Ident>()
        .is_ok_and(|name| name == "strutuct" && input.peek(Token![!]))
}

fn parse_strutuct(
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
    declarations: &mut Vec<Declaration>,
    output: &mut Vec<ExplicitDeclaration>,
) -> Result<()> {
    let attrs = input.call(Attribute::parse_outer)?;
    let name: Ident = input.parse()?;
    if name != "strutuct" {
        return Err(Error::new(name.span(), "expected `strutuct!`"));
    }
    input.parse::<Token![!]>()?;
    let content;
    braced!(content in input);
    let body: TokenStream = content.parse()?;
    let parser =
        |input: ParseStream<'_>| parse_explicit_declaration(input, tokens, declarations, &attrs);
    let kind = parser.parse2(body.clone()).map_err(|error| {
        Error::new(
            error.span(),
            format!(
                "invalid explicit declaration `{}`: {error}",
                explicit_root_name(&body)
            ),
        )
    })?;
    let generated_members = explicit_generated_members(tokens, kind.operator);
    let storage = explicit_storage_fields(&attrs, kind.leaf)?;
    let body = explicit_storage_types(body);
    let default_derives = if explicit_has_derive(&body)? {
        TokenStream::new()
    } else if matches!(
        kind.leaf,
        Some(LeafKind::Adj | LeafKind::Eoc | LeafKind::Eof)
    ) {
        quote!(#[derive(Debug, Clone, PartialEq, Eq, Hash)])
    } else {
        quote!(#[derive(Debug, ::m2_syn::Spanned)])
    };
    output.push(ExplicitDeclaration {
        default_derives,
        body,
        generated_members,
        storage,
    });
    Ok(())
}

fn explicit_root_name(tokens: &TokenStream) -> String {
    tokens
        .clone()
        .into_iter()
        .find_map(|token| match token {
            TokenTree::Ident(name) => Some(name.to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "<unknown>".to_owned())
}

fn explicit_has_derive(tokens: &TokenStream) -> Result<bool> {
    let parser = |input: ParseStream<'_>| {
        let has_derive = input
            .call(Attribute::parse_outer)?
            .iter()
            .any(|attribute| attribute.path().is_ident("derive"));
        let _: TokenStream = input.parse()?;
        Ok(has_derive)
    };
    parser.parse2(tokens.clone())
}

fn explicit_generated_members(
    tokens: &TokenDefinitions,
    operator: Option<OperatorKind>,
) -> TokenStream {
    operator
        .into_iter()
        .flat_map(|kind| tokens.operator_variants(kind))
        .map(|token| {
            let variant = tokens.variant_name(&token.name);
            let pattern = &token.pattern;
            quote!(#variant(::m2_syn::Token![#pattern]))
        })
        .collect()
}

fn explicit_storage_types(tokens: TokenStream) -> TokenStream {
    tokens
        .into_iter()
        .map(|token| match token {
            TokenTree::Ident(name) if name == "Lines" => {
                TokenTree::Ident(Ident::new("Vec", name.span()))
            }
            TokenTree::Group(group) => {
                let mut rewritten =
                    Group::new(group.delimiter(), explicit_storage_types(group.stream()));
                rewritten.set_span(group.span());
                TokenTree::Group(rewritten)
            }
            token => token,
        })
        .collect()
}

fn explicit_storage_fields(attrs: &[Attribute], leaf: Option<LeafKind>) -> Result<TokenStream> {
    let mut fields = TokenStream::new();
    if leaf == Some(LeafKind::QualifiedIdent) {
        fields.extend(quote! {
            package_qualifier: qf!(Option<::m2_syn::Span>),
            index_qualifier: qf!(Option<::m2_syn::Span>),
        });
    }
    if leaf.is_some() {
        fields.extend(quote! {
            span: ::m2_syn::Span,
            leading_trivia: ::m2_syn::LeadingTrivia,
            trailing_trivia: ::m2_syn::TrailingTrivia,
        });
    }
    for attr in attrs {
        if attr.path().is_ident("comments") {
            fields.extend(quote!(comments: ::m2_syn::CommentTrivia,));
        } else if attr.path().is_ident("literal") {
            fields.extend(quote! {
                span: ::m2_syn::Span,
                leading_trivia: ::m2_syn::LeadingTrivia,
                trailing_trivia: ::m2_syn::TrailingTrivia,
            });
        } else if attr.path().is_ident("delimiter") {
            let delimiter: Ident = attr.parse_args()?;
            fields.extend(match delimiter.to_string().as_str() {
                "parenthesis" => quote!(delimiter: ::m2_syn::Delimiter![()],),
                "bracket" => quote!(delimiter: ::m2_syn::Delimiter![[]],),
                "brace" => quote!(delimiter: ::m2_syn::Delimiter![{}],),
                "angle_bar" => quote!(delimiter: ::m2_syn::Delimiter![<| |>],),
                _ => return Err(Error::new(delimiter.span(), "unknown delimiter family")),
            });
        }
    }
    Ok(fields)
}

fn parse_explicit_declaration(
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
    declarations: &mut Vec<Declaration>,
    syntax_attrs: &[Attribute],
) -> Result<ExplicitDeclarationKind> {
    let _attrs = input.call(Attribute::parse_outer)?;
    let name: Ident = input.parse()?;
    let boxed = syntax_attrs
        .iter()
        .any(|attribute| attribute.path().is_ident("boxed"));

    if input.is_empty() {
        let operator = operator_kind(syntax_attrs)?;
        let declared_leaf = leaf_kind(syntax_attrs)?;
        if operator.is_some() && declared_leaf.is_some() {
            return Err(Error::new(
                name.span(),
                "an explicit declaration cannot be both a leaf and an operator category",
            ));
        }
        let leaf = declared_leaf.or_else(|| operator.is_none().then(|| inferred_leaf_kind(&name)));
        let expression = match (operator, leaf) {
            (Some(operator), None) => GrammarExpr::Sum(Vec::new(), Some(operator)),
            (None, Some(leaf)) => GrammarExpr::Leaf(leaf),
            _ => {
                return Err(Error::new(
                    name.span(),
                    "an empty explicit declaration must be exactly one leaf or operator category",
                ));
            }
        };
        declarations.push(Declaration {
            name,
            expression,
            boxed,
            cell_stream: false,
            comments: false,
            explicit: true,
        });
        return Ok(ExplicitDeclarationKind { leaf, operator });
    }

    let fields = input.fork();
    let _field_attrs = fields.call(Attribute::parse_outer)?;
    if fields.peek(syn::Ident) {
        let _: Ident = fields.parse()?;
        if fields.peek(Token![:]) {
            let fields = parse_explicit_fields(input, tokens, declarations)?;
            let expression = wrap_struct_attributes(
                GrammarExpr::Product(fields, ProductStyle::Named),
                syntax_attrs,
            )?;
            declarations.push(Declaration {
                name,
                expression,
                boxed,
                cell_stream: syntax_attrs
                    .iter()
                    .any(|attribute| attribute.path().is_ident("cell_stream")),
                comments: syntax_attrs
                    .iter()
                    .any(|attribute| attribute.path().is_ident("comments")),
                explicit: true,
            });
            return Ok(ExplicitDeclarationKind::default());
        }
    }

    let variants = parse_explicit_variants(&name, input, tokens, declarations)?;

    declarations.push(Declaration {
        name,
        expression: GrammarExpr::Sum(variants, None),
        boxed,
        cell_stream: syntax_attrs
            .iter()
            .any(|attribute| attribute.path().is_ident("cell_stream")),
        comments: syntax_attrs
            .iter()
            .any(|attribute| attribute.path().is_ident("comments")),
        explicit: true,
    });
    Ok(ExplicitDeclarationKind::default())
}

fn parse_explicit_nested_body(
    name: &Ident,
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
    declarations: &mut Vec<Declaration>,
) -> Result<GrammarExpr> {
    if input.is_empty() {
        return Ok(GrammarExpr::Product(Vec::new(), ProductStyle::Named));
    }

    let fields = input.fork();
    let _attrs = fields.call(Attribute::parse_outer)?;
    let _name: Ident = fields.parse()?;
    if fields.peek(Token![:]) {
        return Ok(GrammarExpr::Product(
            parse_explicit_fields(input, tokens, declarations)?,
            ProductStyle::Named,
        ));
    }

    Ok(GrammarExpr::Sum(
        parse_explicit_variants(name, input, tokens, declarations)?,
        None,
    ))
}

fn parse_explicit_variants(
    parent: &Ident,
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
    declarations: &mut Vec<Declaration>,
) -> Result<Vec<GrammarField>> {
    let mut variants = Vec::new();
    while !input.is_empty() {
        let _attrs = input.call(Attribute::parse_outer)?;
        let variant: Ident = input.parse()?;
        let expression = if input.peek(Token![|]) {
            parse_explicit_type(input, tokens, declarations)?
        } else if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            let payload = format_ident!("{parent}{variant}");
            let expression = parse_explicit_nested_body(&payload, &content, tokens, declarations)?;
            declarations.push(explicit_declaration(payload.clone(), expression));
            GrammarExpr::Named(payload)
        } else if input.peek(syn::token::Paren) {
            let payload;
            parenthesized!(payload in input);
            let expression = parse_explicit_type(&payload, tokens, declarations)?;
            if !payload.is_empty() {
                return Err(payload.error("a syntax variant accepts exactly one payload"));
            }
            expression
        } else {
            return Err(input
                .error("a syntax variant needs `(Type)`, `{ nested declaration }`, or `|Type|`"));
        };
        variants.push(GrammarField {
            name: Some(variant),
            expression,
        });
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        }
    }
    Ok(variants)
}

fn explicit_declaration(name: Ident, expression: GrammarExpr) -> Declaration {
    Declaration {
        name,
        expression,
        boxed: false,
        cell_stream: false,
        comments: false,
        explicit: true,
    }
}

fn parse_explicit_fields(
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
    declarations: &mut Vec<Declaration>,
) -> Result<Vec<GrammarField>> {
    let mut fields = Vec::new();
    while !input.is_empty() {
        let _attrs = input.call(Attribute::parse_outer)?;
        let name: Ident = input.parse()?;
        input.parse::<Token![:]>()?;
        fields.push(GrammarField {
            name: Some(name),
            expression: parse_explicit_type(input, tokens, declarations)?,
        });
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        } else if !input.is_empty() {
            return Err(input.error("expected `,` between explicit syntax fields"));
        }
    }
    Ok(fields)
}

fn parse_explicit_type(
    input: ParseStream<'_>,
    tokens: &TokenDefinitions,
    declarations: &mut Vec<Declaration>,
) -> Result<GrammarExpr> {
    let mut expression = if input.peek(Token![|]) {
        input.parse::<Token![|]>()?;
        let name: Ident = input.parse()?;
        input.parse::<Token![|]>()?;
        let body = if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            parse_explicit_nested_body(&name, &content, tokens, declarations)?
        } else {
            GrammarExpr::Product(Vec::new(), ProductStyle::Named)
        };
        declarations.push(explicit_declaration(name.clone(), body));
        GrammarExpr::Named(name)
    } else {
        let name: Ident = input.parse()?;
        if name == "Token" && input.peek(Token![!]) {
            input.parse::<Token![!]>()?;
            let pattern;
            bracketed!(pattern in input);
            let pattern: TokenStream = pattern.parse()?;
            GrammarExpr::Token {
                name: tokens.resolve(&pattern)?,
                pattern,
            }
        } else if input.peek(Token![<]) {
            input.parse::<Token![<]>()?;
            let inner = parse_explicit_type(input, tokens, declarations)?;
            input.parse::<Token![>]>()?;
            match name.to_string().as_str() {
                "Vec" => GrammarExpr::Repeated(Box::new(inner), " "),
                "Lines" => GrammarExpr::Repeated(Box::new(inner), "\n"),
                "Punctuated" => GrammarExpr::Punctuated(Box::new(inner)),
                "Delimited" => GrammarExpr::Delimited(Box::new(inner)),
                "Option" => GrammarExpr::Optional(Box::new(inner)),
                "Box" => GrammarExpr::Boxed(Box::new(inner)),
                _ => {
                    return Err(Error::new(
                        name.span(),
                        "unsupported explicit syntax wrapper",
                    ));
                }
            }
        } else if input.peek(syn::token::Brace) {
            let content;
            braced!(content in input);
            let body = parse_explicit_nested_body(&name, &content, tokens, declarations)?;
            declarations.push(explicit_declaration(name.clone(), body));
            GrammarExpr::Named(name)
        } else {
            GrammarExpr::Named(name)
        }
    };

    while input.peek(Token![?]) || input.peek(Token![*]) {
        if input.peek(Token![?]) {
            input.parse::<Token![?]>()?;
            expression = GrammarExpr::Optional(Box::new(expression));
        } else {
            input.parse::<Token![*]>()?;
            expression = GrammarExpr::Boxed(Box::new(expression));
        }
    }
    Ok(expression)
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
    let cell_stream = attrs
        .iter()
        .any(|attribute| attribute.path().is_ident("cell_stream"));
    let comments = attrs
        .iter()
        .any(|attribute| attribute.path().is_ident("comments"));

    if input.peek(Token![;]) {
        input.parse::<Token![;]>()?;
        let leaf = leaf_kind(&attrs)?.unwrap_or_else(|| inferred_leaf_kind(&name));
        return Ok(Declaration {
            name,
            expression: GrammarExpr::Leaf(leaf),
            boxed,
            cell_stream,
            comments,
            explicit: false,
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
        cell_stream,
        comments,
        explicit: false,
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
    let operator = operator_kind(&attrs)?;
    Ok(Declaration {
        name,
        expression: GrammarExpr::Sum(variants, operator),
        boxed,
        cell_stream: false,
        comments: false,
        explicit: false,
    })
}

fn leaf_kind(attrs: &[Attribute]) -> Result<Option<LeafKind>> {
    attrs
        .iter()
        .find(|attribute| attribute.path().is_ident("leaf"))
        .map(|attribute| {
            let kind: Ident = attribute.parse_args()?;
            match kind.to_string().as_str() {
                "empty" => Ok(LeafKind::Empty),
                "adj" => Ok(LeafKind::Adj),
                "eoc" => Ok(LeafKind::Eoc),
                "eof" => Ok(LeafKind::Eof),
                "float" => Ok(LeafKind::Float),
                "integer" => Ok(LeafKind::Integer),
                "qualified" => Ok(LeafKind::QualifiedIdent),
                _ => Err(Error::new(kind.span(), "unknown leaf kind")),
            }
        })
        .transpose()
}

fn inferred_leaf_kind(name: &Ident) -> LeafKind {
    match name.to_string().as_str() {
        "Symbol" => LeafKind::QualifiedIdent,
        "Empty" => LeafKind::Empty,
        "Adj" => LeafKind::Adj,
        "EOC" => LeafKind::Eoc,
        "EOF" => LeafKind::Eof,
        "FloatLiteral" => LeafKind::Float,
        "IntegerLiteral" => LeafKind::Integer,
        _ => LeafKind::Ident,
    }
}

fn operator_kind(attrs: &[Attribute]) -> Result<Option<OperatorKind>> {
    attrs
        .iter()
        .find(|attribute| attribute.path().is_ident("operator"))
        .map(|attribute| {
            let role: Ident = attribute.parse_args()?;
            match role.to_string().as_str() {
                "prefix" => Ok(OperatorKind::Prefix),
                "binary" => Ok(OperatorKind::Binary),
                "postfix" => Ok(OperatorKind::Postfix),
                "augmented" => Ok(OperatorKind::Augmented),
                _ => Err(Error::new(role.span(), "unknown operator role")),
            }
        })
        .transpose()
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
            cell_stream: false,
            comments: false,
            explicit: false,
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
            let flags = DefinitionFlags {
                boxed: declaration.boxed,
                cell_stream: declaration.cell_stream,
                comments: declaration.comments,
                explicit: declaration.explicit,
            };
            self.lower_declaration(declaration.name, declaration.expression, flags)?;
        }
        Ok((self.structs, self.enums))
    }

    fn lower_declaration(
        &mut self,
        name: Ident,
        expression: GrammarExpr,
        flags: DefinitionFlags,
    ) -> Result<()> {
        match expression {
            GrammarExpr::Leaf(leaf) => {
                self.push_struct(name, StructFields::Leaf(leaf), None, flags)
            }
            GrammarExpr::Product(fields, style) => {
                let fields = lower_fields(&name, fields, style)?;
                self.push_struct(name, StructFields::Product { fields, style }, None, flags);
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
                    flags,
                );
            }
            GrammarExpr::Node(kind, fields) => {
                let fields = lower_fields(&name, fields, ProductStyle::Named)?;
                self.push_node(name, kind.to_string(), fields, flags);
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
                    flags,
                );
            }
            GrammarExpr::Sum(variants, operator) => {
                self.lower_sum(name, variants, operator, flags)?
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
        flags: DefinitionFlags,
    ) -> Result<()> {
        if let Some(kind) = operator {
            if !variants.is_empty() {
                return Err(Error::new(
                    name.span(),
                    "operator enums derive their variants",
                ));
            }
            let mut definition = EnumDefinition::operator(name, kind, self.tokens);
            definition.explicit = flags.explicit;
            self.enums.push(definition);
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
                        DefinitionFlags {
                            explicit: flags.explicit,
                            ..DefinitionFlags::default()
                        },
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
                        DefinitionFlags {
                            explicit: flags.explicit,
                            ..DefinitionFlags::default()
                        },
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
                        DefinitionFlags {
                            explicit: flags.explicit,
                            ..DefinitionFlags::default()
                        },
                    );
                    (variant_name.clone(), variant_name)
                }
                expression => {
                    let variant_name = required_variant_name(variant.name, &name)?;
                    lowered.push(VariantDefinition {
                        name: variant_name,
                        shape: lower_field_shape(expression)?,
                    });
                    continue;
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
            boxed: flags.boxed,
            explicit: flags.explicit,
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
        flags: DefinitionFlags,
    ) {
        self.structs.push(StructDefinition {
            name,
            fields,
            wrapper,
            cst_kind: None,
            boxed: flags.boxed,
            cell_stream: flags.cell_stream,
            comments: flags.comments,
            explicit: flags.explicit,
        });
    }

    fn push_node(
        &mut self,
        name: Ident,
        cst_kind: String,
        fields: Vec<FieldDefinition>,
        flags: DefinitionFlags,
    ) {
        self.structs.push(StructDefinition {
            name,
            fields: StructFields::Product {
                fields,
                style: ProductStyle::Named,
            },
            wrapper: None,
            cst_kind: Some(cst_kind),
            boxed: flags.boxed,
            cell_stream: flags.cell_stream,
            comments: flags.comments,
            explicit: flags.explicit,
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
    use crate::syntax::{LeafKind, Shape, StructFields, Syntax};
    use quote::quote;
    use syn::parse2;

    #[test]
    fn parses_and_lowers_structs_enums_and_attributes() {
        let syntax: Syntax = parse2(quote! {
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

    #[test]
    fn explicit_declarations_supply_atoms_and_nested_type_families() {
        let syntax: Syntax = parse2(quote! {
            tokens {
                [=] { infix } (14, R)
                [+] { pref, bin, aug } (50, L)
                [.] [.?]
            }

            #[operator(prefix)]
            strutuct! {
                #[derive(Debug, ::m2_syn::Spanned)]
                PrefixOperator
            }

            strutuct! {
                #[derive(Debug, ::m2_syn::Spanned)]
                Symbol
            }

            strutuct! {
                #[derive(Debug, ::m2_syn::Spanned)]
                MemberOwner
                operator: |MemberOperator| {
                    Dot(Token![.])
                    OptionalDot(Token![.?])
                },
            }

            strutuct! {
                #[derive(Debug, ::m2_syn::Spanned)]
                Recursive
                child: Recursive*,
            }
        })
        .unwrap();

        assert!(syntax.structs.iter().any(|node| node.name == "Symbol"
            && node.explicit
            && matches!(node.fields, StructFields::Leaf(LeafKind::QualifiedIdent))));
        assert!(syntax.enums.iter().any(|node| node.name == "PrefixOperator"
            && node.explicit
            && node.variants.len() == 1));
        assert!(syntax.enums.iter().any(|node| node.name == "MemberOperator"
            && node.explicit
            && node.variants.len() == 2));
        assert!(syntax.structs.iter().any(|node| {
            node.name == "Recursive"
                && matches!(
                    &node.fields,
                    StructFields::Product { fields, .. }
                        if matches!(fields[0].shape, Shape::Boxed(_))
                )
        }));

        let expansion = syntax.expand().unwrap();
        let rendered = expansion.to_string();
        assert!(rendered.contains("PrefixOperator Add (:: m2_syn :: Token ! [+])"));
        assert!(rendered.contains("package_qualifier"));
        syn::parse2::<syn::File>(expansion).unwrap();
    }
}
