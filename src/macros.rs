// macro_rules! ast_leaf {
//     ($name:ident, $type:ty) => {
//
//

[#derive(Debug, Spanned, Default, Clone, Copy, PartialEq, Eq)]
pub struct Token11 {
    span: Span
}

pub fn Token11<S: Spanned>(span: S) -> Token11 {
    Token11 {
        span: span.span()
    }
}

impl DisplayCode for Token11 {
    fn display() -> &'static str { "11" }
}

impl Parse for Token11 {
    fn parse(input: Stream) -> Result<Self> {
        input.parse_token()
    }
}


pub struct Assignment {
    pub left: Symbol,
}
