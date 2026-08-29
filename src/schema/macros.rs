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

/// Declares typed syntax nodes, token atoms, and enabled derived APIs.
///
/// The main crate selects the internal procedural-macro entry point so that
/// `pretty` and `walk` are resolved as `m2-syn` dependency features rather
/// than as features of the crate invoking this macro.
#[macro_export]
macro_rules! syntax {
    ($($syntax:tt)*) => {
        $crate::__m2_syn_syntax! { $($syntax)* }
    };
}

/// Names the token type generated for an M2 spelling.
///
/// Rust spacing and string-fragment boundaries do not affect the result, so
/// `Token![!=]`, `Token![! =]`, `Token!["!="]`, and mixed forms name the same
/// type. String fragments can contain characters that Rust cannot tokenize.
#[macro_export]
macro_rules! Token {
    [$($spelling:tt)*] => {
        $crate::__m2_syn_token!($($spelling)*)
    };
}

/// Names the delimiter atom for a delimiter family, as in `Delimiter![()]`.
#[macro_export]
macro_rules! Delimiter {
    [()] => { $crate::nodes::_ParenthesisDelimiter };
    [[]] => { $crate::nodes::_BracketDelimiter };
    [{}] => { $crate::nodes::_BraceDelimiter };
    [<| |>] => { $crate::nodes::_AngleBarDelimiter };
    [$($unsupported:tt)*] => {
        compile_error!("delimiter was not declared")
    };
}
