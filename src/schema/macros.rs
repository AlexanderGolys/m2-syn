//! Spelling-to-type macros for the generated token and delimiter atoms.
//!
//! These are the only two `macro_rules!` in the crate, and they are ordinary
//! source rather than part of the `syntax!` expansion: rust-analyzer does not
//! register `macro_rules!` produced by a procedural macro, which would leave
//! every call site unresolved in the editor (rust-lang/rust-analyzer#10644).
//!
//! Neither needs the schema. `Token!` defers the spelling-to-name computation
//! to the `token!` procedural macro, and the delimiter families are a fixed
//! set.

/// Names the token type generated for a spelling, as in `Token![;]`.
#[macro_export]
macro_rules! Token {
    [$($spelling:tt)*] => {
        $crate::__m2_syn_token!($($spelling)*)
    };
}

/// Names the delimiter atom for a delimiter family, as in `Delimiter![()]`.
#[macro_export]
macro_rules! Delimiter {
    [()] => { $crate::_ParenthesisDelimiter };
    [[]] => { $crate::_BracketDelimiter };
    [{}] => { $crate::_BraceDelimiter };
    [<| |>] => { $crate::_AngleBarDelimiter };
    [$($unsupported:tt)*] => {
        compile_error!("delimiter was not declared")
    };
}
