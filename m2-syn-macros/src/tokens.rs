use std::iter;

use proc_macro2::{Delimiter, Ident, Punct, Spacing, Span, TokenStream, TokenTree};
use quote::{format_ident, quote};
use syn::parse::{Parse as SynParse, ParseStream};
use syn::spanned::Spanned;
use syn::{Error, LitStr, Result, Token, braced, bracketed, parse_str, token};

use crate::roles::{ParseFlags, ParseInfo};
use crate::syntax::Delimiter as DelimiterFamily;

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
    implicit: bool,
    marker: bool,
    /// The spelling this one is another spelling of: `->` for `["→"] = [->]`.
    synonym_of: Option<String>,
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
        if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            let content;
            bracketed!(content in input);
            let target = token_spelling(&content.parse()?)?;
            return patterns
                .into_iter()
                .map(|pattern| {
                    Ok(Self {
                        spelling: token_spelling(&pattern)?,
                        pattern,
                        parse: None,
                        augmented: false,
                        implicit: false,
                        marker: false,
                        synonym_of: Some(target.clone()),
                    })
                })
                .collect();
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
                    implicit: flags.implicit,
                    marker: flags.marker,
                    synonym_of: None,
                })
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawTokenKind {
    Punct,
    Ident,
    /// A word that adjacency may also imply, spelled empty: `SPACE` in
    /// `f SPACE x`, and nothing at all in `f x`.
    Implicit,
    /// A position that is never written: the end of a cell, `EOC`.
    Marker,
}

#[derive(Clone)]
pub(crate) struct TokenDefinition {
    pub(crate) name: Ident,
    pub(crate) pattern: TokenStream,
    pub(crate) spelling: String,
    /// Other spellings of the same token, as M2 has `→` for `->`.
    synonyms: Vec<String>,
    parse: Option<ParseInfo>,
    raw_kind: RawTokenKind,
    augmented: bool,
}

