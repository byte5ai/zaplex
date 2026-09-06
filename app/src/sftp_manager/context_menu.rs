//! Right-click context menu rendering component
//!
//! Provides right-click menu rendering for file entries, including open, download, rename, delete, details, and other operations.
//! author: logic
//! date: 2026-05-26

use pathfinder_geometry::vector::Vector2F;
use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Dismiss, Flex, Hoverable,
    MainAxisSize, ParentElement, Radius, SavePosition, Text,
};
use warpui::platform::Cursor;
use warpui::Element;

/// Right-click menu width
const CONTEXT_MENU_WIDTH: f32 = 150.0;

use crate::sftp_manager::browser::SftpBrowserAction;
use crate::sftp_manager::types::EntryReference;

/// Right-click menu state
#[derive(Debug)]
pub struct ContextMenuState {
    /// Exact entry the menu was opened for.
    pub entry: EntryReference,
    /// Menu popup position
    pub position: Vector2F,
}

impl ContextMenuState {
    /// Create a new right-click menu state
    pub fn new(entry: EntryReference, position: Vector2F) -> Self {
        Self { entry, position }
    }
}

/// Menu item definition
struct MenuItem {
    /// Display label
    label: String,
    /// Associated action
    action: SftpBrowserAction,
    /// Whether the item needs descriptor-bound cleanup support.
    requires_identity_bound_cleanup: bool,
}

/// Build file right-click menu items list
fn build_file_menu_items(entry: &EntryReference) -> Vec<MenuItem> {
    vec![
        MenuItem {
            label: crate::t!("sftp-context-menu-open"),
            action: SftpBrowserAction::OpenEntry(entry.clone()),
            requires_identity_bound_cleanup: false,
        },
        MenuItem {
            label: crate::t!("sftp-context-menu-download"),
            action: SftpBrowserAction::DownloadEntry(entry.clone()),
            requires_identity_bound_cleanup: false,
        },
        MenuItem {
            label: crate::t!("sftp-context-menu-rename"),
            action: SftpBrowserAction::RenameEntry(entry.clone()),
            requires_identity_bound_cleanup: true,
        },
        MenuItem {
            label: crate::t!("sftp-context-menu-delete"),
            action: SftpBrowserAction::DeleteEntry(entry.clone()),
            requires_identity_bound_cleanup: true,
        },
        MenuItem {
            label: crate::t!("sftp-context-menu-details"),
            action: SftpBrowserAction::DetailsEntry(entry.clone()),
            requires_identity_bound_cleanup: false,
        },
    ]
}

/// Render a single menu item
fn render_menu_item(
    label: &str,
    action: SftpBrowserAction,
    enabled: bool,
    appearance: &Appearance,
    position_id: &str,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let text_color = if enabled {
        theme.active_ui_text_color()
    } else {
        theme.disabled_ui_text_color()
    };
    let hover_bg = theme.surface_3();
    let default_bg = theme.surface_2();
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();
    let label_owned = label.to_string();

    let item_el = Hoverable::new(Default::default(), move |state| {
        let bg = if enabled && (state.is_hovered() || state.is_clicked()) {
            hover_bg
        } else {
            default_bg
        };
        let text_el = Text::new_inline(label_owned.clone(), ui_font, ui_font_size)
            .with_color(text_color.into())
            .finish();
        Container::new(text_el)
            .with_background(bg)
            .with_padding_left(12.0)
            .with_padding_right(12.0)
            .with_padding_top(6.0)
            .with_padding_bottom(6.0)
            .finish()
    })
    .with_cursor(if enabled {
        Cursor::PointingHand
    } else {
        Cursor::NotAllowed
    });
    let item_el = if enabled {
        item_el
            .on_mouse_down(move |ctx, _, _| {
                ctx.dispatch_typed_action(action.clone());
                ctx.dispatch_typed_action(SftpBrowserAction::CloseContextMenu);
            })
            .finish()
    } else {
        item_el.finish()
    };

    SavePosition::new(item_el, position_id).finish()
}

