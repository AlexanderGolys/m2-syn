//! Integration tests for storing every comment in the slot where it occurs.

use m2_syn::nodes::SourceFile;
use m2_syn::{CommentTrivia, Lex, parse_file, parse_file_recovering, parse_native};

/// Sources whose comments sit at each kind of ownership boundary.
const COMMENT_PLACEMENT_SOURCES: [&str; 14] = [
    "-- doc\n(x)",
    "(x) -- c",
    "x\n-- c",
    "-* b *- x",
    "x -* b *-",
    "{x, -- c\ny}",
    "-- a\n\n-- b\nx",
    "x\n-- c\n\ny",
    "x\n\n-- c\n",
    "-- only",
    "(-* b *-)",
    "-- d\n-* b *-\nx",
    "f(x, -- c\n  y)\n-- tail\n",
    "x := (-- c\n  1)",
];

/// The text of each outer and inner comment, independent of the parse's source.
fn comment_texts(comments: &CommentTrivia) -> [Vec<&str>; 2] {
    [comments.outer(), comments.inner()]
        .map(|comments| comments.iter().map(|comment| comment.text()).collect())
}

fn both_parsers(source: &str) -> [SourceFile; 3] {
    [
        parse_native(source).expect("the native parser accepts the source"),
        parse_file(source).expect("the tree-sitter parser accepts the source"),
        parse_file_recovering(source)
            .expect("the recovering tree-sitter parser accepts the source")
            .value,
    ]
}

#[test]
fn every_comment_survives_a_parse_and_emit_round_trip_in_place() {
    for source in COMMENT_PLACEMENT_SOURCES {
        for file in both_parsers(source) {
            assert_eq!(file.to_m2(), source, "{source:?}");
        }
    }
}

#[test]
fn the_backends_assign_every_cell_the_same_comments() {
    for source in COMMENT_PLACEMENT_SOURCES {
        let [native, sitter, recovered] = both_parsers(source);
        for other in [&sitter, &recovered] {
            assert_eq!(other.cells.len(), native.cells.len(), "{source:?}");
            for (left, right) in native.cells.iter().zip(&other.cells) {
                assert_eq!(
                    comment_texts(&left.comments()),
                    comment_texts(&right.comments()),
                    "{source:?}"
                );
            }
            assert_eq!(
                comment_texts(&other.comments()),
                comment_texts(&native.comments()),
                "{source:?}"
            );
        }
    }
}

#[test]
fn a_documentation_block_before_a_group_is_the_cells_outer_comment() {
    for file in both_parsers("-- doc\n(x)") {
        let comments = file.cells[0].comments();

        assert_eq!(comments.outer().len(), 1);
        assert_eq!(comments.outer()[0].text(), "-- doc");
        assert!(comments.inner().is_empty());
        assert_eq!(file.cells[0].leading_trivia.as_slice().len(), 1);
    }
}

#[test]
fn comments_after_the_last_token_belong_to_the_last_cell() {
    for file in both_parsers("x\n\n-- c\n") {
        assert_eq!(
            comment_texts(&file.cells[0].comments()),
            [vec![], vec!["-- c"]]
        );
        assert!(file.leading_trivia.is_empty());
    }
}

#[test]
fn a_file_without_cells_owns_its_comments() {
    for file in both_parsers("-- only") {
        assert!(file.cells.is_empty());
        assert_eq!(file.leading_trivia.as_slice().len(), 1);
        assert_eq!(comment_texts(&file.comments()), [vec![], vec!["-- only"]]);
    }
}

#[test]
fn layout_beside_a_comment_next_to_a_delimiter_survives() {
    for source in [
        "( -- c\nx)",
        "(x\n-- c\n)",
        "f(-- c\n)",
        "x = (\n  -- c\n  1)",
        "(x -- c   \n)",
    ] {
        for file in both_parsers(source) {
            assert_eq!(file.to_m2(), source, "{source:?}");
        }
    }
}

#[test]
fn layout_beside_a_comment_in_an_unclosed_group_survives_recovery() {
    for source in ["( -- c", "(-- c\n", "[\n\n-- c", "{\t-- c"] {
        let native = m2_syn::parse_native_recovering(source)
            .expect("the recovering native parser accepts the source")
            .value;
        let sitter = parse_file_recovering(source)
            .expect("the recovering tree-sitter parser accepts the source")
            .value;

        assert_eq!(native.to_m2(), source, "{source:?}");
        assert_eq!(sitter.to_m2(), source, "{source:?}");
    }
}
