//! Interfaces for expanding Macaulay2 syntax with Rust code.

use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::CellStream;
use crate::fold::{self, Fold};
use crate::nodes::{
    AssignOperator, BinaryOperator, Expr, ExprAssign, ExprBinary, ExprOption, Literal,
};
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
    fn fold_expr_assign(&mut self, node: ExprAssign) -> ExprAssign {
        let mut node = fold::fold_expr_assign(self, node);
        let installs_method = matches!(node.op, AssignOperator::Local(_))
            && matches!(
                &*node.left,
                Expr::Binary(ExprBinary {
                    op: BinaryOperator::Space(_),
                    ..
                })
            );
        if let Some(typical_value) = installs_method
            .then(|| literal_return_type(&node.right))
            .flatten()
        {
            node.right = Box::new(Expr::from(ExprOption {
                key: Box::new(typical_value),
                fat_arrow_token: parse_quote_m2!(=>),
                value: node.right,
            }));
        }
        node
    }
}

/// The type of the literal returned by a function, as an M2 class name.
fn literal_return_type(function: &Expr) -> Option<Expr> {
    let Expr::Function(function) = function else {
        return None;
    };
    let Expr::Literal(literal) = function.body.as_ref() else {
        return None;
    };
    Some(match literal {
        Literal::Integer(_) => parse_quote_m2!(ZZ),
        Literal::Float(_) => parse_quote_m2!(RR),
        Literal::String(_) | Literal::RawString(_) => parse_quote_m2!(String),
    })
}
