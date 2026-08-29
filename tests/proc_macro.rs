//! Integration tests for proc macro.

use m2_syn::{CellStream, LiteralReturnTypes, ProcMacro, lex_str};

fn expand(source: &str) -> String {
    let input = CellStream::new(lex_str(source).expect("example source should lex"))
        .expect("example source should form complete cells");
    LiteralReturnTypes
        .expand(input, &())
        .expect("example source should expand")
        .to_string()
}

#[test]
fn literal_return_types_annotates_direct_literal_bodies() {
    let input = r#"integerMethod = method();
integerMethod ZZ := value -> 1;
floatMethod = method();
floatMethod ZZ := value -> 1.5;
stringMethod = method();
stringMethod ZZ := value -> "ok";
rawStringMethod = method();
rawStringMethod ZZ := value -> ///ok///;"#;

    let expected = r#"integerMethod = method();
integerMethod ZZ :=ZZ=>value -> 1;
floatMethod = method();
floatMethod ZZ :=RR=>value -> 1.5;
stringMethod = method();
stringMethod ZZ :=String=>value -> "ok";
rawStringMethod = method();
rawStringMethod ZZ :=String=>value -> ///ok///;"#;

    assert_eq!(expand(input), expected);
}

#[test]
fn literal_return_types_preserves_annotations_unknown_bodies_and_comments() {
    let input = r#"existingMethod = method();
existingMethod ZZ := String => value -> "ok";
unknownMethod = method();
-- Keep this explanation with the installation.
unknownMethod ZZ := value -> value;"#;

    assert_eq!(expand(input), input);
}

#[test]
fn literal_return_types_is_idempotent() {
    let once = expand("integerMethod = method(); integerMethod ZZ := value -> 1;");

    assert_eq!(expand(&once), once);
}
