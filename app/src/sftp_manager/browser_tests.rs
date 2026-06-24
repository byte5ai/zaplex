//! UI unit tests for the SFTP browser view
//!
//! Verifies view state management and action-handling logic. Uses App::test() + a mock platform,
//! with no dependency on a real SSH connection (the view starts in the Disconnected state).
//! author: logic
//! date: 2026-05-27

use std::path::PathBuf;

use warp_core::ui::appearance::Appearance;
use warpui::platform::WindowStyle;
use warpui::TypedActionView;

use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;

use pathfinder_geometry::vector::Vector2F;

use super::browser::{SftpBrowserAction, SftpBrowserView};
use super::types::{ConnectionState, Dialog, TransferDirection, TransferState};
use crate::editor::EditorView;

/// Initializes the minimal set of singletons required by the tests
fn initialize_app(app: &mut warpui::App) {
    use crate::workspace::ToastStack;

    initialize_settings_for_tests(app);
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
    app.add_singleton_model(|_| ToastStack);

    // The SSH manager needs a SQLite path; use a temporary file so that failed queries don't panic
    let temp_db = std::env::temp_dir().join("warp_sftp_test.sqlite");
    let _ = warp_ssh_manager::set_database_path(temp_db);
}

/// Creates a SftpBrowserView and places it in a window
///
/// The view starts in the Disconnected state (no SSH connection), which does not affect the UI state-logic tests.
fn create_view(app: &mut warpui::App) -> (warpui::WindowId, warpui::ViewHandle<SftpBrowserView>) {
    app.add_window(WindowStyle::NotStealFocus, |ctx| {
        SftpBrowserView::new("test-node".to_string(), ctx)
    })
}

// ============================================================
// Drag state tests
// ============================================================

/// Verifies that DragFilesEnter sets is_drag_hovering to true
#[test]
fn test_drag_files_enter() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DragFilesEnter, ctx);
        });

        view.read(&app, |view, _| {
            assert!(
                view.is_drag_hovering,
                "After DragFilesEnter, is_drag_hovering should be true"
            );
        });
    });
}

/// Verifies that DragFilesLeave sets is_drag_hovering to false
#[test]
fn test_drag_files_leave() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // First enter the hover state
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DragFilesEnter, ctx);
        });
        // Then leave
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DragFilesLeave, ctx);
        });

        view.read(&app, |view, _| {
            assert!(
                !view.is_drag_hovering,
                "After DragFilesLeave, is_drag_hovering should be false"
            );
        });
    });
}

/// Verifies that DragAndDropFiles resets is_drag_hovering
#[test]
fn test_drag_and_drop_resets_hover() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // First enter the hover state
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DragFilesEnter, ctx);
        });
        // Drop the files (no SFTP connection, so the transfer fails but does not crash)
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::DragAndDropFiles(vec![PathBuf::from("/tmp/test.txt")]),
                ctx,
            );
        });

        view.read(&app, |view, _| {
            assert!(
                !view.is_drag_hovering,
                "After DragAndDropFiles, is_drag_hovering should be reset to false"
            );
        });
    });
}

// ============================================================
// Selection state tests
// ============================================================

/// Verifies that SelectEntry selects an entry
#[test]
fn test_select_entry() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::SelectEntry(0), ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.selected.contains(&0), "After SelectEntry(0), index 0 should be selected");
        });
    });
}

/// Verifies SelectEntry selection toggling (single-select mode: re-selecting the same item keeps it selected)
#[test]
fn test_toggle_select_entry() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // Select index 2
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::SelectEntry(2), ctx);
        });
        view.read(&app, |view, _| {
            assert!(view.selected.contains(&2));
        });

        // Select index 5 → clears the previous selection, keeping only 5
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::SelectEntry(5), ctx);
        });
        view.read(&app, |view, _| {
            assert!(!view.selected.contains(&2), "After SelectEntry(5), index 2 should be deselected");
            assert!(view.selected.contains(&5), "After SelectEntry(5), index 5 should be selected");
        });
    });
}

// ============================================================
// Search filter tests
// ============================================================

/// Verifies that SetSearchFilter sets the search text
#[test]
fn test_set_search_filter() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::SetSearchFilter("txt".to_string()), ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(view.search_filter.as_deref(), Some("txt"));
        });
    });
}

