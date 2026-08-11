use crate::*;

fn write_joined<T>(values: &[T], separator: &str, output: &mut TokenStream)
where
    T: ToTokens,
{
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push_synthetic(separator);
        }
        value.to_tokens(output);
    }
}

fn write_infix<L, O, R>(left: &L, operator: &O, right: &R, output: &mut TokenStream)
where
    L: ToTokens + ?Sized,
    O: ToTokens + ?Sized,
    R: ToTokens + ?Sized,
{
    left.to_tokens(output);
    output.push_space();
    operator.to_tokens(output);
    output.push_space();
    right.to_tokens(output);
}

fn write_sequence_elements(values: &[SequenceElement], output: &mut TokenStream) {
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push_synthetic(match (&values[index - 1], value) {
                (SequenceElement::Component(_), SequenceElement::Component(_)) => ", ",
                _ => " ",
            });
        }
        value.to_tokens(output);
    }
}

fn write_parenthesized_elements(values: &[ParenthesizedElement], output: &mut TokenStream) {
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push_space();
        }
        value.to_tokens(output);
    }
}

impl ToTokens for SourceFile {
    fn to_tokens(&self, output: &mut TokenStream) {
        for (index, element) in self.elements.iter().enumerate() {
            if index != 0 {
                output.push_end_of_cell(Span::detached());
            }
            element.to_tokens(output);
        }
        output.push_end_of_file(Span::detached());
    }
}

impl ToTokens for Cell {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.value.to_tokens(output);
    }
}

impl ToTokens for NakedSequence {
    fn to_tokens(&self, output: &mut TokenStream) {
        write_joined(&self.elements, ", ", output);
    }
}

impl ToTokens for Muted {
    fn to_tokens(&self, output: &mut TokenStream) {
        write_joined(&self.elements, ", ", output);
        self._terminator.to_tokens(output);
    }
}

macro_rules! delimited_sequence {
    ($($node:ty => $delimiter:expr),* $(,)?) => {
        $(
            impl ToTokens for $node {
                fn to_tokens(&self, output: &mut TokenStream) {
                    let mut contents = TokenStream::new();
                    write_sequence_elements(&self.elements, &mut contents);
                    output.push_group($delimiter, contents, self.span());
                }
            }
        )*
    };
}

delimited_sequence! {
    Array => Delimiter::Bracket,
    List => Delimiter::Brace,
    AngleBarList => Delimiter::AngleBar,
    Sequence => Delimiter::Parenthesis,
}

impl ToTokens for ParenthesizedExpression {
    fn to_tokens(&self, output: &mut TokenStream) {
        let mut contents = TokenStream::new();
        write_parenthesized_elements(&self.elements, &mut contents);
        output.push_group(Delimiter::Parenthesis, contents, self.span());
    }
}

impl ToTokens for StringLiteral {
    fn to_tokens(&self, output: &mut TokenStream) {
        let mut contents = TokenStream::new();
        for element in &self.elements {
            element.to_tokens(&mut contents);
        }
        output.push_text(format!("\"{contents}\""), self.span());
    }
}

impl ToTokens for RawStringLiteral {
    fn to_tokens(&self, output: &mut TokenStream) {
        let mut contents = TokenStream::new();
        for element in &self.elements {
            element.to_tokens(&mut contents);
        }
        output.push_text(format!("///{contents}///"), self.span());
    }
}

macro_rules! infix_nodes {
    ($($node:ty),* $(,)?) => {
        $(
            impl ToTokens for $node {
                fn to_tokens(&self, output: &mut TokenStream) {
                    write_infix(&self.left, &self.operator, &self.right, output);
                }
            }
        )*
    };
}

infix_nodes! {
    BinaryExpression,
    SymbolAssignment,
    LocalSymbolAssignment,
    BinaryAssignment,
    BinaryInstallation,
    PrefixAssignment,
    PrefixInstallation,
    PostfixAssignment,
    PostfixInstallation,
    StructuredAssignment,
    LocalStructuredAssignment,
    EvaluatedAssignment,
    OptionExpression,
}

impl ToTokens for PrefixExpression {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.operator.to_tokens(output);
        if matches!(self.operator, PrefixOperator::Not(_)) {
            output.push_space();
        }
        self.operand.to_tokens(output);
    }
}

impl ToTokens for PostfixExpression {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.operand.to_tokens(output);
        self.operator.to_tokens(output);
    }
}

impl ToTokens for LambdaExpression {
    fn to_tokens(&self, output: &mut TokenStream) {
        write_infix(&self.parameters, &self.operator, &self.body, output);
    }
}

