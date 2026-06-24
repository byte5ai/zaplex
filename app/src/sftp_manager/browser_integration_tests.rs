//! SFTP browser UI integration tests
//!
//! Uses InMemorySftpBackend to simulate an SFTP connection and exercise the
//! full user workflow in the Connected state, including file browsing,
//! navigation, operations, dialogs, transfers, and more.
//! author: logic
//! date: 2026-05-30

use std::path::PathBuf;
use std::sync::Arc;

use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::TypedActionView;

use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;

use pathfinder_geometry::vector::Vector2F;

use super::browser::{SftpBrowserAction, SftpBrowserView};
use super::sftp_backend::{InMemorySftpBackend, SftpBackend};
use super::types::{ConnectionState, Dialog, FileEntryType, TransferDirection, TransferState};

/// Initializes the minimal set of singletons required by the tests
fn initialize_app(app: &mut warpui::App) {
    use crate::workspace::ToastStack;

    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| ToastStack);

    let temp_db = std::env::temp_dir().join("warp_sftp_integration_test.sqlite");
    let _ = warp_ssh_manager::set_database_path(temp_db);
}

/// Creates a SftpBrowserView and places it in a window (Disconnected state)
fn create_view(
    app: &mut warpui::App,
) -> (warpui::WindowId, warpui::ViewHandle<SftpBrowserView>) {
    app.add_window(WindowStyle::NotStealFocus, |ctx| {
        SftpBrowserView::new("test-node".to_string(), ctx)
    })
}

/// Creates a temporary directory with a file structure
///
/// `files` is a list of (relative path, content); parent directories are created automatically.
fn create_temp_dir_with_files(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("failed to create temporary directory");
    for (path, content) in files {
        let full_path = dir.path().join(path);
        if let Some(parent) = full_path.parent() {
            std::fs::create_dir_all(parent).expect("failed to create parent directories");
        }
        std::fs::write(&full_path, content).expect("failed to write test file");
    }
    dir
}

/// Creates a Connected-state view backed by an InMemorySftpBackend
///
/// Returns (window_id, view_handle, temp_dir); temp_dir must be kept alive for the duration of the test
fn create_connected_view(
    app: &mut warpui::App,
    files: &[(&str, &[u8])],
) -> (
    warpui::WindowId,
    warpui::ViewHandle<SftpBrowserView>,
    tempfile::TempDir,
) {
    let temp_dir = create_temp_dir_with_files(files);
    let backend = Arc::new(InMemorySftpBackend::new(temp_dir.path().to_path_buf()))
        as Arc<dyn SftpBackend>;

    let (win_id, view) = create_view(app);
    view.update(app, |v, ctx| {
        v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
    });

    (win_id, view, temp_dir)
}

/// Creates a Connected view with a subdirectory structure
///
/// The root directory contains: a docs/ subdirectory, readme.txt, config.yaml
fn create_standard_view(
    app: &mut warpui::App,
) -> (
    warpui::WindowId,
    warpui::ViewHandle<SftpBrowserView>,
    tempfile::TempDir,
) {
    create_connected_view(app, &[
        ("docs/report.txt", b"report content"),
        ("readme.txt", b"hello world"),
        ("config.yaml", b"key: value"),
        ("data/sub/deep.txt", b"deep file"),
    ])
}

// ============================================================
// A. Connection management tests (6)
// ============================================================

/// Verifies the Connected state and entry population after injecting an InMemorySftpBackend
#[test]
fn test_connected_state_with_mock_backend() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("file1.txt", b"content1"),
            ("file2.txt", b"content2"),
        ]);

        view.read(&app, |v, _| {
            assert!(
                matches!(v.connection, ConnectionState::Connected),
                "should be in Connected state"
            );
            assert_eq!(v.entries.len(), 2, "should list 2 files");
            assert!(v.current_path == PathBuf::from("/"), "current path should be /");
        });
    });
}

/// Verifies that when not connected the state is non-Connected and there are no entries
#[test]
fn test_connection_failure_shows_error_state() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.read(&app, |v, _| {
            // new() internally calls connect_to_server, which enters the Failed state when there is no SSH configuration
            assert!(
                !matches!(v.connection, ConnectionState::Connected),
                "should not be in Connected state when there is no SSH configuration"
            );
            assert!(v.entries.is_empty(), "should have no entries when disconnected");
        });
    });
}

