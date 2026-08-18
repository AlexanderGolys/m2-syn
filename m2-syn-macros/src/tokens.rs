use std::collections::{BTreeMap, BTreeSet};
use std::iter;

use proc_macro2::{Delimiter, Ident, Punct, Spacing, Span, TokenStream, TokenTree};
use quote::{format_ident, quote};
use syn::parse::{Parse as SynParse, ParseStream};
use syn::spanned::Spanned;
use syn::{Error, Expr, LitStr, Result, Token, braced, bracketed, parenthesized, parse_str};

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

#[derive(Clone)]
struct OperatorDeclaration {
    pattern: TokenStream,
    name: String,
    spelling: String,
    parse_info: ParseInfo,
}

impl SynParse for OperatorDeclaration {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let pattern_content;
        bracketed!(pattern_content in input);
        let info_content;
        braced!(info_content in input);

        let pattern = pattern_content.parse()?;
        let spelling = token_spelling(&pattern)?;
        let name = token_name(&pattern, NameStyle::Operator)?;
        let flags: ParseFlags = info_content.parse()?;
        let parse_info = flags.into_parse_info(input)?;

        Ok(Self {
            pattern,
            name,
            spelling,
            parse_info,
        })
    }
}

#[derive(Clone, Default)]
struct ParseInfo {
    binary: Option<(u8, u8)>,
    prefix: Option<(u8, u8)>,
    postfix: Option<u8>,
    special: Option<(u8, u8)>,
    augmented: bool,
}

impl ParseInfo {
    fn operator_kinds(&self) -> BTreeSet<OperatorKind> {
        let mut kinds = BTreeSet::new();
        if self.prefix.is_some() {
            kinds.insert(OperatorKind::Prefix);
        }
        if self.binary.is_some() {
            kinds.insert(OperatorKind::Binary);
        }
        if self.postfix.is_some() {
            kinds.insert(OperatorKind::Postfix);
        }
        kinds
    }