impl TokenDefinition {
    /// Every spelling of the token, its own first.
    fn spellings(&self) -> impl Iterator<Item = &str> + Clone {
        iter::once(self.spelling.as_str()).chain(self.synonyms.iter().map(String::as_str))
    }
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
            .flat_map(TokenDefinition::spellings);
        let postfix_operator_spellings = self
            .definitions
            .iter()
            .filter(|x| {
                x.parse
                    .is_some_and(|rule| rule.is_operator(OperatorKind::Postfix))
            })
            .flat_map(TokenDefinition::spellings);
        let augmented_operator_spellings = self
            .definitions
            .iter()
            .filter(|x| x.augmented)
            .flat_map(TokenDefinition::spellings);
        let keyword_spellings = self
            .definitions
            .iter()
            .filter(|x| {
                x.raw_kind == RawTokenKind::Ident
                    && !x.parse.is_some_and(ParseInfo::is_any_operator)
            })
            .flat_map(TokenDefinition::spellings)
            .filter(|spelling| raw_token_kind(spelling) == RawTokenKind::Ident);
        let word_token_spellings = self
            .definitions
            .iter()
            .filter(|x| matches!(x.raw_kind, RawTokenKind::Ident | RawTokenKind::Implicit));
        let continuation_spellings = word_token_spellings
            .clone()
            .filter(|x| x.parse.is_some_and(|parse| !parse.acts()))
            .flat_map(TokenDefinition::spellings);
        let word_token_spellings = word_token_spellings
            .flat_map(TokenDefinition::spellings)
            .filter(|spelling| raw_token_kind(spelling) == RawTokenKind::Ident);
        let synonyms = self.definitions.iter().flat_map(|x| {
            let spelling = &x.spelling;
            x.synonyms
                .iter()
                .map(move |synonym| quote!((#synonym, #spelling)))
        });
        let punctuation_spellings = self
            .definitions
            .iter()
            .flat_map(TokenDefinition::spellings)
            .filter(|s| !s.is_empty() && s.chars().all(|c| !c.is_ascii_alphanumeric() && c != ' '));
        let definitions = self.definitions.iter().map(|definition| {
            let name = &definition.name;
            let type_name = name;
            let spelling = &definition.spelling;
            let spellings = definition.spellings();
            let punct_spellings = spellings
                .clone()
                .filter(|spelling| raw_token_kind(spelling) == RawTokenKind::Punct);
            let word_spellings = spellings
                .clone()
                .filter(|spelling| raw_token_kind(spelling) == RawTokenKind::Ident);
            // Any spelling, a word also behind `Core$`.
            let spelled_matches = quote! {
                match token {
                    ::m2_syn::TokenTree::Punct(token) => {
                        [#(#punct_spellings),*].contains(&token.text())
                    }
                    ::m2_syn::TokenTree::Ident(token) => {
                        let text = token.text();
                        [#(#word_spellings),*].contains(&text.strip_prefix("Core$").unwrap_or(text))
                    }
                    _ => false,
                }
            };
            let (raw_new, raw_from_span, raw_matches) = match definition.raw_kind {
                RawTokenKind::Punct => (
                    quote!(::m2_syn::TokenTree::Punct(::m2_syn::Punct::new(#spelling, span))),
                    quote!(::m2_syn::TokenTree::Punct(::m2_syn::Punct::from_span(span))),
                    spelled_matches,
                ),
                RawTokenKind::Ident => (
                    quote!(::m2_syn::TokenTree::Ident(::m2_syn::IdentToken::new(#spelling, span))),
                    quote!(::m2_syn::TokenTree::Ident(::m2_syn::IdentToken::from_span(
                        span
                    ))),
                    spelled_matches,
                ),
                RawTokenKind::Marker => (
                    quote!(::m2_syn::TokenTree::Punct(::m2_syn::Punct::from_span(span))),
                    quote!(::m2_syn::TokenTree::Punct(::m2_syn::Punct::from_span(span))),
                    quote! {
                        let _ = token;
                        false
                    },
                ),
                RawTokenKind::Implicit => (
                    quote!(::m2_syn::TokenTree::Punct(::m2_syn::Punct::new("", span))),
                    quote! {
                        if span.text().is_some_and(str::is_empty) {
                            ::m2_syn::TokenTree::Punct(::m2_syn::Punct::from_span(span))
                        } else {
                            ::m2_syn::TokenTree::Ident(::m2_syn::IdentToken::from_span(span))
                        }
                    },
                    quote! {
                        match token {
                            ::m2_syn::TokenTree::Punct(token) => token.text().is_empty(),
                            ::m2_syn::TokenTree::Ident(token) => {
                                token.text() == #spelling
                                    || token.text().strip_prefix("Core$") == Some(#spelling)
                            }
                            _ => false,
                        }
                    },
                ),
            };
            let lex_body = if definition.raw_kind == RawTokenKind::Marker {
                quote!(let _ = output;)
            } else {
                quote!(::m2_syn::Lex::to_tokens(&self.raw, output);)
            };
            let pretty_impl = quote! {
                impl ::m2_syn::PrettyTree for #type_name {
                    fn pretty_tree(&self) -> ::m2_syn::PrettyNode {
                        ::m2_syn::PrettyNode::token(
                            ::std::format!(
                                "Token![{}]",
                                <Self as ::m2_syn::Token>::SPELLING,
                            ),
                            // As written, with an implied token shown as `∅`.
                            self.raw.spelling().filter(|text| !text.is_empty()).unwrap_or("∅"),
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
                }

                #[doc(hidden)]
                #[allow(non_snake_case)]
                pub fn #type_name<S: ::m2_syn::Spanned>(span: S) -> #type_name {
                    let span = span.expect_span();
                    #type_name {
                        raw: <qf!(Box<_>)>::new(#raw_new),
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

                impl ::m2_syn::Lex for #type_name {
                    fn to_tokens(&self, output: &mut ::m2_syn::TokenStream) {
                        #lex_body
                    }
                }

                #pretty_impl

                impl<N> ::m2_syn::Migrate<N> for #type_name
                where
                    N: ::m2_syn::ExternalCstNode,
                {
                    fn matches(node: &N) -> bool {
                        [#(#spellings),*]
                            .into_iter()
                            .any(|spelling| node.identity().matches(spelling, false))
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

            /// The words `Core$word` also spells. Punctuation has no such
            /// alias: `Core$+` lexes as `Core$` followed by `+`.
            pub(crate) const GENERATED_WORD_TOKEN_SPELLINGS: &[&str] = &[
                #(#word_token_spellings),*
            ];

            /// The words that only continue a construct, which M2 matches by
            /// their exact spelling: `then` in `if x then y`. Their `Core$`
            /// aliases are therefore syntax errors wherever they stand.
            pub(crate) const GENERATED_CONTINUATION_SPELLINGS: &[&str] = &[
                #(#continuation_spellings),*
            ];

            /// Every other spelling of a token, beside the token's own: M2's
            /// `→` for `->` and `threadVariable` for `threadLocal`.
            pub(crate) const GENERATED_SYNONYMS: &[(&str, &str)] = &[
                #(#synonyms),*
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

        let (synonyms, tokens): (Vec<_>, Vec<_>) = tokens
            .into_iter()
            .partition(|token| token.synonym_of.is_some());
        for token in tokens {
            let name = token_name(&token.pattern)?;
            let raw_kind = if token.marker {
                RawTokenKind::Marker
            } else if token.implicit {
                RawTokenKind::Implicit
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
                    synonyms: Vec::new(),
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
                        synonyms: Vec::new(),
                        augmented: true,
                    },
                )?;
            }
        }

        for synonym in synonyms {
            let target = synonym.synonym_of.expect("partitioned on the target");
            let definition = definitions
                .iter_mut()
                .find(|definition| definition.spelling == target)
                .ok_or_else(|| {
                    Error::new(
                        synonym.pattern.span(),
                        format!("`{target}` is not declared before its synonym"),
                    )
                })?;
            definition.synonyms.push(synonym.spelling);
        }

        Ok(Self { definitions })
    }
}

fn expand_delimiters() -> TokenStream {
    let definitions = DelimiterFamily::ALL.into_iter().map(|family| {
        let name = family.type_name();
        let kind = format_ident!("{}", family.name());
        let kind = quote!(DelimiterKind::#kind);
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
                    Self { spans }
                }

                fn spans(&self) -> DoubleSpan {
                    self.spans.clone()
                }
            }

            impl DelimiterValue for #name {
                fn from_delimiter(delimiter: Delimiter) -> Option<Self> {
                    (delimiter.kind == Self::KIND).then_some(Self {
                        spans: delimiter.spans,
                    })
                }

                fn delimiter(&self) -> Delimiter {
                    Delimiter::new(Self::KIND, self.spans.clone())
                }

                fn expected() -> &'static str {
                    Self::KIND.opening()
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
                ParseStream, Span, Spanned, Lex, TokenParseError, TokenStream,
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
    let mut declarations = Vec::new();
    while !input.is_empty() {
        declarations.extend(TokenDeclaration::parse(input)?);
    }
    Ok(declarations)
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
    let base = if spelling == "(*)" {
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
/// A delimiter pair, `Token![()]`, names its delimiter atom, and nothing
/// written, `Token![]` or `Token![" "]`, is adjacency: `f x` is `f SPACE x`.
pub(crate) fn token_type_name(pattern: &TokenStream) -> Result<Ident> {
    let spelling = token_spelling(pattern)?;
    if let Some(family) = DelimiterFamily::spelled(&spelling) {
        return Ok(family.type_name());
    }
    let name = if spelling.is_empty() {
        token_name(&quote!(SPACE))?
    } else {
        token_name(pattern)?
    };
    ident(&name, Span::call_site())
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

/// The M2 spelling Rust tokens and string fragments write together:
/// `"!" =`, `(*)`, and `"< | |>"` spell `!=`, `(*)`, and `<||>`. M2 spellings
/// hold no whitespace, so whitespace anywhere only separates fragments.
pub(crate) fn token_spelling(pattern: &TokenStream) -> Result<String> {
    pattern
        .clone()
        .into_iter()
        .map(|tree| {
            Ok(match tree {
                TokenTree::Group(group) => {
                    let (open, close) = match group.delimiter() {
                        Delimiter::Parenthesis => ("(", ")"),
                        Delimiter::Bracket => ("[", "]"),
                        Delimiter::Brace => ("{", "}"),
                        Delimiter::None => ("", ""),
                    };
                    format!("{open}{}{close}", token_spelling(&group.stream())?)
                }
                TokenTree::Ident(ident) => ident.to_string(),
                TokenTree::Literal(literal) => parse_str::<LitStr>(&literal.to_string())
                    .map_err(|_| {
                        Error::new(literal.span(), "token literals must be string literals")
                    })?
                    .value()
                    .split_whitespace()
                    .collect(),
                TokenTree::Punct(punctuation) => punctuation.as_char().to_string(),
            })
        })
        .collect()
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
                [=] { infix } (14, R)
                [+] { pref, bin, aug } (50, L)
                [!] { post } (72)
                [not] { pref } (34)
                [(*)] { post } (64)
                ["\\"] { bin, aug } (58, R)
                [SPACE] { bin, implicit } (62, R)
                [if] (62, 12)
                [symbol] [threadLocal] [threadVariable] (62, 74)
                [;] [,] ["``"]
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
            ["→"]
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
            [+] { bin } (PREC_ADDITION, R)
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
            [+] { bin, post } (50, L)
        })
        .err()
        .expect("combining an operator and a postfix action must be rejected");

        assert!(error.to_string().contains("one parser role"));
    }
}