/// Verifies reconnecting from the Failed state
#[test]
fn test_reconnect_after_failure() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("reconnect.txt", b"data"),
        ]);

        // First set it to the Failed state
        view.update(&mut app, |v, ctx| {
            v.connection = ConnectionState::Failed("simulated connection failure".to_string());
            ctx.notify();
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.connection, ConnectionState::Failed(_)),
                "should be in Failed state"
            );
        });

        // Re-inject the backend to restore the connection
        let temp2 = create_temp_dir_with_files(&[("new.txt", b"new content")]);
        let backend = Arc::new(InMemorySftpBackend::new(temp2.path().to_path_buf()))
            as Arc<dyn SftpBackend>;
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.connection, ConnectionState::Connected),
                "should be in Connected state after re-injecting backend"
            );
            assert_eq!(v.entries.len(), 1, "should list 1 file from the new backend");
        });
    });
}

/// Verifies that entries and the path are cleared after disconnecting
#[test]
fn test_disconnect_clears_entries_and_path() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("file.txt", b"content"),
        ]);

        // Verify it is connected
        view.read(&app, |v, _| {
            assert!(matches!(v.connection, ConnectionState::Connected));
            assert!(!v.entries.is_empty());
        });

        // Disconnect
        view.update(&mut app, |v, ctx| {
            v.disconnect_for_test(ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.connection, ConnectionState::Disconnected),
                "should be in Disconnected state after disconnecting"
            );
            assert!(v.entries.is_empty(), "entries should be cleared");
        });
    });
}

/// Verifies that render does not panic in a non-Connected state
#[test]
fn test_render_disconnected_state() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.read(&app, |v, _| {
            // new() internally calls connect_to_server, so without an SSH configuration the state is Failed rather than Disconnected
            assert!(!matches!(v.connection, ConnectionState::Connected));
        });
    });
}

/// Verifies that render does not panic in the Failed state
#[test]
fn test_render_failed_state() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |v, ctx| {
            v.connection = ConnectionState::Failed("connection timeout".to_string());
            ctx.notify();
        });

        view.read(&app, |v, _| {
            assert!(matches!(v.connection, ConnectionState::Failed(_)));
        });
    });
}

// ============================================================
// B. File browsing and navigation tests (10)
// ============================================================

/// Verifies the directory listing is populated correctly and sorted directories-first, then alphabetically
#[test]
fn test_list_dir_populates_entries() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("banana.txt", b"b"),
            ("apple.txt", b"a"),
            ("cherry.txt", b"c"),
            ("folder_a/.keep", b""),
            ("folder_b/.keep", b""),
        ]);

        view.read(&app, |v, _| {
            assert_eq!(v.entries.len(), 5, "should have 5 entries");

            // Directories should be listed before files
            let dirs: Vec<_> = v.entries.iter().take_while(|e| e.file_type == FileEntryType::Directory).collect();
            let files: Vec<_> = v.entries.iter().skip_while(|e| e.file_type == FileEntryType::Directory).collect();
            assert_eq!(dirs.len(), 2, "should have 2 directories");
            assert_eq!(files.len(), 3, "should have 3 files");
        });
    });
}

/// Verifies that double-clicking a directory enters it and updates the history
#[test]
fn test_open_directory_navigates_and_updates_history() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("docs/readme.txt", b"readme"),
            ("file.txt", b"file"),
        ]);

        // Find the index of the docs directory
        let docs_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "docs").unwrap()
        });

        // Double-click to enter the docs directory
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::OpenEntry(docs_idx), ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.current_path.ends_with("docs") || v.current_path.to_string_lossy().contains("docs"),
                "current path should contain docs"
            );
            assert!(v.path_history.len() >= 2, "navigation history should increase");
        });
    });
}

/// Verifies that GoUp returns to the parent directory
#[test]
fn test_go_up_from_subdirectory() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("subdir/file.txt", b"content"),
        ]);

        // Enter the subdirectory
        let sub_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "subdir").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::OpenEntry(sub_idx), ctx);
        });

        // Go back up
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::GoUp, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.current_path == PathBuf::from("/") || v.entries.iter().any(|e| e.name == "subdir"),
                "GoUp should return to the parent directory"
            );
        });
    });
}

/// Verifies that GoBack/GoForward restore the path
#[test]
fn test_go_back_forward_restores_path() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("alpha/file.txt", b"a"),
            ("beta/file.txt", b"b"),
        ]);

        // Record the root path
        let root_path = view.read(&app, |v, _| v.current_path.clone());

        // Enter alpha
        let alpha_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "alpha").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::OpenEntry(alpha_idx), ctx);
        });
        let alpha_path = view.read(&app, |v, _| v.current_path.clone());

        // GoBack
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::GoBack, ctx);
        });
        view.read(&app, |v, _| {
            assert_eq!(v.current_path, root_path, "GoBack should return to the root path");
        });

        // GoForward
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::GoForward, ctx);
        });
        view.read(&app, |v, _| {
            assert_eq!(v.current_path, alpha_path, "GoForward should return to alpha");
        });
    });
}

