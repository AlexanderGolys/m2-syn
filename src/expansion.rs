//! Interfaces for expanding Macaulay2 syntax with Rust code.

use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::CellStream;
use crate::fold::{self, Fold};
use crate::nodes::{Expr, ExprAssignInstallMethod, ExprLambda, ExprLiteral, TypeAnnotation};
use crate::{CellStreamError, NativeParseError, NativeParser, Parser, Tokenize, parse_quote_m2};

/// A procedural macro over independently parseable Macaulay2 cells.
///
/// Implementations may parse the cells into the typed syntax graph, transform
/// raw tokens directly, perform arbitrary Rust computations, or combine those
/// approaches. The caller decides whether expansion happens for one submitted
/// cell or for a complete source file.
pub trait ProcMacro<Args = ()> {
    /// The error reported when expansion cannot produce a cell stream.
    type Error: Error;

    /// Expands `input` using the invocation arguments in `args`.
    fn expand(&mut self, input: CellStream, args: &Args) -> Result<CellStream, Self::Error>;
}

#[derive(Debug, Default)]
/// A procedural macro that annotates methods returning a direct literal.
///
/// Existing return annotations are retained. Integer, floating-point, and
/// string literals produce `ZZ`, `RR`, and `String` annotations respectively;
/// every other function body is left unchanged.
///
/// # Examples
///
/// ```
/// use m2_syn::{CellStream, ProcMacro, lex_str};
/// use m2_syn::LiteralReturnTypes;
///
/// let input = CellStream::new(lex_str("f = method(); f ZZ := x -> 1").unwrap()).unwrap();
/// let output = LiteralReturnTypes.expand(input, &()).unwrap();
/// assert!(output.to_string().contains("f ZZ :=ZZ=>x -> 1"));
/// ```
pub struct LiteralReturnTypes;

#[derive(Debug)]
/// A failure to parse or reconstruct input expanded by [`LiteralReturnTypes`].
pub enum LiteralReturnTypesError {
    /// The cells were not valid typed Macaulay2 syntax.
    Parse(NativeParseError),
    /// The transformed syntax could not be partitioned back into cells.
    Tokenize(CellStreamError),
}

impl Display for LiteralReturnTypesError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(error) => Display::fmt(error, formatter),
            Self::Tokenize(error) => Display::fmt(error, formatter),
        }
    }
}

impl Error for LiteralReturnTypesError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Parse(error) => Some(error),
            Self::Tokenize(error) => Some(error),
        }
    }
}

impl ProcMacro for LiteralReturnTypes {
    type Error = LiteralReturnTypesError;

    fn expand(&mut self, input: CellStream, _args: &()) -> Result<CellStream, Self::Error> {
        let mut parser = NativeParser;
        let syntax = parser
            .parse_cells(input)
            .map_err(LiteralReturnTypesError::Parse)?;
        self.fold_source_file(syntax)
            .to_cell_stream()
            .map_err(LiteralReturnTypesError::Tokenize)
    }
}

impl Fold for LiteralReturnTypes {
    fn fold_expr_assign_install_method(
        &mut self,
        node: ExprAssignInstallMethod,
    ) -> ExprAssignInstallMethod {
        let mut node = fold::fold_expr_assign_install_method(self, node);
        if node.typical_value.is_none() {
            node.typical_value = literal_return_annotation(&node.function);
        }
        node
    }
}

/// Handles literal return annotation.
fn literal_return_annotation(function: &Expr) -> Option<TypeAnnotation> {
    let Expr::Lambda(lambda) = function else {
        return None;
    };
    let body = match lambda {
        ExprLambda::Variadic(lambda) => lambda.body.as_ref(),
        ExprLambda::List(lambda) => lambda.body.as_ref(),
    };
    let Expr::Literal(literal) = body else {
        return None;
    };
    let typical_value = match literal {
        ExprLiteral::Integer(_) => parse_quote_m2!(ZZ),
        ExprLiteral::Float(_) => parse_quote_m2!(RR),
        ExprLiteral::String(_) | ExprLiteral::RawString(_) => parse_quote_m2!(String),
    };
    Some(TypeAnnotation::new(typical_value, parse_quote_m2!(=>)))
}