/// Verifies that ClearSearchFilter clears the search text
#[test]
fn test_clear_search_filter() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // First set it
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::SetSearchFilter("log".to_string()), ctx);
        });
        // Then clear it
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::ClearSearchFilter, ctx);
        });

        view.read(&app, |view, _| {
            assert!(
                view.search_filter.is_none(),
                "After ClearSearchFilter, search_filter should be None"
            );
        });
    });
}

// ============================================================
// Navigation tests
// ============================================================

/// Verifies that NavigateUp at the root directory does not change the path
#[test]
fn test_navigate_up_from_root() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("/"));
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::NavigateUp, ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(
                view.current_path,
                PathBuf::from("/"),
                "NavigateUp from root directory should not change the path"
            );
        });
    });
}

// ============================================================
// Initial state tests
// ============================================================

/// Verifies that the view's initial state is correct
#[test]
fn test_initial_state() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.read(&app, |view, _| {
            assert!(view.entries.is_empty(), "Initial entry list should be empty");
            assert!(view.selected.is_empty(), "Initial selection set should be empty");
            assert!(view.transfers.is_empty(), "Initial transfer list should be empty");
            assert!(view.search_filter.is_none(), "Initial search filter should be None");
            assert!(!view.is_drag_hovering, "Initial drag hover state should be false");
        });
    });
}

// ============================================================
// Context menu tests
// ============================================================

/// Verifies that the ContextMenu action sets the context_menu state and selects the entry
#[test]
fn test_context_menu_sets_state() {
    use pathfinder_geometry::vector::Vector2F;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        let position = Vector2F::new(100.0, 200.0);
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::ContextMenu {
                    index: 3,
                    position,
                },
                ctx,
            );
        });

        view.read(&app, |view, _| {
            assert!(
                view.context_menu.is_some(),
                "After ContextMenu, context_menu should be Some"
            );
            let cm = view.context_menu.as_ref().unwrap();
            assert_eq!(cm.entry_index, 3, "entry_index should be 3");
            assert_eq!(cm.position, position, "position should match the provided value");
            assert!(
                view.selected.contains(&3),
                "After ContextMenu, index 3 should be selected"
            );
        });
    });
}

/// Verifies that CloseContextMenu clears the context_menu state
#[test]
fn test_close_context_menu_clears_state() {
    use pathfinder_geometry::vector::Vector2F;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // First open the context menu
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::ContextMenu {
                    index: 1,
                    position: Vector2F::new(50.0, 50.0),
                },
                ctx,
            );
        });
        view.read(&app, |view, _| {
            assert!(view.context_menu.is_some(), "Menu should be open");
        });

        // Close the menu
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::CloseContextMenu, ctx);
        });

        view.read(&app, |view, _| {
            assert!(
                view.context_menu.is_none(),
                "After CloseContextMenu, context_menu should be None"
            );
        });
    });
}

/// Verifies that ContextMenu replaces the previous menu state
#[test]
fn test_context_menu_replaces_previous() {
    use pathfinder_geometry::vector::Vector2F;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // Open the first menu
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::ContextMenu {
                    index: 0,
                    position: Vector2F::new(10.0, 10.0),
                },
                ctx,
            );
        });

        // Open the second menu (different position and index)
        let new_position = Vector2F::new(300.0, 400.0);
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::ContextMenu {
                    index: 5,
                    position: new_position,
                },
                ctx,
            );
        });

        view.read(&app, |view, _| {
            let cm = view.context_menu.as_ref().unwrap();
            assert_eq!(cm.entry_index, 5, "Should update to new entry_index");
            assert_eq!(
                cm.position, new_position,
                "Should update to new position"
            );
            assert!(
                view.selected.contains(&5),
                "Should select new index 5"
            );
            assert!(
                !view.selected.contains(&0),
                "Should deselect old index 0"
            );
        });
    });
}

// ============================================================
// Context menu boundary-condition tests
// ============================================================

/// Verifies that ContextMenu is handled correctly when index=0
#[test]
fn test_context_menu_zero_index() {
    use pathfinder_geometry::vector::Vector2F;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        let position = Vector2F::new(0.0, 0.0);
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::ContextMenu {
                    index: 0,
                    position,
                },
                ctx,
            );
        });

        view.read(&app, |view, _| {
            let cm = view.context_menu.as_ref().unwrap();
            assert_eq!(cm.entry_index, 0, "index=0 should be saved correctly");
            assert_eq!(cm.position, position, "position should be saved correctly");
            assert!(view.selected.contains(&0), "Should select index 0");
        });
    });
}

