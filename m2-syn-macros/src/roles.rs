//! Parser roles used by token declarations.

use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use syn::parse::{Parse as SynParse, ParseStream};
use syn::{Error, Result, Token, parenthesized};

use crate::tokens::OperatorKind;

const BINARY: u8 = 1 << 0;
const PREFIX: u8 = 1 << 1;
const POSTFIX: u8 = 1 << 2;
const INFIX: u8 = 1 << 3;
const PREFIX_BINARY: u8 = PREFIX | BINARY;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct ParseInfo {
    precedence: u8,
    role: Role,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Precedence,
    Postfix,
    Binary(u8),
    Infix(u8),
    Prefix(u8),
    PrefixBinary(u8, u8),
    Unary(u8),
}

impl ParseInfo {
    pub(crate) fn is_any_operator(self) -> bool {
        matches!(
            self.role,
            Role::Postfix
                | Role::Binary(_)
                | Role::Infix(_)
                | Role::Prefix(_)
                | Role::PrefixBinary(_, _)
        )
    }

    pub(crate) fn is_operator(self, kind: OperatorKind) -> bool {
        matches!(
            (self.role, kind),
            (Role::Postfix, OperatorKind::Postfix)
                | (Role::Binary(_), OperatorKind::Binary)
                | (Role::Prefix(_), OperatorKind::Prefix)
                | (
                    Role::PrefixBinary(_, _),
                    OperatorKind::Prefix | OperatorKind::Binary
                )
        )
    }