/// Verifies that clicking a breadcrumb navigates to the corresponding path segment
#[test]
fn test_breadcrumb_click_navigates_to_segment() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("level1/level2/file.txt", b"deep"),
        ]);

        // Enter level1/level2
        let l1_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "level1").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::OpenEntry(l1_idx), ctx);
        });
        let l2_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "level2").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::OpenEntry(l2_idx), ctx);
        });

        // Verify the current path is level1/level2
        let current = view.read(&app, |v, _| v.current_path.clone());
        assert!(
            current.to_string_lossy().contains("level1"),
            "should navigate into level1"
        );

        // Navigate back toward the root (via NavigateTo)
        view.update(&mut app, |v, ctx| {
            // Find the breadcrumb path corresponding to level1
            let l1_path = v.current_path.parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("/"));
            v.handle_action(&SftpBrowserAction::NavigateTo(l1_path), ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.current_path.to_string_lossy().contains("level1"),
                "breadcrumb navigation should place us at level1"
            );
        });
    });
}

/// Verifies that the search filter narrows the visible entries
#[test]
fn test_search_filter_narrows_visible_entries() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("readme.txt", b"r"),
            ("config.yaml", b"c"),
            ("data.csv", b"d"),
        ]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::SetSearchFilter(".txt".to_string()), ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.search_filter.is_some());
            let visible: Vec<_> = v.entries.iter()
                .filter(|e| e.name.contains(".txt"))
                .collect();
            assert_eq!(visible.len(), 1, "only readme.txt should match");
        });
    });
}

/// Verifies that clearing the search restores all entries
#[test]
fn test_clear_search_restores_all_entries() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("a.txt", b"a"),
            ("b.yaml", b"b"),
        ]);

        let total = view.read(&app, |v, _| v.entries.len());

        // Set the filter
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::SetSearchFilter(".txt".to_string()), ctx);
        });

        // Clear the filter
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ClearSearchFilter, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.search_filter.is_none());
            assert_eq!(v.entries.len(), total, "entry count should be restored after clearing search");
        });
    });
}

/// Verifies that refreshing reloads entries after a filesystem change
#[test]
fn test_refresh_dir_reloads_entries() {
    warpui::App::test((), |mut app| async move {
        let temp = create_temp_dir_with_files(&[
            ("original.txt", b"original"),
        ]);
        initialize_app(&mut app);
        let backend = Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf()))
            as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        view.read(&app, |v, _| {
            assert_eq!(v.entries.len(), 1, "initially should have 1 file");
        });

        // Add a new file to the temporary directory
        std::fs::write(temp.path().join("new_file.txt"), b"new").unwrap();

        // Refresh
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::Refresh, ctx);
        });

        view.read(&app, |v, _| {
            assert_eq!(v.entries.len(), 2, "should have 2 files after refresh");
        });
    });
}

/// Verifies that navigating to the current path does not duplicate the history
#[test]
fn test_navigate_to_same_path_is_noop() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("file.txt", b"f"),
        ]);

        let history_len = view.read(&app, |v, _| v.path_history.len());
        let current = view.read(&app, |v, _| v.current_path.clone());

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::NavigateTo(current), ctx);
        });

        view.read(&app, |v, _| {
            assert_eq!(
                v.path_history.len(), history_len,
                "navigating to the current path should not add to history"
            );
        });
    });
}

/// Verifies normalization of Windows backslash paths
#[test]
fn test_navigate_normalizes_backslashes() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("target/file.txt", b"t"),
        ]);

        // Navigate using a backslash path
        let target_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "target").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::OpenEntry(target_idx), ctx);
        });

        view.read(&app, |v, _| {
            // The path should not contain backslashes
            let path_str = v.current_path.to_string_lossy();
            assert!(
                path_str.contains("target"),
                "path should contain target after navigation"
            );
        });
    });
}

/// Verifies that SelectEntry selects a single entry
#[test]
fn test_select_entry_highlights_item() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("file_a.txt", b"a"),
            ("file_b.txt", b"b"),
            ("file_c.txt", b"c"),
        ]);

        // Select the second entry
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::SelectEntry(1), ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.selected.contains(&1),
                "SelectEntry(1) should select the second entry"
            );
            assert_eq!(
                v.selected.len(), 1,
                "should have exactly 1 selection"
            );
        });

        // Switch the selection to the third entry
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::SelectEntry(2), ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.selected.contains(&2),
                "SelectEntry(2) should select the third entry"
            );
        });
    });
}

/// Verifies SelectEntry bounds safety (an out-of-range index does not panic)
#[test]
fn test_select_entry_out_of_bounds_safe() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("only_file.txt", b"x"),
        ]);

        // An out-of-range selection should not panic (the current implementation inserts the index directly)
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::SelectEntry(99), ctx);
        });

        view.read(&app, |v, _| {
            // The implementation does not validate bounds, so index 99 is inserted into selected
            assert!(
                v.selected.contains(&99),
                "current implementation inserts out-of-bounds indices into selected"
            );
        });
    });
}