/// Verifies that ContextMenu does not panic for a large index value
#[test]
fn test_context_menu_large_index() {
    use pathfinder_geometry::vector::Vector2F;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        let position = Vector2F::new(500.0, 600.0);
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::ContextMenu {
                    index: 999,
                    position,
                },
                ctx,
            );
        });

        view.read(&app, |view, _| {
            let cm = view.context_menu.as_ref().unwrap();
            assert_eq!(cm.entry_index, 999, "Large index should be saved correctly");
            assert!(view.selected.contains(&999), "Should select large index");
        });
    });
}

/// Verifies that ContextMenu handles negative coordinates correctly
#[test]
fn test_context_menu_negative_position() {
    use pathfinder_geometry::vector::Vector2F;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        let position = Vector2F::new(-50.0, -100.0);
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::ContextMenu {
                    index: 1,
                    position,
                },
                ctx,
            );
        });

        view.read(&app, |view, _| {
            let cm = view.context_menu.as_ref().unwrap();
            assert_eq!(cm.position, position, "Negative coordinates should be saved correctly");
        });
    });
}

/// Verifies that CloseContextMenu does not panic when no menu is open
#[test]
fn test_close_context_menu_when_none() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // No menu in the initial state
        view.read(&app, |view, _| {
            assert!(view.context_menu.is_none(), "Initially there should be no menu");
        });

        // Closing directly should not panic
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::CloseContextMenu, ctx);
        });

        view.read(&app, |view, _| {
            assert!(
                view.context_menu.is_none(),
                "After closing, context_menu should still be None"
            );
        });
    });
}

/// Verifies that ContextMenu clears the previous selection and selects the new entry
#[test]
fn test_context_menu_clears_previous_selection() {
    use pathfinder_geometry::vector::Vector2F;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // First select entries 2 and 3 (via two SelectEntry calls)
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::SelectEntry(2), ctx);
        });
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::SelectEntry(3), ctx);
        });
        view.read(&app, |view, _| {
            assert!(view.selected.contains(&3), "Should select 3");
            assert!(!view.selected.contains(&2), "Single-select mode should clear 2");
        });

        // Right-click on entry 7
        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::ContextMenu {
                    index: 7,
                    position: Vector2F::new(200.0, 300.0),
                },
                ctx,
            );
        });

        view.read(&app, |view, _| {
            assert!(view.selected.contains(&7), "Should select 7");
            assert!(!view.selected.contains(&3), "Should clear old selection 3");
            assert_eq!(view.selected.len(), 1, "Should have only one selected item");
        });
    });
}

/// Verifies that repeatedly opening and closing the menu does not leak state
#[test]
fn test_context_menu_multiple_open_close_cycles() {
    use pathfinder_geometry::vector::Vector2F;

    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        for i in 0..5 {
            // Open the menu
            view.update(&mut app, |view, ctx| {
                view.handle_action(
                    &SftpBrowserAction::ContextMenu {
                        index: i,
                        position: Vector2F::new(i as f32 * 10.0, i as f32 * 20.0),
                    },
                    ctx,
                );
            });
            view.read(&app, |view, _| {
                assert!(view.context_menu.is_some(), "The {i}-th open should succeed");
            });

            // Close the menu
            view.update(&mut app, |view, ctx| {
                view.handle_action(&SftpBrowserAction::CloseContextMenu, ctx);
            });
            view.read(&app, |view, _| {
                assert!(
                    view.context_menu.is_none(),
                    "After the {i}-th close, context_menu should be None"
                );
            });
        }
    });
}

// ============================================================
// Menu-item action tests
// ============================================================

/// Verifies that the SftpBrowserAction::DetailsEntry variant is constructed correctly
#[test]
fn test_action_details_entry() {
    let action = SftpBrowserAction::DetailsEntry(42);
    assert!(matches!(action, SftpBrowserAction::DetailsEntry(42)));
}

/// Verifies that the SftpBrowserAction::DeleteEntry variant is constructed correctly
#[test]
fn test_action_delete_entry() {
    let action = SftpBrowserAction::DeleteEntry(10);
    assert!(matches!(action, SftpBrowserAction::DeleteEntry(10)));
}

