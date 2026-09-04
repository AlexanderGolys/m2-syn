use std::iter;

use proc_macro2::{Delimiter, Ident, Punct, Spacing, Span, TokenStream, TokenTree};
use quote::{format_ident, quote};
use syn::parse::{Parse as SynParse, ParseStream};
use syn::spanned::Spanned;
use syn::{Error, LitStr, Result, braced, bracketed, parse_str, token};

use crate::roles::{ParseFlags, ParseInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum OperatorKind {
    Prefix,
    Binary,
    Postfix,
    Augmented,
}

impl OperatorKind {
    pub(crate) const ALL: [Self; 4] = [Self::Prefix, Self::Binary, Self::Postfix, Self::Augmented];

    pub(crate) const fn category_name(self) -> &'static str {
        match self {
            Self::Prefix => "PrefixOperator",
            Self::Binary => "BinaryOperator",
            Self::Postfix => "PostfixOperator",
            Self::Augmented => "AugmentedOperator",
        }
    }
}

#[derive(Clone)]
struct TokenDeclaration {
    pattern: TokenStream,
    spelling: String,
    parse: Option<ParseInfo>,
    /// Whether the token also has an augmented-assignment form, `+` giving
    /// `+=`. The form itself is synthesized in [`TokenDefinitions::new`],
    /// where `=`'s own precedence is known.
    augmented: bool,
    trivia: bool,
}

