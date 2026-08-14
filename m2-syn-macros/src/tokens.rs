use std::collections::{BTreeMap, BTreeSet};
use std::iter;

use proc_macro2::{Delimiter, Ident, Punct, Spacing, Span, TokenStream, TokenTree};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::{Error, LitStr, Result, Token, braced, bracketed, parse_str};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum OperatorKind {
    Prefix,
    Binary,
    Postfix,
}

impl OperatorKind {
    pub(crate) fn enum_name(self) -> Ident {
        match self {
            Self::Prefix => format_ident!("PrefixOperator"),
            Self::Binary => format_ident!("BinaryOperator"),
            Self::Postfix => format_ident!("PostfixOperator"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum DeclarationKind {
    Prefix,
    Binary,
    Postfix,
    Augmented,
}

impl Parse for DeclarationKind {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let ident: Ident = input.parse()?;
        match ident.to_string().as_str() {
            "pref" => Ok(Self::Prefix),
            "bin" => Ok(Self::Binary),
            "post" => Ok(Self::Postfix),
            "aug" => Ok(Self::Augmented),
            _ => Err(Error::new(
                ident.span(),
                "expected `pref`, `bin`, `post`, or `aug`",
            )),
        }
    }
}

impl From<DeclarationKind> for Option<OperatorKind> {
    fn from(value: DeclarationKind) -> Self {
        match value {
            DeclarationKind::Prefix => Some(OperatorKind::Prefix),
            DeclarationKind::Binary => Some(OperatorKind::Binary),
            DeclarationKind::Postfix => Some(OperatorKind::Postfix),
            DeclarationKind::Augmented => None,
        }
    }
}

#[derive(Clone)]
struct OperatorDeclaration {
    pattern: TokenStream,
    name: String,
    spelling: String,
    kinds: BTreeSet<DeclarationKind>,
}

impl Parse for OperatorDeclaration {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let pattern_content;
        bracketed!(pattern_content in input);
        let kinds_content;
        braced!(kinds_content in input);

        let pattern = pattern_content.parse()?;
        let kinds = Punctuated::<DeclarationKind, Token![,]>::parse_terminated(&kinds_content)?
            .into_iter()
            .collect::<BTreeSet<_>>();
        let spelling = token_spelling(&pattern)?;
        let name = token_name(&pattern, NameStyle::Operator)?;

        Ok(Self {
            pattern,
            name,
            spelling,
            kinds,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum NameStyle {
    Operator,
    Keyword,
    Marker,
    Punctuation,
}

#[derive(Clone)]
struct PlainDeclaration {
    pattern: TokenStream,
    style: NameStyle,
}

impl PlainDeclaration {
    fn parse(input: ParseStream<'_>, style: NameStyle) -> Result<Self> {
        let pattern_content;
        bracketed!(pattern_content in input);
        Ok(Self {
            pattern: pattern_content.parse()?,
            style,
        })
    }
}

#[derive(Clone)]
pub(crate) struct TokenDefinition {
    pub(crate) name: Ident,
    pub(crate) pattern: TokenStream,
    pub(crate) spelling: String,
    pub(crate) operators: BTreeSet<OperatorKind>,
}

pub(crate) struct TokenDefinitions {
    definitions: Vec<TokenDefinition>,
}

impl TokenDefinitions {
    pub(crate) fn iter(&self) -> impl Iterator<Item = &TokenDefinition> {
        self.definitions.iter()
    }

    pub(crate) fn operator_variants(
        &self,
        kind: OperatorKind,
    ) -> impl Iterator<Item = &TokenDefinition> {
        self.definitions
            .iter()
            .filter(move |definition| definition.operators.contains(&kind))
    }

    pub(crate) fn resolve(&self, pattern: &TokenStream) -> Result<Ident> {
        let spelling = token_spelling(pattern)?;
        self.definitions
            .iter()
            .find(|definition| definition.spelling == spelling)
            .map(|definition| definition.name.clone())
            .ok_or_else(|| Error::new_spanned(pattern, "token was not declared in a token stage"))
    }

    pub(crate) fn spelling(&self, name: &Ident) -> Option<&str> {
        self.definitions
            .iter()
            .find(|definition| definition.name == *name)
            .map(|definition| definition.spelling.as_str())
    }

    pub(crate) fn pattern(&self, name: &Ident) -> Option<&TokenStream> {
        self.definitions
            .iter()
            .find(|definition| definition.name == *name)
            .map(|definition| &definition.pattern)
    }

    pub(crate) fn variant_name(&self, name: &Ident) -> Ident {
        let name = name.to_string();
        let friendly = name.strip_suffix("Keyword").unwrap_or(&name);
        Ident::new(friendly, name_span(name.as_str(), &self.definitions))
    }

    pub(crate) fn expand(&self) -> TokenStream {
        let definitions = self.definitions.iter().map(|definition| {
            let name = &definition.name;
            let spelling = &definition.spelling;
            let emit = quote!(output.push_text(#spelling, span););

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

                impl ::m2_syn::Token for #name {}

                impl ::m2_syn::ToTokens for #name {
                    fn to_tokens(&self, output: &mut ::m2_syn::TokenStream) {
                        let span = ::m2_syn::Spanned::span(self);
                        #emit
                    }
                }

                impl<N> ::m2_syn::Reconstruct<N> for #name
                where
                    N: ::m2_syn::CstNode,
                {
                    fn matches(node: &N) -> bool {
                        node.identity().matches(#spelling, false)
                    }

                    fn reconstruct(node: N) -> Result<Self, ::m2_syn::ReconstructError> {
                        if !<Self as ::m2_syn::Reconstruct<N>>::matches(&node) {
                            return Err(::m2_syn::ReconstructError::wrong_node(
                                #spelling,
                                false,
                                node.identity(),
                            ));
                        }
                        Ok(Self::new(node.span()))
                    }
                }
            }
        });
        let arms = self.definitions.iter().map(|definition| {
            let name = &definition.name;
            let pattern = &definition.pattern;
            quote! { [#pattern] => { $crate::#name }; }
        });
        quote! {
            #(#definitions)*

            #[macro_export]
            macro_rules! Token {
                #(#arms)*
                [$($unsupported:tt)*] => {
                    compile_error!("token was not declared in `syntax!`")
                };
            }
        }
    }

    fn new(
        operators: Vec<OperatorDeclaration>,
        plain: impl IntoIterator<Item = PlainDeclaration>,
    ) -> Result<Self> {
        let mut definitions = Vec::<TokenDefinition>::new();
        let mut names = BTreeMap::<String, usize>::new();

        for operator in operators {
            let categories = operator
                .kinds
                .iter()
                .copied()
                .filter_map(Option::<OperatorKind>::from)
                .collect();
            insert_definition(
                &mut definitions,
                &mut names,
                TokenDefinition {
                    name: ident(&operator.name, operator.pattern.span())?,
                    pattern: operator.pattern.clone(),
                    spelling: operator.spelling.clone(),
                    operators: categories,
                },
            )?;

            if operator.kinds.contains(&DeclarationKind::Augmented) {
                let pattern = augmented_pattern(&operator.pattern)?;
                let name = format!("{}Eql", operator.name);
                insert_definition(
                    &mut definitions,
                    &mut names,
                    TokenDefinition {
                        name: ident(&name, pattern.span())?,
                        pattern,
                        spelling: format!("{}=", operator.spelling),
                        operators: BTreeSet::from([OperatorKind::Binary]),
                    },
                )?;
            }
        }

        for declaration in plain {
            let spelling = token_spelling(&declaration.pattern)?;
            let name = token_name(&declaration.pattern, declaration.style)?;
            insert_definition(
                &mut definitions,
                &mut names,
                TokenDefinition {
                    name: ident(&name, declaration.pattern.span())?,
                    pattern: declaration.pattern,
                    spelling,
                    operators: BTreeSet::new(),
                },
            )?;
        }

        Ok(Self { definitions })
    }
}

fn name_span(name: &str, definitions: &[TokenDefinition]) -> Span {
    definitions
        .iter()
        .find(|definition| definition.name == name)
        .map_or_else(Span::call_site, |definition| definition.name.span())
}

impl Parse for TokenDefinitions {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let operators = parse_operator_stage(input)?;
        let keywords = parse_plain_stage(input, "keywords", NameStyle::Keyword)?;
        let markers = parse_plain_stage(input, "markers", NameStyle::Marker)?;
        let punctuation = parse_plain_stage(input, "punct", NameStyle::Punctuation)?;

        Self::new(
            operators,
            keywords.into_iter().chain(markers).chain(punctuation),
        )
    }
}

fn parse_operator_stage(input: ParseStream<'_>) -> Result<Vec<OperatorDeclaration>> {
    parse_stage_name(input, "tokens", false)?;
    let content;
    braced!(content in input);
    let mut declarations = Vec::new();
    while !content.is_empty() {
        declarations.push(content.parse()?);
    }
    Ok(declarations)
}

fn parse_plain_stage(
    input: ParseStream<'_>,
    expected: &str,
    style: NameStyle,
) -> Result<Vec<PlainDeclaration>> {
    parse_stage_name(input, expected, true)?;
    let content;
    braced!(content in input);
    let mut declarations = Vec::new();
    while !content.is_empty() {
        declarations.push(PlainDeclaration::parse(&content, style)?);
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        }
    }
    Ok(declarations)
}

fn parse_stage_name(input: ParseStream<'_>, expected: &str, colon: bool) -> Result<()> {
    let name: Ident = input.parse()?;
    if name != expected {
        return Err(Error::new(
            name.span(),
            format!("expected `{expected}` token stage"),
        ));
    }
    if colon {
        input.parse::<Token![:]>()?;
    }
    Ok(())
}

fn insert_definition(
    definitions: &mut Vec<TokenDefinition>,
    names: &mut BTreeMap<String, usize>,
    definition: TokenDefinition,
) -> Result<()> {
    let name = definition.name.to_string();
    if let Some(index) = names.get(&name).copied() {
        let existing = &mut definitions[index];
        if existing.spelling != definition.spelling
            || existing.pattern.to_string() != definition.pattern.to_string()
        {
            return Err(Error::new(
                definition.name.span(),
                format!(
                    "token name `{name}` is already generated for `{}`",
                    existing.spelling
                ),
            ));
        }
        existing.operators.extend(definition.operators);
    } else {
        names.insert(name, definitions.len());
        definitions.push(definition);
    }
    Ok(())
}

fn token_name(pattern: &TokenStream, style: NameStyle) -> Result<String> {
    let base = if pattern.is_empty() {
        "Adj".to_owned()
    } else {
        pattern
            .clone()
            .into_iter()
            .map(token_tree_name)
            .collect::<Result<String>>()?
    };

    Ok(match style {
        NameStyle::Keyword => format!("{base}Keyword"),
        NameStyle::Operator | NameStyle::Marker | NameStyle::Punctuation => base,
    })
}

fn token_tree_name(tree: TokenTree) -> Result<String> {
    match tree {
        TokenTree::Group(group) => {
            if group.delimiter() != Delimiter::Parenthesis {
                return Err(Error::new(group.span(), "groups are not valid tokens"));
            }
            let mut inner = group.stream().into_iter();
            match (inner.next(), inner.next()) {
                (Some(TokenTree::Punct(punctuation)), None) if punctuation.as_char() == '*' => {
                    Ok("Graded".to_owned())
                }
                _ => Err(Error::new(
                    group.span(),
                    "only `(*)` is supported as a grouped token",
                )),
            }
        }
        TokenTree::Ident(ident) if ident == "_" => Ok(punctuation_name('_')?.to_owned()),
        TokenTree::Ident(ident) => Ok(word_name(&ident.to_string())),
        TokenTree::Literal(literal) => parse_str::<LitStr>(&literal.to_string())
            .map_err(|_| Error::new(literal.span(), "token literals must be string literals"))?
            .value()
            .chars()
            .map(punctuation_name)
            .collect(),
        TokenTree::Punct(punctuation) => punctuation_name(punctuation.as_char()).map(str::to_owned),
    }
}

fn word_name(word: &str) -> String {
    if word.chars().all(|character| !character.is_lowercase()) {
        let mut characters = word.chars();
        return characters
            .next()
            .into_iter()
            .flat_map(char::to_uppercase)
            .chain(characters.flat_map(char::to_lowercase))
            .collect();
    }

    let mut characters = word.chars();
    characters
        .next()
        .into_iter()
        .flat_map(char::to_uppercase)
        .chain(characters)
        .collect()
}

fn token_spelling(pattern: &TokenStream) -> Result<String> {
    if pattern.is_empty() {
        return Ok(String::new());
    }

    let mut spelling = String::new();
    for tree in pattern.clone() {
        match tree {
            TokenTree::Group(group) => {
                if token_tree_name(TokenTree::Group(group.clone()))? == "Graded" {
                    spelling.push_str("(*)");
                }
            }
            TokenTree::Ident(ident) => spelling.push_str(&ident.to_string()),
            TokenTree::Literal(literal) => {
                spelling.push_str(
                    &parse_str::<LitStr>(&literal.to_string())
                        .map_err(|_| {
                            Error::new(literal.span(), "token literals must be string literals")
                        })?
                        .value(),
                );
            }
            TokenTree::Punct(punctuation) => spelling.push(punctuation.as_char()),
        }
    }
    Ok(spelling)
}

fn augmented_pattern(pattern: &TokenStream) -> Result<TokenStream> {
    if pattern.is_empty()
        || pattern
            .clone()
            .into_iter()
            .any(|tree| matches!(tree, TokenTree::Group(_)))
    {
        return Err(Error::new_spanned(
            pattern,
            "an empty or grouped token cannot be augmented",
        ));
    }

    let trees = pattern.clone().into_iter().collect::<Vec<_>>();
    if let [TokenTree::Literal(literal)] = trees.as_slice() {
        let mut value = parse_str::<LitStr>(&literal.to_string())
            .map_err(|_| Error::new(literal.span(), "token literals must be string literals"))?
            .value();
        value.push('=');
        let literal = LitStr::new(&value, literal.span());
        return Ok(quote!(#literal));
    }

    let mut trees = trees;
    if let Some(TokenTree::Punct(punctuation)) = trees.last_mut() {
        let mut joined = Punct::new(punctuation.as_char(), Spacing::Joint);
        joined.set_span(punctuation.span());
        *punctuation = joined;
    }
    let mut result = trees.into_iter().collect::<TokenStream>();
    result.extend(iter::once(TokenTree::Punct(Punct::new(
        '=',
        Spacing::Alone,
    ))));
    Ok(result)
}

fn ident(name: &str, span: Span) -> Result<Ident> {
    parse_str::<Ident>(name).map_err(|_| Error::new(span, "generated an invalid token name"))
}

fn punctuation_name(character: char) -> Result<&'static str> {
    match character {
        '.' => Ok("Dot"),
        ',' => Ok("Cma"),
        ';' => Ok("Scl"),
        ':' => Ok("Col"),
        '#' => Ok("Hsh"),
        '@' => Ok("Att"),
        '$' => Ok("Dlr"),
        '%' => Ok("Mod"),
        '^' => Ok("Crt"),
        '&' => Ok("Amp"),
        '*' => Ok("Mul"),
        '+' => Ok("Add"),
        '-' => Ok("Sub"),
        '=' => Ok("Eql"),
        '<' => Ok("Lst"),
        '>' => Ok("Gst"),
        '!' => Ok("Bng"),
        '?' => Ok("Qsm"),
        '~' => Ok("Tld"),
        '|' => Ok("Pip"),
        '/' => Ok("Slh"),
        '_' => Ok("Ubs"),
        '\\' => Ok("Bsl"),
        '·' => Ok("Cdt"),
        '⊠' => Ok("Box"),
        '⧢' => Ok("Sfp"),
        _ => Err(Error::new(
            Span::call_site(),
            format!("unsupported punctuation character `{character}`"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use syn::parse2;

    fn declarations() -> TokenDefinitions {
        parse2(quote! {
            tokens {
                [+] {pref, bin, aug}
                [!] {post}
                [not] {pref}
                [(*)] {post}
                ["\\"] {bin, aug}
                [SPACE] {bin}
                [] {bin}
            }
            keywords: { [if] [symbol] [threadLocal] [threadVariable] }
            markers: {}
            punct: { [;], [,] }
        })
        .unwrap()
    }

    #[test]
    fn stages_generate_names_and_augmented_tokens() {
        let definitions = declarations();
        let names = definitions
            .iter()
            .map(|definition| (definition.name.to_string(), definition.spelling.clone()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(names["Add"], "+");
        assert_eq!(names["AddEql"], "+=");
        assert_eq!(names["BslEql"], "\\=");
        assert_eq!(names["Graded"], "(*)");
        assert_eq!(names["Adj"], "");
        assert_eq!(names["Space"], "SPACE");
        assert_eq!(names["IfKeyword"], "if");
        assert_eq!(names["ThreadLocalKeyword"], "threadLocal");
        assert_eq!(names["ThreadVariableKeyword"], "threadVariable");
        assert_eq!(names["Scl"], ";");
    }

    #[test]
    fn categories_include_only_the_requested_operator_forms() {
        let definitions = declarations();
        let prefix = definitions
            .operator_variants(OperatorKind::Prefix)
            .map(|definition| definition.name.to_string())
            .collect::<BTreeSet<_>>();
        let binary = definitions
            .operator_variants(OperatorKind::Binary)
            .map(|definition| definition.name.to_string())
            .collect::<BTreeSet<_>>();
        let postfix = definitions
            .operator_variants(OperatorKind::Postfix)
            .map(|definition| definition.name.to_string())
            .collect::<BTreeSet<_>>();

        assert_eq!(prefix, BTreeSet::from(["Add".into(), "Not".into()]));
        assert_eq!(
            binary,
            BTreeSet::from([
                "Adj".into(),
                "Add".into(),
                "AddEql".into(),
                "Bsl".into(),
                "BslEql".into(),
                "Space".into(),
            ])
        );
        assert_eq!(postfix, BTreeSet::from(["Bng".into(), "Graded".into()]));
    }
}