/// Verifies that the SftpBrowserAction::RenameEntry variant is constructed correctly
#[test]
fn test_action_rename_entry() {
    let action = SftpBrowserAction::RenameEntry(5);
    assert!(matches!(action, SftpBrowserAction::RenameEntry(5)));
}

/// Verifies that the SftpBrowserAction::DownloadEntry variant is constructed correctly
#[test]
fn test_action_download_entry() {
    let action = SftpBrowserAction::DownloadEntry(3);
    assert!(matches!(action, SftpBrowserAction::DownloadEntry(3)));
}

/// Verifies that the SftpBrowserAction::OpenEntry variant is constructed correctly
#[test]
fn test_action_open_entry() {
    let action = SftpBrowserAction::OpenEntry(1);
    assert!(matches!(action, SftpBrowserAction::OpenEntry(1)));
}

/// Verifies that the SftpBrowserAction::ContextMenu variant is constructed correctly
#[test]
fn test_action_context_menu_variant() {
    use pathfinder_geometry::vector::Vector2F;
    let action = SftpBrowserAction::ContextMenu {
        index: 3,
        position: Vector2F::new(100.0, 200.0),
    };
    assert!(matches!(
        action,
        SftpBrowserAction::ContextMenu {
            index: 3,
            ..
        }
    ));
}

/// Verifies that the SftpBrowserAction::CloseContextMenu variant is constructed correctly
#[test]
fn test_action_close_context_menu_variant() {
    let action = SftpBrowserAction::CloseContextMenu;
    assert!(matches!(action, SftpBrowserAction::CloseContextMenu));
}

// ============================================================
// DeleteEntry action-handling tests
// ============================================================

/// Verifies that DeleteEntry does not panic when there is no SFTP connection
#[test]
fn test_delete_entry_no_connection() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // Running DeleteEntry with no SFTP connection should not panic
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DeleteEntry(0), ctx);
        });
    });
}

/// Verifies that RenameEntry does not panic when there is no SFTP connection
#[test]
fn test_rename_entry_no_connection() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::RenameEntry(0), ctx);
        });
    });
}

// ============================================================
// Category 1: dialog-operation no-connection safety tests
// ============================================================

/// Verifies that ConfirmDelete does not panic with no dialog and no connection
#[test]
fn test_confirm_delete_no_connection_no_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::ConfirmDelete, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies that ConfirmDelete is handled safely when a dialog exists but there is no connection
#[test]
fn test_confirm_delete_no_connection_with_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.dialog = Some(Dialog::DeleteConfirm {
                paths: vec![PathBuf::from("/tmp/test")],
                is_dirs: vec![false],
            });
            view.handle_action(&SftpBrowserAction::ConfirmDelete, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies that ConfirmRename does not panic with no dialog and no connection
#[test]
fn test_confirm_rename_no_connection_no_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::ConfirmRename, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies that ConfirmRename shows an error and closes the dialog when a dialog exists but there is no connection
#[test]
fn test_confirm_rename_no_connection_with_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.dialog = Some(Dialog::Rename {
                path: PathBuf::from("/home/old.txt"),
                original_name: "old.txt".to_string(),
            });
            // First enter a non-empty name to skip the empty-name check
            view.rename_editor.update(ctx, |e: &mut EditorView, ctx| {
                e.set_buffer_text("new_name", ctx);
            });
            view.handle_action(&SftpBrowserAction::ConfirmRename, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies that ConfirmNewFolder does not panic with no dialog and no connection
#[test]
fn test_confirm_new_folder_no_connection_no_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::ConfirmNewFolder, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies that ConfirmNewFolder shows an error and closes the dialog when a dialog exists but there is no connection
#[test]
fn test_confirm_new_folder_no_connection_with_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.dialog = Some(Dialog::CreateFolder {
                parent_path: PathBuf::from("/home"),
            });
            // First enter a non-empty name to skip the empty-name check
            view.new_folder_editor.update(ctx, |e: &mut EditorView, ctx| {
                e.set_buffer_text("new_folder", ctx);
            });
            view.handle_action(&SftpBrowserAction::ConfirmNewFolder, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies that ConfirmMove does not panic with no dialog and no connection
#[test]
fn test_confirm_move_no_connection_no_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::ConfirmMove, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies that ConfirmMove shows an error and closes the dialog when a dialog exists but there is no connection
#[test]
fn test_confirm_move_no_connection_with_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.dialog = Some(Dialog::Move {
                source: PathBuf::from("/home/file.txt"),
                target_dir: PathBuf::from("/home/backup"),
            });
            view.handle_action(&SftpBrowserAction::ConfirmMove, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

// ============================================================
// Category 2: navigation boundary tests
// ============================================================

/// Verifies that NavigateTo to the current path does not create a duplicate history entry
#[test]
fn test_navigate_to_same_path() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::NavigateTo(PathBuf::from("/")), ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("/"));
            assert_eq!(view.path_history.len(), 1);
        });
    });
}