impl TokenDeclaration {
    fn parse(input: ParseStream<'_>) -> Result<Vec<Self>> {
        let mut patterns = Vec::new();
        while input.peek(token::Bracket) {
            let content;
            bracketed!(content in input);
            patterns.push(content.parse()?);
        }
        if patterns.is_empty() {
            return Err(input.error("an operator group cannot be empty"));
        }
        let flags = if input.peek(token::Brace) {
            let content;
            braced!(content in input);
            content.parse()?
        } else {
            ParseFlags::default()
        };
        let parse = flags.resolve(input)?;
        patterns
            .into_iter()
            .map(|pattern| {
                Ok(Self {
                    spelling: token_spelling(&pattern)?,
                    pattern,
                    parse,
                    augmented: flags.augmented,
                    trivia: flags.trivia,
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawTokenKind {
    Punct,
    Ident,
    Whitespace,
}

#[derive(Clone)]
pub(crate) struct TokenDefinition {
    pub(crate) name: Ident,
    pub(crate) pattern: TokenStream,
    pub(crate) spelling: String,
    parse: Option<ParseInfo>,
    raw_kind: RawTokenKind,
    augmented: bool,
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
        self.definitions.iter().filter(move |x| match kind {
            OperatorKind::Augmented => x.augmented,
            _ => x.parse.is_some_and(|rule| rule.is_operator(kind)),
        })
    }

    pub(crate) fn resolve(&self, pattern: &TokenStream) -> Result<Ident> {
        let spelling = token_spelling(pattern)?;
        self.definitions
            .iter()
            .find(|x| x.spelling == spelling)
            .map(|x| x.name.clone())
            .ok_or_else(|| Error::new_spanned(pattern, "token was not declared in a token stage"))
    }

    pub(crate) fn spelling(&self, name: &Ident) -> Option<&str> {
        self.definitions
            .iter()
            .find(|x| x.name == *name)
            .map(|x| x.spelling.as_str())
    }

    pub(crate) fn pattern(&self, name: &Ident) -> Option<&TokenStream> {
        self.definitions
            .iter()
            .find(|x| x.name == *name)
            .map(|x| &x.pattern)
    }

    pub(crate) fn variant_name(&self, name: &Ident) -> Ident {
        let name = name.to_string();
        let friendly = name.strip_prefix('_').unwrap_or(&name);
        Ident::new(friendly, name_span(name.as_str(), &self.definitions))
    }

    pub(crate) fn expand(&self) -> TokenStream {
        let delimiters = expand_delimiters();
        let operator_spellings = self
            .definitions
            .iter()
            .filter(|x| x.parse.is_some_and(ParseInfo::is_any_operator))
            .map(|x| x.spelling.as_str());
        let postfix_operator_spellings = self
            .definitions
            .iter()
            .filter(|x| {
                x.parse
                    .is_some_and(|rule| rule.is_operator(OperatorKind::Postfix))
            })
            .map(|x| x.spelling.as_str());
        let augmented_operator_spellings = self
            .definitions
            .iter()
            .filter(|x| x.augmented)
            .map(|x| x.spelling.as_str());
        let keyword_spellings = self
            .definitions
            .iter()
            .filter(|x| {
                x.raw_kind == RawTokenKind::Ident
                    && !x.parse.is_some_and(ParseInfo::is_any_operator)
            })
            .map(|x| x.spelling.as_str());
        let word_token_spellings = self
            .definitions
            .iter()
            .filter(|x| x.raw_kind == RawTokenKind::Ident)
            .map(|x| x.spelling.as_str());
        let punctuation_spellings = self
            .definitions
            .iter()
            .map(|x| x.spelling.as_str())
            .filter(|s| !s.is_empty() && s.chars().all(|c| !c.is_ascii_alphanumeric() && c != ' '));
        let definitions = self.definitions.iter().map(|definition| {
            let name = &definition.name;
            let type_name = name;
            let spelling = &definition.spelling;
            let (raw_new, raw_from_span, raw_matches) = match definition.raw_kind {
                RawTokenKind::Punct => (
                    quote!(::m2_syn::TokenTree::Punct(::m2_syn::Punct::new(#spelling, span))),
                    quote!(::m2_syn::TokenTree::Punct(::m2_syn::Punct::from_span(span))),
                    quote! {
                        match token {
                            ::m2_syn::TokenTree::Punct(token) => token.text() == #spelling,
                            _ => false,
                        }
                    },
                ),
                RawTokenKind::Ident => (
                    quote!(::m2_syn::TokenTree::Ident(::m2_syn::IdentToken::new(#spelling, span))),
                    quote!(::m2_syn::TokenTree::Ident(::m2_syn::IdentToken::from_span(
                        span
                    ))),
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
                    quote!(::m2_syn::TokenTree::Trivia(::m2_syn::Trivia::from_span(
                        ::m2_syn::TriviaKind::Whitespace,
                        span,
                    ))),
                    quote! {
                        match token {
                            ::m2_syn::TokenTree::Trivia(token) => {
                                token.kind() == ::m2_syn::TriviaKind::Whitespace
                                    && !token.contains_line_break()
                            }
                            ::m2_syn::TokenTree::Ident(token) => token.text() == #spelling,
                            _ => false,
                        }
                    },
                ),
            };
            let pretty_impl = quote! {
                impl ::m2_syn::PrettyTree for #type_name {
                    fn pretty_tree(&self) -> ::m2_syn::PrettyNode {
                        ::m2_syn::PrettyNode::token(
                            ::std::format!(
                                "Token![{}]",
                                <Self as ::m2_syn::Token>::SPELLING,
                            ),
                            <Self as ::m2_syn::Token>::SPELLING,
                            ::m2_syn::Spanned::expect_span(self),
                        )
                    }
                }
            };

            quote! {

                #[derive(Debug, Clone, PartialEq, Eq, ::m2_syn::Spanned)]
                #[doc = concat!("The typed `Token![", #spelling, "]` atom.")]
                pub struct #type_name {
                    raw: qf!(Box<::m2_syn::TokenTree>),
                    trailing_trivia: ::m2_syn::TrailingTrivia,
                }

                #[doc(hidden)]
                #[allow(non_snake_case)]
                pub fn #type_name<S: ::m2_syn::Spanned>(span: S) -> #type_name {
                    let span = span.expect_span();
                    #type_name {
                        raw: <qf!(Box<_>)>::new(#raw_new),
                        trailing_trivia: ::m2_syn::TrailingTrivia::default(),
                    }
                }

                impl ::m2_syn::Token for #type_name {
                    const SPELLING: &'static str = #spelling;

                    fn matches_token_tree(token: &::m2_syn::TokenTree) -> bool {
                        #raw_matches
                    }

                    fn from_token_tree(token: ::m2_syn::TokenTree) -> qf!(Option<Self>) {
                        Self::matches_token_tree(&token).then_some(Self {
                            raw: <qf!(Box<_>)>::new(token),
                            trailing_trivia: ::m2_syn::TrailingTrivia::default(),
                        })
                    }
                }

                impl ::m2_syn::Terminal for #type_name {
                    fn leading_trivia(&self) -> &::m2_syn::LeadingTrivia {
                        self.raw.leading_trivia()
                    }

                    fn leading_trivia_mut(&mut self) -> &mut ::m2_syn::LeadingTrivia {
                        self.raw.leading_trivia_mut()
                    }

                    fn trailing_trivia(&self) -> &::m2_syn::TrailingTrivia {
                        &self.trailing_trivia
                    }

                    fn trailing_trivia_mut(&mut self) -> &mut ::m2_syn::TrailingTrivia {
                        &mut self.trailing_trivia
                    }
                }

                impl ::m2_syn::Parse for #type_name {
                    fn parse(
                        input: &mut ::m2_syn::ParseStream,
                    ) -> Result<Self, ::m2_syn::TokenParseError> {
                        input.parse_token()
                    }
                }

                impl ::m2_syn::Lex for #type_name {
                    fn to_tokens(&self, output: &mut ::m2_syn::TokenStream) {
                        ::m2_syn::Lex::to_tokens(&self.raw, output);
                        ::m2_syn::Lex::to_tokens(&self.trailing_trivia, output);
                    }
                }

                #pretty_impl

                impl<N> ::m2_syn::Migrate<N> for #type_name
                where
                    N: ::m2_syn::ExternalCstNode,
                {
                    fn matches(node: &N) -> bool {
                        node.identity().matches(#spelling, false)
                    }

                    fn migrate(node: N) -> Result<Self, ::m2_syn::MigrateError> {
                        if !<Self as ::m2_syn::Migrate<N>>::matches(&node) {
                            return Err(::m2_syn::MigrateError::wrong_node(
                                #spelling,
                                false,
                                node.identity(),
                            ));
                        }
                        let span = node.expect_span();
                        Ok(#type_name {
                            raw: <qf!(Box<_>)>::new(#raw_from_span),
                            trailing_trivia: ::m2_syn::TrailingTrivia::default(),
                        })
                    }
                }
            }
        });
        // Emitted sorted, so the lookup is a binary search over data rather
        // than a generated function body.
        let mut rules = self
            .definitions
            .iter()
            .filter_map(|definition| Some((definition.spelling.as_str(), definition.parse?)))
            .collect::<Vec<_>>();
        rules.sort_by_key(|(spelling, _)| *spelling);
        let rule_entries = rules.into_iter().map(|(spelling, rule)| {
            let rule = rule.expand();
            quote! { (#spelling, #rule) }
        });
        quote! {
            #delimiters

            pub(crate) const GENERATED_OPERATOR_SPELLINGS: &[&str] = &[
                #(#operator_spellings),*
            ];

            pub(crate) const GENERATED_POSTFIX_OPERATOR_SPELLINGS: &[&str] = &[
                #(#postfix_operator_spellings),*
            ];

            pub(crate) const GENERATED_AUGMENTED_OPERATOR_SPELLINGS: &[&str] = &[
                #(#augmented_operator_spellings),*
            ];

            pub(crate) const GENERATED_KEYWORD_SPELLINGS: &[&str] = &[
                #(#keyword_spellings),*
            ];

            pub(crate) const GENERATED_WORD_TOKEN_SPELLINGS: &[&str] = &[
                #(#word_token_spellings),*
            ];

            pub(crate) const GENERATED_PUNCTUATION_SPELLINGS: &[&str] = &[
                #(#punctuation_spellings),*
            ];

            #(#definitions)*

            /// What a declared token does when the parser reaches it: its
            /// parsing precedence, plus the binding strengths its role needs.
            #[derive(Clone, Copy, Debug, PartialEq, Eq)]
            pub enum TokenParseRule {
                /// `(precedence)` — takes part only through its precedence.
                Precedence(u8),
                /// `(precedence)`
                Postfix(u8),
                /// `(precedence, binary_strength)`
                Binary(u8, u8),
                /// `(precedence, operand_strength)`
                Prefix(u8, u8),
                /// `(precedence, binary_strength, unary_strength)`
                PrefixBinary(u8, u8, u8),
                /// `(precedence, operand_strength)` for a non-operator prefix.
                Unary(u8, u8),
            }

            /// Every declared token that carries a parse rule, sorted by
            /// spelling. Tokens the declaration gives no role are absent.
            pub const TOKEN_PARSE_RULES: &[(&str, TokenParseRule)] = &[
                #(#rule_entries),*
            ];
        }
    }

    fn new(tokens: Vec<TokenDeclaration>) -> Result<Self> {
        let mut definitions = Vec::<TokenDefinition>::new();
        // `x += y` binds exactly like the assignment it abbreviates, so every
        // augmented form takes its rule from the `=` declaration.
        let augmented_parse = tokens
            .iter()
            .find(|token| token.spelling == "=")
            .and_then(|token| token.parse)
            .and_then(ParseInfo::augmented_form);

        for token in tokens {
            let name = token_name(&token.pattern)?;
            let raw_kind = if token.trivia {
                RawTokenKind::Whitespace
            } else {
                raw_token_kind(&token.spelling)
            };
            insert_definition(
                &mut definitions,
                TokenDefinition {
                    name: ident(&name, token.pattern.span())?,
                    pattern: token.pattern.clone(),
                    spelling: token.spelling.clone(),
                    parse: token.parse,
                    raw_kind,
                    augmented: false,
                },
            )?;

            if token.augmented {
                let Some(augmented) = augmented_parse else {
                    return Err(Error::new(
                        token.pattern.span(),
                        "`aug` needs an `[=]` operator declaration to take its rule from",
                    ));
                };
                let pattern = augmented_pattern(&token.pattern)?;
                let name = format!("{name}Eql");
                insert_definition(
                    &mut definitions,
                    TokenDefinition {
                        name: ident(&name, pattern.span())?,
                        pattern,
                        spelling: format!("{}=", token.spelling),
                        parse: Some(augmented),
                        raw_kind: RawTokenKind::Punct,
                        augmented: true,
                    },
                )?;
            }
        }

        Ok(Self { definitions })
    }
}

fn expand_delimiters() -> TokenStream {
    let definitions = [
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
        let pretty_impl = quote! {
            impl PrettyTree for #name {
                fn pretty_tree(&self) -> PrettyNode {
                    PrettyNode::delimiter(Self::KIND, self.spans.clone())
                }
            }
        };
        quote! {
            #[derive(Debug, Clone, PartialEq, Eq, Spanned)]
            #[doc = concat!("The typed `", stringify!(#name), "` delimiter atom.")]
            pub struct #name {
                spans: DoubleSpan,
                opening_trivia: TrailingTrivia,
                closing_trivia: TrailingTrivia,
            }

            #[doc(hidden)]
            #[allow(non_snake_case)]
            pub fn #name<S: Spanned>(span: S) -> #name {
                let span = span.expect_span();
                <#name as DelimiterToken>::new(DoubleSpan::new(span.clone(), span))
            }

            impl DelimiterToken for #name {
                const KIND: DelimiterKind = #kind;

                fn new(spans: DoubleSpan) -> Self {
                    Self {
                        spans,
                        opening_trivia: TrailingTrivia::default(),
                        closing_trivia: TrailingTrivia::default(),
                    }
                }

                fn spans(&self) -> DoubleSpan {
                    self.spans.clone()
                }
            }

            impl DelimiterValue for #name {
                fn from_delimiter(delimiter: Delimiter) -> Option<Self> {
                    (delimiter.kind == Self::KIND).then_some(Self {
                        spans: delimiter.spans.clone(),
                        opening_trivia: delimiter.opening_trivia().clone(),
                        closing_trivia: delimiter.closing_trivia().clone(),
                    })
                }

                fn delimiter(&self) -> Delimiter {
                    let mut delimiter = Delimiter::new(Self::KIND, self.spans.clone());
                    *delimiter.opening_trivia_mut() = self.opening_trivia.clone();
                    *delimiter.closing_trivia_mut() = self.closing_trivia.clone();
                    delimiter
                }

                fn expected() -> &'static str {
                    Self::KIND.opening()
                }

                fn opening_trivia(&self) -> &TrailingTrivia {
                    &self.opening_trivia
                }

                fn opening_trivia_mut(&mut self) -> &mut TrailingTrivia {
                    &mut self.opening_trivia
                }

                fn closing_trivia(&self) -> &TrailingTrivia {
                    &self.closing_trivia
                }

                fn closing_trivia_mut(&mut self) -> &mut TrailingTrivia {
                    &mut self.closing_trivia
                }
            }

            impl Parse for #name {
                fn parse(
                    input: &mut ParseStream,
                ) -> Result<Self, TokenParseError> {
                    parse_delimiter(input)
                }
            }

            impl Lex for #name {
                fn to_tokens(&self, output: &mut TokenStream) {
                    self.surround(output, TokenStream::new());
                }
            }

            #pretty_impl
        }
    });

    quote! {
        #[doc(hidden)]
        mod __m2_syn_delimiters {
            use ::m2_syn::{
                Delimiter, DelimiterKind, DelimiterToken, DelimiterValue, DoubleSpan, Parse,
                ParseStream, Span, Spanned, Lex, TokenParseError, TokenStream, TrailingTrivia,
                parse_delimiter,
            };
            use ::m2_syn::{PrettyNode, PrettyTree};

            #(#definitions)*
        }

        #[doc(hidden)]
        pub use __m2_syn_delimiters::*;

    }
}