    fn expand(&self) -> Option<TokenStream> {
        if let Some((precedence, strength)) = &self.special {
            return Some(quote!($parse_info::binary(#precedence, #strength)));
        }
        if let Some(precedence) = &self.postfix {
            return Some(quote!($parse_info::postfix(#precedence)));
        }
        match (&self.binary, &self.prefix) {
            (Some((precedence, binary_strength)), Some((_, unary_strength))) => {
                Some(quote!($parse_info::prefix_binary(
                    #precedence,
                    #binary_strength,
                    #unary_strength,
                )))
            }
            (Some((precedence, strength)), None) => {
                Some(quote!($parse_info::binary(#precedence, #strength)))
            }
            (None, Some((precedence, unary_strength))) => {
                Some(quote!($parse_info::prefix(#precedence, #unary_strength)))
            }
            (None, None) => None,
        }
    }

    fn signature(&self) -> String {
        self.expand()
            .map_or_else(String::new, |value| value.to_string())
    }
}

/// The bare `bin`/`pref`/`post`/`infix`/`aug` tags inside a token's `{...}`
/// block. The precedence numbers that give these tags meaning live outside
/// the braces, in the trailing `(precedence, binary_strength, unary_strength)`
/// triple, so a shared precedence is written once per token instead of once
/// per tag.
#[derive(Default)]
struct ParseFlags {
    binary: bool,
    prefix: bool,
    postfix: bool,
    infix: bool,
    augmented: bool,
}

impl SynParse for ParseFlags {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut flags = Self::default();
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            let slot = match name.to_string().as_str() {
                "bin" => &mut flags.binary,
                "pref" => &mut flags.prefix,
                "post" => &mut flags.postfix,
                "infix" => &mut flags.infix,
                "aug" => &mut flags.augmented,
                _ => {
                    return Err(Error::new(
                        name.span(),
                        "expected `bin`, `pref`, `post`, `infix`, or `aug`",
                    ));
                }
            };
            if std::mem::replace(slot, true) {
                return Err(Error::new(name.span(), "duplicate parser metadata tag"));
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else if !input.is_empty() {
                return Err(input.error("expected `,` between parser metadata tags"));
            }
        }
        Ok(flags)
    }
}

impl ParseFlags {
    fn requires_precedence(&self) -> bool {
        self.binary || self.prefix || self.postfix || self.infix
    }

    /// Consumes the trailing precedence triple from `input` (the token
    /// declaration's outer stream, past the closing `}`) and combines it with
    /// these flags into a full [`ParseInfo`].
    fn into_parse_info(self, input: ParseStream<'_>) -> Result<ParseInfo> {
        let ordinary_actions =
            usize::from(self.binary || self.prefix) + usize::from(self.postfix);
        if ordinary_actions + usize::from(self.infix) > 1 {
            return Err(Error::new(
                Span::call_site(),
                "a token cannot combine operator, postfix, and special parser actions",
            ));
        }

        let mut info = ParseInfo {
            augmented: self.augmented,
            ..ParseInfo::default()
        };
        if self.requires_precedence() {
            let (precedence, binary_strength, unary_strength) = parse_precedence_triple(input)?;
            if self.binary || self.infix {
                let strength = binary_strength.ok_or_else(|| {
                    Error::new(
                        Span::call_site(),
                        "`bin`/`infix` require a binary strength in the second precedence slot",
                    )
                })?;
                if self.binary {
                    info.binary = Some((precedence, strength));
                } else {
                    info.special = Some((precedence, strength));
                }
            } else if binary_strength.is_some() {
                return Err(Error::new(
                    Span::call_site(),
                    "the binary strength slot is only used by `bin`/`infix`; write `_`",
                ));
            }
            if self.prefix {
                let strength = unary_strength.ok_or_else(|| {
                    Error::new(
                        Span::call_site(),
                        "`pref` requires a unary strength in the third precedence slot",
                    )
                })?;
                info.prefix = Some((precedence, strength));
            } else if unary_strength.is_some() {
                return Err(Error::new(
                    Span::call_site(),
                    "the unary strength slot is only used by `pref`; write `_`",
                ));
            }
            if self.postfix {
                info.postfix = Some(precedence);
            }
        }
        Ok(info)
    }
}

/// Parses the trailing `(precedence, binary_strength, unary_strength)` triple
/// that follows a token's `{ bin, pref, ... }` flags. Each strength slot is
/// either a `u8` literal or `_` for a flag that doesn't use it.
fn parse_precedence_triple(input: ParseStream<'_>) -> Result<(u8, Option<u8>, Option<u8>)> {
    let content;
    parenthesized!(content in input);
    let precedence = parse_precedence_slot(&content)?.ok_or_else(|| {
        Error::new(
            content.span(),
            "the precedence slot is required and cannot be `_`",
        )
    })?;
    content.parse::<Token![,]>()?;
    let binary_strength = parse_precedence_slot(&content)?;
    content.parse::<Token![,]>()?;
    let unary_strength = parse_precedence_slot(&content)?;
    if !content.is_empty() {
        return Err(content.error("expected exactly three precedence slots"));
    }
    Ok((precedence, binary_strength, unary_strength))
}

fn parse_precedence_slot(input: ParseStream<'_>) -> Result<Option<u8>> {
    if input.peek(Token![_]) {
        input.parse::<Token![_]>()?;
        return Ok(None);
    }
    let literal: syn::LitInt = input.parse()?;
    Ok(Some(literal.base10_parse()?))
}

#[derive(Debug, Clone, Copy)]
enum NameStyle {
    Operator,
    Keyword,
    Marker,
    Punctuation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawTokenKind {
    Punct,
    Ident,
    Whitespace,
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
    parse_info: ParseInfo,
    is_operator: bool,
    is_keyword: bool,
    raw_kind: RawTokenKind,
}

pub(crate) struct TokenDefinitions {
    precedences: Vec<(Ident, Expr)>,
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
        let delimiters = expand_delimiters();
        let precedence_names = self.precedences.iter().map(|(name, _)| name);
        let precedence_values = self.precedences.iter().map(|(_, value)| value);
        let operator_spellings = self
            .definitions
            .iter()
            .filter(|definition| definition.is_operator)
            .map(|definition| definition.spelling.as_str());
        let postfix_operator_spellings = self
            .definitions
            .iter()
            .filter(|definition| definition.operators.contains(&OperatorKind::Postfix))
            .map(|definition| definition.spelling.as_str());
        let keyword_spellings = self
            .definitions
            .iter()
            .filter(|definition| definition.is_keyword)
            .map(|definition| definition.spelling.as_str());
        let punctuation_spellings = self
            .definitions
            .iter()
            .map(|definition| definition.spelling.as_str())
            .filter(|spelling| {
                !spelling.is_empty()
                    && spelling
                        .chars()
                        .all(|character| !character.is_ascii_alphanumeric() && character != ' ')
            });
        let definitions = self.definitions.iter().map(|definition| {
            let name = &definition.name;
            let type_name = hidden_token_name(name);
            let spelling = &definition.spelling;
            let (raw_new, raw_matches) = match definition.raw_kind {
                RawTokenKind::Punct => (
                    quote!(::m2_syn::TokenTree::Punct(::m2_syn::Punct::new(#spelling, span))),
                    quote! {
                        match token {
                            ::m2_syn::TokenTree::Punct(token) => token.text() == #spelling,
                            _ => false,
                        }
                    },
                ),
                RawTokenKind::Ident => (
                    quote!(::m2_syn::TokenTree::Ident(::m2_syn::IdentToken::new(#spelling, span))),
                    quote! {
                        match token {
                            ::m2_syn::TokenTree::Ident(token) => {
                                token.text() == #spelling
                                    || token.text().strip_prefix("Core$") == Some(#spelling)
                            }
                            _ => false,
                        }
                    },
                ),
                RawTokenKind::Whitespace => (
                    quote!(::m2_syn::TokenTree::Trivia(::m2_syn::Trivia::new(
                        ::m2_syn::TriviaKind::Whitespace,
                        " ",
                        span,
                    ))),
                    quote! {
                        match token {
                            ::m2_syn::TokenTree::Trivia(token) => {
                                token.kind() == ::m2_syn::TriviaKind::Whitespace
                                    && !token.contains_line_break()
                            }
                            ::m2_syn::TokenTree::Ident(token) => token.text() == "SPACE",
                            _ => false,
                        }
                    },
                ),
            };

            quote! {

                #[derive(Debug, Clone, PartialEq, Eq, ::m2_syn::Spanned)]
                pub struct #type_name {
                    raw: ::std::boxed::Box<::m2_syn::TokenTree>,
                }

                #[doc(hidden)]
                #[allow(non_snake_case)]
                pub fn #type_name<S: ::m2_syn::Spanned>(span: S) -> #type_name {
                    let span = span.span();
                    #type_name { raw: ::std::boxed::Box::new(#raw_new) }
                }

                impl ::m2_syn::Token for #type_name {
                    const SPELLING: &'static str = #spelling;

                    fn matches_token_tree(token: &::m2_syn::TokenTree) -> bool {
                        #raw_matches
                    }

                    fn from_token_tree(token: ::m2_syn::TokenTree) -> ::std::option::Option<Self> {
                        Self::matches_token_tree(&token).then_some(Self {
                            raw: ::std::boxed::Box::new(token),
                        })
                    }
                }

                impl ::m2_syn::Parse for #type_name {
                    fn parse(
                        input: &mut ::m2_syn::ParseStream,
                    ) -> Result<Self, ::m2_syn::TokenParseError> {
                        input.parse_token()
                    }
                }

                impl ::m2_syn::ToTokens for #type_name {
                    fn to_tokens(&self, output: &mut ::m2_syn::TokenStream) {
                        ::m2_syn::ToTokens::to_tokens(&self.raw, output);
                    }
                }

                impl ::m2_syn::PrettyTree for #type_name {
                    fn pretty_tree(&self) -> ::m2_syn::PrettyNode {
                        ::m2_syn::PrettyNode::token(
                            ::std::format!("Token![{}]", <Self as ::m2_syn::Token>::SPELLING),
                            <Self as ::m2_syn::Token>::SPELLING,
                            ::m2_syn::Spanned::span(self),
                        )
                    }
                }

                impl<N> ::m2_syn::Reconstruct<N> for #type_name
                where
                    N: ::m2_syn::ExternalCstNode,
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
                        Ok(#type_name(node.span()))
                    }
                }
            }
        });
        let arms = self.definitions.iter().map(|definition| {
            let name = hidden_token_name(&definition.name);
            let pattern = &definition.pattern;
            quote! { [#pattern] => { $crate::#name }; }
        });
        let parse_info_arms = self.definitions.iter().filter_map(|definition| {
            let spelling = &definition.spelling;
            let parse_info = definition.parse_info.expand()?;
            Some(quote! { #spelling => Some(#parse_info), })
        });
        quote! {
            #delimiters

            #(pub(crate) const #precedence_names: u8 = #precedence_values;)*

            pub(crate) const GENERATED_OPERATOR_SPELLINGS: &[&str] = &[
                #(#operator_spellings),*
            ];

            pub(crate) const GENERATED_POSTFIX_OPERATOR_SPELLINGS: &[&str] = &[
                #(#postfix_operator_spellings),*
            ];

            pub(crate) const GENERATED_KEYWORD_SPELLINGS: &[&str] = &[
                #(#keyword_spellings),*
            ];

            pub(crate) const GENERATED_PUNCTUATION_SPELLINGS: &[&str] = &[
                #(#punctuation_spellings),*
            ];

            #(#definitions)*

            #[macro_export]
            macro_rules! Token {
                #(#arms)*
                [$($unsupported:tt)*] => {
                    compile_error!("token was not declared in `syntax!`")
                };
            }

            #[doc(hidden)]
            #[macro_export]
            macro_rules! __m2_syn_parse_info {
                ($spelling:expr, $parse_info:ident) => {
                    match $spelling {
                        #(#parse_info_arms)*
                        _ => None,
                    }
                };
            }
        }
    }

    fn new(
        precedences: Vec<(Ident, Expr)>,
        augmented_parse_info: ParseInfo,
        operators: Vec<OperatorDeclaration>,
        plain: impl IntoIterator<Item = PlainDeclaration>,
    ) -> Result<Self> {
        let mut definitions = Vec::<TokenDefinition>::new();
        let mut names = BTreeMap::<String, usize>::new();

        for operator in operators {
            let raw_kind = raw_token_kind(&operator.spelling, &operator.pattern, false);
            let categories = operator.parse_info.operator_kinds();
            insert_definition(
                &mut definitions,
                &mut names,
                TokenDefinition {
                    name: ident(&operator.name, operator.pattern.span())?,
                    pattern: operator.pattern.clone(),
                    spelling: operator.spelling.clone(),
                    operators: categories,
                    parse_info: operator.parse_info.clone(),
                    is_operator: true,
                    is_keyword: false,
                    raw_kind,
                },
            )?;

            if operator.parse_info.augmented {
                let pattern = augmented_pattern(&operator.pattern)?;
                let name = format!("{}Eql", operator.name);
                insert_definition(
                    &mut definitions,
                    &mut names,
                    TokenDefinition {
                        name: ident(&name, pattern.span())?,
                        pattern,
                        spelling: format!("{}=", operator.spelling),
                        operators: augmented_parse_info.operator_kinds(),
                        parse_info: augmented_parse_info.clone(),
                        is_operator: true,
                        is_keyword: false,
                        raw_kind: RawTokenKind::Punct,
                    },
                )?;
            }
        }

        for declaration in plain {
            let spelling = token_spelling(&declaration.pattern)?;
            let name = token_name(&declaration.pattern, declaration.style)?;
            let is_keyword = matches!(declaration.style, NameStyle::Keyword);
            let raw_kind = raw_token_kind(&spelling, &declaration.pattern, is_keyword);
            insert_definition(
                &mut definitions,
                &mut names,
                TokenDefinition {
                    name: ident(&name, declaration.pattern.span())?,
                    pattern: declaration.pattern,
                    spelling,
                    operators: BTreeSet::new(),
                    parse_info: ParseInfo::default(),
                    is_operator: false,
                    is_keyword,
                    raw_kind,
                },
            )?;
        }

        Ok(Self {
            precedences,
            definitions,
        })
    }
}

fn expand_delimiters() -> TokenStream {
    let definitions = [
        (
            format_ident!("_EmptyDelimiter"),
            quote!(DelimiterKind::Empty),
        ),
        (
            format_ident!("_SemicolonDelimiter"),
            quote!(DelimiterKind::Semicolon),
        ),
        (
            format_ident!("_ParenthesisDelimiter"),
            quote!(DelimiterKind::Parenthesis),
        ),
        (
            format_ident!("_BracketDelimiter"),
            quote!(DelimiterKind::Bracket),
        ),
        (
            format_ident!("_BraceDelimiter"),
            quote!(DelimiterKind::Brace),
        ),
        (
            format_ident!("_AngleBarDelimiter"),
            quote!(DelimiterKind::AngleBar),
        ),
    ]
    .into_iter()
    .map(|(name, kind)| {
        quote! {
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Spanned)]
            pub struct #name {
                span: Span,
            }

            #[doc(hidden)]
            #[allow(non_snake_case)]
            pub fn #name<S: Spanned>(span: S) -> #name {
                <#name as DelimiterToken>::new(span.span())
            }

            impl DelimiterToken for #name {
                const KIND: DelimiterKind = #kind;

                fn new(span: Span) -> Self {
                    Self { span }
                }
            }

            impl Parse for #name {
                fn parse(
                    input: &mut ParseStream,
                ) -> Result<Self, TokenParseError> {
                    parse_delimiter(input)
                }
            }

            impl ToTokens for #name {
                fn to_tokens(&self, output: &mut TokenStream) {
                    self.surround(output, TokenStream::new());
                }
            }

            impl PrettyTree for #name {
                fn pretty_tree(&self) -> PrettyNode {
                    PrettyNode::delimiter(Self::KIND, self.span())
                }
            }
        }
    });

    quote! {
        #[doc(hidden)]
        mod __m2_syn_delimiters {
            use ::m2_syn::{
                DelimiterKind, DelimiterToken, Parse, ParseStream, PrettyNode, PrettyTree, Span,
                Spanned, ToTokens, TokenParseError, TokenStream, parse_delimiter,
            };

            #(#definitions)*
        }

        #[doc(hidden)]
        pub use __m2_syn_delimiters::*;

        #[macro_export]
        macro_rules! Delimiter {
            [] => { $crate::_EmptyDelimiter };
            [;] => { $crate::_SemicolonDelimiter };
            [()] => { $crate::_ParenthesisDelimiter };
            [[]] => { $crate::_BracketDelimiter };
            [{}] => { $crate::_BraceDelimiter };
            [<| |>] => { $crate::_AngleBarDelimiter };
            [$($unsupported:tt)*] => {
                compile_error!("delimiter was not declared")
            };
        }
    }
}

fn name_span(name: &str, definitions: &[TokenDefinition]) -> Span {
    definitions
        .iter()
        .find(|definition| definition.name == name)
        .map_or_else(Span::call_site, |definition| definition.name.span())
}

fn hidden_token_name(name: &Ident) -> Ident {
    format_ident!("_{}", name, span = name.span())
}

impl SynParse for TokenDefinitions {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let precedences = parse_precedence_stage(input)?;
        let augmented_parse_info = parse_augmented_stage(input)?;
        let operators = parse_operator_stage(input)?;
        let keywords = parse_plain_stage(input, "keywords", NameStyle::Keyword)?;
        let markers = parse_plain_stage(input, "markers", NameStyle::Marker)?;
        let punctuation = parse_plain_stage(input, "punct", NameStyle::Punctuation)?;

        Self::new(
            precedences,
            augmented_parse_info,
            operators,
            keywords.into_iter().chain(markers).chain(punctuation),
        )
    }
}

fn parse_precedence_stage(input: ParseStream<'_>) -> Result<Vec<(Ident, Expr)>> {
    parse_stage_name(input, "precedence", true)?;
    let content;
    braced!(content in input);
    let mut values = Vec::new();
    while !content.is_empty() {
        let name: Ident = content.parse()?;
        content.parse::<Token![=]>()?;
        let value: Expr = content.parse()?;
        if !name.to_string().starts_with("PREC_") {
            return Err(Error::new(
                name.span(),
                "precedence names must start with `PREC_`",
            ));
        }
        if values.iter().any(|(existing, _)| existing == &name) {
            return Err(Error::new(name.span(), "duplicate precedence name"));
        }
        values.push((name, value));
        if content.peek(Token![,]) {
            content.parse::<Token![,]>()?;
        } else if !content.is_empty() {
            return Err(content.error("expected `,` between precedence declarations"));
        }
    }
    Ok(values)
}

fn parse_augmented_stage(input: ParseStream<'_>) -> Result<ParseInfo> {
    parse_stage_name(input, "augmented", true)?;
    let content;
    parenthesized!(content in input);
    let precedence: syn::LitInt = content.parse()?;
    content.parse::<Token![,]>()?;
    let strength: syn::LitInt = content.parse()?;
    if !content.is_empty() {
        return Err(content.error("expected exactly `(precedence, binary_strength)`"));
    }
    Ok(ParseInfo {
        binary: Some((precedence.base10_parse()?, strength.base10_parse()?)),
        ..ParseInfo::default()
    })
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
        if existing.parse_info.signature().is_empty() {
            existing.parse_info = definition.parse_info;
        } else if !definition.parse_info.signature().is_empty()
            && existing.parse_info.signature() != definition.parse_info.signature()
        {
            return Err(Error::new(
                definition.name.span(),
                format!(
                    "token `{}` has conflicting parser metadata",
                    existing.spelling
                ),
            ));
        }
        existing.is_operator |= definition.is_operator;
        existing.is_keyword |= definition.is_keyword;
        if existing.raw_kind != definition.raw_kind {
            return Err(Error::new(
                definition.name.span(),
                format!("token name `{name}` has incompatible raw token categories"),
            ));
        }
    } else {
        names.insert(name, definitions.len());
        definitions.push(definition);
    }
    Ok(())
}

fn raw_token_kind(spelling: &str, _pattern: &TokenStream, is_keyword: bool) -> RawTokenKind {
    if spelling == "SPACE" {
        RawTokenKind::Whitespace
    } else if is_keyword
        || spelling
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic())
    {
        RawTokenKind::Ident
    } else {
        RawTokenKind::Punct
    }
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
        '`' => Ok("Btk"),
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
            precedence: {}
            augmented: (14, 13)
            tokens {
                [+] { pref, bin, aug } (50, 50, 50)
                [!] { post } (72, _, _)
                [not] { pref } (34, _, 34)
                [(*)] { post } (64, _, _)
                ["\\"] { bin, aug } (58, 57, _)
                [SPACE] { bin } (62, 61, _)
            }
            keywords: { [if] [symbol] [threadLocal] [threadVariable] }
            markers: { ["``"] }
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
        assert_eq!(names["BtkBtk"], "``");
        assert_eq!(names["AddEql"], "+=");
        assert_eq!(names["BslEql"], "\\=");
        assert_eq!(names["Graded"], "(*)");
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
                "Add".into(),
                "AddEql".into(),
                "Bsl".into(),
                "BslEql".into(),
                "Space".into(),
            ])
        );
        assert_eq!(postfix, BTreeSet::from(["Bng".into(), "Graded".into()]));
    }

