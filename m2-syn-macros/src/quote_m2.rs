use proc_macro::TokenStream;
use proc_macro2::{Delimiter, Ident, Spacing, Span, TokenStream as TokenStream2, TokenTree};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Pat, Result, Token};

struct Repetition {
    pattern: Pat,
    _in: Token![in],
    iterator: Expr,
}

impl Parse for Repetition {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        Ok(Self {
            pattern: input.call(Pat::parse_single)?,
            _in: input.parse()?,
            iterator: input.parse()?,
        })
    }
}

pub fn expand(input: TokenStream) -> TokenStream {
    let output = format_ident!("__m2_output", span = Span::mixed_site());
    let mut group_index = 0;
    match expand_stream(input.into(), &output, &mut group_index) {
        Ok(statements) => quote!({
            let mut #output = ::m2_syn::TokenStream::new();
            #(#statements)*
            #output
        })
        .into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_stream(
    input: TokenStream2,
    output: &Ident,
    group_index: &mut usize,
) -> Result<Vec<TokenStream2>> {
    let trees = input.into_iter().collect::<Vec<_>>();
    let mut statements = Vec::new();
    let mut index = 0;

    while index < trees.len() {
        let statement = match &trees[index] {
            TokenTree::Punct(_) if punct_pair(&trees, index, '<', '|') => {
                let closing = matching_angle_bar(&trees, index)?;
                let contents = trees[index + 2..closing]
                    .iter()
                    .cloned()
                    .collect::<TokenStream2>();
                let group_output =
                    format_ident!("__m2_group_{}", *group_index, span = Span::mixed_site());
                *group_index += 1;
                let nested = expand_stream(contents, &group_output, group_index)?;
                index = closing + 1;
                quote! {
                    let mut #group_output = ::m2_syn::TokenStream::new();
                    #(#nested)*
                    #output.append_fragment(&::m2_syn::Group::new(
                        ::m2_syn::Delimiter::new(
                            ::m2_syn::DelimiterKind::AngleBar,
                            ::m2_syn::Span::detached(),
                        ),
                        #group_output,
                    ));
                }
            }
            TokenTree::Punct(punctuation)
                if punctuation.as_char() == '|' && punct_pair(&trees, index, '|', '>') =>
            {
                return Err(syn::Error::new(
                    punctuation.span(),
                    "unexpected angle-bar closing delimiter",
                ));
            }
            TokenTree::Punct(punctuation)
                if punctuation.as_char() == '$'
                    && matches!(trees.get(index + 1), Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Bracket)
                    && matches!(trees.get(index + 2), Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace) =>
            {
                let Some(TokenTree::Group(header)) = trees.get(index + 1) else {
                    unreachable!()
                };
                let Some(TokenTree::Group(template)) = trees.get(index + 2) else {
                    unreachable!()
                };
                let Repetition {
                    pattern, iterator, ..
                } = syn::parse2(header.stream())?;
                let iteration_output =
                    format_ident!("__m2_iteration_{}", *group_index, span = Span::mixed_site());
                *group_index += 1;
                let nested = expand_stream(template.stream(), &iteration_output, group_index)?;
                index += 2;
                quote! {
                    for #pattern in #iterator {
                        let mut #iteration_output = ::m2_syn::TokenStream::new();
                        #(#nested)*
                        #output.append_fragment(&#iteration_output);
                    }
                }
            }
            TokenTree::Punct(punctuation)
                if punctuation.as_char() == '$'
                    && matches!(trees.get(index + 1), Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis) =>
            {
                let TokenTree::Group(group) = &trees[index + 1] else {
                    unreachable!()
                };
                let expression = syn::parse2::<syn::Expr>(group.stream())?;
                index += 1;
                quote!(#output.append_fragment(&(#expression));)
            }
            TokenTree::Ident(identifier) => {
                let text = identifier.to_string();
                quote! {
                    #output.append_fragment(&::m2_syn::IdentToken::new(
                            #text,
                            ::m2_syn::Span::detached(),
                    ));
                }
            }
            TokenTree::Literal(literal) => {
                let text = literal.to_string();
                let kind = if text.starts_with('"') {
                    quote!(::m2_syn::LiteralKind::String)
                } else if text.contains(['.', 'e', 'E']) {
                    quote!(::m2_syn::LiteralKind::Float)
                } else {
                    quote!(::m2_syn::LiteralKind::Integer)
                };
                quote! {
                    #output.append_fragment(&::m2_syn::Literal::new(
                        #kind,
                        #text.into(),
                        ::m2_syn::Span::detached(),
                    ));
                }
            }
            TokenTree::Punct(punctuation) => {
                let mut text = punctuation.as_char().to_string();
                while matches!(trees[index], TokenTree::Punct(ref punct) if punct.spacing() == Spacing::Joint)
                {
                    let Some(TokenTree::Punct(next)) = trees.get(index + 1) else {
                        break;
                    };
                    text.push(next.as_char());
                    index += 1;
                }
                quote! {
                    #output.append_fragment(&::m2_syn::Punct::new(
                        #text,
                        ::m2_syn::Span::detached(),
                    ));
                }
            }
            TokenTree::Group(group) => {
                let group_output =
                    format_ident!("__m2_group_{}", *group_index, span = Span::mixed_site());
                *group_index += 1;
                let nested = expand_stream(group.stream(), &group_output, group_index)?;
                let delimiter = match group.delimiter() {
                    Delimiter::Parenthesis => quote!(::m2_syn::DelimiterKind::Parenthesis),
                    Delimiter::Bracket => quote!(::m2_syn::DelimiterKind::Bracket),
                    Delimiter::Brace => quote!(::m2_syn::DelimiterKind::Brace),
                    Delimiter::None => {
                        statements.push(quote! {
                            let mut #group_output = ::m2_syn::TokenStream::new();
                            #(#nested)*
                            #output.append_fragment(&#group_output);
                        });
                        index += 1;
                        continue;
                    }
                };
                quote! {
                    let mut #group_output = ::m2_syn::TokenStream::new();
                    #(#nested)*
                    #output.append_fragment(&::m2_syn::Group::new(
                            ::m2_syn::Delimiter::new(
                                #delimiter,
                                ::m2_syn::Span::detached(),
                            ),
                            #group_output,
                    ));
                }
            }
        };

        statements.push(statement);
        index += 1;
    }

    Ok(statements)
}

fn punct_pair(trees: &[TokenTree], index: usize, first: char, second: char) -> bool {
    matches!(
        (trees.get(index), trees.get(index + 1)),
        (Some(TokenTree::Punct(left)), Some(TokenTree::Punct(right)))
            if left.as_char() == first
                && left.spacing() == Spacing::Joint
                && right.as_char() == second
    )
}

fn matching_angle_bar(trees: &[TokenTree], opening: usize) -> Result<usize> {
    let mut depth = 1usize;
    let mut index = opening + 2;
    while index < trees.len() {
        if punct_pair(trees, index, '<', '|') {
            depth += 1;
            index += 2;
        } else if punct_pair(trees, index, '|', '>') {
            depth -= 1;
            if depth == 0 {
                return Ok(index);
            }
            index += 2;
        } else {
            index += 1;
        }
    }

    let span = trees
        .get(opening)
        .map_or_else(Span::call_site, TokenTree::span);
    Err(syn::Error::new(span, "unclosed angle-bar delimiter"))
}
