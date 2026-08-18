use m2_syn::{ParsedFile, SourceId, Spanned, ToTokens};

#[test]
fn parsed_file_projects_one_current_typed_tree() {
    let file = ParsedFile::from_source_with_id("left+right", SourceId(70)).unwrap();

    assert_eq!(file.original_source(), "left+right");
    assert_eq!(file.source_id(), SourceId(70));
    assert_eq!(file.to_source(), file.cst().to_code());
    assert_eq!(file.token_stream().to_string(), file.to_source());
    assert_eq!(file.cell_stream().to_string(), file.to_source());
}

#[test]
fn parsed_file_edits_feed_every_output_projection() {
    let file = ParsedFile::from_source("left + right")
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
    let file = ParsedFile::from_source_native("value").unwrap();

    assert!(file.pretty_cst().starts_with("SourceFile"));
    assert!(file.pretty_cst().contains("value → value  Symbol"));
    assert!(!file.pretty_cst().contains("::"));
    assert!(file.pretty_tokens().starts_with("token │"));
    assert!(file.pretty_tokens().contains("IDENT"));
    assert!(file.pretty_tokens().contains(" value "));
    assert!(file.pretty_tokens().contains("EOC"));
}

#[test]
fn combined_report_labels_source_tokens_and_the_complete_tree() {
    let file = ParsedFile::from_source_native("f = (x,)").unwrap();
    let report = file.pretty_report().to_string();

    assert!(report.contains("╭─ SOURCE"));
    assert!(report.contains("╭─ TOKEN STREAM"));
    assert!(report.contains("OPEN·Parenthesis"));
    assert!(report.contains("CLOSE·Parenthesis"));
    assert!(report.contains("╭─ TYPED CST"));
    assert!(report.contains("Punctuated · 2 items"));
    assert!(report.contains("opening → (  Token"));
    assert!(report.contains("closing → )  Token"));
    assert!(!report.contains("@detached"));
    assert!(!report.contains("TRIVIA"));
    assert!(!report.contains('\u{1b}'));

    let colored = file
        .pretty_report()
        .ansi(true)
        .spans(false)
        .trivia(true)
        .to_string();
    assert!(colored.contains("\u{1b}["));
    assert!(colored.contains("TRIVIA"));
    assert!(!colored.contains("@s"));
}

#[test]
fn implicit_source_ids_distinguish_separately_parsed_files() {
    let first = ParsedFile::from_source("first").unwrap();
    let second = ParsedFile::from_source("second").unwrap();

    assert_ne!(first.source_id(), second.source_id());
    assert_eq!(first.cst().span().source().unwrap(), first.source_id());
    assert_eq!(second.cst().span().source().unwrap(), second.source_id());
}
