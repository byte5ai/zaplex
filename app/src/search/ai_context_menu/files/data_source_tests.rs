use crate::search::{
    ai_context_menu::{
        files::data_source::{file_data_source_for_pwd, fuzzy_match_files, FileSnapshot},
        mixer::AIContextMenuSearchableAction,
    },
    data_source::Query,
    files::{model::FileSearchModel, search_item::FileSearchResult},
    item::SearchItem,
    mixer::AsyncDataSource,
};
use crate::{terminal::model::session::Session, workspace::ActiveSession};
use repo_metadata::repositories::DetectedRepositories;
use repo_metadata::RepoMetadataModel;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;
use warpui::platform::WindowStyle;
use warpui::r#async::block_on;
use warpui::windowing::WindowManager;
use warpui::SingletonEntity;
use warpui::{elements::Empty, App, AppContext, Element, Entity, TypedActionView, View};
struct TestView;

impl Entity for TestView {
    type Event = ();
}

impl View for TestView {
    fn ui_name() -> &'static str {
        "AIContextMenuFilesDataSourceTestView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        Empty::new().finish()
    }
}

impl TypedActionView for TestView {
    type Action = ();
}

#[test]
fn test_single_term_query() {
    let path = "/src/components/button.rs";
    let query = "button";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_multi_term_query_all_match() {
    let path = "/src/components/button.rs";
    let query = "src button";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    // Should have matches for both "src" and "button"
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_multi_term_query_partial_match() {
    let path = "/src/components/button.rs";
    let query = "src nonexistent";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    // Should return None because "nonexistent" doesn't match
    assert!(result.is_none());
}

#[test]
fn test_multi_term_query_combined_score() {
    let path = "/src/components/button.rs";

    let single_result = FileSearchModel::fuzzy_match_path(path, "button").unwrap();
    let multi_result = FileSearchModel::fuzzy_match_path(path, "src button").unwrap();

    // Multi-term query should have a higher score (combination of both matches)
    assert!(multi_result.score >= single_result.score);
}

#[test]
fn test_three_term_query() {
    let path = "/src/components/ui/button.rs";
    let query = "src ui button";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_single() {
    let path = "/src/components/button.rs";
    let query = "comp*.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
#[cfg(feature = "local_fs")]
fn test_git_changed_files_no_boost_with_query() {
    // Test that git changed files don't get boosted when there's a query
    let normal_file = "normal_file.rs";
    let changed_file = "changed_file.rs";
    let query = "file";

    let normal_match = FileSearchModel::fuzzy_match_path(normal_file, query);
    let changed_match = FileSearchModel::fuzzy_match_path(changed_file, query);

    assert!(normal_match.is_some());
    assert!(changed_match.is_some());

    // Without the git boost, scores should be similar for similar filenames
    let normal_score = normal_match.unwrap().score;
    let changed_score = changed_match.unwrap().score;

    // Scores should be roughly equal (within reasonable range)
    assert!((normal_score - changed_score).abs() < 1000);
}

#[test]
fn test_filename_priority_with_spaces() {
    let path = "/some/deep/path/important_file.rs";
    let query = "deep important";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    // Should match both "deep" in path and "important" in filename
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_star_in_filename() {
    let path = "/src/components/button.rs";
    let query = "*.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_star_in_directory() {
    let path = "/src/components/button.rs";
    let query = "src/*/button.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_star_multiple() {
    let path = "/src/components/ui/button.rs";
    let query = "*/ui/*.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_question_mark() {
    let path = "/src/components/button.rs";
    let query = "butto?.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_mixed_with_spaces() {
    let path = "/src/components/ui/button.rs";
    let query = "ui *.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_no_match() {
    let path = "/src/components/button.rs";
    let query = "*.py";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_none());
}

#[test]
fn test_wildcard_exact_match() {
    let path = "/src/components/button.rs";
    let query = "/src/components/button.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    // Should match the entire path
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_case_insensitive() {
    let path = "/src/Components/Button.rs";
    let query = "*/button.RS";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_at_beginning() {
    let path = "/src/components/button.rs";
    let query = "*button.rs";

    let result = FileSearchModel::fuzzy_match_path(path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_example_ui_star_rs() {
    // Test the specific example from the request: "ui/*.rs"
    let path1 = "/src/ui/button.rs";
    let path2 = "/src/ui/input.rs";
    let path3 = "/src/ui/modal.rs";
    let path4 = "/src/components/button.rs"; // Should not match
    let query = "ui/*.rs";

    // These should match
    let result1 = FileSearchModel::fuzzy_match_path(path1, query);
    assert!(result1.is_some());
    assert!(result1.unwrap().score > 0);

    let result2 = FileSearchModel::fuzzy_match_path(path2, query);
    assert!(result2.is_some());
    assert!(result2.unwrap().score > 0);

    let result3 = FileSearchModel::fuzzy_match_path(path3, query);
    assert!(result3.is_some());
    assert!(result3.unwrap().score > 0);

    // This should not match because it's not in the ui directory
    let result4 = FileSearchModel::fuzzy_match_path(path4, query);
    assert!(result4.is_none());
}

#[test]
fn test_path_proximity_ranking() {
    let path = "/very/long/deeply/nested/path/to/components/test.rs";

    // Test that matches closer to the filename get higher scores than matches at the beginning
    let result_near_filename = FileSearchModel::fuzzy_match_path_single(path, "components");
    let result_at_beginning = FileSearchModel::fuzzy_match_path_single(path, "very");

    assert!(result_near_filename.is_some());
    assert!(result_at_beginning.is_some());

    let near_score = result_near_filename.unwrap().score;
    let beginning_score = result_at_beginning.unwrap().score;

    // "components" is closer to the filename than "very", so it should have a higher score
    assert!(
        near_score > beginning_score,
        "Expected matches closer to filename to have higher scores. Near: {near_score}, Beginning: {beginning_score}"
    );
}

#[test]
fn test_directory_search_support() {
    crate::i18n::init(Some("en"));
    use crate::search::ai_context_menu::files::search_item::FileSearchItem;
    use fuzzy_match::FuzzyMatchResult;

    // Test that directories can be created with is_directory flag
    let directory_item = FileSearchItem {
        path: PathBuf::from("src/components/"),
        match_result: FuzzyMatchResult::no_match(),
        is_directory: true,
    };

    let file_item = FileSearchItem {
        path: PathBuf::from("src/components/button.rs"),
        match_result: FuzzyMatchResult::no_match(),
        is_directory: false,
    };

    assert!(directory_item.is_directory);
    assert!(!file_item.is_directory);

    // Test accessibility labels
    assert!(directory_item
        .accessibility_label()
        .starts_with("Directory:"));
    assert!(file_item.accessibility_label().starts_with("File:"));
}

#[test]
fn test_directory_fuzzy_matching() {
    // Test fuzzy matching works for directory paths
    let directory_path = "src/components/ui";
    let query = "comp ui";

    let result = FileSearchModel::fuzzy_match_path(directory_path, query);
    assert!(result.is_some());

    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_directory_vs_file_scoring() {
    // Test that files and directories with similar names get similar scores
    let directory_path = "src/components";
    let file_path = "src/components.rs";
    let query = "components";

    let dir_result = FileSearchModel::fuzzy_match_path(directory_path, query);
    let file_result = FileSearchModel::fuzzy_match_path(file_path, query);

    assert!(dir_result.is_some());
    assert!(file_result.is_some());

    // Both should have good scores since they match the query well
    let dir_score = dir_result.unwrap().score;
    let file_score = file_result.unwrap().score;

    assert!(dir_score > 0);
    assert!(file_score > 0);

    // Both should have reasonable scores - the exact relative scoring may vary
    // but both should be positive and meaningful
    assert!(dir_score > 0);
    assert!(file_score > 0);

    // The specific relative ordering depends on the fuzzy matching algorithm
    // Both are valid matches for "components" query
}

#[test]
fn test_mixed_file_directory_search() {
    // Test searching through a mix of files and directories
    let paths_and_types = vec![
        ("src/components", true),
        ("src/components/button.rs", false),
        ("src/components/ui", true),
        ("src/components/ui/modal.rs", false),
        ("tests/components", true),
        ("tests/components/button_test.rs", false),
    ];

    let query = "components";

    for (path, is_directory) in paths_and_types {
        let result = FileSearchModel::fuzzy_match_path(path, query);
        assert!(result.is_some(), "Failed to match path: {path}");

        let match_result = result.unwrap();
        assert!(match_result.score > 0, "Zero score for path: {path}");
        assert!(
            !match_result.matched_indices.is_empty(),
            "No match indices for path: {path}"
        );

        // Verify we can create search items for both types
        let search_item = crate::search::ai_context_menu::files::search_item::FileSearchItem {
            path: PathBuf::from(path),
            match_result,
            is_directory,
        };

        assert_eq!(search_item.is_directory, is_directory);
    }
}

#[test]
fn test_proximity_ranking_realistic_scenario() {
    // Realistic scenario: searching for "button" in different paths
    let path1 = "/very/long/deeply/nested/path/with/many/segments/button.rs"; // "button" at end
    let path2 = "/button/very/long/deeply/nested/path/with/many/segments/file.rs"; // "button" at start

    let result1 = FileSearchModel::fuzzy_match_path_single(path1, "button").unwrap();
    let result2 = FileSearchModel::fuzzy_match_path_single(path2, "button").unwrap();

    // path1 should have higher score due to "button" being closer to the filename
    assert!(
        result1.score > result2.score,
        "Expected filename match to rank higher than directory match. Filename: {}, Directory: {}",
        result1.score,
        result2.score
    );
}

fn make_file(path: &str) -> FileSearchResult {
    FileSearchResult {
        path: path.to_string(),
        project_directory: "/project".to_string(),
        is_directory: false,
    }
}

fn make_dir(path: &str) -> FileSearchResult {
    FileSearchResult {
        path: path.to_string(),
        project_directory: "/project".to_string(),
        is_directory: true,
    }
}

#[test]
fn test_fuzzy_match_files_non_empty_query() {
    let contents = vec![
        make_file("src/components/button.rs"),
        make_file("src/components/modal.rs"),
        make_file("src/utils/helpers.rs"),
    ];

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files: HashSet::new(),
        query_text: "button".to_string(),
        last_opened: HashMap::new(),
    }))
    .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].accept_result(),
        AIContextMenuSearchableAction::InsertFilePath {
            file_path: "src/components/button.rs".to_string()
        }
    );
}

#[test]
fn test_fuzzy_match_files_directory_boost() {
    let contents = vec![make_dir("src/components"), make_file("src/components.rs")];

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files: HashSet::new(),
        query_text: "components".to_string(),
        last_opened: HashMap::new(),
    }))
    .unwrap();

    assert_eq!(results.len(), 2);
    // File should score higher than directory due to the +100 boost
    let file_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/components.rs".to_string(),
                }
        })
        .expect("file result not found");
    let dir_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/components".to_string(),
                }
        })
        .expect("dir result not found");
    assert!(file_result.score() > dir_result.score());
}