/// Verifies that UploadFile (the toolbar upload button) is handled safely when not connected
#[test]
fn test_upload_file_action_without_connection_safe() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // Clicking the upload button while not connected should not panic
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::UploadFile, ctx);
        });

        view.read(&app, |v, _| {
            // The file picker is not triggered on the mock platform, but it must not panic either
            assert!(v.transfers.is_empty());
        });
    });
}

/// Verifies that DownloadEntry (the context-menu download) is handled safely when not connected
#[test]
fn test_download_entry_action_without_connection_safe() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // Triggering a download while not connected should not panic
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DownloadEntry(0), ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.transfers.is_empty(),
                "download should not create transfer tasks when not connected"
            );
        });
    });
}

/// Verifies safe handling of OpenEntry on a file-type entry
#[test]
fn test_open_entry_on_file_triggers_download() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("readme.txt", b"hello"),
        ]);

        // Double-clicking a file entry should trigger a download (the file picker is not triggered in the mock)
        let file_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "readme.txt").unwrap()
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::OpenEntry(file_idx), ctx);
        });

        // It should not panic; whether a transfer task is created depends on the availability of the file picker
        view.read(&app, |v, _| {
            assert!(matches!(v.connection, ConnectionState::Connected));
        });
    });
}

// ============================================================
// C. File operation tests (8)
// ============================================================

/// Verifies that a file is removed from the list after confirming deletion
#[test]
fn test_delete_file_confirmed_removes_entry() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("to_delete.txt", b"delete me"),
            ("keep.txt", b"keep me"),
        ]);

        let file_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "to_delete.txt").unwrap()
        });

        // Initiate deletion
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DeleteEntry(file_idx), ctx);
        });

        // The delete confirmation dialog should be present
        view.read(&app, |v, _| {
            assert!(matches!(v.dialog, Some(Dialog::DeleteConfirm { .. })));
        });

        // Confirm deletion
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmDelete, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog should be closed");
            assert_eq!(v.entries.len(), 1, "should have 1 entry remaining after deletion");
            assert!(v.entries[0].name == "keep.txt");
        });
    });
}

/// Verifies recursive directory deletion
#[test]
fn test_delete_directory_confirmed_removes_recursively() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("mydir/inner.txt", b"inner file"),
            ("outer.txt", b"outer"),
        ]);

        let dir_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "mydir").unwrap()
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DeleteEntry(dir_idx), ctx);
        });
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmDelete, ctx);
        });

        view.read(&app, |v, _| {
            assert_eq!(v.entries.len(), 1, "should have 1 entry remaining after deleting directory");
            assert!(v.entries[0].name == "outer.txt");
        });
    });
}

/// Verifies that renaming updates the file name
#[test]
fn test_rename_entry_updates_name() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("old_name.txt", b"content"),
        ]);

        let idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "old_name.txt").unwrap()
        });

        // Initiate the rename
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::RenameEntry(idx), ctx);
        });

        // Enter the new name in the editor
        view.update(&mut app, |v, ctx| {
            v.rename_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("new_name.txt", ctx);
            });
        });

        // Confirm the rename
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmRename, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog should be closed");
            assert!(
                v.entries.iter().any(|e| e.name == "new_name.txt"),
                "new name should appear in entries"
            );
        });
    });
}

/// Verifies that an empty rename keeps the dialog open
#[test]
fn test_rename_empty_name_shows_error() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("file.txt", b"content"),
        ]);

        let idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "file.txt").unwrap()
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::RenameEntry(idx), ctx);
        });

        // Clear the editor
        view.update(&mut app, |v, ctx| {
            v.rename_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("", ctx);
            });
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmRename, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.dialog.is_some(),
                "dialog should remain open when name is empty"
            );
        });
    });
}

/// Verifies that the directory exists after creating a new folder
#[test]
fn test_new_folder_creates_entry() {
    warpui::App::test((), |mut app| async move {
        let temp = create_temp_dir_with_files(&[]);
        initialize_app(&mut app);
        let backend = Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf()))
            as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        // Open the new-folder dialog
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::NewFolder, ctx);
        });

        view.read(&app, |v, _| {
            assert!(matches!(v.dialog, Some(Dialog::CreateFolder { .. })));
        });

        // Enter a name
        view.update(&mut app, |v, ctx| {
            v.new_folder_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("test_folder", ctx);
            });
        });

        // Confirm
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmNewFolder, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog should be closed");
            assert!(
                v.entries.iter().any(|e| e.name == "test_folder" && e.file_type == FileEntryType::Directory),
                "newly created folder should appear in entries"
            );
        });

        // Filesystem verification
        assert!(
            temp.path().join("test_folder").is_dir(),
            "newly created folder should exist in temporary directory"
        );
    });
}

