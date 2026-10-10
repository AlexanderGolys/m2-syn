//! Integration tests for retaining the layout that follows the final cell.

use m2_syn::nodes::SourceFile;
use m2_syn::{Lex, ParsedFile, Spanned, parse_file, parse_native};

/// Sources whose final bytes belong to no cell.
const TRAILING_LAYOUT_SOURCES: [&str; 10] = [
    "x = 1\n",
    "x = 1\n\n\n",
    "x = 1   ",
    "x = 1\n\t\n  ",
    "f x\ng y\n\n  \n",
    "x = 1 -- trailing comment\n\n",
    "-- comment only\n",
    "   \n",
    "\n\n",
    "",
];

fn both_parsers(source: &str) -> [SourceFile; 2] {
    [
        parse_native(source).expect("the native parser accepts the source"),
        parse_file(source).expect("the tree-sitter parser accepts the source"),
    ]
}

#[test]
fn every_input_byte_survives_a_parse_and_emit_round_trip() {
    for source in TRAILING_LAYOUT_SOURCES {
        for file in both_parsers(source) {
            assert_eq!(file.to_m2(), source, "{source:?}");
        }
    }
}

#[test]
fn the_end_of_file_marker_spans_the_layout_after_the_last_token() {
    for (source, retained) in [
        ("x = 1\n", "\n"),
        ("x = 1\n\n\n", "\n\n\n"),
        ("x = 1   ", "   "),
        ("x = 1", ""),
        ("   \n", "   \n"),
        ("", ""),
    ] {
        for file in both_parsers(source) {
            assert_eq!(file.eof.span().text(), Some(retained), "{source:?}");
        }
    }
}

#[test]
fn the_source_file_span_covers_the_whole_document() {
    for source in TRAILING_LAYOUT_SOURCES {
        for file in both_parsers(source) {
            assert_eq!(file.span().text(), Some(source), "{source:?}");
        }
    }
}

#[test]
fn a_document_of_pure_layout_is_owned_by_the_end_of_file_marker() {
    for file in both_parsers("   \n") {
        assert!(file.cells.is_empty());
        assert_eq!(file.eof.span().bounds(), Some((0, 4)));
        assert_eq!(file.to_m2(), "   \n");
    }
}

#[test]
fn owned_files_reproduce_their_original_source() {
    for source in TRAILING_LAYOUT_SOURCES {
        let sitter = ParsedFile::from_source(source).expect("tree-sitter accepts the source");
        let native = ParsedFile::from_source_native(source).expect("the native parser accepts it");

        assert_eq!(sitter.to_source(), source, "{source:?}");
        assert_eq!(native.to_source(), source, "{source:?}");
    }
}

#[test]
fn a_cell_appended_after_the_marker_is_still_emitted_before_it() {
    let mut file = parse_native("x = 1\n\n").expect("the native parser accepts the source");
    let appended = parse_native("y = 2")
        .expect("the native parser accepts the appended source")
        .cells
        .pop()
        .expect("the appended source holds one cell");

    file.cells.push(appended);

    // The marker no longer describes layout physically following the last cell,
    // so it contributes nothing rather than reinstating stale text between or
    // after the cells.
    assert_eq!(file.to_m2(), "x = 1\ny = 2");
}