/// Verifies that NavigateTo updates correctly for a deep path
#[test]
fn test_navigate_to_deep_path() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::NavigateTo(PathBuf::from("/a/b/c/d")),
                ctx,
            );
        });

        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("/a/b/c/d"));
            assert_eq!(view.path_history.len(), 2);
        });
    });
}

/// Verifies that NavigateTo normalizes backslashes to forward slashes
#[test]
fn test_navigate_to_backslash_path() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::NavigateTo(PathBuf::from(r"home\user")),
                ctx,
            );
        });

        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("home/user"));
        });
    });
}

/// Verifies that GoBack does nothing at the initial history position
#[test]
fn test_go_back_at_initial() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::GoBack, ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("/"));
        });
    });
}

/// Verifies that GoForward does nothing at the initial history position
#[test]
fn test_go_forward_at_initial() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::GoForward, ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("/"));
        });
    });
}

/// Verifies that GoUp does nothing from the root path
#[test]
fn test_go_up_from_root_via_action() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::GoUp, ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("/"));
        });
    });
}

/// Verifies that GoBack/GoForward history tracking is correct after multi-step navigation
#[test]
fn test_multiple_navigate_then_back_forward() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::NavigateTo(PathBuf::from("/home")), ctx);
            view.handle_action(&SftpBrowserAction::NavigateTo(PathBuf::from("/var")), ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("/var"));
            assert_eq!(view.path_history.len(), 3);
            assert_eq!(view.history_index, 2);
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::GoBack, ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("/home"));
            assert_eq!(view.history_index, 1);
        });

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::GoForward, ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("/var"));
            assert_eq!(view.history_index, 2);
        });
    });
}

// ============================================================
// Category 3: dialog open/close cycle tests
// ============================================================

/// Verifies that NewFolder opens the CreateFolder dialog
#[test]
fn test_new_folder_opens_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::NewFolder, ctx);
        });

        view.read(&app, |view, _| {
            assert!(matches!(view.dialog, Some(Dialog::CreateFolder { .. })));
        });
    });
}

/// Verifies that CloseDialog clears the dialog
#[test]
fn test_close_dialog_clears() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::NewFolder, ctx);
            view.handle_action(&SftpBrowserAction::CloseDialog, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies that ConfirmOverwrite closes the dialog
#[test]
fn test_confirm_overwrite_closes_dialog() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.dialog = Some(Dialog::OverwriteConfirm {
                source: PathBuf::from("/a"),
                target: PathBuf::from("/b"),
                file_size: 0,
                direction: TransferDirection::Download,
            });
            view.handle_action(&SftpBrowserAction::ConfirmOverwrite, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies that CloseDialog does not panic when there is no dialog
#[test]
fn test_close_dialog_when_none() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::CloseDialog, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies the stability of repeated dialog open/close cycles
#[test]
fn test_dialog_multiple_cycles() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        for _ in 0..3 {
            view.update(&mut app, |view, ctx| {
                view.handle_action(&SftpBrowserAction::NewFolder, ctx);
            });
            view.read(&app, |view, _| {
                assert!(view.dialog.is_some());
            });

            view.update(&mut app, |view, ctx| {
                view.handle_action(&SftpBrowserAction::CloseDialog, ctx);
            });
            view.read(&app, |view, _| {
                assert!(view.dialog.is_none());
            });
        }
    });
}

// ============================================================
// Category 4: transfer-task lifecycle tests
// ============================================================

/// Verifies that cancelling a non-existent task ID does not panic
#[test]
fn test_cancel_transfer_nonexistent_id() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::CancelTransfer(999), ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.transfers.is_empty());
        });
    });
}