#[test]
#[cfg(feature = "local_fs")]
fn test_file_data_source_for_pwd_holistic_behavior() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| DetectedRepositories::default());
        app.add_singleton_model(RepoMetadataModel::new);
        app.add_singleton_model(FileSearchModel::new);
        app.add_singleton_model(|_| ActiveSession::default());
        let test_dir = tempdir().expect("failed to create temp dir");
        let test_dir_path = test_dir.path().to_path_buf();
        fs::write(test_dir_path.join("needle.rs"), "fn main() {}")
            .expect("failed to write test file");
        fs::write(test_dir_path.join("other.txt"), "other").expect("failed to write test file");

        let (window_id, _view) = app.add_window(WindowStyle::NotStealFocus, |_ctx| TestView);
        WindowManager::handle(&app).update(&mut app, |windowing_state, _ctx| {
            windowing_state.overwrite_for_test(windowing_state.stage(), Some(window_id));
        });

        ActiveSession::handle(&app).update(&mut app, |active_session, ctx| {
            active_session.set_session_for_test(
                window_id,
                Arc::new(Session::test()),
                Some(test_dir_path),
                None,
                ctx,
            );
        });

        let data_source = app.read(file_data_source_for_pwd);

        let broad_query_results = app
            .read(|ctx| {
                data_source.run_query(
                    &Query {
                        text: "*".to_string(),
                        filters: HashSet::new(),
                    },
                    ctx,
                )
            })
            .await
            .expect("broad query run failed");
        assert!(broad_query_results.is_empty());
        let focused_query_results = app
            .read(|ctx| {
                data_source.run_query(
                    &Query {
                        text: "needle".to_string(),
                        filters: HashSet::new(),
                    },
                    ctx,
                )
            })
            .await
            .expect("focused query run failed");
        assert_eq!(focused_query_results.len(), 1);
        assert_eq!(
            focused_query_results[0].accept_result(),
            AIContextMenuSearchableAction::InsertFilePath {
                file_path: "needle.rs".to_string()
            }
        );
    });
}