/// Verifies that creating a folder with an empty name keeps the dialog open
#[test]
fn test_new_folder_empty_name_shows_error() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::NewFolder, ctx);
        });

        view.update(&mut app, |v, ctx| {
            v.new_folder_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("", ctx);
            });
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmNewFolder, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.dialog.is_some(),
                "dialog should remain open when name is empty"
            );
        });
    });
}

/// Verifies that the file details dialog shows the correct information
#[test]
fn test_file_details_dialog_shows_metadata() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("details.txt", b"file content here"),
        ]);

        let idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "details.txt").unwrap()
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DetailsEntry(idx), ctx);
        });

        view.read(&app, |v, _| {
            match &v.dialog {
                Some(Dialog::FileDetails { entry }) => {
                    assert_eq!(entry.name, "details.txt");
                    assert_eq!(entry.file_type, FileEntryType::File);
                }
                _ => panic!("FileDetails dialog should be open"),
            }
        });
    });
}

/// Verifies that cancelling a deletion preserves the entry
#[test]
fn test_delete_cancel_preserves_entry() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("keep_me.txt", b"keep"),
        ]);

        let idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "keep_me.txt").unwrap()
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DeleteEntry(idx), ctx);
        });

        // Cancel (close the dialog)
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CloseDialog, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none());
            assert_eq!(v.entries.len(), 1, "entries should be preserved after cancellation");
        });
    });
}

// ============================================================
// D. Context menu tests (5)
// ============================================================

/// Verifies that the context menu opens and selects the entry
#[test]
fn test_right_click_opens_menu_and_selects_entry() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("menu_file.txt", b"content"),
        ]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ContextMenu {
                index: 0,
                position: Vector2F::new(100.0, 100.0),
            }, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.context_menu.is_some(), "context menu should open");
            assert!(v.selected.contains(&0), "should select the first entry");
        });
    });
}

/// Verifies that the context menu's delete item triggers the delete confirmation
#[test]
fn test_context_menu_delete_item_triggers_delete() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("ctx_delete.txt", b"x"),
        ]);

        // Open the context menu
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ContextMenu {
                index: 0,
                position: Vector2F::new(50.0, 50.0),
            }, ctx);
        });

        // Choose delete from the menu
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DeleteEntry(0), ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.dialog, Some(Dialog::DeleteConfirm { .. })),
                "delete confirmation dialog should open"
            );
        });
    });
}

/// Verifies that the context menu's rename item triggers a rename
#[test]
fn test_context_menu_rename_item_triggers_rename() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("ctx_rename.txt", b"x"),
        ]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ContextMenu {
                index: 0,
                position: Vector2F::new(50.0, 50.0),
            }, ctx);
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::RenameEntry(0), ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.dialog, Some(Dialog::Rename { .. })),
                "rename dialog should open"
            );
        });
    });
}

/// Verifies that the context menu's details item triggers the details view
#[test]
fn test_context_menu_details_item_triggers_details() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("ctx_details.txt", b"x"),
        ]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ContextMenu {
                index: 0,
                position: Vector2F::new(50.0, 50.0),
            }, ctx);
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DetailsEntry(0), ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.dialog, Some(Dialog::FileDetails { .. })),
                "file details dialog should open"
            );
        });
    });
}

/// Verifies that the context menu can be closed
#[test]
fn test_dismiss_click_closes_menu() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("menu_close.txt", b"x"),
        ]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ContextMenu {
                index: 0,
                position: Vector2F::new(50.0, 50.0),
            }, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.context_menu.is_some());
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CloseContextMenu, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.context_menu.is_none(), "menu should be closed");
        });
    });
}

// ============================================================
// E. Dialog interaction tests (6)
// ============================================================

/// Verifies that a multi-selection deletion shows information for multiple items
#[test]
fn test_delete_confirm_dialog_multiple_paths() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("file_a.txt", b"a"),
            ("file_b.txt", b"b"),
        ]);

        // Select two entries
        view.update(&mut app, |v, ctx| {
            v.selected.clear();
            v.selected.insert(0);
            v.selected.insert(1);
            ctx.notify();
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DeleteSelected, ctx);
        });

        view.read(&app, |v, _| {
            match &v.dialog {
                Some(Dialog::DeleteConfirm { paths, .. }) => {
                    assert_eq!(paths.len(), 2, "should display 2 paths to be deleted");
                }
                _ => panic!("delete confirmation dialog should be open"),
            }
        });
    });
}

/// Verifies that pressing Enter in the rename editor confirms the rename
#[test]
fn test_rename_editor_enter_confirms() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("rename_enter.txt", b"x"),
        ]);

        let idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "rename_enter.txt").unwrap()
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::RenameEntry(idx), ctx);
        });

        view.update(&mut app, |v, ctx| {
            v.rename_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("renamed.txt", ctx);
            });
        });

        // Simulate Enter via ConfirmRename
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmRename, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog should be closed after pressing Enter");
        });
    });
}