/// Verifies that cancelling a non-existent task with ID 0 does not panic
#[test]
fn test_cancel_transfer_zero_id() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::CancelTransfer(0), ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.transfers.is_empty());
        });
    });
}

/// Verifies that DownloadSaveAs does not panic for an out-of-range index and does not create an orphaned task
#[test]
fn test_download_save_as_out_of_range() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::DownloadSaveAs {
                    index: 100,
                    local_path: "/tmp/out.txt".to_string(),
                },
                ctx,
            );
        });

        view.read(&app, |view, _| {
            assert!(view.transfers.is_empty());
            assert_eq!(view.next_transfer_id, 1);
        });
    });
}

/// Verifies that DownloadSaveAs does not panic for index=0 with an empty entry list
#[test]
fn test_download_save_as_zero_index_empty() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::DownloadSaveAs {
                    index: 0,
                    local_path: "/tmp/out.txt".to_string(),
                },
                ctx,
            );
        });

        view.read(&app, |view, _| {
            assert!(view.transfers.is_empty());
        });
    });
}

/// Verifies that ExecuteUpload marks the task as Failed for a non-existent local file with no connection
#[test]
fn test_execute_upload_nonexistent_file() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::ExecuteUpload("/no/such/file.txt".to_string()),
                ctx,
            );
        });

        view.read(&app, |view, _| {
            assert_eq!(view.transfers.len(), 1);
            assert!(matches!(
                view.transfers[0].state,
                TransferState::Failed(_)
            ));
        });
    });
}

// ============================================================
// Category 5: DetailsEntry boundary tests
// ============================================================

/// Verifies that DetailsEntry does not panic for an out-of-range index
#[test]
fn test_details_entry_out_of_range() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DetailsEntry(999), ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies that DetailsEntry does not panic for index=0 with no entries
#[test]
fn test_details_entry_zero_empty() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DetailsEntry(0), ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies that DetailsEntry does not panic for a very large index
#[test]
fn test_details_entry_usize_max() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DetailsEntry(usize::MAX), ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

// ============================================================
// Category 6: OpenEntry / DownloadEntry no-entry tests
// ============================================================

/// Verifies that OpenEntry does not panic for an out-of-range index and leaves the path unchanged
#[test]
fn test_open_entry_out_of_range() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::OpenEntry(999), ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("/"));
        });
    });
}

/// Verifies that OpenEntry does not panic for index=0 with no entries
#[test]
fn test_open_entry_zero_empty() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::OpenEntry(0), ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(view.current_path, PathBuf::from("/"));
        });
    });
}

/// Verifies that DownloadEntry does not panic with an empty entry list
#[test]
fn test_download_entry_empty_entries() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DownloadEntry(0), ctx);
        });
        // Passing means not panicking
    });
}

// ============================================================
// Category 7: selection and deletion boundary tests
// ============================================================

/// Verifies that DeleteSelected does not panic with an empty selection set
#[test]
fn test_delete_selected_empty_selection() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DeleteSelected, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies that DeleteSelected does not panic when there is a selection but no entries
#[test]
fn test_delete_selected_no_entries() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::SelectEntry(0), ctx);
            view.handle_action(&SftpBrowserAction::DeleteSelected, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies that SelectEntry accepts usize::MAX without panicking
#[test]
fn test_select_entry_usize_max() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::SelectEntry(usize::MAX), ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.selected.contains(&usize::MAX));
        });
    });
}

/// Verifies that each SelectEntry call clears the previous selection
#[test]
fn test_multiple_select_clears_previous() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::SelectEntry(1), ctx);
            view.handle_action(&SftpBrowserAction::SelectEntry(3), ctx);
            view.handle_action(&SftpBrowserAction::SelectEntry(7), ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(view.selected.len(), 1);
            assert!(view.selected.contains(&7));
            assert!(!view.selected.contains(&1));
            assert!(!view.selected.contains(&3));
        });
    });
}

// ============================================================
// Category 8: render safety tests
// ============================================================

