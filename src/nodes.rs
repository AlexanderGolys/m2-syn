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

    AdjacentExpression ::= node(adjacent_expression,
        left: Expr,
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

    OptionExpression ::= node(option,
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

    QuoteExpression ::= (
        specifier: unfielded QuoteSpecifier,
        token: Symbol,
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
        AdjacentExpression,
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
        OptionExpression,
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

include!("gen/tokens.rs");
include!("gen/nodes.rs");
include!("gen/visit.rs");
include!("gen/visit_mut.rs");
include!("gen/fold.rs");

pub(crate) fn canonical_keyword_spelling(spelling: &str) -> &str {
    let Some(keyword) = spelling.strip_prefix("Core$") else {
        return spelling;
    };
    if GENERATED_KEYWORD_SPELLINGS.contains(&keyword) {
        keyword
    } else {
        spelling
    }
}

/// An expression evaluated in global scope.
///
/// Rust Analyzer presents the implementations below as the concrete type
/// hierarchy for cells.
pub trait CellNode: ::m2_syn::Spanned + ::m2_syn::ToTokens {}

impl CellNode for AnyCell {}
impl CellNode for ExpressionCell {}
impl CellNode for SequenceCell {}
impl CellNode for MutedCell {}

impl ::m2_syn::ToCellStream for SourceFile {
    fn to_cell_stream(&self, source_id: ::m2_syn::SourceId) -> ::m2_syn::CellStream {
        let cells = self
            .elements
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                let mut stream = ::m2_syn::ToTokens::to_token_stream(cell);
                let (kind, closing) = match cell {
                    AnyCell::MutedCell(_) => {
                        let semicolon =
                            stream.pop().expect("a muted cell emits its semicolon last");
                        (
                            ::m2_syn::DelimiterKind::Semicolon,
                            ::m2_syn::Spanned::span(&semicolon),
                        )
                    }
                    AnyCell::ExpressionCell(_) | AnyCell::SequenceCell(_) => {
                        let span = ::m2_syn::Spanned::span(cell);
                        let closing = span
                            .source()
                            .ok()
                            .zip(span.end_point().ok())
                            .map(|(source, point)| {
                                ::m2_syn::Span::new(source, ::m2_syn::TextRange::from_point(point))
                            })
                            .unwrap_or_else(::m2_syn::Span::detached);
                        (::m2_syn::DelimiterKind::Empty, closing)
                    }
                };
                if index != 0 {
                    let mut with_leading_newline = ::m2_syn::TokenStream::new();
                    with_leading_newline.push_trivia(::m2_syn::Trivia::new(
                        ::m2_syn::TriviaKind::LineBreak,
                        "\n",
                        ::m2_syn::Span::detached(),
                    ));
                    with_leading_newline.extend([stream]);
                    stream = with_leading_newline;
                }
                let span = ::m2_syn::Spanned::span(cell);
                let opening = span
                    .source()
                    .ok()
                    .zip(span.start_point().ok())
                    .map(|(source, point)| {
                        ::m2_syn::Span::new(source, ::m2_syn::TextRange::from_point(point))
                    })
                    .unwrap_or_else(::m2_syn::Span::detached);
                ::m2_syn::CellBlock::new(
                    ::m2_syn::Delimiter::new(kind, ::m2_syn::DoubleSpan::new(opening, closing)),
                    stream,
                )
            })
            .collect();
        ::m2_syn::CellStream::new(cells, source_id)
    }
}