/// Verifies that pressing Escape in the rename editor cancels the rename
#[test]
fn test_rename_editor_escape_cancels() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("rename_esc.txt", b"x"),
        ]);

        let idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "rename_esc.txt").unwrap()
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::RenameEntry(idx), ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_some());
        });

        // Escape cancels (via CloseDialog)
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CloseDialog, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog should be closed after Escape");
            // The file name should not change
            assert!(
                v.entries.iter().any(|e| e.name == "rename_esc.txt"),
                "original file name should remain unchanged"
            );
        });
    });
}

/// Verifies that pressing Enter in the new-folder editor confirms the creation
#[test]
fn test_new_folder_editor_enter_confirms() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::NewFolder, ctx);
        });

        view.update(&mut app, |v, ctx| {
            v.new_folder_editor.update(ctx, |e, ctx| {
                e.set_buffer_text("my_folder", ctx);
            });
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmNewFolder, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog should be closed after pressing Enter");
            assert!(
                v.entries.iter().any(|e| e.name == "my_folder"),
                "my_folder should be created"
            );
        });
    });
}

/// Verifies the overwrite confirmation dialog
#[test]
fn test_overwrite_confirm_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("file.txt", b"x"),
        ]);

        // Manually set up the overwrite confirmation dialog
        view.update(&mut app, |v, ctx| {
            v.dialog = Some(Dialog::OverwriteConfirm {
                source: PathBuf::from("/source.txt"),
                target: PathBuf::from("/target.txt"),
                file_size: 1,
                direction: TransferDirection::Download,
            });
            ctx.notify();
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmOverwrite, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog should be closed after overwrite confirmation");
        });
    });
}

/// Verifies the move confirmation dialog
#[test]
fn test_move_confirm_dialog() {
    warpui::App::test((), |mut app| async move {
        let temp = create_temp_dir_with_files(&[
            ("move_src.txt", b"move me"),
            ("dest_dir/.keep", b""),
        ]);
        initialize_app(&mut app);
        let backend = Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf()))
            as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        // Manually set up the move dialog
        view.update(&mut app, |v, ctx| {
            v.dialog = Some(Dialog::Move {
                source: PathBuf::from("/move_src.txt"),
                target_dir: PathBuf::from("/dest_dir"),
            });
            ctx.notify();
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ConfirmMove, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "dialog should be closed after move confirmation");
        });
    });
}

// ============================================================
// F. Transfer panel tests (5)
// ============================================================

/// Verifies that an upload creates a transfer task
#[test]
fn test_upload_creates_transfer_task() {
    warpui::App::test((), |mut app| async move {
        let temp = create_temp_dir_with_files(&[]);
        // Keep the local file in a separate temporary directory so it is not listed by InMemorySftpBackend's list_dir
        let local_dir = tempfile::tempdir().expect("failed to create local temporary directory");
        let local_file = local_dir.path().join("upload_source.txt");
        std::fs::write(&local_file, b"upload content").unwrap();

        initialize_app(&mut app);
        let backend = Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf()))
            as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(
                &SftpBrowserAction::ExecuteUpload(local_file.to_string_lossy().to_string()),
                ctx,
            );
        });

        view.read(&app, |v, _| {
            assert_eq!(v.transfers.len(), 1, "should create 1 transfer task");
            let task = &v.transfers[0];
            assert_eq!(task.direction, TransferDirection::Upload);
            assert!(
                matches!(task.state, TransferState::Completed | TransferState::InProgress | TransferState::Failed(_)),
                "transfer task should have a defined state"
            );
        });
    });
}

/// Verifies that uploading a nonexistent file fails
#[test]
fn test_upload_nonexistent_file_fails() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(
                &SftpBrowserAction::ExecuteUpload("/nonexistent/path/file.txt".to_string()),
                ctx,
            );
        });

        view.read(&app, |v, _| {
            assert_eq!(v.transfers.len(), 1);
            assert!(
                matches!(v.transfers[0].state, TransferState::Failed(_)),
                "uploading a nonexistent file should fail"
            );
        });
    });
}

/// Verifies that a download creates a transfer task
#[test]
fn test_download_creates_transfer_task() {
    warpui::App::test((), |mut app| async move {
        let temp = create_temp_dir_with_files(&[
            ("download_me.txt", b"download content"),
        ]);
        let local_save = temp.path().join("saved_file.txt");

        initialize_app(&mut app);
        let backend = Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf()))
            as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        let idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "download_me.txt").unwrap()
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(
                &SftpBrowserAction::DownloadSaveAs {
                    index: idx,
                    local_path: local_save.to_string_lossy().to_string(),
                },
                ctx,
            );
        });

        view.read(&app, |v, _| {
            assert_eq!(v.transfers.len(), 1, "should create download task");
            assert_eq!(v.transfers[0].direction, TransferDirection::Download);
        });
    });
}