fn name_span(name: &str, definitions: &[TokenDefinition]) -> Span {
    definitions
        .iter()
        .find(|definition| definition.name == name)
        .map_or_else(Span::call_site, |definition| definition.name.span())
}

impl SynParse for TokenDefinitions {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let tokens = parse_token_stage(input)?;
        Self::new(tokens)
    }
}

fn parse_token_stage(input: ParseStream<'_>) -> Result<Vec<TokenDeclaration>> {
    parse_stage_name(input, "tokens")?;
    let content;
    braced!(content in input);
    let mut declarations = Vec::new();
    while !content.is_empty() {
        declarations.extend(TokenDeclaration::parse(&content)?);
    }
    Ok(declarations)
}

fn parse_stage_name(input: ParseStream<'_>, expected: &str) -> Result<()> {
    let name: Ident = input.parse()?;
    if name != expected {
        return Err(Error::new(
            name.span(),
            format!("expected `{expected}` token stage"),
        ));
    }
    Ok(())
}

fn insert_definition(
    definitions: &mut Vec<TokenDefinition>,
    definition: TokenDefinition,
) -> Result<()> {
    let name = definition.name.to_string();
    if let Some(index) = definitions
        .iter()
        .position(|existing| existing.name == definition.name)
    {
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
        match (existing.parse, definition.parse) {
            (Some(existing_rule), Some(rule)) if existing_rule != rule => {
                return Err(Error::new(
                    definition.name.span(),
                    format!(
                        "token `{}` has conflicting parser metadata",
                        existing.spelling
                    ),
                ));
            }
            (None, rule) => existing.parse = rule,
            _ => {}
        }
        if existing.raw_kind != definition.raw_kind {
            return Err(Error::new(
                definition.name.span(),
                format!("token name `{name}` has incompatible raw token categories"),
            ));
        }
    } else {
        definitions.push(definition);
    }
    Ok(())
}

