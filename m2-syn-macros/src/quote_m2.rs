use proc_macro::TokenStream;
use proc_macro2::{Delimiter, Group, Ident, Spacing, Span, TokenStream as TokenStream2, TokenTree};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Pat, Result, Token};

use crate::base_crate;

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
    let base = base_crate();
    let mut group_index = 0;
    match expand_stream(input.into(), &output, &mut group_index) {
        Ok(statements) => quote!({
            let mut #output = #base::TokenStream::new();
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
    let base = base_crate();
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
                    let mut #group_output = #base::TokenStream::new();
                    #(#nested)*
                    #output.append_fragment(&#base::TokenGroup::new(
                        #base::Delimiter::new(
                            #base::DelimiterKind::AngleBar,
                            #base::DoubleSpan::detached(),
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
            TokenTree::Punct(punctuation) if punctuation.as_char() == '$' => {
                if let (Some(header), Some(template)) = (
                    group_at(&trees, index + 1, Delimiter::Bracket),
                    group_at(&trees, index + 2, Delimiter::Brace),
                ) {
                    index += 2;
                    expand_repetition(header, template, output, group_index)?
                } else if let Some(group) = group_at(&trees, index + 1, Delimiter::Parenthesis) {
                    let expression = syn::parse2::<Expr>(group.stream())?;
                    index += 1;
                    quote!(#output.append_fragment(&(#expression));)
                } else {
                    expand_static_run(&trees, &mut index, output, &base)
                }
            }
            TokenTree::Ident(_) | TokenTree::Literal(_) | TokenTree::Punct(_) => {
                expand_static_run(&trees, &mut index, output, &base)
            }
            TokenTree::Group(group) => expand_group(group, output, group_index)?,
        };

        statements.push(statement);
        index += 1;
    }

    Ok(statements)
}

fn group_at(trees: &[TokenTree], index: usize, delimiter: Delimiter) -> Option<&Group> {
    match trees.get(index) {
        Some(TokenTree::Group(group)) if group.delimiter() == delimiter => Some(group),
        _ => None,
    }
}

fn expand_group(group: &Group, output: &Ident, group_index: &mut usize) -> Result<TokenStream2> {
    let base = base_crate();
    let group_output = format_ident!("__m2_group_{}", *group_index, span = Span::mixed_site());
    *group_index += 1;
    let nested = expand_stream(group.stream(), &group_output, group_index)?;
    let append = match group.delimiter() {
        Delimiter::None => quote!(#output.append_fragment(&#group_output);),
        delimiter => {
            let kind = match delimiter {
                Delimiter::Parenthesis => quote!(#base::DelimiterKind::Parenthesis),
                Delimiter::Bracket => quote!(#base::DelimiterKind::Bracket),
                Delimiter::Brace => quote!(#base::DelimiterKind::Brace),
                Delimiter::None => unreachable!(),
            };
            quote! {
                #output.append_fragment(&#base::TokenGroup::new(
                    #base::Delimiter::new(#kind, #base::DoubleSpan::detached()),
                    #group_output,
                ));
            }
        }
    };
    Ok(quote! {
        let mut #group_output = #base::TokenStream::new();
        #(#nested)*
        #append
    })
}

fn expand_repetition(
    header: &Group,
    template: &Group,
    output: &Ident,
    group_index: &mut usize,
) -> Result<TokenStream2> {
    let base = base_crate();
    let Repetition {
        pattern, iterator, ..
    } = syn::parse2(header.stream())?;
    let iteration_output =
        format_ident!("__m2_iteration_{}", *group_index, span = Span::mixed_site());
    *group_index += 1;
    let nested = expand_stream(template.stream(), &iteration_output, group_index)?;
    Ok(quote! {
        for #pattern in #iterator {
            let mut #iteration_output = #base::TokenStream::new();
            #(#nested)*
            #output.append_fragment(&#iteration_output);
        }
    })
}

fn expand_static_run(
    trees: &[TokenTree],
    index: &mut usize,
    output: &Ident,
    base: &TokenStream2,
) -> TokenStream2 {
    let mut spelling = String::new();
    let mut previous_word = false;
    let mut cursor = *index;
    while let Some(tree) = trees.get(cursor) {
        if cursor != *index && begins_dynamic_or_group(trees, cursor) {
            break;
        }
        let word = matches!(tree, TokenTree::Ident(_) | TokenTree::Literal(_));
        if previous_word && word {
            spelling.push(' ');
        }
        match tree {
            TokenTree::Ident(identifier) => spelling.push_str(&identifier.to_string()),
            TokenTree::Literal(literal) => spelling.push_str(&literal.to_string()),
            TokenTree::Punct(punctuation) => spelling.push(punctuation.as_char()),
            TokenTree::Group(_) => break,
        }
        previous_word = word;
        cursor += 1;
    }
    *index = cursor - 1;
    quote! {
        #output.append_fragment(&#base::__quote_fragment(#spelling));
    }
}

fn begins_dynamic_or_group(trees: &[TokenTree], index: usize) -> bool {
    match trees.get(index) {
        Some(TokenTree::Group(_)) => true,
        Some(TokenTree::Punct(punctuation)) if punctuation.as_char() == '$' => {
            group_at(trees, index + 1, Delimiter::Parenthesis).is_some()
                || (group_at(trees, index + 1, Delimiter::Bracket).is_some()
                    && group_at(trees, index + 2, Delimiter::Brace).is_some())
        }
        Some(TokenTree::Punct(_)) => {
            punct_pair(trees, index, '<', '|') || punct_pair(trees, index, '|', '>')
        }
        Some(TokenTree::Ident(_) | TokenTree::Literal(_)) | None => false,
    }
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
