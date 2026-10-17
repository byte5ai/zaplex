use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::compute_candidate_paths;
use crate::workspace::tab_settings::{DirectoryTabColor, DirectoryTabColors};

fn colors(entries: &[(&str, DirectoryTabColor)]) -> DirectoryTabColors {
    let map: HashMap<String, DirectoryTabColor> = entries
        .iter()
        .map(|(path, color)| ((*path).to_string(), *color))
        .collect();
    DirectoryTabColors(map)
}

fn all_exist(_: &Path) -> bool {
    true
}

#[test]
fn test_results_are_sorted_alphabetically_by_canonical_key() {
    let indexed = vec![
        PathBuf::from("/nonexistent/zulu"),
        PathBuf::from("/nonexistent/alpha"),
    ];
    let persisted = vec![PathBuf::from("/nonexistent/mango")];
    let existing = DirectoryTabColors::default();

    let candidates = compute_candidate_paths(indexed, persisted, &existing, all_exist);

    assert_eq!(
        candidates,
        vec![
            PathBuf::from("/nonexistent/alpha"),
            PathBuf::from("/nonexistent/mango"),
            PathBuf::from("/nonexistent/zulu"),
        ]
    );
}
