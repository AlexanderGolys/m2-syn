//! The token table: every Macaulay2 operator, keyword, and separator with its
//! parser roles and binding strengths.
//!
//! Each row declares a token type, named by `Token![…]` with its spelling,
//! and the rows' roles collect the operators into [`PrefixOperator`],
//! [`BinaryOperator`], [`PostfixOperator`], and [`AugmentedOperator`].

use crate::qf;
use crate::traversal::{__walk, fold, visit, visit_mut};

// P = precedence, B = binary binding strength, U = unary binding strength.
// `L` and `R` derive B from P. Columns absent for a role are omitted, and
// spellings with identical metadata share one row.
crate::tokens! {
    // Operators                          roles                 (P, B, U)
    [->] [<-] [=] [=>]                   { infix }              (14, R)
    [>>]                                 { bin, aug }           (14, R)
    [<<]                                 { bin, pref, aug }     (18, L)
    [|-]                                 { bin, pref, aug }     (20, L)
    [===>]                               { bin, aug }           (22, R)
    [<===]                               { bin, pref }          (22, R)
    [<==>]                               { bin, aug }           (24, R)
    [==>]                                { bin, aug }           (26, R)
    [<==]                                { bin, pref }          (26, R)
    [or]                                 { bin }                (28, R)
    [??]                                 { bin, pref, aug }     (28, R)
    [xor]                                { bin }                (30, R)
    [and]                                { bin }                (32, R)
    [not]                                { pref }               (34)
    [!=] [=!=] [==] [===]                { bin }                (36, R)
    [<] [<=] [>] [>=] [?]                { bin, pref }          (36, R)
    [~]                                  { bin, pref, aug }     (36, R)
    [||]                                 { bin, aug }           (38, L)
    [:]                                  { bin, aug }           (40, R)
    [|]                                  { bin, aug }           (42, L)
    [^^]                                 { bin, aug }           (44, L)
    [&]                                  { bin, aug }           (46, L)
    [..] [..<]                           { bin, aug }           (48, L)
    [++]                                 { bin, aug }           (50, L)
    [+] [-]                              { bin, pref, aug }     (50, L)
    ["·"]                                { bin, aug }           (52, L)
    [**] ["⊠"] ["⧢"]                     { bin, aug }           (54, L)
    [%] [/] [/ /]                        { bin, aug }           (58, L)
    [*]                                  { bin, pref, aug }     (58, L)
    ["\\"] ["\\\\"]                      { bin, aug }           (58, R)
    [@]                                  { bin, aug }           (60, R)
    [SPACE]                              { bin, implicit }      (62, R)
    [(*)]                                { post }               (64)
    [@@] [@@?]                           { bin, aug }           (66, L)
    [^*] [^~] [_*] [_~]                  { post }               (68)
    [#?] [.] [.?]                        { bin }                (70, L)
    [^<] [^<=] [^>] [^>=]                { bin }                (70, L)
    [_<] [_<=] [_>] [_>=]                { bin }                (70, L)
    [^] [^**] [_] [|_]                   { bin, aug }           (70, L)
    [#]                                  { bin, pref }          (70, L, 61)
    [!] [^!] [_!]                        { post }               (72)

    // Keywords
    [else] [then] [do] [list] [except]                          (12)
    [when] [of] [in] [from] [to]                                (16)
    [break] [continue] [return]
    [catch] [throw] [trap]
    [if] [while] [try]
    [TEST] [time] [timing]
    [elapsedTime] [elapsedTiming]
    [breakpoint] [profile] [shield]
    [step] [finish]                                             (62, 12)
    [for] [new]                                                 (62, 16)
    [symbol] [global] [local] [threadLocal]                     (62, 74)

    // Other spellings of a token, as M2's `findSynonyms` lists them
    ["→"] = [->]
    ["←"] = [<-]
    ["⇒"] = [=>]
    ["∨"] = [or]
    ["∧"] = [and]
    ["≠"] = [!=]
    ["≤"] = [<=]
    ["≥"] = [>=]
    ["≪"] = [<<]
    ["≫"] = [>>]
    ["⊢"] = [|-]
    ["⊕"] = [++]
    ["⊗"] = [**]
    [threadVariable] = [threadLocal]

    // Separators
    [;]                                                         (8)
    [,]                                                         (10)

    // Positions, never written: the beginning and end of a file and of a cell
    [BOF] [EOF] [EOC]                    { marker }
}
