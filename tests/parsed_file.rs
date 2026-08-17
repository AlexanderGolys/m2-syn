use m2_syn::{ParsedFile, SourceId, ToTokens};

#[test]
fn parsed_file_projects_one_current_typed_tree() {
    let file = ParsedFile::parse_with_source_id("left+right", SourceId(70)).unwrap();

    assert_eq!(file.original_source(), "left+right");
    assert_eq!(file.source_id(), SourceId(70));
    assert_eq!(file.to_source(), file.cst().to_code());
    assert_eq!(file.token_stream().to_string(), file.to_source());
    assert_eq!(file.cell_stream().to_string(), file.to_source());
}

#[test]
fn parsed_file_edits_feed_every_output_projection() {
    let file = ParsedFile::parse("left + right")
        .unwrap()
        .edit(|cst| cst.elements.clear());

    assert_eq!(file.original_source(), "left + right");
    assert!(file.cst().elements.is_empty());
    assert!(file.token_stream().is_empty());
    assert!(file.cell_stream().is_empty());
    assert_eq!(file.to_source(), "");
    assert_eq!(file.to_string(), "");
}

#[test]
fn parsed_file_exposes_readable_debug_projections() {
    let file = ParsedFile::parse_native("value").unwrap();

    assert!(file.pretty_cst().starts_with("SourceFile {"));
    assert!(file.pretty_cst().contains("Symbol"));
    assert!(file.pretty_tokens().starts_with("TokenStream("));
    assert!(file.pretty_tokens().contains("IdentToken"));
}
