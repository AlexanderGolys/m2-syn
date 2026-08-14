use std::collections::HashMap;

use m2_syn::visit::Visit;
use m2_syn::{
    Array, BinaryExpression, BinaryOperator, Component, Expr, ForLoop, IfStatement, IntegerLiteral,
    List, LoopBody, ParenthesizedExpression, ParseError, ReturnStatement, Sequence, SourceId,
    StringLiteral, Symbol, WhileLoop, parse_file,
};

#[derive(Debug, Clone, PartialEq, Eq)]
enum TypeRange<T, Q> {
    Known(Vec<T>),
    Deferred(Q),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Fact<T, V, Q> {
    types: TypeRange<T, Q>,
    value: Option<V>,
}

#[derive(Debug, Clone, Copy)]
enum CollectionKind {
    Array,
    List,
    Sequence,
}

#[derive(Debug, Clone, Copy)]
enum ControlKind {
    If,
    For,
    While,
    Return,
}

trait InferenceContext {
    type Type: Clone;
    type Value: Clone;
    type Query: Clone;

    fn integer(&self, literal: &IntegerLiteral) -> FactOf<Self>;
    fn string(&self, literal: &StringLiteral) -> FactOf<Self>;
    fn symbol(&self, symbol: &Symbol) -> FactOf<Self>;
    fn binary(
        &self,
        node: &BinaryExpression,
        left: FactOf<Self>,
        right: FactOf<Self>,
    ) -> FactOf<Self>;
    fn collection(&self, kind: CollectionKind, elements: Vec<FactOf<Self>>) -> FactOf<Self>;
    fn control(&self, kind: ControlKind, neighbours: Vec<FactOf<Self>>) -> FactOf<Self>;
}

type FactOf<C> = Fact<
    <C as InferenceContext>::Type,
    <C as InferenceContext>::Value,
    <C as InferenceContext>::Query,
>;

struct TypeChecker<'context, C: InferenceContext> {
    context: &'context C,
    results: Vec<FactOf<C>>,
}

impl<C: InferenceContext> TypeChecker<'_, C> {
    fn infer(&mut self, expression: &Expr) -> FactOf<C> {
        let old_len = self.results.len();
        self.visit_expr(expression);
        assert_eq!(self.results.len(), old_len + 1);
        self.results.pop().expect("expression produced one fact")
    }

    fn components(&mut self, components: &[Component]) -> Vec<FactOf<C>> {
        components
            .iter()
            .filter_map(|component| match component {
                Component::Expr(expression) => Some(self.infer(expression)),
                Component::EmptyComponent(_) => None,
            })
            .collect()
    }

    fn loop_body(&mut self, body: &LoopBody) -> Vec<FactOf<C>> {
        body.listed_value
            .iter()
            .chain(&body.ignored_value)
            .map(|expression| self.infer(expression))
            .collect()
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

    fn visit_binary_expression(&mut self, expression: &'ast BinaryExpression) {
        let left = self.infer(&expression.left);
        let right = self.infer(&expression.right);
        self.results
            .push(self.context.binary(expression, left, right));
    }

    fn visit_parenthesized_expression(&mut self, expression: &'ast ParenthesizedExpression) {
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

    fn visit_if_statement(&mut self, statement: &'ast IfStatement) {
        let mut neighbours = vec![
            self.infer(&statement.condition),
            self.infer(&statement.then_clause.value),
        ];
        if let Some(clause) = &statement.else_clause {
            neighbours.push(self.infer(&clause.value));
        }
        self.results
            .push(self.context.control(ControlKind::If, neighbours));
    }

    fn visit_for_loop(&mut self, statement: &'ast ForLoop) {
        let mut neighbours = Vec::new();
        if let Some(range) = &statement.range {
            neighbours.extend(
                range
                    .iterated_collection
                    .iter()
                    .chain(&range.range_start)
                    .chain(&range.range_end)
                    .map(|expression| self.infer(expression)),
            );
        }
        neighbours.extend(statement.filter.iter().map(|value| self.infer(value)));
        neighbours.extend(self.loop_body(&statement.body));
        self.results
            .push(self.context.control(ControlKind::For, neighbours));
    }

    fn visit_while_loop(&mut self, statement: &'ast WhileLoop) {
        let mut neighbours = vec![self.infer(&statement.condition)];
        neighbours.extend(self.loop_body(&statement.body));
        self.results
            .push(self.context.control(ControlKind::While, neighbours));
    }

    fn visit_return_statement(&mut self, statement: &'ast ReturnStatement) {
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
enum DemoType {
    Integer,
    Number,
    String,
    Array,
    List,
    Sequence,
    Thing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DemoValue {
    Integer(i64),
}

type DemoFact = Fact<DemoType, DemoValue, String>;

struct DemoRegistry {
    symbols: HashMap<String, DemoFact>,
}

impl DemoRegistry {
    fn known(types: Vec<DemoType>, value: Option<DemoValue>) -> DemoFact {
        Fact {
            types: TypeRange::Known(types),
            value,
        }
    }

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
            literal.text.parse().ok().map(DemoValue::Integer),
        )
    }

    fn string(&self, _literal: &StringLiteral) -> DemoFact {
        Self::known(vec![DemoType::String, DemoType::Thing], None)
    }

    fn symbol(&self, symbol: &Symbol) -> DemoFact {
        self.symbols
            .get(&symbol.text)
            .cloned()
            .unwrap_or_else(|| Self::deferred(format!("symbol `{}`", symbol.text)))
    }

    fn binary(&self, node: &BinaryExpression, left: DemoFact, right: DemoFact) -> DemoFact {
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

fn infer(source: &str, context: &DemoRegistry) -> Result<DemoFact, ParseError> {
    let file = parse_file(source, SourceId(1))?;
    let expression = match &file.elements[0] {
        m2_syn::AnyCell::ExpressionCell(cell) => cell.value.as_ref(),
        _ => panic!("expected an expression cell"),
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