/// Helper: create an `Instant` that is `millis` milliseconds after a baseline.
/// We use small sleeps to guarantee monotonically increasing instants.
fn instant_offset(millis: u64) -> instant::Instant {
    // Sleep to advance the clock relative to previous calls.
    std::thread::sleep(std::time::Duration::from_millis(millis));
    instant::Instant::now()
}

#[test]
fn test_fuzzy_query_recently_opened_bonus() {
    let opened_at = instant::Instant::now();
    let last_opened = HashMap::from([("src/components/opened_button.rs".to_string(), opened_at)]);

    let contents = vec![
        make_file("src/components/opened_button.rs"),
        make_file("src/components/other_button.rs"),
    ];

    let results = block_on(fuzzy_match_files(FileSnapshot {
        contents: Arc::new(contents),
        git_changed_files: HashSet::new(),
        query_text: "button".to_string(),
        last_opened,
    }))
    .unwrap();

    assert_eq!(results.len(), 2);
    let opened_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/components/opened_button.rs".to_string(),
                }
        })
        .expect("opened file not found");
    let other_result = results
        .iter()
        .find(|r| {
            r.accept_result()
                == AIContextMenuSearchableAction::InsertFilePath {
                    file_path: "src/components/other_button.rs".to_string(),
                }
        })
        .expect("other file not found");
    assert!(
        opened_result.score() > other_result.score(),
        "Expected recently-opened file to score higher in fuzzy mode. Opened: {:?}, Other: {:?}",
        opened_result.score(),
        other_result.score(),
    );
}
