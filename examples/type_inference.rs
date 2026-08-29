//! Demonstrates type inference.

use std::collections::HashMap;

use m2_syn::nodes::{
    Array, BinaryOperator, Component, Expr, ExprControlReturn, ExprForBare, ExprForIn,
    ExprForRange, ExprIf, ExprOpBinRegular, ExprWhile, Holder, IntegerLiteral, List, LoopBody,
    PackedContent, Sequence, StringLiteral, Symbol,
};
use m2_syn::visit::Visit;
use m2_syn::{ParseError, parse_file};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Alternatives used by the TypeRange example.
enum TypeRange<T, Q> {
    /// The Known alternative.
    Known(Vec<T>),
    /// The Deferred alternative.
    Deferred(Q),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// State used by the Fact example.
struct Fact<T, V, Q> {
    /// The types.
    types: TypeRange<T, Q>,
    /// The inferred value, when evaluation produced one.
    value: Option<V>,
}

#[derive(Debug, Clone, Copy)]
/// Alternatives used by the CollectionKind example.
enum CollectionKind {
    /// The Array alternative.
    Array,
    /// The List alternative.
    List,
    /// The Sequence alternative.
    Sequence,
}

#[derive(Debug, Clone, Copy)]
/// Alternatives used by the ControlKind example.
enum ControlKind {
    /// The If alternative.
    If,
    /// The For alternative.
    For,
    /// The While alternative.
    While,
    /// The Return alternative.
    Return,
}

/// Behavior demonstrated by InferenceContext.
trait InferenceContext {
    /// The representation of an inferred M2 type.
    type Type: Clone;
    /// The representation of a known compile-time value.
    type Value: Clone;
    /// The representation of deferred type information.
    type Query: Clone;

    /// Infers an integer literal.
    fn integer(&self, literal: &IntegerLiteral) -> FactOf<Self>;
    /// Infers a string literal.
    fn string(&self, literal: &StringLiteral) -> FactOf<Self>;
    /// Looks up a symbol's inferred fact.
    fn symbol(&self, symbol: &Symbol) -> FactOf<Self>;
    /// Combines the facts inferred for a binary expression's operands.
    fn binary(
        &self,
        node: &ExprOpBinRegular,
        left: FactOf<Self>,
        right: FactOf<Self>,
    ) -> FactOf<Self>;
    /// Infers a collection from its element facts.
    fn collection(&self, kind: CollectionKind, elements: Vec<FactOf<Self>>) -> FactOf<Self>;
    /// Infers a control form from the facts of its neighboring expressions.
    fn control(&self, kind: ControlKind, neighbours: Vec<FactOf<Self>>) -> FactOf<Self>;
}

/// Convenient alias for FactOf.
type FactOf<C> = Fact<
    <C as InferenceContext>::Type,
    <C as InferenceContext>::Value,
    <C as InferenceContext>::Query,
>;

/// State used by the TypeChecker example.
struct TypeChecker<'context, C: InferenceContext> {
    /// The context.
    context: &'context C,
    /// The results.
    results: Vec<FactOf<C>>,
}

impl<C: InferenceContext> TypeChecker<'_, C> {
    /// Infers one expression and removes its completed fact from the visit stack.
    fn infer(&mut self, expression: &Expr) -> FactOf<C> {
        let old_len = self.results.len();
        self.visit_expr(expression);
        assert_eq!(self.results.len(), old_len + 1);
        self.results.pop().expect("expression produced one fact")
    }

    /// Infers every nonempty component of optional packed content.
    fn components(&mut self, content: &Option<PackedContent>) -> Vec<FactOf<C>> {
        match content {
            None => Vec::new(),
            Some(PackedContent::Expr(expression)) => vec![self.infer(expression)],
            Some(PackedContent::Pack(pack)) => pack
                .pack
                .iter()
                .filter_map(|component| match component {
                    Component::Expr(expression) => Some(self.infer(expression)),
                    Component::Empty(_) => None,
                })
                .collect(),
        }
    }

    /// Infers the expressions contributed by a loop body.
    fn loop_body(&mut self, body: &LoopBody) -> Vec<FactOf<C>> {
        let mut neighbours = Vec::new();
        if let Some(step) = &body.list_step {
            neighbours.push(self.infer(&step.expr));
        }
        if let Some(step) = &body.do_step {
            neighbours.push(self.infer(&step.statement));
        }
        neighbours
    }
}

