//! Integration tests for the layout written between consecutive cells.

use m2_syn::nodes::SourceFile;
use m2_syn::{Lex, parse_file, parse_file_recovering, parse_native, parse2, quote_m2};

#[test]
fn cells_written_on_one_line_stay_on_one_line() {
    for source in ["1;1", "x = 1;y = 2", "a;b;c", "f x;  g y;\nh z"] {
        assert_eq!(parse_native(source).unwrap().to_m2(), source, "{source:?}");
        assert_eq!(parse_file(source).unwrap().to_m2(), source, "{source:?}");
        assert_eq!(
            parse_file_recovering(source).unwrap().value.to_m2(),
            source,
            "{source:?}"
        );
    }
}

#[test]
fn quoted_cells_are_separated_by_a_line_break() {
    let quoted = parse2::<SourceFile>(quote_m2! { left; right }).unwrap();

    assert_eq!(quoted.to_m2(), "left;\nright");
}