    pub(crate) fn expand(self) -> TokenStream {
        let precedence = self.precedence;
        match self.role {
            Role::Precedence => quote!(TokenParseRule::Precedence(#precedence)),
            Role::Postfix => quote!(TokenParseRule::Postfix(#precedence)),
            Role::Binary(binary) | Role::Infix(binary) => {
                quote!(TokenParseRule::Binary(#precedence, #binary))
            }
            Role::Prefix(unary) => quote!(TokenParseRule::Prefix(#precedence, #unary)),
            Role::PrefixBinary(binary, unary) => {
                quote!(TokenParseRule::PrefixBinary(#precedence, #binary, #unary))
            }
            Role::Unary(strength) => quote!(TokenParseRule::Unary(#precedence, #strength)),
        }
    }

    pub(crate) fn augmented_form(self) -> Option<Self> {
        let (Role::Binary(binary) | Role::Infix(binary) | Role::PrefixBinary(binary, _)) =
            self.role
        else {
            return None;
        };
        Some(Self {
            precedence: self.precedence,
            role: Role::Binary(binary),
        })
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct ParseFlags {
    roles: u8,
    pub(crate) augmented: bool,
    pub(crate) trivia: bool,
}

impl SynParse for ParseFlags {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut flags = Self::default();
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            let tag = name.to_string();
            match tag.as_str() {
                "bin" | "pref" | "post" | "infix" => {
                    let role = match tag.as_str() {
                        "bin" => BINARY,
                        "pref" => PREFIX,
                        "post" => POSTFIX,
                        _ => INFIX,
                    };
                    if flags.roles & role != 0 {
                        return Err(Error::new(name.span(), "duplicate parser role"));
                    }
                    flags.roles |= role;
                    if flags.roles.count_ones() > 1 && flags.roles != PREFIX_BINARY {
                        return Err(Error::new(
                            name.span(),
                            "a token takes one parser role, or `bin` together with `pref`",
                        ));
                    }
                }
                "aug" if !flags.augmented => flags.augmented = true,
                "aug" => return Err(Error::new(name.span(), "duplicate `aug` tag")),
                "trivia" if !flags.trivia => flags.trivia = true,
                "trivia" => return Err(Error::new(name.span(), "duplicate `trivia` tag")),
                _ => {
                    return Err(Error::new(
                        name.span(),
                        "expected `bin`, `pref`, `post`, `infix`, `aug`, or `trivia`",
                    ));
                }
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
    pub(crate) fn resolve(
        self,
        input: ParseStream<'_>,
        precedences: &Precedences,
    ) -> Result<Option<ParseInfo>> {
        if self.roles == 0 && !input.peek(syn::token::Paren) {
            return Ok(None);
        }
        let (precedence, binary, unary) = parse_precedence_slots(input, precedences)?;
        // A prefix operator binds its operand at its own precedence unless the
        // table says otherwise, so the third slot is normally left out.
        let unary_strength = unary.unwrap_or(precedence);
        let binary_strength = || {
            binary.ok_or_else(|| {
                Error::new(
                    Span::call_site(),
                    "a binary role needs `L`, `R`, or a strength in the second slot",
                )
            })
        };
        let role = match self.roles {
            0 => binary.map_or(Role::Precedence, Role::Unary),
            BINARY => Role::Binary(binary_strength()?),
            PREFIX => Role::Prefix(unary_strength),
            PREFIX_BINARY => Role::PrefixBinary(binary_strength()?, unary_strength),
            POSTFIX => Role::Postfix,
            INFIX => Role::Infix(binary_strength()?),
            _ => unreachable!(),
        };
        let unused_slot = match role {
            Role::Precedence | Role::Postfix => binary.is_some() || unary.is_some(),
            Role::Binary(_) | Role::Infix(_) | Role::Unary(_) => unary.is_some(),
            Role::Prefix(_) => binary.is_some(),
            Role::PrefixBinary(_, _) => false,
        };
        if unused_slot {
            return Err(Error::new(
                Span::call_site(),
                "a precedence slot was written for a role that does not use it",
            ));
        }
        Ok(Some(ParseInfo { precedence, role }))
    }
}

fn parse_precedence_slots(
    input: ParseStream<'_>,
    precedences: &Precedences,
) -> Result<(u8, Option<u8>, Option<u8>)> {
    let content;
    parenthesized!(content in input);
    let precedence = parse_precedence_slot(&content, precedences)?.ok_or_else(|| {
        Error::new(
            content.span(),
            "the precedence slot is required and cannot be `_`",
        )
    })?;
    if content.is_empty() {
        return Ok((precedence, None, None));
    }
    content.parse::<Token![,]>()?;

    let binary = if content.peek(syn::Ident) {
        let fork = content.fork();
        let associativity: Ident = fork.parse()?;
        match associativity.to_string().as_str() {
            "L" => {
                content.parse::<Ident>()?;
                Some(precedence)
            }
            "R" => {
                content.parse::<Ident>()?;
                Some(precedence.saturating_sub(1))
            }
            _ => parse_precedence_slot(&content, precedences)?,
        }
    } else {
        parse_precedence_slot(&content, precedences)?
    };

    let unary = if content.is_empty() {
        None
    } else {
        content.parse::<Token![,]>()?;
        parse_precedence_slot(&content, precedences)?
    };
    if !content.is_empty() {
        return Err(content.error("expected at most three precedence slots"));
    }
    Ok((precedence, binary, unary))
}

pub(crate) fn parse_precedence_slot(
    input: ParseStream<'_>,
    precedences: &Precedences,
) -> Result<Option<u8>> {
    if input.peek(Token![_]) {
        input.parse::<Token![_]>()?;
        return Ok(None);
    }
    if input.peek(syn::Ident) {
        let name: Ident = input.parse()?;
        return precedences
            .iter()
            .find(|(declared, _)| declared == &name)
            .map(|(_, value)| *value)
            .map(Some)
            .ok_or_else(|| {
                Error::new(
                    name.span(),
                    format!("`{name}` is not a declared precedence"),
                )
            });
    }
    let literal: syn::LitInt = input.parse()?;
    Ok(Some(literal.base10_parse()?))
}

pub(crate) type Precedences = Vec<(Ident, u8)>;