impl<'ast, C: InferenceContext> Visit<'ast> for TypeChecker<'_, C> {
    fn visit_integer_literal(&mut self, literal: &'ast IntegerLiteral) {
        self.results.push(self.context.integer(literal));
    }

    fn visit_string_literal(&mut self, literal: &'ast StringLiteral) {
        self.results.push(self.context.string(literal));
    }

    fn visit_symbol(&mut self, symbol: &'ast Symbol) {
        self.results.push(self.context.symbol(symbol));
    }

    fn visit_expr_op_bin_regular(&mut self, expression: &'ast ExprOpBinRegular) {
        let left = self.infer(&expression.left_operand);
        let right = self.infer(&expression.right_operand);
        self.results
            .push(self.context.binary(expression, left, right));
    }

    fn visit_holder(&mut self, expression: &'ast Holder) {
        let fact = expression.value.as_deref().map(|value| self.infer(value));
        self.results.push(fact.unwrap_or_else(|| {
            self.context
                .collection(CollectionKind::Sequence, Vec::new())
        }));
    }

    fn visit_array(&mut self, array: &'ast Array) {
        let elements = self.components(&array.elements);
        self.results
            .push(self.context.collection(CollectionKind::Array, elements));
    }

    fn visit_list(&mut self, list: &'ast List) {
        let elements = self.components(&list.elements);
        self.results
            .push(self.context.collection(CollectionKind::List, elements));
    }

    fn visit_sequence(&mut self, sequence: &'ast Sequence) {
        let elements = self.components(&sequence.elements);
        self.results
            .push(self.context.collection(CollectionKind::Sequence, elements));
    }

    fn visit_expr_if(&mut self, statement: &'ast ExprIf) {
        let mut neighbours = vec![
            self.infer(&statement.condition),
            self.infer(&statement.then_clause.expr),
        ];
        if let Some(clause) = &statement.else_clause {
            neighbours.push(self.infer(&clause.expr));
        }
        self.results
            .push(self.context.control(ControlKind::If, neighbours));
    }

    fn visit_expr_for_bare(&mut self, statement: &'ast ExprForBare) {
        let mut neighbours = Vec::new();
        neighbours.extend(
            statement
                .condition
                .iter()
                .map(|condition| self.infer(&condition.condition)),
        );
        neighbours.extend(self.loop_body(&statement.body));
        self.results
            .push(self.context.control(ControlKind::For, neighbours));
    }

    fn visit_expr_for_range(&mut self, statement: &'ast ExprForRange) {
        let mut neighbours = Vec::new();
        if let Some(bound) = &statement.lower_bound {
            neighbours.push(self.infer(&bound.value));
        }
        if let Some(bound) = &statement.upper_bound {
            neighbours.push(self.infer(&bound.value));
        }
        neighbours.extend(
            statement
                .condition
                .iter()
                .map(|condition| self.infer(&condition.condition)),
        );
        neighbours.extend(self.loop_body(&statement.body));
        self.results
            .push(self.context.control(ControlKind::For, neighbours));
    }

    fn visit_expr_for_in(&mut self, statement: &'ast ExprForIn) {
        let mut neighbours = vec![self.infer(&statement.collection)];
        neighbours.extend(
            statement
                .condition
                .iter()
                .map(|condition| self.infer(&condition.condition)),
        );
        neighbours.extend(self.loop_body(&statement.body));
        self.results
            .push(self.context.control(ControlKind::For, neighbours));
    }

    fn visit_expr_while(&mut self, statement: &'ast ExprWhile) {
        let mut neighbours = vec![self.infer(&statement.condition)];
        neighbours.extend(self.loop_body(&statement.body));
        self.results
            .push(self.context.control(ControlKind::While, neighbours));
    }

    fn visit_expr_control_return(&mut self, statement: &'ast ExprControlReturn) {
        let neighbours = statement
            .value
            .iter()
            .map(|value| self.infer(value))
            .collect();
        self.results
            .push(self.context.control(ControlKind::Return, neighbours));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Alternatives used by the DemoType example.
enum DemoType {
    /// The Integer alternative.
    Integer,
    /// The Number alternative.
    Number,
    /// The String alternative.
    String,
    /// The Array alternative.
    Array,
    /// The List alternative.
    List,
    /// The Sequence alternative.
    Sequence,
    /// The Thing alternative.
    Thing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Alternatives used by the DemoValue example.
enum DemoValue {
    /// The Integer alternative.
    Integer(i64),
}

/// Convenient alias for DemoFact.
type DemoFact = Fact<DemoType, DemoValue, String>;

/// State used by the DemoRegistry example.
struct DemoRegistry {
    /// The symbols.
    symbols: HashMap<String, DemoFact>,
}

impl DemoRegistry {
    /// Handles known.
    fn known(types: Vec<DemoType>, value: Option<DemoValue>) -> DemoFact {
        Fact {
            types: TypeRange::Known(types),
            value,
        }
    }

    /// Handles deferred.
    fn deferred(query: impl Into<String>) -> DemoFact {
        Fact {
            types: TypeRange::Deferred(query.into()),
            value: None,
        }
    }
}

impl InferenceContext for DemoRegistry {
    type Type = DemoType;
    type Value = DemoValue;
    type Query = String;

    fn integer(&self, literal: &IntegerLiteral) -> DemoFact {
        Self::known(
            vec![DemoType::Integer, DemoType::Number, DemoType::Thing],
            literal.text().parse().ok().map(DemoValue::Integer),
        )
    }

    fn string(&self, _literal: &StringLiteral) -> DemoFact {
        Self::known(vec![DemoType::String, DemoType::Thing], None)
    }

    fn symbol(&self, symbol: &Symbol) -> DemoFact {
        self.symbols
            .get(symbol.text())
            .cloned()
            .unwrap_or_else(|| Self::deferred(format!("symbol `{}`", symbol.text())))
    }

    fn binary(&self, node: &ExprOpBinRegular, left: DemoFact, right: DemoFact) -> DemoFact {
        match (&node.operator, left.value, right.value) {
            (
                BinaryOperator::Add(_),
                Some(DemoValue::Integer(left)),
                Some(DemoValue::Integer(right)),
            ) => Self::known(
                vec![DemoType::Integer, DemoType::Number, DemoType::Thing],
                Some(DemoValue::Integer(left + right)),
            ),
            _ => Self::deferred("installed binary signature"),
        }
    }

    fn collection(&self, kind: CollectionKind, _elements: Vec<DemoFact>) -> DemoFact {
        let ty = match kind {
            CollectionKind::Array => DemoType::Array,
            CollectionKind::List => DemoType::List,
            CollectionKind::Sequence => DemoType::Sequence,
        };
        Self::known(vec![ty, DemoType::Thing], None)
    }

    fn control(&self, kind: ControlKind, _neighbours: Vec<DemoFact>) -> DemoFact {
        Self::deferred(format!("{kind:?} typing rule"))
    }
}

/// Handles infer.
fn infer(source: &str, context: &DemoRegistry) -> Result<DemoFact, ParseError> {
    let file = parse_file(source)?;
    let expression = match &file.cells[0].content {
        PackedContent::Expr(expression) => expression.as_ref(),
        PackedContent::Pack(_) => panic!("expected one expression"),
    };
    Ok(TypeChecker {
        context,
        results: Vec::new(),
    }
    .infer(expression))
}

fn main() -> Result<(), ParseError> {
    let context = DemoRegistry {
        symbols: HashMap::from([(
            "answer".into(),
            DemoRegistry::known(
                vec![DemoType::Integer, DemoType::Number, DemoType::Thing],
                Some(DemoValue::Integer(40)),
            ),
        )]),
    };

    let known = infer("(answer + 2)", &context)?;
    let deferred = infer("missing + 2", &context)?;
    let collection = infer("{answer, missing}", &context)?;

    println!("known: {known:?}");
    println!("deferred: {deferred:?}");
    println!("collection: {collection:?}");
    Ok(())
}
