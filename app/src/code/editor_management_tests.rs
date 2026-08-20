use super::CodeSource;

#[test]
fn generated_documents_are_pathless_read_only_and_not_restorable() {
    let source = CodeSource::GeneratedReadOnly {
        title: "Codex transcript".to_string(),
    };

    assert!(source.is_generated_read_only());
    assert_eq!(source.path(), None);
    assert_eq!(source.location(), None);
    assert!(!source.is_restorable());
    assert_eq!(source.telemetry_source_name(), "generated_read_only");
}