/// Verifies initial state consistency (the constructor attempts to connect, so the state may be Failed or Disconnected)
#[test]
fn test_render_disconnected_state() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.read(&app, |view, _| {
            // The constructor calls connect_to_server; with no SSH service in the test environment, the state is Failed
            assert!(matches!(
                view.connection,
                ConnectionState::Failed(_) | ConnectionState::Disconnected
            ));
            assert!(!view.is_loading);
            assert!(view.entries.is_empty());
            assert!(view.dialog.is_none());
            assert!(view.context_menu.is_none());
        });
    });
}

/// Verifies that the drag hover state is set correctly
#[test]
fn test_render_with_drag_hover() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DragFilesEnter, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.is_drag_hovering);
        });
    });
}

/// Verifies that the search filter state is set correctly
#[test]
fn test_render_with_search_filter() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::SetSearchFilter("test".to_string()), ctx);
        });

        view.read(&app, |view, _| {
            assert_eq!(view.search_filter.as_deref(), Some("test"));
        });
    });
}

/// Verifies that the context menu state is set correctly
#[test]
fn test_render_with_context_menu() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::ContextMenu {
                    index: 0,
                    position: Vector2F::new(10.0, 20.0),
                },
                ctx,
            );
        });

        view.read(&app, |view, _| {
            assert!(view.context_menu.is_some());
        });
    });
}

/// Verifies that the dialog-open state is set correctly
#[test]
fn test_render_with_dialog_open() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::NewFolder, ctx);
        });

        view.read(&app, |view, _| {
            assert!(view.dialog.is_some());
        });
    });
}

/// Verifies that the state is correct after a transfer task is created
#[test]
fn test_render_with_transfer_task() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(
                &SftpBrowserAction::ExecuteUpload("/tmp/x.txt".to_string()),
                ctx,
            );
        });

        view.read(&app, |view, _| {
            assert_eq!(view.transfers.len(), 1);
        });
    });
}

/// Verifies that no panic occurs when all overlays are present at once
#[test]
fn test_render_all_overlays_combined() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DragFilesEnter, ctx);
            view.handle_action(&SftpBrowserAction::SetSearchFilter("x".to_string()), ctx);
            view.handle_action(
                &SftpBrowserAction::ContextMenu {
                    index: 0,
                    position: Vector2F::new(5.0, 5.0),
                },
                ctx,
            );
            view.handle_action(&SftpBrowserAction::NewFolder, ctx);
            view.handle_action(
                &SftpBrowserAction::ExecuteUpload("/tmp/test.txt".to_string()),
                ctx,
            );
        });

        view.read(&app, |view, _| {
            assert!(view.is_drag_hovering);
            assert!(view.search_filter.is_some());
            assert!(view.context_menu.is_some());
            assert!(view.dialog.is_some());
            assert_eq!(view.transfers.len(), 1);
        });
    });
}

/// Verifies that the state is correctly cleared after all overlays are closed
#[test]
fn test_render_after_close_all_overlays() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        // Open all overlays
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DragFilesEnter, ctx);
            view.handle_action(&SftpBrowserAction::SetSearchFilter("x".to_string()), ctx);
            view.handle_action(
                &SftpBrowserAction::ContextMenu {
                    index: 0,
                    position: Vector2F::new(5.0, 5.0),
                },
                ctx,
            );
            view.handle_action(&SftpBrowserAction::NewFolder, ctx);
        });

        // Close all overlays
        view.update(&mut app, |view, ctx| {
            view.handle_action(&SftpBrowserAction::DragFilesLeave, ctx);
            view.handle_action(&SftpBrowserAction::ClearSearchFilter, ctx);
            view.handle_action(&SftpBrowserAction::CloseContextMenu, ctx);
            view.handle_action(&SftpBrowserAction::CloseDialog, ctx);
        });

        view.read(&app, |view, _| {
            assert!(!view.is_drag_hovering);
            assert!(view.search_filter.is_none());
            assert!(view.context_menu.is_none());
            assert!(view.dialog.is_none());
        });
    });
}

/// Verifies the initial path history state
#[test]
fn test_render_path_history_initial() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.read(&app, |view, _| {
            assert_eq!(view.path_history, vec![PathBuf::from("/")]);
            assert_eq!(view.history_index, 0);
        });
    });
}

/// Verifies that the initial is_loading is false
#[test]
fn test_render_is_loading_initial_false() {
    warpui::App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (_, view) = create_view(&mut app);

        view.read(&app, |view, _| {
            assert!(!view.is_loading);
        });
    });
}
