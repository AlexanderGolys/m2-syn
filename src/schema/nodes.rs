crate::syntax! {
// P = precedence, B = binary binding strength, U = unary binding strength.
// `L` and `R` derive B from P. Columns absent for a role are omitted, and
// spellings with identical metadata share one row.
tokens {
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
    [symbol] [global] [local] [threadLocal] [threadVariable]    (62, 74)

    // Separators
    [;]                                                         (8)
    [,]                                                         (10)
}

// Leaves retain their source text. Canonical names select the specialized
// storage and parsing behavior supplied by the schema macro.
strutuct! { Symbol }
strutuct! { Empty }
strutuct! { Adj }
strutuct! { EOC }
strutuct! { EOF }
strutuct! { EscapeSequence }
strutuct! { FloatLiteral }
strutuct! { IntegerLiteral }
strutuct! { RawStringContent }
strutuct! { StringContent }

// `Lines<T>` is stored as `Vec<T>` and printed one element per line.
#[comments]
#[cell_stream]
strutuct!{ SourceFile
    cells: Lines<Cell>,
    eof: EOF,
}

#[comments]
strutuct!{ Cell
    content: PackedContent,
    semicolon: Token![;]?,
    eoc: EOC,
}

strutuct!{ Component
    Expr(Expr*)
    Empty(Empty)
}

strutuct!{ ExprPack
    pack: Punctuated<Component>,
}

strutuct!{ PackedContent
    Expr(Expr*)
    Pack(ExprPack)
}

strutuct!{ Statement
    content: PackedContent,
    semicolon: Token![;],
}

#[delimiter(bracket)]
strutuct!{ Array
    statements: Vec<Statement>,
    elements: PackedContent?,
}

#[delimiter(brace)]
strutuct!{ List
    statements: Vec<Statement>,
    elements: PackedContent?,
}

#[delimiter(angle_bar)]
strutuct!{ AngleBarList
    statements: Vec<Statement>,
    elements: PackedContent?,
}

#[delimiter(parenthesis)]
strutuct!{ Sequence
    statements: Vec<Statement>,
    elements: PackedContent?,
}

#[delimiter(parenthesis)]
strutuct!{ Holder
    muted: Vec<Statement>,
    value: Expr*?,
}

strutuct!{ StringElement
    Escape(EscapeSequence)
    Content(StringContent)
}

#[literal(string)]
strutuct!{ StringLiteral
    elements: Vec<StringElement>,
}

strutuct!{ RawStringElement
    Escape(EscapeSequence)
    Content(RawStringContent)
}

#[literal(raw_string)]
strutuct!{ RawStringLiteral
    elements: Vec<RawStringElement>,
}

strutuct!{ AssignmentArgument
    List(AssignmentList*)
    Empty(Empty)
    Symbol(Symbol)
    Op(ExprOp)
    // `x <- y` as a target: M2 reads it as an installation target for `<-`.
    Evaluated(ExprAssignEvaluated)
}

strutuct!{ AssignmentListContent
    Argument(AssignmentArgument)
    Pack {
        arguments: Punctuated<AssignmentArgument>,
    }
}

strutuct!{ AssignmentList
    contents: Delimited<Option<AssignmentListContent>>,
}

strutuct!{ ArgumentListContent
    Symbol(Symbol)
    Pack {
        arguments: Punctuated<Symbol>,
    }
}

// These products are shared by otherwise independent expression families.
strutuct!{ TypeAnnotation
    typical_value: Expr*,
    operator: Token![=>],
}

strutuct!{ ThenClause
    then_token: Token![then],
    expr: Expr*,
}

strutuct!{ ElseClause
    else_token: Token![else],
    expr: Expr*,
}

strutuct!{ RangeBoundFrom
    from_token: Token![from],
    value: Expr*,
}

strutuct!{ RangeBoundTo
    to_token: Token![to],
    value: Expr*,
}

strutuct!{ WhenCondition
    when_token: Token![when],
    condition: Expr*,
}

strutuct!{ ListStep
    list_token: Token![list],
    expr: Expr*,
}

strutuct!{ DoStep
    do_token: Token![do],
    statement: Expr*,
}

strutuct!{ LoopBody
    list_step: ListStep?,
    do_step: DoStep?,
}

strutuct!{ OfClass
    of_token: Token![of],
    parent: Expr*,
}

strutuct!{ FromInstance
    from_token: Token![from],
    instance: Expr*,
}

strutuct!{ TryFallback
    Else(ElseClause)
    Except {
        except_token: Token![except],
        exception: Symbol,
        do_token: Token![do],
        value: Expr*,
    }
}

// Semantic alternatives are nested so every layer becomes a dispatch point.
// The short layer names keep the generated cumulative type names readable.
strutuct!{ Expr
    Literal {
        Float(FloatLiteral)
        Integer(IntegerLiteral)
        RawString(RawStringLiteral)
        String(StringLiteral)
    }

    Collection {
        Sequence(Sequence)
        List(List)
        Array(Array)
        AngleBar(AngleBarList)
    }
    Holder(Holder)
    Op {
        Bin {
            Adj {
                left_operand: Expr*,
                operator: Adj,
                right_operand: Expr*,
            }
            Regular {
                left_operand: Expr*,
                operator: BinaryOperator,
                right_operand: Expr*,
            }
            Member {
                Dot {
                    receiver: Expr*,
                    operator: Token![.],
                    member: Expr*,
                }
                Optional {
                    receiver: Expr*,
                    operator: Token![.?],
                    member: Expr*,
                }
            }
            Index {
                receiver: Expr*,
                operator: Token![#],
                index: Expr*,
            }
        }
        Prefix {
            operator: PrefixOperator,
            operand: Expr*,
        }
        Postfix {
            operand: Expr*,
            operator: PostfixOperator,
        }
    }
    Assign {
        Simple {
            Global {
                variable: Symbol,
                operator: Token![=],
                value: Expr*,
            }
            Local {
                variable: Symbol,
                operator: Token![:=],
                value: Expr*,
            }
        }
        Parallel {
            Global {
                argument_list: AssignmentList*,
                operator: Token![=],
                value: Expr*,
            }
            Local {
                argument_list: AssignmentList*,
                operator: Token![:=],
                value: Expr*,
            }
        }
        Evaluated {
            evaluated_domain: Expr*,
            operator: Token![<-],
            value: Expr*,
        }
        Augmented {
            target: Expr*,
            operator: AugmentedOperator,
            value: Expr*,
        }
        Install {
            Method {
                method: Expr*,
                operator: Adj,
                signature: Expr*,
                assign_op: Token![:=],
                typical_value: TypeAnnotation?,
                function: Expr*,
            }
            Op {
                domain_expr: ExprOp,
                assign_op: Token![:=],
                typical_value: TypeAnnotation?,
                function: Expr*,
            }
            New {
                new_statement: ExprNew,
                operator: Token![:=],
                function: Expr*,
            }
        }
        Op {
            argument_expr: ExprOp,
            operator: Token![=],
            value_expr: Expr*,
        }
    }
    Option {
        key: Expr*,
        operator: Token![=>],
        value: Expr*,
    }
    Lambda {
        Variadic {
            argument: Symbol,
            operator: Token![->],
            body: Expr*,
        }
        List {
            arguments: Delimited<Option<ArgumentListContent>>,
            operator: Token![->],
            body: Expr*,
        }
    }
    If {
        if_token: Token![if],
        condition: Expr*,
        then_clause: ThenClause,
        else_clause: ElseClause?,
    }
    For {
        Bare {
            for_token: Token![for],
            variable: Symbol,
            condition: WhenCondition?,
            body: LoopBody,
        }
        Range {
            for_token: Token![for],
            variable: Symbol,
            lower_bound: RangeBoundFrom?,
            upper_bound: RangeBoundTo?,
            condition: WhenCondition?,
            body: LoopBody,
        }
        In {
            for_token: Token![for],
            variable: Symbol,
            in_token: Token![in],
            collection: Expr*,
            condition: WhenCondition?,
            body: LoopBody,
        }
    }
    While {
        while_token: Token![while],
        condition: Expr*,
        body: LoopBody,
    }
    Try {
        try_token: Token![try],
        value: Expr*,
        on_success: ThenClause?,
        fallback: TryFallback?,
    }
    Quote {
        Symbol {
            specifier: Token![symbol],
            symbol: Symbol,
        }
        Local {
            specifier: Token![local],
            symbol: Symbol,
        }
        Global {
            specifier: Token![global],
            symbol: Symbol,
        }
        ThreadVariable {
            specifier: Token![threadVariable],
            symbol: Symbol,
        }
        ThreadLocal {
            specifier: Token![threadLocal],
            symbol: Symbol,
        }
    }
    New {
        new_token: Token![new],
        class: Expr*,
        parent_class: OfClass?,
        instance: FromInstance?,
    }
    Control {
        Break {
            break_token: Token![break],
            value: Expr*?,
        }
        Continue {
            continue_token: Token![continue],
            value: Expr*?,
        }
        Return {
            return_token: Token![return],
            value: Expr*?,
        }
    }
    Catch {
        catch_token: Token![catch],
        value: Expr*,
    }
    Throw {
        throw_token: Token![throw],
        value: Expr*,
    }
    Trap {
        trap_token: Token![trap],
        value: Expr*,
    }
    Debug {
        Test {
            keyword: Token![TEST],
            value: Expr*?,
        }
        Step {
            keyword: Token![step],
            value: Expr*?,
        }
        Finish {
            keyword: Token![finish],
            value: Expr*?,
        }
        Shield {
            keyword: Token![shield],
            value: Expr*?,
        }
        Breakpoint {
            keyword: Token![breakpoint],
            value: Expr*?,
        }
    }
    Profile {
        ElapsedTime {
            modifier: Token![elapsedTime],
            value: Expr*,
        }
        ElapsedTiming {
            modifier: Token![elapsedTiming],
            value: Expr*,
        }
        Profile {
            modifier: Token![profile],
            value: Expr*,
        }
        Time {
            modifier: Token![time],
            value: Expr*,
        }
        Timing {
            modifier: Token![timing],
            value: Expr*,
        }
    }
    Symbol(Symbol)
}
}
