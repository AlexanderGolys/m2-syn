//! The spelling-to-type macro for the generated token and delimiter atoms.
//!
//! These are ordinary source rather than part of the `tokens!` expansion:
//! rust-analyzer does not
//! register `macro_rules!` produced by a procedural macro, which would leave
//! every call site unresolved in the editor (rust-lang/rust-analyzer#10644).
//!
//! It does not need the schema: `Token!` defers the spelling-to-name
//! computation to the `token!` procedural macro.

/// Names the token type generated for an M2 spelling, or the delimiter atom
/// for a delimiter pair: `Token![+]`, `Token![()]`, `Token![<||>]`.
///
/// Rust spacing and string-fragment boundaries do not affect the result, so
/// `Token![!=]`, `Token![! =]`, `Token!["!="]`, and mixed forms name the same
/// type, and whitespace inside a string is stripped too, so `Token!["/ /"]`
/// is `Token!["//"]`. String fragments can contain characters that Rust
/// cannot tokenize. Nothing written, `Token![]` or `Token![" "]`, names
/// adjacency, `Token![SPACE]`.
#[macro_export]
macro_rules! Token {
    [$($spelling:tt)*] => {
        $crate::__m2_syn_token!($($spelling)*)
    };
}