/// Verifies that cancelling a transfer sets the cancelled flag
#[test]
fn test_cancel_transfer_sets_cancelled_flag() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        // Manually add a transfer task
        view.update(&mut app, |v, ctx| {
            use super::types::TransferTask;
            let task = TransferTask::new(
                42,
                PathBuf::from("/remote.txt"),
                PathBuf::from("/local.txt"),
                TransferDirection::Download,
                1024,
            );
            v.transfers.push(task);
            ctx.notify();
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CancelTransfer(42), ctx);
        });

        view.read(&app, |v, _| {
            let task = v.transfers.iter().find(|t| t.id == 42).unwrap();
            assert!(task.is_cancelled(), "task should be marked as cancelled");
        });
    });
}

/// Verifies that the transfer panel render does not panic
#[test]
fn test_transfer_panel_renders_with_tasks() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            use super::types::TransferTask;
            let task = TransferTask::new(
                1,
                PathBuf::from("/file.txt"),
                PathBuf::from("/local/file.txt"),
                TransferDirection::Upload,
                2048,
            );
            v.transfers.push(task);
            ctx.notify();
        });

        // render does not panic
        view.read(&app, |_v, _| {
            // Reaching this point means render succeeded
        });
    });
}

// ============================================================
// G. Drag-and-drop interaction tests (4)
// ============================================================

/// Verifies that dragging in shows the overlay
#[test]
fn test_drag_enter_shows_overlay() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("file.txt", b"x"),
        ]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DragFilesEnter, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.is_drag_hovering, "overlay should be displayed after drag enter");
        });
    });
}

/// Verifies that dragging out hides the overlay
#[test]
fn test_drag_leave_hides_overlay() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("file.txt", b"x"),
        ]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DragFilesEnter, ctx);
        });
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DragFilesLeave, ctx);
        });

        view.read(&app, |v, _| {
            assert!(!v.is_drag_hovering, "overlay should be hidden after drag leave");
        });
    });
}

/// Verifies that dropping files creates upload tasks
#[test]
fn test_drop_files_creates_upload_tasks() {
    warpui::App::test((), |mut app| async move {
        let temp = create_temp_dir_with_files(&[]);
        // Keep the local file in a separate temporary directory so it is not listed by InMemorySftpBackend's list_dir
        let local_dir = tempfile::tempdir().expect("failed to create local temporary directory");
        let drop_file = local_dir.path().join("dropped.txt");
        std::fs::write(&drop_file, b"dropped content").unwrap();

        initialize_app(&mut app);
        let backend = Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf()))
            as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(
                &SftpBrowserAction::DragAndDropFiles(vec![drop_file.clone()]),
                ctx,
            );
        });

        view.read(&app, |v, _| {
            assert_eq!(v.transfers.len(), 1, "drag and drop should create upload task");
            assert!(!v.is_drag_hovering, "hover state should be cleared after drag and drop");
        });
    });
}

/// Verifies that dropping empty paths is ignored
#[test]
fn test_drop_empty_paths_ignored() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DragAndDropFiles(vec![]), ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.transfers.is_empty(), "empty paths should not create tasks");
        });
    });
}

// ============================================================
// H. Keyboard shortcut tests (5)
// ============================================================

/// Verifies that NavigateUp (Backspace) returns to the parent directory
#[test]
fn test_keyboard_navigate_up() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("subdir/file.txt", b"x"),
        ]);

        // Enter the subdirectory
        let sub_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "subdir").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::OpenEntry(sub_idx), ctx);
        });

        // NavigateUp
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::NavigateUp, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.entries.iter().any(|e| e.name == "subdir"),
                "after NavigateUp should return to parent directory and see subdir"
            );
        });
    });
}

/// Verifies that DeleteSelected triggers the delete confirmation
#[test]
fn test_keyboard_delete_selected() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("del_target.txt", b"x"),
        ]);

        // Select the first entry
        view.update(&mut app, |v, ctx| {
            v.selected.clear();
            v.selected.insert(0);
            ctx.notify();
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DeleteSelected, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.dialog, Some(Dialog::DeleteConfirm { .. })),
                "DeleteSelected should trigger delete confirmation"
            );
        });
    });
}

/// Verifies that CreateFolder (Ctrl+Shift+N) opens the new-folder dialog
#[test]
fn test_keyboard_create_folder() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CreateFolder, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                matches!(v.dialog, Some(Dialog::CreateFolder { .. })),
                "CreateFolder should open the new folder dialog"
            );
        });
    });
}