fn raw_token_kind(spelling: &str) -> RawTokenKind {
    if spelling
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
    {
        RawTokenKind::Ident
    } else {
        RawTokenKind::Punct
    }
}

fn token_name(pattern: &TokenStream) -> Result<String> {
    let spelling = token_spelling(pattern)?;
    let base = if spelling.is_empty() {
        "Adj".to_owned()
    } else if spelling == "(*)" {
        "Graded".to_owned()
    } else if spelling
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic())
        && spelling
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        word_name(&spelling)
    } else {
        spelling.chars().map(punctuation_name).collect()
    };

    Ok(format!("_{base}"))
}

/// The type a `Token![…]` invocation resolves to, derived from its spelling.
pub(crate) fn token_type_name(pattern: &TokenStream) -> Result<Ident> {
    ident(&token_name(pattern)?, Span::call_site())
}

fn grouped_token_spelling(group: &proc_macro2::Group) -> Result<&'static str> {
    if group.delimiter() != Delimiter::Parenthesis {
        return Err(Error::new(group.span(), "groups are not valid tokens"));
    }
    let mut inner = group.stream().into_iter();
    match (inner.next(), inner.next()) {
        (Some(TokenTree::Punct(punctuation)), None) if punctuation.as_char() == '*' => Ok("(*)"),
        _ => Err(Error::new(
            group.span(),
            "only `(*)` is supported as a grouped token",
        )),
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
                spelling.push_str(grouped_token_spelling(&group)?);
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

fn punctuation_name(character: char) -> String {
    match character {
        '.' => "Dot".to_owned(),
        ',' => "Cma".to_owned(),
        ';' => "Scl".to_owned(),
        ':' => "Col".to_owned(),
        '#' => "Hsh".to_owned(),
        '@' => "Att".to_owned(),
        '$' => "Dlr".to_owned(),
        '%' => "Mod".to_owned(),
        '^' => "Crt".to_owned(),
        '&' => "Amp".to_owned(),
        '*' => "Mul".to_owned(),
        '+' => "Add".to_owned(),
        '-' => "Sub".to_owned(),
        '=' => "Eql".to_owned(),
        '<' => "Lst".to_owned(),
        '>' => "Gst".to_owned(),
        '!' => "Bng".to_owned(),
        '?' => "Qsm".to_owned(),
        '~' => "Tld".to_owned(),
        '|' => "Pip".to_owned(),
        '/' => "Slh".to_owned(),
        '_' => "Ubs".to_owned(),
        '\\' => "Bsl".to_owned(),
        '`' => "Btk".to_owned(),
        '·' => "Cdt".to_owned(),
        '⊠' => "Box".to_owned(),
        '⧢' => "Sfp".to_owned(),
        _ => format!("U{:06X}", character as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;
    use std::collections::{BTreeMap, BTreeSet};
    use syn::parse2;

    fn declarations() -> TokenDefinitions {
        parse2(quote! {
            tokens {
                [=] { infix } (14, R)
                [+] { pref, bin, aug } (50, L)
                [!] { post } (72)
                [not] { pref } (34)
                [(*)] { post } (64)
                ["\\"] { bin, aug } (58, R)
                [SPACE] { bin, trivia } (62, R)
                [if] (62, 12)
                [symbol] [threadLocal] [threadVariable] (62, 74)
                [;] [,] ["``"]
            }
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

        assert_eq!(names["_Add"], "+");
        assert_eq!(names["_BtkBtk"], "``");
        assert_eq!(names["_AddEql"], "+=");
        assert_eq!(names["_BslEql"], "\\=");
        assert_eq!(names["_Graded"], "(*)");
        assert_eq!(names["_Space"], "SPACE");
        assert_eq!(names["_If"], "if");
        assert_eq!(names["_ThreadLocal"], "threadLocal");
        assert_eq!(names["_ThreadVariable"], "threadVariable");
        assert_eq!(names["_Scl"], ";");
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
        let augmented = definitions
            .operator_variants(OperatorKind::Augmented)
            .map(|definition| definition.name.to_string())
            .collect::<BTreeSet<_>>();

        assert_eq!(prefix, BTreeSet::from(["_Add".into(), "_Not".into()]));
        assert_eq!(
            binary,
            BTreeSet::from([
                "_Add".into(),
                "_AddEql".into(),
                "_Bsl".into(),
                "_BslEql".into(),
                "_Space".into(),
            ])
        );
        assert_eq!(postfix, BTreeSet::from(["_Bng".into(), "_Graded".into()]));
        assert_eq!(
            augmented,
            BTreeSet::from(["_AddEql".into(), "_BslEql".into()])
        );
    }

    #[test]
    fn lexical_operator_sets_distinguish_postfix_operators() {
        let definitions = declarations();
        let operators = definitions
            .iter()
            .filter(|definition| definition.parse.is_some_and(ParseInfo::is_any_operator))
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
    fn token_type_names_match_the_generated_definitions() {
        let name = |spelling: &str| {
            token_type_name(&spelling.parse().expect("valid token spelling"))
                .expect("nameable spelling")
                .to_string()
        };

        // Punctuation composes one fragment per character.
        assert_eq!(name("!="), "_BngEql");
        assert_eq!(name("#?"), "_HshQsm");
        assert_eq!(name(";"), "_Scl");
        assert_eq!(name(","), "_Cma");
        assert_eq!(name("and"), "_And");
        assert_eq!(name("not"), "_Not");
        assert_eq!(name("SPACE"), "_Space");
        assert_eq!(name("if"), "_If");
        assert_eq!(name("symbol"), "_Symbol");
        assert_eq!(name("list"), "_List");
    }

    #[test]
    fn token_type_names_normalize_mixed_literal_and_rust_fragments() {
        let names = [
            quote!(!=),
            quote!(! =),
            quote!("!="),
            quote!("!" =),
            quote!(!"="),
            quote!("!" "="),
        ]
        .map(|pattern| token_type_name(&pattern).unwrap().to_string());

        assert!(names.iter().all(|name| name == "_BngEql"));
        assert_eq!(token_type_name(&quote!("n" ot)).unwrap(), "_Not");
    }

    #[test]
    fn string_fragments_name_characters_rust_cannot_tokenize() {
        let definitions = parse2::<TokenDefinitions>(quote! {
            tokens { ["→"] }
        })
        .unwrap();

        assert_eq!(definitions.iter().next().unwrap().name, "_U002192");
        assert_eq!(token_type_name(&quote!("→")).unwrap(), "_U002192");
    }

    #[test]
    fn word_tokens_are_operators_only_when_tagged() {
        let definitions = declarations();
        let keywords = definitions
            .iter()
            .filter(|definition| {
                definition.raw_kind == RawTokenKind::Ident
                    && !definition.parse.is_some_and(ParseInfo::is_any_operator)
            })
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
            tokens { [+] { bin } (PREC_ADDITION, R) }
        })
        .err()
        .expect("a named precedence value must be rejected");

        assert!(
            error
                .to_string()
                .contains("precedence slots must be integer literals")
        );
    }

    #[test]
    fn operator_and_postfix_actions_cannot_combine() {
        let error = parse2::<TokenDefinitions>(quote! {
            tokens { [+] { bin, post } (50, L) }
        })
        .err()
        .expect("combining an operator and a postfix action must be rejected");

        assert!(error.to_string().contains("one parser role"));
    }
}
