use proc_macro::TokenStream;
use proc_macro2::{Delimiter, Ident, Spacing, Span, TokenStream as TokenStream2, TokenTree};
use quote::{format_ident, quote};
use syn::Result;

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
    let mut previous_wordlike = false;
    let mut index = 0;

    while index < trees.len() {
        let (statement, wordlike, resets_spacing) = match &trees[index] {
            TokenTree::Punct(punctuation)
                if punctuation.as_char() == '$'
                    && matches!(trees.get(index + 1), Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis) =>
            {
                let TokenTree::Group(group) = &trees[index + 1] else {
                    unreachable!()
                };
                let expression = syn::parse2::<syn::Expr>(group.stream())?;
                index += 1;
                (
                    quote!(::m2_syn::ToTokens::to_tokens(&(#expression), &mut #output);),
                    true,
                    false,
                )
            }
            TokenTree::Ident(identifier) => {
                let text = identifier.to_string();
                (
                    quote! {
                        #output.push_ident(::m2_syn::IdentToken::new(
                            #text,
                            ::m2_syn::Span::detached(),
                        ));
                    },
                    true,
                    false,
                )
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
                (
                    quote! {
                        #output.push_literal(::m2_syn::Literal::new(
                            #kind,
                            #text.into(),
                            ::m2_syn::Span::detached(),
                        ));
                    },
                    true,
                    false,
                )
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
                (
                    quote! {
                        #output.push_punct(::m2_syn::Punct::new(
                            #text,
                            ::m2_syn::Span::detached(),
                        ));
                    },
                    false,
                    false,
                )
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
                            ::m2_syn::ToTokens::to_tokens(&#group_output, &mut #output);
                        });
                        previous_wordlike = true;
                        index += 1;
                        continue;
                    }
                };
                (
                    quote! {
                        let mut #group_output = ::m2_syn::TokenStream::new();
                        #(#nested)*
                        #output.push_group(::m2_syn::Group::new(
                            ::m2_syn::Delimiter::new(
                                #delimiter,
                                ::m2_syn::DoubleSpan::new(
                                    ::m2_syn::Span::detached(),
                                    ::m2_syn::Span::detached(),
                                ),
                            ),
                            #group_output,
                        ));
                    },
                    true,
                    false,
                )
            }
        };

        if previous_wordlike && wordlike {
            statements.push(quote! {
                #output.push_trivia(::m2_syn::Trivia::new(
                    ::m2_syn::TriviaKind::Whitespace,
                    " ",
                    ::m2_syn::Span::detached(),
                ));
            });
        }
        statements.push(statement);
        previous_wordlike = if resets_spacing { false } else { wordlike };
        index += 1;
    }

    Ok(statements)
}
