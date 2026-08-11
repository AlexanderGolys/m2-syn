use crate::syntax;

syntax! {
    tokens {
        BngEql [!=],
        Hsh [#],
        HshQst [#?],
        Mod [%],
        ModEql [%=],
        Amp [&],
        AmpEql [&=],
        Mul [*],
        MulMul [**],
        MulMulEql [**=],
        MulEql [*=],
        Add [+],
        AddAdd [++],
        AddAddEql [++=],
        AddEql [+=],
        Cma [,],
        Sub [-],
        SubEql [-=],
        SubGtr [->],
        Dot [.],
        DotDot [..],
        DotDotLss [..<],
        DotDotLssEql [..<=],
        DotDotEql [..=],
        DotQst [.?],
        Div [/],
        DivDiv ["//"],
        DivDivEql ["//="],
        DivEql [/=],
        Col [:],
        ColEql [:=],
        Scl [;],
        Lss [<],
        LssSub [<-],
        LssLss [<<],
        LssLssEql [<<=],
        LssEql [<=],
        LssEqlEql [<==],
        LssEqlEqlEql [<===],
        LssEqlEqlGtr [<==>],
        LssEqlEqlGtrEql [<==>=],
        Eql [=],
        EqlBngEql [=!=],
        EqlEql [==],
        EqlEqlEql [===],
        EqlEqlEqlGtr [===>],
        EqlEqlEqlGtrEql [===>=],
        EqlEqlGtr [==>],
        EqlEqlGtrEql [==>=],
        EqlGtr [=>],
        Gtr [>],
        GtrEql [>=],
        GtrGtr [>>],
        GtrGtrEql [>>=],
        Qst [?],
        QstQst [??],
        QstQstEql [??=],
        Ats [@],
        AtsEql [@=],
        AtsAts [@@],
        AtsAtsEql [@@=],
        AtsAtsQst [@@?],
        AtsAtsQstEql [@@?=],
        Bsl ["\\"],
        BslEql ["\\="],
        BslBsl ["\\\\"],
        BslBslEql ["\\\\="],
        Crt [^],
        CrtMulMul [^**],
        CrtMulMulEql [^**=],
        CrtLss [^<],
        CrtLssEql [^<=],
        CrtEql [^=],
        CrtGtr [^>],
        CrtGtrEql [^>=],
        CrtCrt [^^],
        CrtCrtEql [^^=],
        Und [_],
        UndLss [_<],
        UndLssEql [_<=],
        UndEql [_=],
        UndGtr [_>],
        UndGtrEql [_>=],
        Pip [|],
        PipSub [|-],
        PipSubEql [|-=],
        PipEql [|=],
        PipUnd [|_],
        PipUndEql [|_=],
        PipPip [||],
        PipPipEql [||=],
        Tld [~],
        TldEql [~=],
        Mdt ["·"],
        MdtEql ["·="],
        Bxp ["⊠"],
        BxpEql ["⊠="],
        Shf ["⧢"],
        ShfEql ["⧢="],
        And [and],
        Or [or],
        Spc [SPACE],
        Xor [xor],
        Bng [!],
        LprMulRpr ["(*)"],
        CrtBng [^!],
        CrtMul [^*],
        CrtTld [^~],
        UndBng [_!],
        UndMul [_*],
        UndTld [_~],
        Not [not],
        Test [TEST],
        Breakpoint [breakpoint],
        ElapsedTime [elapsedTime],
        ElapsedTiming [elapsedTiming],
        Finish [finish],
        Profile [profile],
        Shield [shield],
        Step [step],
        Time [time],
        Timing [timing],
        SymbolKeyword [symbol],
        LocalKeyword [local],
        GlobalKeyword [global],
        ThreadVariableKeyword [threadVariable],
        ThreadLocalKeyword [threadLocal],
        Eoc [EOC],
        Eof [EOF]
    }

    pub struct Symbol;
    pub struct Keyword;
    pub struct BlockComment;
    pub struct LineComment;
    pub struct EmptyComponent;
    pub struct EscapeSequence;
    pub struct FloatLiteral;
    pub struct IntegerLiteral;
    pub struct RawStringContent;
    pub struct StringContent;

    pub struct SourceFile {
        #[syntax(unfielded)]
        pub elements: Vec<SourceElement>,
    }

    pub struct Cell {
        #[syntax(unfielded)]
        pub value: CellValue,
    }

    pub struct NakedSequence {
        #[syntax(unfielded)]
        pub elements: Vec<Component>,
    }

    pub struct Muted {
        #[syntax(unfielded)]
        pub elements: Vec<Component>,
        pub _terminator: Scl,
    }

    pub struct Array {
        #[syntax(unfielded)]
        pub elements: Vec<SequenceElement>,
    }

    pub struct List {
        #[syntax(unfielded)]
        pub elements: Vec<SequenceElement>,
    }

    pub struct AngleBarList {
        #[syntax(unfielded)]
        pub elements: Vec<SequenceElement>,
    }

    pub struct Sequence {
        #[syntax(unfielded)]
        pub elements: Vec<SequenceElement>,
    }

    pub struct ParenthesizedExpression {
        #[syntax(unfielded)]
        pub elements: Vec<ParenthesizedElement>,
    }

    pub struct StringLiteral {
        #[syntax(unfielded)]
        pub elements: Vec<StringElement>,
    }

    pub struct RawStringLiteral {
        #[syntax(unfielded)]
        pub elements: Vec<RawStringElement>,
    }

    pub struct BinaryExpression {
        pub left: Expr,
        pub operator: BinaryOperator,
        pub right: Expr,
    }

    pub struct PrefixExpression {
        pub operator: PrefixOperator,
        pub operand: Expr,
    }

    pub struct PostfixExpression {
        pub operand: Expr,
        pub operator: PostfixOperator,
    }

    pub struct LambdaExpression {
        pub parameters: LambdaParameters,
        pub operator: SubGtr,
        pub body: Expr,
    }

    #[syntax(kind = "assignment")]
    pub struct SymbolAssignment {
        pub left: Symbol,
        pub operator: Eql,
        pub right: Expr,
    }

    #[syntax(kind = "local_assignment")]
    pub struct LocalSymbolAssignment {
        pub left: Symbol,
        pub operator: ColEql,
        pub right: Expr,
    }

    pub struct BinaryAssignment {
        pub left: BinaryExpression,
        pub operator: Eql,
        pub right: Expr,
    }

    pub struct BinaryInstallation {
        pub left: BinaryExpression,
        pub operator: ColEql,
        pub right: Expr,
    }

    pub struct PrefixAssignment {
        pub left: PrefixExpression,
        pub operator: Eql,
        pub right: Expr,
    }

    pub struct PrefixInstallation {
        pub left: PrefixExpression,
        pub operator: ColEql,
        pub right: Expr,
    }

    pub struct PostfixAssignment {
        pub left: PostfixExpression,
        pub operator: Eql,
        pub right: Expr,
    }

    pub struct PostfixInstallation {
        pub left: PostfixExpression,
        pub operator: ColEql,
        pub right: Expr,
    }

    #[syntax(kind = "structured_binding")]
    pub struct StructuredAssignment {
        pub left: BindingPack,
        pub operator: Eql,
        pub right: Expr,
    }

    #[syntax(kind = "local_structured_binding")]
    pub struct LocalStructuredAssignment {
        pub left: BindingPack,
        pub operator: ColEql,
        pub right: Expr,
    }

    pub struct EvaluatedAssignment {
        pub left: Expr,
        pub operator: LssSub,
        pub right: Expr,
    }

    #[syntax(kind = "option")]
    pub struct OptionExpression {
        pub left: Expr,
        pub operator: EqlGtr,
        pub right: Expr,
    }

    pub struct LoopBody {
        pub ignored_value: Option<Expr>,
        pub listed_value: Option<Expr>,
    }

    pub struct IterationRange {
        pub iterated_collection: Option<Expr>,
        pub range_end: Option<Expr>,
        pub range_start: Option<Expr>,
    }

    pub struct ForLoop {
        pub variable: Symbol,
        #[syntax(unfielded)]
        pub range: Option<IterationRange>,
        pub filter: Option<Expr>,
        #[syntax(unfielded)]
        pub body: LoopBody,
    }

    pub struct WhileLoop {
        pub condition: Expr,
        pub filter: Option<Expr>,
        #[syntax(unfielded)]
        pub body: LoopBody,
    }

    pub struct ThenClause {
        #[syntax(unfielded)]
        pub value: Expr,
    }

    pub struct ElseClause {
        #[syntax(unfielded)]
        pub value: Expr,
    }

    pub struct IfStatement {
        pub condition: Expr,
        #[syntax(unfielded)]
        pub then_clause: ThenClause,
        #[syntax(unfielded)]
        pub else_clause: Option<ElseClause>,
    }

    pub struct ExceptClause {
        pub exception: Symbol,
        #[syntax(unfielded)]
        pub value: Expr,
    }

    pub struct TryStatement {
        #[syntax(unfielded)]
        pub value: Expr,
        #[syntax(unfielded)]
        pub then_clause: Option<ThenClause>,
        pub fallback: Option<TryFallback>,
    }

    pub struct NewStatement {
        pub class: Expr,
        pub instance: Option<Expr>,
        pub parent: Option<Expr>,
    }

    pub struct DebugClause {
        pub keyword: DebugKeyword,
        #[syntax(unfielded)]
        pub value: Option<Expr>,
    }

    pub struct BreakStatement {
        #[syntax(unfielded)]
        pub value: Option<Expr>,
    }

    pub struct ContinueStatement {
        #[syntax(unfielded)]
        pub value: Option<Expr>,
    }

    pub struct ReturnStatement {
        #[syntax(unfielded)]
        pub value: Option<Expr>,
    }

    pub struct CatchStatement {
        #[syntax(unfielded)]
        pub value: Expr,
    }

    pub struct ThrowStatement {
        #[syntax(unfielded)]
        pub value: Expr,
    }

    pub struct TrapStatement {
        #[syntax(unfielded)]
        pub value: Expr,
    }

    pub struct QuoteExpression {
        pub _specifier: QuoteSpecifier,
        pub token: QuoteValue,
    }

    pub enum SourceElement {
        Cell(Cell),
        Muted(Muted),
    }

    pub enum CellValue {
        Expression(Expr),
        Sequence(NakedSequence),
    }

    pub enum Component {
        Empty(EmptyComponent),
        Expression(Expr),
    }

    pub enum SequenceElement {
        Component(Component),
        Muted(Muted),
    }

    pub enum ParenthesizedElement {
        Expression(Expr),
        Muted(Muted),
    }

    pub enum StringElement {
        Escape(EscapeSequence),
        Content(StringContent),
    }

    pub enum RawStringElement {
        Escape(EscapeSequence),
        Content(RawStringContent),
    }

    pub enum BindingPack {
        Parenthesized(ParenthesizedExpression),
        Sequence(Sequence),
        List(List),
        Array(Array),
        AngleBarList(AngleBarList),
    }

    pub enum LambdaParameters {
        Symbol(Symbol),
        Parenthesized(ParenthesizedExpression),
        Sequence(Sequence),
        List(List),
        Array(Array),
        AngleBarList(AngleBarList),
    }

    pub enum TryFallback {
        Except(ExceptClause),
        Else(ElseClause),
    }

    pub enum QuoteValue {
        Keyword(Keyword),
        Symbol(Symbol),
    }

    pub enum QuoteSpecifier {
        Symbol(SymbolKeyword),
        Local(LocalKeyword),
        Global(GlobalKeyword),
        ThreadVariable(ThreadVariableKeyword),
        ThreadLocal(ThreadLocalKeyword),
    }

    pub enum DebugKeyword {
        Test(Test),
        Breakpoint(Breakpoint),
        ElapsedTime(ElapsedTime),
        ElapsedTiming(ElapsedTiming),
        Finish(Finish),
        Profile(Profile),
        Shield(Shield),
        Step(Step),
        Time(Time),
        Timing(Timing),
    }

    pub enum Assignment {
        Symbol(SymbolAssignment),
        LocalSymbol(LocalSymbolAssignment),
        Binary(BinaryAssignment),
        BinaryInstallation(BinaryInstallation),
        Prefix(PrefixAssignment),
        PrefixInstallation(PrefixInstallation),
        Postfix(PostfixAssignment),
        PostfixInstallation(PostfixInstallation),
        Structured(StructuredAssignment),
        LocalStructured(LocalStructuredAssignment),
        Evaluated(EvaluatedAssignment),
    }

    pub enum Expr {
        AngleBarList(AngleBarList),
        Array(Array),
        Assignment(Assignment),
        Binary(BinaryExpression),
        Break(BreakStatement),
        Catch(CatchStatement),
        Continue(ContinueStatement),
        Debug(DebugClause),
        Float(FloatLiteral),
        For(ForLoop),
        If(IfStatement),
        Integer(IntegerLiteral),
        Lambda(LambdaExpression),
        List(List),
        New(NewStatement),
        Option(OptionExpression),
        Parenthesized(ParenthesizedExpression),
        Postfix(PostfixExpression),
        Prefix(PrefixExpression),
        Quote(QuoteExpression),
        RawString(RawStringLiteral),
        Return(ReturnStatement),
        Sequence(Sequence),
        String(StringLiteral),
        Symbol(Symbol),
        Throw(ThrowStatement),
        Trap(TrapStatement),
        Try(TryStatement),
        While(WhileLoop),
    }

    pub enum BinaryOperator {
        BngEql(BngEql),
        Hsh(Hsh),
        HshQst(HshQst),
        Mod(Mod),
        ModEql(ModEql),
        Amp(Amp),
        AmpEql(AmpEql),
        Mul(Mul),
        MulMul(MulMul),
        MulMulEql(MulMulEql),
        MulEql(MulEql),
        Add(Add),
        AddAdd(AddAdd),
        AddAddEql(AddAddEql),
        AddEql(AddEql),
        Sub(Sub),
        SubEql(SubEql),
        SubGtr(SubGtr),
        Dot(Dot),
        DotDot(DotDot),
        DotDotLss(DotDotLss),
        DotDotLssEql(DotDotLssEql),
        DotDotEql(DotDotEql),
        DotQst(DotQst),
        Div(Div),
        DivDiv(DivDiv),
        DivDivEql(DivDivEql),
        DivEql(DivEql),
        Col(Col),
        ColEql(ColEql),
        Lss(Lss),
        LssSub(LssSub),
        LssLss(LssLss),
        LssLssEql(LssLssEql),
        LssEql(LssEql),
        LssEqlEql(LssEqlEql),
        LssEqlEqlEql(LssEqlEqlEql),
        LssEqlEqlGtr(LssEqlEqlGtr),
        LssEqlEqlGtrEql(LssEqlEqlGtrEql),
        Eql(Eql),
        EqlBngEql(EqlBngEql),
        EqlEql(EqlEql),
        EqlEqlEql(EqlEqlEql),
        EqlEqlEqlGtr(EqlEqlEqlGtr),
        EqlEqlEqlGtrEql(EqlEqlEqlGtrEql),
        EqlEqlGtr(EqlEqlGtr),
        EqlEqlGtrEql(EqlEqlGtrEql),
        EqlGtr(EqlGtr),
        Gtr(Gtr),
        GtrEql(GtrEql),
        GtrGtr(GtrGtr),
        GtrGtrEql(GtrGtrEql),
        Qst(Qst),
        QstQst(QstQst),
        QstQstEql(QstQstEql),
        Ats(Ats),
        AtsEql(AtsEql),
        AtsAts(AtsAts),
        AtsAtsEql(AtsAtsEql),
        AtsAtsQst(AtsAtsQst),
        AtsAtsQstEql(AtsAtsQstEql),
        Bsl(Bsl),
        BslEql(BslEql),
        BslBsl(BslBsl),
        BslBslEql(BslBslEql),
        Crt(Crt),
        CrtMulMul(CrtMulMul),
        CrtMulMulEql(CrtMulMulEql),
        CrtLss(CrtLss),
        CrtLssEql(CrtLssEql),
        CrtEql(CrtEql),
        CrtGtr(CrtGtr),
        CrtGtrEql(CrtGtrEql),
        CrtCrt(CrtCrt),
        CrtCrtEql(CrtCrtEql),
        Und(Und),
        UndLss(UndLss),
        UndLssEql(UndLssEql),
        UndEql(UndEql),
        UndGtr(UndGtr),
        UndGtrEql(UndGtrEql),
        Pip(Pip),
        PipSub(PipSub),
        PipSubEql(PipSubEql),
        PipEql(PipEql),
        PipUnd(PipUnd),
        PipUndEql(PipUndEql),
        PipPip(PipPip),
        PipPipEql(PipPipEql),
        Tld(Tld),
        TldEql(TldEql),
        Mdt(Mdt),
        MdtEql(MdtEql),
        Bxp(Bxp),
        BxpEql(BxpEql),
        Shf(Shf),
        ShfEql(ShfEql),
        And(And),
        Or(Or),
        Spc(Spc),
        Xor(Xor),
    }

    pub enum PrefixOperator {
        Hsh(Hsh),
        Mul(Mul),
        Add(Add),
        Sub(Sub),
        Lss(Lss),
        LssLss(LssLss),
        LssEql(LssEql),
        LssEqlEql(LssEqlEql),
        LssEqlEqlEql(LssEqlEqlEql),
        Gtr(Gtr),
        GtrEql(GtrEql),
        Qst(Qst),
        QstQst(QstQst),
        PipSub(PipSub),
        Tld(Tld),
        Not(Not),
    }

    pub enum PostfixOperator {
        Bng(Bng),
        LprMulRpr(LprMulRpr),
        CrtBng(CrtBng),
        CrtMul(CrtMul),
        CrtTld(CrtTld),
        UndBng(UndBng),
        UndMul(UndMul),
        UndTld(UndTld),
    }

    pub enum SyntaxNode {
        SourceFile(SourceFile),
        Cell(Cell),
        Expr(Expr),
    }
}