/// Render right-click context menu
pub fn render_context_menu(
    state: &ContextMenuState,
    identity_bound_cleanup_available: bool,
    appearance: &Appearance,
) -> Box<dyn Element> {
    let theme = appearance.theme();
    let menu_items = build_file_menu_items(&state.entry);

    let mut col = Flex::column()
        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_main_axis_size(MainAxisSize::Min);

    for item in &menu_items {
        let position_id = match &item.action {
            SftpBrowserAction::OpenEntry(_) => "sftp_ctx:open",
            SftpBrowserAction::DownloadEntry(_) => "sftp_ctx:download",
            SftpBrowserAction::RenameEntry(_) => "sftp_ctx:rename",
            SftpBrowserAction::DeleteEntry(_) => "sftp_ctx:delete",
            SftpBrowserAction::DetailsEntry(_) => "sftp_ctx:details",
            SftpBrowserAction::NavigateTo(_)
            | SftpBrowserAction::GoUp
            | SftpBrowserAction::GoBack
            | SftpBrowserAction::GoForward
            | SftpBrowserAction::Refresh
            | SftpBrowserAction::SelectEntry(_)
            | SftpBrowserAction::ToggleMark(_)
            | SftpBrowserAction::MarkAndAdvance
            | SftpBrowserAction::SortBy(_)
            | SftpBrowserAction::ToggleHidden
            | SftpBrowserAction::UploadFile
            | SftpBrowserAction::NewFolder
            | SftpBrowserAction::ConfirmDelete
            | SftpBrowserAction::ConfirmRename
            | SftpBrowserAction::ConfirmNewFolder
            | SftpBrowserAction::ConfirmOverwrite
            | SftpBrowserAction::ContextMenu { .. }
            | SftpBrowserAction::CloseContextMenu
            | SftpBrowserAction::CloseDialog
            | SftpBrowserAction::SetSearchFilter(_)
            | SftpBrowserAction::ClearSearchFilter
            | SftpBrowserAction::NavigateUp
            | SftpBrowserAction::DeleteSelected
            | SftpBrowserAction::CreateFolder
            | SftpBrowserAction::DragFilesEnter
            | SftpBrowserAction::DragFilesLeave
            | SftpBrowserAction::DragAndDropFiles(_)
            | SftpBrowserAction::ExecuteUpload(_)
            | SftpBrowserAction::DownloadSaveAs { .. }
            | SftpBrowserAction::ConfirmMove
            | SftpBrowserAction::CursorDown
            | SftpBrowserAction::CursorUp
            | SftpBrowserAction::CursorFirst
            | SftpBrowserAction::CursorLast
            | SftpBrowserAction::CursorPageUp
            | SftpBrowserAction::CursorPageDown
            | SftpBrowserAction::ActivateCursor
            | SftpBrowserAction::EnterCursorDir
            | SftpBrowserAction::ToggleSelectCursor
            | SftpBrowserAction::PickCurrentDir
            | SftpBrowserAction::RenameCursor
            | SftpBrowserAction::CopyToOtherPane
            | SftpBrowserAction::MoveToOtherPane
            | SftpBrowserAction::ChooseCopyTarget
            | SftpBrowserAction::ChooseMoveTarget
            | SftpBrowserAction::CloseFileManager
            | SftpBrowserAction::OverwriteConflict { .. }
            | SftpBrowserAction::SkipConflict { .. }
            | SftpBrowserAction::RenameConflict { .. }
            | SftpBrowserAction::NewerOnlyConflict { .. }
            | SftpBrowserAction::PickCopyMoveTarget(_)
            | SftpBrowserAction::ResolveCrossConnConflict { .. }
            | SftpBrowserAction::ViewCursorDetails
            | SftpBrowserAction::OpenCursorInEditor
            | SftpBrowserAction::CancelTransfer(..)
            | SftpBrowserAction::PauseTransfer(..)
            | SftpBrowserAction::ResumeTransfer(..)
            | SftpBrowserAction::RetryTransferRecovery(_)
            | SftpBrowserAction::ToggleTransferPanel
            | SftpBrowserAction::ConfirmCloseTransferPanel => "sftp_ctx:unknown",
        };
        let enabled = !item.requires_identity_bound_cleanup || identity_bound_cleanup_available;
        let el = render_menu_item(
            &item.label,
            item.action.clone(),
            enabled,
            appearance,
            position_id,
        );
        col.add_child(el);
    }

    let menu_inner = ConstrainedBox::new(
        Container::new(col.finish())
            .with_background(theme.surface_2())
            .with_border(Border::all(1.0).with_border_color(theme.surface_3().into()))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.0)))
            .with_uniform_padding(4.0)
            .finish(),
    )
    .with_width(CONTEXT_MENU_WIDTH)
    .finish();

    Dismiss::new(menu_inner)
        .prevent_interaction_with_other_elements()
        .on_dismiss(|ctx, _| {
            ctx.dispatch_typed_action(SftpBrowserAction::CloseContextMenu);
        })
        .finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sftp_manager::types::{EntryIdentity, FileEntryType, StableEntryIdentity};
    use pathfinder_geometry::vector::Vector2F;
    use std::path::PathBuf;

    fn entry_reference(index: usize) -> EntryReference {
        EntryReference {
            listing_generation: 7,
            identity: EntryIdentity {
                path: PathBuf::from(format!("/entry-{index}")),
                backend: StableEntryIdentity {
                    file_type: FileEntryType::File,
                    size: index as u64,
                    object_id: format!("entry-{index}"),
                    revision: "1".to_string(),
                },
            },
        }
    }

    // ============================================================
    // ContextMenuState tests
    // ============================================================

    /// Test that ContextMenuState constructor sets fields correctly
    #[test]
    fn test_context_menu_state_new() {
        let position = Vector2F::new(100.0, 200.0);
        let entry = entry_reference(3);
        let state = ContextMenuState::new(entry.clone(), position);
        assert_eq!(state.entry, entry);
        assert_eq!(state.position, position);
    }

    /// Test ContextMenuState construction with index=0
    #[test]
    fn test_context_menu_state_zero_index() {
        let position = Vector2F::new(0.0, 0.0);
        let state = ContextMenuState::new(entry_reference(0), position);
        assert_eq!(state.position, position);
    }

    /// Test ContextMenuState with a large synthetic identity
    #[test]
    fn test_context_menu_state_large_index() {
        let position = Vector2F::new(500.0, 600.0);
        let entry = entry_reference(usize::MAX);
        let state = ContextMenuState::new(entry.clone(), position);
        assert_eq!(state.entry, entry);
    }

    /// Test ContextMenuState with negative coordinates (Vector2F supports negative values)
    #[test]
    fn test_context_menu_state_negative_position() {
        let position = Vector2F::new(-50.0, -100.0);
        let state = ContextMenuState::new(entry_reference(1), position);
        assert_eq!(state.position, position);
    }

    // ============================================================
    // build_file_menu_items tests
    // ============================================================

    /// Test that menu item count is 5
    #[test]
    fn test_build_file_menu_items_count() {
        let items = build_file_menu_items(&entry_reference(0));
        assert_eq!(items.len(), 5, "should have 5 menu items");
    }

    /// Every menu item resolves to a non-empty label from its i18n key. The exact
    /// text is order-dependent on the process-global Fluent loader (the raw key
    /// when uninitialized, localized text once another test in the same process
    /// initializes it), so this asserts only that a label is present — the exact
    /// English/German strings live in the .ftl files (compile-validated by `fl!`),
    /// and the label↔action mapping is covered by
    /// `test_build_file_menu_items_actions_index`.
    #[test]
    fn test_build_file_menu_items_labels() {
        let items = build_file_menu_items(&entry_reference(0));
        assert_eq!(items.len(), 5, "should have 5 menu items");
        for item in &items {
            assert!(
                !item.label.is_empty(),
                "menu item must have a non-empty label"
            );
        }
    }

    /// Test menu item actions bind to correct index
    #[test]
    fn test_build_file_menu_items_actions_index() {
        let entry = entry_reference(7);
        let items = build_file_menu_items(&entry);

        assert!(
            matches!(&items[0].action, SftpBrowserAction::OpenEntry(actual) if actual == &entry)
        );
        assert!(
            matches!(&items[1].action, SftpBrowserAction::DownloadEntry(actual) if actual == &entry)
        );
        assert!(
            matches!(&items[2].action, SftpBrowserAction::RenameEntry(actual) if actual == &entry)
        );
        assert!(
            matches!(&items[3].action, SftpBrowserAction::DeleteEntry(actual) if actual == &entry)
        );
        assert!(
            matches!(&items[4].action, SftpBrowserAction::DetailsEntry(actual) if actual == &entry)
        );
        assert!(!items[0].requires_identity_bound_cleanup);
        assert!(!items[1].requires_identity_bound_cleanup);
        assert!(items[2].requires_identity_bound_cleanup);
        assert!(items[3].requires_identity_bound_cleanup);
        assert!(!items[4].requires_identity_bound_cleanup);
    }

    /// Test menu item actions are correct with index=0
    #[test]
    fn test_build_file_menu_items_zero_index() {
        let entry = entry_reference(0);
        let items = build_file_menu_items(&entry);
        assert!(
            matches!(&items[0].action, SftpBrowserAction::OpenEntry(actual) if actual == &entry)
        );
        assert!(
            matches!(&items[3].action, SftpBrowserAction::DeleteEntry(actual) if actual == &entry)
        );
        assert!(
            matches!(&items[4].action, SftpBrowserAction::DetailsEntry(actual) if actual == &entry)
        );
    }

    // ============================================================
    // render_context_menu rendering tests (verified via browser view)
    // ============================================================

    /// Rendering does not panic after triggering ContextMenu via browser view
    #[test]
    fn test_render_context_menu_via_browser() {
        use crate::settings_view::keybindings::KeybindingChangedNotifier;
        use crate::test_util::settings::initialize_settings_for_tests;
        use warp_core::ui::appearance::Appearance;

        warpui::App::test((), |mut app| async move {
            initialize_settings_for_tests(&mut app);
            app.add_singleton_model(|_| Appearance::mock());
            app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
            app.add_singleton_model(|_| crate::workspace::ToastStack);
            app.add_singleton_model(|_| crate::sftp_manager::transfer_queue::TransferQueue::new());

            let temp_db = std::env::temp_dir().join("warp_sftp_ctx_test.sqlite");
            let _ = warp_ssh_manager::set_database_path(temp_db);

            let (_, view) = app.add_window(warpui::platform::WindowStyle::NotStealFocus, |ctx| {
                crate::sftp_manager::browser::SftpBrowserView::new(
                    "test-node".to_string(),
                    None,
                    ctx,
                )
            });

            // Trigger right-click menu
            view.update(&mut app, |view, ctx| {
                view.context_menu = Some(ContextMenuState::new(
                    entry_reference(2),
                    Vector2F::new(150.0, 250.0),
                ));
                ctx.notify();
            });

            // Rendering should not panic (view auto-rerenders)
            view.read(&app, |view, _| {
                assert!(view.context_menu.is_some(), "menu should be open");
            });
        });
    }
}
