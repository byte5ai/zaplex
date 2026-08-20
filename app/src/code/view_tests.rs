use std::path::PathBuf;

use super::{can_merge_code_views, header_title};
use crate::code::editor_management::CodeSource;

fn file_source(path: &str) -> CodeSource {
    CodeSource::FileTree {
        path: PathBuf::from(path),
    }
}

#[test]
fn generated_header_uses_its_source_title() {
    let source = CodeSource::GeneratedReadOnly {
        title: "Codex transcript".to_string(),
    };

    assert_eq!(
        header_title(&source, "Untitled".to_string()),
        "Codex transcript"
    );
    assert_eq!(
        header_title(&file_source("main.rs"), "main.rs".to_string()),
        "main.rs"
    );
}

#[test]
fn generated_or_pathless_code_views_cannot_merge() {
    let generated = CodeSource::GeneratedReadOnly {
        title: "Transcript".to_string(),
    };
    let file = file_source("main.rs");
    let new_file = CodeSource::New {
        default_directory: None,
    };

    assert!(!can_merge_code_views(&generated, &file, true));
    assert!(!can_merge_code_views(&file, &generated, false));
    assert!(!can_merge_code_views(&file, &new_file, false));
    assert!(can_merge_code_views(&file, &file, true));
}