    #[test]
    fn lexical_operator_sets_distinguish_postfix_operators() {
        let definitions = declarations();
        let operators = definitions
            .iter()
            .filter(|definition| definition.is_operator)
            .map(|definition| definition.spelling.as_str())
            .collect::<BTreeSet<_>>();
        let postfix = definitions
            .operator_variants(OperatorKind::Postfix)
            .map(|definition| definition.spelling.as_str())
            .collect::<BTreeSet<_>>();

        assert!(operators.contains("+"));
        assert!(operators.contains("+="));
        assert!(operators.contains("!"));
        assert!(operators.contains("not"));
        assert!(!operators.contains("if"));
        assert!(!operators.contains(";"));
        assert_eq!(postfix, BTreeSet::from(["!", "(*)"]));
    }

    #[test]
    fn keyword_stage_remains_distinct_from_word_operators() {
        let definitions = declarations();
        let keywords = definitions
            .iter()
            .filter(|definition| definition.is_keyword)
            .map(|definition| definition.spelling.as_str())
            .collect::<BTreeSet<_>>();

        assert!(keywords.contains("if"));
        assert!(keywords.contains("symbol"));
        assert!(!keywords.contains("not"));
        assert!(!keywords.contains("SPACE"));
    }

    #[test]
    fn precedence_triple_rejects_non_numeric_slots() {
        let error = parse2::<TokenDefinitions>(quote! {
            precedence: {}
            augmented: (14, 13)
            tokens { [+] { bin } (PREC_ADDITION, 50, _) }
            keywords: {}
            markers: {}
            punct: {}
        })
        .err()
        .expect("a named precedence value must be rejected");

        assert!(error.to_string().contains("expected integer literal"));
    }

    #[test]
    fn operator_and_postfix_actions_cannot_combine() {
        let error = parse2::<TokenDefinitions>(quote! {
            precedence: {}
            augmented: (14, 13)
            tokens { [+] { bin, post } (50, 50, _) }
            keywords: {}
            markers: {}
            punct: {}
        })
        .err()
        .expect("combining an operator and a postfix action must be rejected");

        assert!(error.to_string().contains("cannot combine"));
    }
}
