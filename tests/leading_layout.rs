//! Integration tests for retaining the layout that precedes the first cell.

use m2_syn::nodes::SourceFile;
use m2_syn::{Lex, Spanned, parse_file, parse_native};

/// Sources whose opening bytes belong to no cell.
const LEADING_LAYOUT_SOURCES: [&str; 6] = [
    "\nx = 1",
    "  x = 1\n",
    "\n\t\n  x = 1\ny = 2\n",
    "\n-- documentation\nx = 1",
    "   -* note *- x",
    "\n\n(x, y)",
];

fn both_parsers(source: &str) -> [SourceFile; 2] {
    [
        parse_native(source).expect("the native parser accepts the source"),
        parse_file(source).expect("the tree-sitter parser accepts the source"),
    ]
}

#[test]
fn layout_before_the_first_cell_survives_a_parse_and_emit_round_trip() {
    for source in LEADING_LAYOUT_SOURCES {
        for file in both_parsers(source) {
            assert_eq!(file.to_m2(), source, "{source:?}");
            assert_eq!(file.span().text(), Some(source), "{source:?}");
        }
    }
}

#[test]
fn the_beginning_of_file_marker_spans_the_layout_before_the_first_byte() {
    for (source, retained) in [("\n  x = 1", "\n  "), ("x = 1", ""), ("   \n", "")] {
        for file in both_parsers(source) {
            assert_eq!(file.bof.span().text(), Some(retained), "{source:?}");
        }
    }
}

#[test]
fn a_cell_inserted_before_the_first_cell_is_not_preceded_by_stale_layout() {
    let mut file = parse_native("\n  x = 1\n").expect("the native parser accepts the source");
    let inserted = parse_native("y = 2")
        .expect("the native parser accepts the inserted source")
        .cells
        .pop()
        .expect("the inserted source holds one cell");

    file.cells.insert(0, inserted);

    assert_eq!(file.to_m2(), "y = 2\nx = 1\n");
}

#[test]
fn removing_the_first_cell_does_not_reinstate_its_text() {
    let mut file = parse_native("\n  x = 1\ny = 2").expect("the native parser accepts the source");

    file.cells.remove(0);

    assert_eq!(file.to_m2(), "y = 2");
}

#[test]
fn a_cell_appended_after_the_last_cell_keeps_the_opening_layout() {
    let mut file = parse_native("\n  x = 1").expect("the native parser accepts the source");
    let appended = parse_native("y = 2")
        .expect("the native parser accepts the appended source")
        .cells
        .pop()
        .expect("the appended source holds one cell");

    file.cells.push(appended);

    assert_eq!(file.to_m2(), "\n  x = 1\ny = 2");
}
