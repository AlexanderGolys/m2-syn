macro_rules! syntax_schema {
    ($($tokens:tt)*) => {};
}

// This inert invocation is the single source of truth consumed by the
// generator. The checked-in expansion is included below.
syntax_schema! {
    // Structural precedences with no single owning operator token; consumed
    // directly by the native parser. Operator tokens below carry their own
    // precedence numbers inline instead of naming a shared table entry.
    precedence: {
        PREC_CLOSER = 6,
        PREC_SEMICOLON = 8,
        PREC_COMMA = 10,
        PREC_CONTROL = 12,
        PREC_LOOP_CLAUSE = 16,
        PREC_COLLECTION = 56,
        PREC_APPLICATION_RIGHT = 61,
        PREC_APPLICATION = 62,
        PREC_QUOTE = 74,
    }

    augmented: (14, 13)

    // Each row carries (precedence, binary_strength, unary_strength); `_`
    // marks a slot the row's flags don't use.
    tokens {
        [!=]       { bin }             (36, 35, _)
        [#]        { bin, pref }       (70, 70, 61)
        [#?]       { bin }             (70, 70, _)
        [%]        { bin, aug }        (58, 58, _)
        [&]        { bin, aug }        (46, 46, _)
        [*]        { bin, pref, aug }  (58, 58, 58)
        [**]       { bin, aug }        (54, 54, _)
        [+]        { bin, pref, aug }  (50, 50, 50)
        [++]       { bin, aug }        (50, 50, _)
        [-]        { bin, pref, aug }  (50, 50, 50)
        [->]       { infix }           (14, 13, _)
        [.]        { bin }             (70, 70, _)
        [..]       { bin, aug }        (48, 48, _)
        [..<]      { bin, aug }        (48, 48, _)
        [.?]       { bin }             (70, 70, _)
        [/]        { bin, aug }        (58, 58, _)
        [/ /]      { bin, aug }        (58, 58, _)
        [:]        { bin, aug }        (40, 39, _)
        [<]        { bin, pref }       (36, 35, 36)
        [<-]       { infix }           (14, 13, _)
        [<<]       { bin, pref, aug }  (18, 18, 18)
        [<=]       { bin, pref }       (36, 35, 36)
        [<==]      { bin, pref }       (26, 25, 26)
        [<===]     { bin, pref }       (22, 21, 22)
        [<==>]     { bin, aug }        (24, 23, _)
        [=]        { infix }           (14, 13, _)
        [=!=]      { bin }             (36, 35, _)
        [==]       { bin }             (36, 35, _)
        [===]      { bin }             (36, 35, _)
        [===>]     { bin, aug }        (22, 21, _)
        [==>]      { bin, aug }        (26, 25, _)
        [=>]       { infix }           (14, 13, _)
        [>]        { bin, pref }       (36, 35, 36)
        [>=]       { bin, pref }       (36, 35, 36)
        [>>]       { bin, aug }        (14, 13, _)
        [?]        { bin, pref }       (36, 35, 36)
        [??]       { bin, pref, aug }  (28, 27, 28)
        [@]        { bin, aug }        (60, 59, _)
        [@@]       { bin, aug }        (66, 66, _)
        [@@?]      { bin, aug }        (66, 66, _)
        ["\\"]     { bin, aug }        (58, 57, _)
        ["\\\\"]   { bin, aug }        (58, 57, _)
        [^]        { bin, aug }        (70, 70, _)
        [^**]      { bin, aug }        (70, 70, _)
        [^<]       { bin }             (70, 70, _)
        [^<=]      { bin }             (70, 70, _)
        [^>]       { bin }             (70, 70, _)
        [^>=]      { bin }             (70, 70, _)
        [^^]       { bin, aug }        (44, 44, _)
        [_]        { bin, aug }        (70, 70, _)
        [_<]       { bin }             (70, 70, _)
        [_<=]      { bin }             (70, 70, _)
        [_>]       { bin }             (70, 70, _)
        [_>=]      { bin }             (70, 70, _)
        [|]        { bin, aug }        (42, 42, _)
        [|-]       { bin, pref, aug }  (20, 20, 20)
        [|_]       { bin, aug }        (70, 70, _)
        [||]       { bin, aug }        (38, 38, _)
        [~]        { bin, pref, aug }  (36, 35, 36)
        ["·"]      { bin, aug }        (52, 52, _)
        ["⊠"]      { bin, aug }        (54, 54, _)
        ["⧢"]      { bin, aug }        (54, 54, _)
        [and]      { bin }             (32, 31, _)
        [or]       { bin }             (28, 27, _)
        [SPACE]    { bin }             (62, 61, _)
        [xor]      { bin }             (30, 29, _)
        [!]        { post }            (72, _, _)
        [(*)]      { post }            (64, _, _)
        [^!]       { post }            (72, _, _)
        [^*]       { post }            (68, _, _)
        [^~]       { post }            (68, _, _)
        [_!]       { post }            (72, _, _)
        [_*]       { post }            (68, _, _)
        [_~]       { post }            (68, _, _)
        [not]      { pref }            (34, _, 34)
    }

    keywords {
        [break] [breakpoint] [catch] [continue]
        [do] [elapsedTime] [elapsedTiming] [else] [except] [finish]
        [for] [from] [global] [if] [in] [list] [local]
        [new] [of] [profile] [return] [shield]
        [step] [symbol] [TEST] [then] [threadLocal]
        [threadVariable] [throw] [time] [timing] [to] [trap]
        [try] [when] [while]
    }

    markers {}

    punct {
        [;], [,]
    }
    // Leaf nodes retain their source text. Their concrete names are obtained by
    // converting the Rust name to snake_case, matching grammar.js.
    struct Symbol;
    struct BlockComment;
    struct LineComment;
    struct EmptyComponent;
    struct EscapeSequence;
    struct FloatLiteral;
    struct IntegerLiteral;
    struct RawStringContent;
    struct StringContent;

    // `(_)` marks a field addressed positionally rather than by a CST field
    // name. The generator may still give it an artificial Rust name, but
    // that name is not a CST field.
    struct SourceFile {
        elements: (_ lines) Vec<AnyCell>
    }

    #[cst(kind = cell)]
    struct ExpressionCell {
        value: (_) Expr
    }

    #[cst(kind = cell)]
    struct SequenceCell {
        value: (_) NakedSequence
    }

    struct NakedSequence {
        elements: (_) Punctuated<Component>
    }

    #[cst(kind = muted)]
    struct MutedCell {
        elements: (_) Punctuated<Component>,
        semicolon: (_) Token![;],
    }

    #[cst(kind = muted)]
    struct MutedGroup {
        elements: (_) Punctuated<Component>,
        semicolon: (_) Token![;],
    }

    #[delimiter(bracket)]
    struct Array {
        muted: (_) Vec<MutedGroup>,
        elements: (_) Punctuated<Component>,
    }

    #[delimiter(brace)]
    struct List {
        muted: (_) Vec<MutedGroup>,
        elements: (_) Punctuated<Component>,
    }

    #[delimiter(angle_bar)]
    struct AngleBarList {
        muted: (_) Vec<MutedGroup>,
        elements: (_) Punctuated<Component>,
    }

    #[delimiter(parenthesis)]
    struct Sequence {
        muted: (_) Vec<MutedGroup>,
        elements: (_) Punctuated<Component>,
    }

    #[delimiter(parenthesis)]
    struct ParenthesizedExpression {
        muted: (_) Vec<MutedGroup>,
        value: (_) Expr?,
    }

    #[delimiter(string)]
    struct StringLiteral {
        elements: (_) Vec<StringElement>,
    }

    #[delimiter(raw_string)]
    struct RawStringLiteral {
        elements: (_) Vec<RawStringElement>,
    }

    struct BinaryExpression {
        left: Expr,
        operator: BinaryOperator,
        right: Expr,
    }

    #[cst(kind = adjacent_expression)]
    struct AdjacentExpression {
        left: Expr,
        right: Expr,
    }

    struct PrefixExpression {
        operator: PrefixOperator,
        operand: Expr,
    }

    struct PostfixExpression {
        operand: Expr,
        operator: PostfixOperator,
    }

    struct LambdaExpression {
        parameters: LambdaParameters,
        operator: Token![->],
        body: Expr,
    }

    #[cst(kind = assignment)]
    struct Assignment {
        left: Symbol,
        operator: Token![=],
        right: Expr,
    }

    #[cst(kind = local_assignment)]
    struct LocalAssignment {
        left: Symbol,
        operator: Token![:=],
        right: Expr,
    }

    struct BinaryAssignment {
        left: BinaryExpression,
        operator: Token![=],
        right: Expr,
    }

    struct BinaryInstallation {
        left: BinaryExpression,
        operator: Token![:=],
        right: Expr,
    }

    struct PrefixAssignment {
        left: PrefixExpression,
        operator: Token![=],
        right: Expr,
    }

    struct PrefixInstallation {
        left: PrefixExpression,
        operator: Token![:=],
        right: Expr,
    }

    struct PostfixAssignment {
        left: PostfixExpression,
        operator: Token![=],
        right: Expr,
    }

    struct PostfixInstallation {
        left: PostfixExpression,
        operator: Token![:=],
        right: Expr,
    }

    // grammar.js gives the binding pack its own CST field name,
    // `binding_pack`, instead of the `left` used by every other assignment
    // form.
    struct StructuredBinding {
        binding_pack: BindingPack,
        operator: Token![=],
        right: Expr,
    }

    struct LocalStructuredBinding {
        binding_pack: BindingPack,
        operator: Token![:=],
        right: Expr,
    }

    struct EvaluatedAssignment {
        left: Expr,
        operator: Token![<-],
        right: Expr,
    }

    #[cst(kind = option)]
    struct OptionExpression {
        left: Expr,
        operator: Token![=>],
        right: Expr,
    }

    struct ThenClause {
        keyword: (_) Token![then],
        value: (_) Expr,
    }

    struct ElseClause {
        keyword: (_) Token![else],
        value: (_) Expr,
    }

    struct IfStatement {
        keyword: (_) Token![if],
        condition: Expr,
        then_clause: (_) ThenClause,
        else_clause: (_) ElseClause?,
    }

    struct LoopBody {
        list_keyword: (_) Token![list]?,
        listed_value: Expr?,
        do_keyword: (_) Token![do]?,
        ignored_value: Expr?,
    }

    struct IterationRange {
        in_keyword: (_) Token![in]?,
        iterated_collection: Expr?,
        from_keyword: (_) Token![from]?,
        range_start: Expr?,
        to_keyword: (_) Token![to]?,
        range_end: Expr?,
    }

    struct ForLoop {
        keyword: (_) Token![for],
        variable: Symbol,
        range: (_) IterationRange?,
        when_keyword: (_) Token![when]?,
        filter: Expr?,
        body: (_) LoopBody,
    }

    struct WhileLoop {
        keyword: (_) Token![while],
        condition: Expr,
        body: (_) LoopBody,
    }

    struct NewStatement {
        keyword: (_) Token![new],
        class: Expr,
        of_keyword: (_) Token![of]?,
        parent: Expr?,
        from_keyword: (_) Token![from]?,
        instance: Expr?,
    }

    enum DebugKeyword {
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

    struct DebugClause {
        keyword: DebugKeyword,
        value: (_) Expr?,
    }

    struct BreakStatement {
        keyword: (_) Token![break],
        value: (_) Expr?,
    }

    struct ContinueStatement {
        keyword: (_) Token![continue],
        value: (_) Expr?,
    }

    struct ReturnStatement {
        keyword: (_) Token![return],
        value: (_) Expr?,
    }

    struct CatchStatement {
        keyword: (_) Token![catch],
        value: (_) Expr,
    }

    struct ThrowStatement {
        keyword: (_) Token![throw],
        value: (_) Expr,
    }

    struct TrapStatement {
        keyword: (_) Token![trap],
        value: (_) Expr,
    }

    struct ExceptClause {
        keyword: (_) Token![except],
        exception: Symbol,
        do_keyword: (_) Token![do],
        value: (_) Expr,
    }

    enum TryFallback {
        ExceptClause,
        ElseClause,
    }

    struct TryStatement {
        keyword: (_) Token![try],
        value: (_) Expr,
        then_clause: (_) ThenClause?,
        fallback: TryFallback?,
    }

    enum QuoteSpecifier {
        Token![symbol],
        Token![local],
        Token![global],
        Token![threadVariable],
        Token![threadLocal],
    }

    struct QuoteExpression {
        specifier: (_) QuoteSpecifier,
        token: Symbol,
    }

    enum AnyCell {
        ExpressionCell,
        SequenceCell,
        MutedCell,
    }

    enum Component {
        EmptyComponent,
        Expr,
    }

    enum SequenceElement {
        Component,
        MutedGroup,
    }

    enum StringElement {
        EscapeSequence,
        StringContent,
    }

    enum RawStringElement {
        EscapeSequence,
        RawStringContent,
    }

    enum Collection {
        ParenthesizedExpression,
        Sequence,
        List,
        Array,
        AngleBarList,
    }

    enum BindingPack {
        ParenthesizedExpression,
        Sequence,
        List,
        Array,
        AngleBarList,
    }

    enum LambdaParameters {
        Symbol,
        ParenthesizedExpression,
        Sequence,
        List,
        Array,
        AngleBarList,
    }

    enum OperatorExpr {
        AdjacentExpression,
        BinaryExpression,
        PrefixExpression,
        PostfixExpression,
    }

    enum AssignmentExpr {
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

    enum Expr {
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

    enum SyntaxNode {
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

/// An expression evaluated in global scope.
///
/// Rust Analyzer presents the implementations below as the concrete type
/// hierarchy for cells.
pub trait CellNode: ::m2_syn::Spanned + ::m2_syn::ToTokens {
    /// Returns the global cell delimiter represented by this typed node.
    fn cell_delimiter(&self) -> ::m2_syn::DelimiterKind;
}

impl CellNode for AnyCell {
    fn cell_delimiter(&self) -> ::m2_syn::DelimiterKind {
        match self {
            Self::ExpressionCell(_) | Self::SequenceCell(_) => ::m2_syn::DelimiterKind::Empty,
            Self::MutedCell(_) => ::m2_syn::DelimiterKind::Semicolon,
        }
    }
}

impl CellNode for ExpressionCell {
    fn cell_delimiter(&self) -> ::m2_syn::DelimiterKind {
        ::m2_syn::DelimiterKind::Empty
    }
}

impl CellNode for SequenceCell {
    fn cell_delimiter(&self) -> ::m2_syn::DelimiterKind {
        ::m2_syn::DelimiterKind::Empty
    }
}

impl CellNode for MutedCell {
    fn cell_delimiter(&self) -> ::m2_syn::DelimiterKind {
        ::m2_syn::DelimiterKind::Semicolon
    }
}

fn append_cell_node<T: CellNode>(node: &T, output: &mut ::m2_syn::CellStream) {
    let kind = node.cell_delimiter();
    let mut stream = ::m2_syn::ToTokens::to_token_stream(node);
    if kind == ::m2_syn::DelimiterKind::Semicolon {
        stream.pop().expect("a muted cell emits its semicolon last");
    }
    output.push(::m2_syn::CellBlock::new(
        ::m2_syn::Delimiter::new(kind, ::m2_syn::Spanned::span(node)),
        stream,
    ));
}

macro_rules! impl_to_cells_for_cell_node {
    ($($ty:ty),* $(,)?) => {
        $(
            impl ::m2_syn::ToCells for $ty {
                fn to_cells(&self, output: &mut ::m2_syn::CellStream) {
                    append_cell_node(self, output);
                }
            }
        )*
    }
}

impl_to_cells_for_cell_node!(AnyCell, ExpressionCell, SequenceCell, MutedCell);

impl ::m2_syn::ToCells for SourceFile {
    fn to_cells(&self, output: &mut ::m2_syn::CellStream) {
        ::m2_syn::ToCells::to_cells(&self.elements, output);
    }
}

impl ::m2_syn::ToCells for Expr {
    fn to_cells(&self, output: &mut ::m2_syn::CellStream) {
        ::m2_syn::ToCells::to_cells(&::m2_syn::ToTokens::to_token_stream(self), output);
    }
}

macro_rules! impl_to_cells_for_expr_node {
    ($($ty:ty),* $(,)?) => {
        $(
            impl ::m2_syn::ToCells for $ty {
                fn to_cells(&self, output: &mut ::m2_syn::CellStream) {
                    ::m2_syn::ToCells::to_cells(
                        &::m2_syn::ToTokens::to_token_stream(self),
                        output,
                    );
                }
            }
        )*
    };
}

impl_to_cells_for_expr_node!(
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
);

impl ::m2_syn::ToCells for SyntaxNode {
    fn to_cells(&self, output: &mut ::m2_syn::CellStream) {
        match self {
            Self::SourceFile(node) => ::m2_syn::ToCells::to_cells(node, output),
            Self::AnyCell(node) => ::m2_syn::ToCells::to_cells(node, output),
            Self::Expr(node) => ::m2_syn::ToCells::to_cells(node, output),
        }
    }
}