impl ToTokens for LoopBody {
    fn to_tokens(&self, output: &mut TokenStream) {
        if let Some(value) = &self.listed_value {
            output.push_synthetic("list ");
            value.to_tokens(output);
        }
        if let Some(value) = &self.ignored_value {
            if self.listed_value.is_some() {
                output.push_space();
            }
            output.push_synthetic("do ");
            value.to_tokens(output);
        }
    }
}

impl ToTokens for IterationRange {
    fn to_tokens(&self, output: &mut TokenStream) {
        if let Some(value) = &self.iterated_collection {
            output.push_synthetic("in ");
            value.to_tokens(output);
            return;
        }
        if let Some(value) = &self.range_start {
            output.push_synthetic("from ");
            value.to_tokens(output);
        }
        if let Some(value) = &self.range_end {
            if self.range_start.is_some() {
                output.push_space();
            }
            output.push_synthetic("to ");
            value.to_tokens(output);
        }
    }
}

impl ToTokens for ForLoop {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_synthetic("for ");
        self.variable.to_tokens(output);
        if let Some(range) = &self.range {
            output.push_space();
            range.to_tokens(output);
        }
        if let Some(filter) = &self.filter {
            output.push_synthetic(" when ");
            filter.to_tokens(output);
        }
        output.push_space();
        self.body.to_tokens(output);
    }
}

impl ToTokens for WhileLoop {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_synthetic("while ");
        self.condition.to_tokens(output);
        if let Some(filter) = &self.filter {
            output.push_synthetic(" when ");
            filter.to_tokens(output);
        }
        output.push_space();
        self.body.to_tokens(output);
    }
}

impl ToTokens for ThenClause {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_synthetic("then ");
        self.value.to_tokens(output);
    }
}

impl ToTokens for ElseClause {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_synthetic("else ");
        self.value.to_tokens(output);
    }
}

impl ToTokens for IfStatement {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_synthetic("if ");
        self.condition.to_tokens(output);
        output.push_space();
        self.then_clause.to_tokens(output);
        if let Some(clause) = &self.else_clause {
            output.push_space();
            clause.to_tokens(output);
        }
    }
}

impl ToTokens for ExceptClause {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_synthetic("except ");
        self.exception.to_tokens(output);
        output.push_synthetic(" do ");
        self.value.to_tokens(output);
    }
}

impl ToTokens for TryStatement {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_synthetic("try ");
        self.value.to_tokens(output);
        if let Some(clause) = &self.then_clause {
            output.push_space();
            clause.to_tokens(output);
        }
        if let Some(fallback) = &self.fallback {
            output.push_space();
            fallback.to_tokens(output);
        }
    }
}

impl ToTokens for NewStatement {
    fn to_tokens(&self, output: &mut TokenStream) {
        output.push_synthetic("new ");
        self.class.to_tokens(output);
        if let Some(parent) = &self.parent {
            output.push_synthetic(" of ");
            parent.to_tokens(output);
        }
        if let Some(instance) = &self.instance {
            output.push_synthetic(" from ");
            instance.to_tokens(output);
        }
    }
}

impl ToTokens for DebugClause {
    fn to_tokens(&self, output: &mut TokenStream) {
        self.keyword.to_tokens(output);
        if let Some(value) = &self.value {
            output.push_space();
            value.to_tokens(output);
        }
    }
}

macro_rules! prefixed_expression {
    ($($node:ty => $prefix:literal),* $(,)?) => {
        $(
            impl ToTokens for $node {
                fn to_tokens(&self, output: &mut TokenStream) {
                    output.push_synthetic($prefix);
                    if let Some(value) = &self.value {
                        output.push_space();
                        value.to_tokens(output);
                    }
                }
            }
        )*
    };
}

prefixed_expression! {
    BreakStatement => "break",
    ContinueStatement => "continue",
    ReturnStatement => "return",
}

macro_rules! required_prefixed_expression {
    ($($node:ty => $prefix:literal),* $(,)?) => {
        $(
            impl ToTokens for $node {
                fn to_tokens(&self, output: &mut TokenStream) {
                    output.push_synthetic($prefix);
                    output.push_space();
                    self.value.to_tokens(output);
                }
            }
        )*
    };
}

required_prefixed_expression! {
    CatchStatement => "catch",
    ThrowStatement => "throw",
    TrapStatement => "trap",
}

impl ToTokens for QuoteExpression {
    fn to_tokens(&self, output: &mut TokenStream) {
        self._specifier.to_tokens(output);
        output.push_space();
        self.token.to_tokens(output);
    }
}