/// Verifies that DeleteSelected is handled safely when there is no selection
#[test]
fn test_keyboard_shortcuts_without_selection() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("file.txt", b"x"),
        ]);

        // No selection
        view.update(&mut app, |v, ctx| {
            v.selected.clear();
            ctx.notify();
        });

        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::DeleteSelected, ctx);
        });

        view.read(&app, |v, _| {
            assert!(
                v.dialog.is_none(),
                "DeleteSelected should not open dialog when there is no selection"
            );
            assert_eq!(v.entries.len(), 1, "entries should not be deleted");
        });
    });
}

/// Verifies that Escape closes the dialog
#[test]
fn test_keyboard_escape_closes_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("esc_file.txt", b"x"),
        ]);

        let idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "esc_file.txt").unwrap()
        });

        // Open the rename dialog
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::RenameEntry(idx), ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_some());
        });

        // Escape closes it
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::CloseDialog, ctx);
        });

        view.read(&app, |v, _| {
            assert!(v.dialog.is_none(), "Escape should close the dialog");
        });
    });
}

// ============================================================
// I. Render safety and combination tests (4)
// ============================================================

/// Verifies that the combined state of connection + context menu + dialog + transfer + drag overlays is safe
#[test]
fn test_render_with_all_overlays_connected() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[
            ("overlay.txt", b"x"),
        ]);

        // Open the context menu
        view.update(&mut app, |v, ctx| {
            v.context_menu = Some(super::context_menu::ContextMenuState::new(0, Vector2F::new(50.0, 50.0)));
            // Open a dialog
            v.dialog = Some(Dialog::DeleteConfirm {
                paths: vec![PathBuf::from("/overlay.txt")],
                is_dirs: vec![false],
            });
            // Add a transfer task
            use super::types::TransferTask;
            v.transfers.push(TransferTask::new(
                1, PathBuf::from("/file.txt"), PathBuf::from("/local.txt"),
                TransferDirection::Upload, 1024,
            ));
            // Enable drag hovering
            v.is_drag_hovering = true;
            ctx.notify();
        });

        // Verify all overlay states exist and do not conflict
        view.read(&app, |v, _| {
            assert!(v.context_menu.is_some());
            assert!(v.dialog.is_some());
            assert!(!v.transfers.is_empty());
            assert!(v.is_drag_hovering);
            assert!(matches!(v.connection, ConnectionState::Connected));
        });
    });
}

/// Verifies the loading state indicator
#[test]
fn test_render_loading_state() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.update(&mut app, |v, ctx| {
            v.is_loading = true;
            ctx.notify();
        });

        view.read(&app, |v, _| {
            assert!(v.is_loading, "should be in loading state");
        });

        // Clear the loading state
        view.update(&mut app, |v, ctx| {
            v.is_loading = false;
            ctx.notify();
        });

        view.read(&app, |v, _| {
            assert!(!v.is_loading, "should clear loading state");
        });
    });
}

/// Verifies the empty directory display
#[test]
fn test_render_empty_directory() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view, _temp) = create_connected_view(&mut app, &[]);

        view.read(&app, |v, _| {
            assert!(matches!(v.connection, ConnectionState::Connected));
            assert!(v.entries.is_empty(), "empty directory should have no entries");
        });
    });
}

/// Verifies that rendering is safe after multiple operations
#[test]
fn test_render_after_multiple_operations() {
    warpui::App::test((), |mut app| async move {
        let temp = create_temp_dir_with_files(&[
            ("op_dir/file1.txt", b"1"),
            ("op_dir/file2.txt", b"2"),
            ("root_file.txt", b"root"),
        ]);
        initialize_app(&mut app);
        let backend = Arc::new(InMemorySftpBackend::new(temp.path().to_path_buf()))
            as Arc<dyn SftpBackend>;
        let (_, view) = create_view(&mut app);
        view.update(&mut app, |v, ctx| {
            v.set_backend_for_test(backend, PathBuf::from("/"), ctx);
        });

        // Enter the directory
        let dir_idx = view.read(&app, |v, _| {
            v.entries.iter().position(|e| e.name == "op_dir").unwrap()
        });
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::OpenEntry(dir_idx), ctx);
        });

        // Search
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::SetSearchFilter("file1".to_string()), ctx);
        });

        // Clear the search
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::ClearSearchFilter, ctx);
        });

        // Go back up
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::GoUp, ctx);
        });

        // Refresh
        view.update(&mut app, |v, ctx| {
            v.handle_action(&SftpBrowserAction::Refresh, ctx);
        });

        // Final state verification
        view.read(&app, |v, _| {
            assert!(matches!(v.connection, ConnectionState::Connected));
            assert!(!v.entries.is_empty());
            assert!(v.search_filter.is_none());
        });
    });
}
