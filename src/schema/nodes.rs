syntax! {
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

    // Each row carries its parsing precedence, then only what its role needs:
    // `L`/`R` for a binary operator's associativity, and a unary strength
    // only where it differs from the precedence.
    tokens {
        [->] [<-] [=] [=>]              { infix }               (14, R)
        [>>]                            { bin, aug }            (14, R)
        [<<]                            { bin, pref, aug }      (18, L)
        [|-]                            { bin, pref, aug }      (20, L)
        [===>]                          { bin, aug }            (22, R)
        [<===]                          { bin, pref }           (22, R)
        [<==>]                          { bin, aug }            (24, R)
        [==>]                           { bin, aug }            (26, R)
        [<==]                           { bin, pref }           (26, R)
        [or]                            { bin }                 (28, R)
        [??]                            { bin, pref, aug }      (28, R)
        [xor]                           { bin }                 (30, R)
        [and]                           { bin }                 (32, R)
        [not]                           { pref }                (34)
        [!=] [=!=] [==] [===]           { bin }                 (36, R)
        [<] [<=] [>] [>=] [?]           { bin, pref }           (36, R)
        [~]                             { bin, pref, aug }      (36, R)
        [||]                            { bin, aug }            (38, L)
        [:]                             { bin, aug }            (40, R)
        [|]                             { bin, aug }            (42, L)
        [^^]                            { bin, aug }            (44, L)
        [&]                             { bin, aug }            (46, L)
        [..] [..<]                      { bin, aug }            (48, L)
        [++]                            { bin, aug }            (50, L)
        [+] [-]                         { bin, pref, aug }      (50, L)
        ["·"]                           { bin, aug }            (52, L)
        [**] ["⊠"] ["⧢"]                { bin, aug }            (54, L)
        [%] [/] [/ /]                   { bin, aug }            (58, L)
        [*]                             { bin, pref, aug }      (58, L)
        ["\\"] ["\\\\"]                 { bin, aug }            (58, R)
        [@]                             { bin, aug }            (60, R)
        [(*)]                           { post }                (64)
        [@@] [@@?]                      { bin, aug }            (66, L)
        [^*] [^~] [_*] [_~]             { post }                (68)
        [#?] [.] [.?]                   { bin }                 (70, L)
        [^<] [^<=] [^>] [^>=]           { bin }                 (70, L)
        [_<] [_<=] [_>] [_>=]           { bin }                 (70, L)
        [^] [^**] [_] [|_]              { bin, aug }            (70, L)
        [#]                             { bin, pref }           (70, L, 61)
        [!] [^!] [_!]                   { post }                (72)


        [else] [then] [do] [list] [except]                      (12)
        [when] [of] [in] [from] [to]                            (16)
        [break] [continue] [return] [catch] [throw] [trap]
        [if] [while] [try]
        [TEST] [time] [timing] [elapsedTime] [elapsedTiming]
        [breakpoint] [profile] [shield] [step] [finish]
            (PREC_APPLICATION, PREC_CONTROL)
        [for] [new]
            (PREC_APPLICATION, PREC_LOOP_CLAUSE)
        [symbol] [global] [local] [threadLocal] [threadVariable]
            (PREC_APPLICATION, PREC_QUOTE)
        [;] (PREC_SEMICOLON)
        [,] (PREC_COMMA)
    }

    #[operator(prefix)]
    enum PrefixOperator {}

    #[operator(binary)]
    enum BinaryOperator {}

    #[operator(postfix)]
    enum PostfixOperator {}

    // Leaf nodes retain their source text. Their concrete names are obtained by
    // converting the Rust name to snake_case, matching grammar.js.
    #[leaf(qualified)]
    struct Symbol;
    #[leaf(empty)]
    struct Empty;
    #[leaf(adj)]
    struct Adj;
    #[leaf(eoc)]
    struct EOC;
    #[leaf(eof)]
    struct EOF;
    struct EscapeSequence;
    #[leaf(float)]
    struct FloatLiteral;
    #[leaf(integer)]
    struct IntegerLiteral;
    struct RawStringContent;
    struct StringContent;

    // `(_)` marks a field addressed positionally rather than by a CST field
    // name. The macro may still give it an artificial Rust name, but that
    // name is not a CST field. `Lines<T>` is `Vec<T>` printed one per line.
    #[comments]
    #[cell_stream]
    struct SourceFile {
        cells: Lines<Cell>,
        eof: EOF
    }

    #[comments]
    struct Cell {
        content: ExprPack,
        semicolon: Token![;]?,
        eoc: EOC,
    }

    enum Component {
        Expr,
        Empty
    }

    struct ExprPack {
        pack: Punctuated<Component>
    }

    struct Statement {
        content: ExprPack,
        semicolon: Token![;]
    }


    #[delimiter(bracket)]
    struct Array {
        statements: Vec<Statement>,
        elements: ExprPack?
    }

    #[delimiter(brace)]
    struct List {
        statements: Vec<Statement>,
        elements: ExprPack?
    }

    #[delimiter(angle_bar)]
    struct AngleBarList {
        statements: Vec<Statement>,
        elements: ExprPack?
    }

    #[delimiter(parenthesis)]
    struct Sequence {
        statements: Vec<Statement>,
        elements: ExprPack?
    }

    #[delimiter(parenthesis)]
    struct Holder {
        muted: Vec<Statement>,
        value: Expr?,
    }

    #[literal(string)]
    struct StringLiteral {
        elements: Vec<StringElement>,
    }

    #[literal(raw_string)]
    struct RawStringLiteral {
        elements: Vec<RawStringElement>,
    }

    struct BinaryExpr {
        left_operand: Expr,
        operator: BinaryOperator,
        right_operand: Expr,
    }

    struct AdjacentExpr {
        left_operand: Expr,
        operator: Adj,
        right_operand: Expr,
    }

    struct PrefixExpr {
        operator: PrefixOperator,
        operand: Expr,
    }

    struct PostfixExpr {
        operand: Expr,
        operator: PostfixOperator,
    }



    enum LambdaParameters {
        Variadic(Symbol),
        FixedArity(Delimited<Punctuated<Symbol>?>)
    }

    struct LambdaExpression {
        parameters: LambdaParameters,
        operator: Token![->],
        body: Expr,
    }

    struct GlobalBinding {
        variable: Symbol,
        operator: Token![=],
        value: Expr,
    }

    struct LocalBinding {
        variable: Symbol,
        operator: Token![:=],
        value: Expr,
    }


    struct TypeAnnotation {
        typical_value: Expr,
        operator: Token![=>]
    }


    struct OperatorAssignment {
        argument_expr: OperatorExpr,
        operator: Token![=],
        value_expr: Expr,
    }


    struct OperatorInstallation {
        domain_expr: OperatorExpr,
        assign_op: Token![:=],
        typical_value: TypeAnnotation?,
        function: Expr,
    }

    struct MethodInstallation {
        method: Expr,
        signature: Expr,
        assign_op: Token![:=],
        typical_value: TypeAnnotation?,
        function: Expr,
    }

    struct NewInstallation {
        new_statement: NewStatement,
        operator: Token![:=],
        function: Expr,
    }


    enum Installation {
        MethodInstallation,
        OperatorInstallation,
        NewInstallation,
    }

    enum AssignmentPackComponent {
        AssignmentPack,
        Empty,
        Symbol,
        OperatorExpr,
    }

    struct AssignmentPack*(
        Delimited<Punctuated<AssignmentPackComponent>?>)


    struct GlobalParallelAssignment {
        argument_pack: AssignmentPack,
        operator: Token![=],
        value: Expr,
    }

    struct LocalParallelAssignment {
        argument_pack: AssignmentPack,
        operator: Token![:=],
        value: Expr,
    }



    enum SimpleBinding {
        GlobalBinding,
        LocalBinding,
    }

    enum ParallelAssignment {
        GlobalParallelAssignment,
        LocalParallelAssignment,
    }

    struct EvaluatedAssignment {
        evaluated_domain: Expr,
        operator: Token![<-],
        value: Expr,
    }

    struct OptionExpression {
        key: Expr,
        operator: Token![=>],
        value: Expr,
    }

    enum Assignment {
        SimpleBinding,
        ParallelAssignment,
        EvaluatedAssignment,
        Installation,
        OperatorAssignment,
    }



    struct ThenClause {
        then_token: Token![then],
        expr: Expr,
    }

    struct ElseClause {
        else_token: Token![else],
        expr: Expr
    }

    struct IfStatement {
        if_token: Token![if],
        condition: Expr,
        then_clause: ThenClause,
        else_clause: ElseClause?,
    }

    struct RangeBoundFrom {
        from_token: Token![from],
        value: Expr,
    }

    struct RangeBoundTo {
        to_token: Token![to],
        value: Expr,
    }

    struct IterRange {
        lower_bound: RangeBoundFrom?,
        upper_bound: RangeBoundTo?,
    }

    struct IterIn {
        in_token: Token![in],
        collection: Expr,
    }

    struct WhenCondition {
        when_token: Token![when],
        condition: Expr,
    }

    struct ListStep {
        list_token: Token![list],
        expr: Expr,
    }

    struct DoStep {
        do_token: Token![do],
        statement: Expr,
    }

    struct LoopBody {
        list_step: ListStep?,
        do_step: DoStep?,
    }

    enum IterationDomain {
        IterRange,
        IterIn,
    }

    struct ForLoop {
        for_token: Token![for],
        variable: Symbol,
        iteration_domain: IterationDomain?,
        when_condition: WhenCondition?,
        body: LoopBody,
    }

    struct WhileLoop {
        while_token: Token![while],
        condition: Expr,
        body: LoopBody,
    }

    struct OfClass {
        of_token: Token![of],
        parent: Expr,
    }

    struct FromInstance {
        from_token: Token![from],
        instance: Expr,
    }

    struct NewStatement {
        new_token: Token![new],
        class: Expr,
        parent_class: OfClass?,
        instance: FromInstance?,
    }


    enum ProfilingModifier {
        Token![elapsedTime],
        Token![elapsedTiming],
        Token![profile],
        Token![time],
        Token![timing],
    }


    struct ProfilingExpr {
        profiling_modifier: ProfilingModifier,
        value: Expr,
    }

    enum DebugKeyword {
        Token![TEST],
        Token![step],
        Token![finish],
        Token![shield],
        Token![breakpoint],
    }

    struct DebugExpr {
        debug_token: DebugKeyword,
        value: Expr?,
    }


    struct BreakExpr {
        break_token: Token![break],
        value: Expr?,
    }

    struct ContinueExpr {
        continue_token: Token![continue],
        value: Expr?,
    }

    struct ReturnExpr {
        return_token: Token![return],
        value: Expr?,
    }

    enum ControlFlow {
        BreakExpr,
        ContinueExpr,
        ReturnExpr,
    }

    struct Catch {
        catch_token: Token![catch],
        value: Expr,
    }

    struct Throw {
        throw_token: Token![throw],
        value: Expr,
    }

    struct Trap {
        trap_token: Token![trap],
        value: Expr,
    }

    struct ExceptDo {
        except_token: Token![except],
        exception: Symbol,
        do_token: Token![do],
        value: Expr,
    }

    enum  TryFallback {
        ElseClause,
        ExceptDo,
    }

    struct TryStatement {
        try_token: Token![try],
        value: Expr,
        on_success: ThenClause?,
        fallback: TryFallback?
    }

    enum QuoteSpecifier {
        Token![symbol],
        Token![local],
        Token![global],
        Token![threadVariable],
        Token![threadLocal],
    }

    struct QuoteExpression {
        specifier: QuoteSpecifier,
        symbol: Symbol,
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
        Sequence,
        List,
        Array,
        AngleBarList,
    }

    enum OperatorExpr {
        AdjacentExpr,
        BinaryExpr,
        PrefixExpr,
        PostfixExpr,
    }

    enum Literal {
        FloatLiteral,
        IntegerLiteral,
        RawStringLiteral,
        StringLiteral,
    }

    enum Expr* {
        Literal,
        Collection,
        Holder,
        OperatorExpr,
        Assignment,
        OptionExpression,
        LambdaExpression,
        IfStatement,
        ForLoop,
        WhileLoop,
        TryStatement,
        QuoteExpression,
        NewStatement,
        ControlFlow,
        Catch,
        Throw,
        Trap,
        DebugExpr,
        ProfilingExpr,
        Symbol,
    }
}
