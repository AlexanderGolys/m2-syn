//! A comment that touches a closing delimiter follows the group rather than
//! entering it.

use m2_syn::{Lex, parse_file, parse_native};

/// Sources whose comment starts at the byte right after a group's closer.
const ABUTTING_COMMENT_SOURCES: [&str; 5] = ["()-- c", "{}-- c", "[]-- c", "(x)-- c", "f()-- c"];

#[test]
fn a_comment_abutting_a_closer_is_emitted_after_it() {
    for source in ABUTTING_COMMENT_SOURCES {
        assert_eq!(parse_native(source).unwrap().to_m2(), source, "{source:?}");
        assert_eq!(parse_file(source).unwrap().to_m2(), source, "{source:?}");
    }
}
