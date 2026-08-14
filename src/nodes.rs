macro_rules! syntax_schema {
    ($($tokens:tt)*) => {};
}

// This inert invocation is the single source of truth consumed by the
// generator. The checked-in expansion is included below.
syntax_schema! {
    tokens {
        [!=]        {bin}
        [#]         {pref, bin}
        [#?]		{bin}
        [%]			{bin, aug}
        [&]			{bin, aug}
        [*]			{bin, pref, aug}
        [**]		{bin, aug}
        [+]			{bin, pref, aug}
        [++]		{bin, aug}
        [-]			{pref, bin, aug}
        [->]		{}
        [.]			{bin}
        [..]		{bin, aug}
        [..<]		{bin, aug}
        [.?]		{bin}
        [/]			{bin, aug}
        [/ /]		{bin, aug}
        [:]			{bin, aug}
        [<]			{pref, bin}
        [<-]		{}
        [<<]		{pref, bin, aug}
        [<=]		{pref, bin}
        [<==]		{pref, bin}
        [<===]		{pref, bin}
        [<==>]		{bin, aug}
        [=]			{}
        [=!=]		{bin}
        [==]		{bin}
        [===]		{bin}
        [===>]		{bin, aug}
        [==>]		{bin, aug}
        [=>]		{}
        [>]			{pref, bin}
        [>=]		{pref, bin}
        [>>]		{bin, aug}
        [?]			{pref, bin}
        [??]		{pref, bin, aug}
        [@]			{bin, aug}
        [@@]		{bin, aug}
        [@@?]		{bin, aug}
        ["\\"]		{bin, aug}
        ["\\\\"]    {bin, aug}
        [^]			{bin, aug}
        [^**]		{bin, aug}
        [^<]		{bin}
        [^<=]		{bin}
        [^>]		{bin}
        [^>=]		{bin}
        [^^]		{bin, aug}
        [_]			{bin, aug}
        [_<]		{bin}
        [_<=]		{bin}
        [_>]		{bin}
        [_>=]		{bin}
        [|]			{bin, aug}
        [|-]		{pref, bin, aug}
        [|_]		{bin, aug}
        [||]		{bin, aug}
        [~]			{pref, bin, aug}
        ["·"]		{bin, aug}
        ["⊠"]		{bin, aug}
        ["⧢"]		{bin, aug}
        [and]		{bin}
        [or]		{bin}
        [SPACE]		{bin}
        []          {bin}
        [xor]		{bin}
        [!]			{post}
        [(*)]		{post}
        [^!]		{post}
        [^*]		{post}
        [^~]		{post}
        [_!]		{post}
        [_*]		{post}
        [_~]		{post}
        [not]		{pref}
    }

    keywords: {
        [break] [breakpoint] [catch] [continue]
        [do] [elapsedTime] [elapsedTiming] [else] [except] [finish]
        [for] [from] [global] [if] [in] [list] [local]
        [new] [of] [profile] [return] [shield]
        [step] [symbol] [TEST] [then] [threadLocal]
        [threadVariable] [throw] [time] [timing] [to] [trap]
        [try] [when] [while]
    }

    markers: {}

    punct: {
        [;], [,]
    }

    // Leaf nodes retain their source text. Their concrete names are obtained by
    // converting the Rust name to snake_case, matching grammar.js.
    Symbol ::= leaf
    Keyword ::= leaf
    BlockComment ::= leaf
    LineComment ::= leaf
    EmptyComponent ::= leaf
    EscapeSequence ::= leaf
    FloatLiteral ::= leaf
    IntegerLiteral ::= leaf
    RawStringContent ::= leaf
    StringContent ::= leaf

    // Unlabelled items correspond to unfielded CST children. The generator may
    // give them artificial Rust field names, but those names are not CST fields.
    SourceFile ::= (elements: unfielded lines(AnyCell))

    ExpressionCell ::= node(cell,
        value: unfielded Expr,
    )

    SequenceCell ::= node(cell,
        value: unfielded NakedSequence,
    )

    NakedSequence ::= (elements: unfielded punct(Component))

    MutedCell ::= node(muted,
        elements: unfielded punct(Component),
        Token![;],
    )

    MutedGroup ::= node(muted,
        elements: unfielded punct(Component),
        Token![;],
    )

    Array ::= bracket(
        muted: unfielded [MutedGroup],
        elements: unfielded punct(Component),
    )

    List ::= brace(
        muted: unfielded [MutedGroup],
        elements: unfielded punct(Component),
    )

    AngleBarList ::= angle_bar(
        muted: unfielded [MutedGroup],
        elements: unfielded punct(Component),
    )

    Sequence ::= paren(
        muted: unfielded [MutedGroup],
        elements: unfielded punct(Component),
    )

    ParenthesizedExpression ::= paren(
        muted: unfielded [MutedGroup],
        value: unfielded Expr?,
    )

    StringLiteral ::= string(elements: unfielded [StringElement])
    RawStringLiteral ::= raw_string(elements: unfielded [RawStringElement])

    BinaryExpression ::= (
        left: Expr,
        operator: BinaryOperator,
        right: Expr,
    )

    PrefixExpression ::= (
        operator: PrefixOperator,
        operand: Expr,
    )

    PostfixExpression ::= (
        operand: Expr,
        operator: PostfixOperator,
    )

    LambdaExpression ::= (
        parameters: LambdaParameters,
        operator: Token![->],
        body: Expr,
    )

    Assignment ::= node(assignment,
        left: Symbol,
        operator: Token![=],
        right: Expr,
    )

    LocalAssignment ::= node(local_assignment,
        left: Symbol,
        operator: Token![:=],
        right: Expr,
    )

    BinaryAssignment ::= (
        left: BinaryExpression,
        operator: Token![=],
        right: Expr,
    )

    BinaryInstallation ::= (
        left: BinaryExpression,
        operator: Token![:=],
        right: Expr,
    )

    PrefixAssignment ::= (
        left: PrefixExpression,
        operator: Token![=],
        right: Expr,
    )

    PrefixInstallation ::= (
        left: PrefixExpression,
        operator: Token![:=],
        right: Expr,
    )

    PostfixAssignment ::= (
        left: PostfixExpression,
        operator: Token![=],
        right: Expr,
    )

    PostfixInstallation ::= (
        left: PostfixExpression,
        operator: Token![:=],
        right: Expr,
    )

    StructuredBinding ::= node(assignment,
        left: BindingPack,
        operator: Token![=],
        right: Expr,
    )

    LocalStructuredBinding ::= node(local_assignment,
        left: BindingPack,
        operator: Token![:=],
        right: Expr,
    )

    EvaluatedAssignment ::= (
        left: Expr,
        operator: Token![<-],
        right: Expr,
    )

    Option ::= (
        left: Expr,
        operator: Token![=>],
        right: Expr,
    )


    ThenClause ::= (
        Token![then],
        value: unfielded Expr,
    )

    ElseClause ::= (
        Token![else],
        value: unfielded Expr,
    )

    IfStatement ::= (
        Token![if],
        condition: Expr,
        then_clause: unfielded ThenClause,
        else_clause: unfielded ElseClause?,
    )

    LoopBody ::= (
        (Token![list], listed_value: Expr)?,
        (Token![do], ignored_value: Expr)?,
    )

    IterationRange ::= (
        (Token![in], iterated_collection: Expr)?,
        (Token![from], range_start: Expr)?,
        (Token![to], range_end: Expr)?,
    )

    ForLoop ::= (
        Token![for],
        variable: Symbol,
        range: unfielded IterationRange?,
        (Token![when], filter: Expr)?,
        body: unfielded LoopBody,
    )

    WhileLoop ::= (
        Token![while],
        condition: Expr,
        body: unfielded LoopBody,
    )

    NewStatement ::= (
        Token![new],
        class: Expr,
        (Token![of], parent: Expr)?,
        (Token![from], instance: Expr)?,
    )

    DebugKeyword ::= {
        Token![step],
        Token![finish],
        Token![shield],
        Token![TEST],
        Token![time],
        Token![timing],
        Token![breakpoint],
        Token![elapsedTime],
        Token![elapsedTiming],
        Token![profile],
    }

    DebugClause ::= (
        keyword: DebugKeyword,
        value: unfielded Expr?,
    )

    BreakStatement ::= (Token![break], value: unfielded Expr?)
    ContinueStatement ::= (Token![continue], value: unfielded Expr?)
    ReturnStatement ::= (Token![return], value: unfielded Expr?)
    CatchStatement ::= (Token![catch], value: unfielded Expr)
    ThrowStatement ::= (Token![throw], value: unfielded Expr)
    TrapStatement ::= (Token![trap], value: unfielded Expr)

    ExceptClause ::= (
        Token![except],
        exception: Symbol,
        Token![do],
        value: unfielded Expr,
    )



    TryFallback ::= {
        ExceptClause,
        ElseClause,
    }

    TryStatement ::= (
        Token![try],
        value: unfielded Expr,
        then_clause: unfielded ThenClause?,
        fallback: TryFallback?,
    )

    QuoteSpecifier ::= {
        Token![symbol],
        Token![local],
        Token![global],
        Token![threadVariable],
        Token![threadLocal],
    }

    QuoteValue ::= {
        Keyword,
        Symbol,
    }

    QuoteExpression ::= (
        specifier: unfielded QuoteSpecifier,
        token: QuoteValue,
    )

    AnyCell ::= {
        ExpressionCell,
        SequenceCell,
        MutedCell,
    }

    Component ::= {
        EmptyComponent,
        Expr,
    }

    SequenceElement ::= {
        Component,
        MutedGroup,
    }

    StringElement ::= {
        EscapeSequence,
        StringContent,
    }

    RawStringElement ::= {
        EscapeSequence,
        RawStringContent,
    }

    Collection ::= {
        ParenthesizedExpression,
        Sequence,
        List,
        Array,
        AngleBarList,
    }

    BindingPack ::= {
        ParenthesizedExpression,
        Sequence,
        List,
        Array,
        AngleBarList,
    }

    LambdaParameters ::= {
        Symbol,
        ParenthesizedExpression,
        Sequence,
        List,
        Array,
        AngleBarList,
    }

    OperatorExpr ::= {
        BinaryExpression,
        PrefixExpression,
        PostfixExpression,
    }

    AssignmentExpr ::= {
        Assignment,
        LocalAssignment,
        BinaryAssignment,
        BinaryInstallation,
        PrefixAssignment,
        PrefixInstallation,
        PostfixAssignment,
        PostfixInstallation,
        StructuredBinding,
        LocalStructuredBinding,
        EvaluatedAssignment,
    }

    Expr ::= {
        Collection,
        OperatorExpr,
        AssignmentExpr,
        Option,
        LambdaExpression,
        IfStatement,
        ForLoop,
        WhileLoop,
        TryStatement,
        QuoteExpression,
        NewStatement,
        BreakStatement,
        ContinueStatement,
        ReturnStatement,
        CatchStatement,
        ThrowStatement,
        TrapStatement,
        DebugClause,
        FloatLiteral,
        IntegerLiteral,
        RawStringLiteral,
        StringLiteral,
        Symbol,
    }

    SyntaxNode ::= {
        SourceFile,
        AnyCell,
        Expr,
    }
}

include!("gen/kind.rs");
include!("gen/tokens.rs");
include!("gen/nodes.rs");
include!("gen/visit.rs");
include!("gen/visit_mut.rs");
include!("gen/fold.rs");

/// An expression evaluated in global scope.
///
/// Rust Analyzer presents the implementations below as the concrete type
/// hierarchy for cells.
pub trait Cell: ::m2_syn::AstNode<Kind = SyntaxKind> + ::m2_syn::ToTokens {}

impl Cell for AnyCell {}
impl Cell for ExpressionCell {}
impl Cell for SequenceCell {}
impl Cell for MutedCell {}
