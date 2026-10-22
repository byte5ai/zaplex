//! Native file-manager acceptance over an isolated, real local filesystem.

use std::time::Duration;

use warpui::elements::get_rich_content_position_id;
use warpui::event::Event;
use warpui::integration::{AssertionOutcome, TestStep};
use warpui::keymap::PerPlatformKeystroke;

use super::sftp;
use crate::sftp_manager::types::Dialog;

pub fn select_entry(name: &'static str) -> TestStep {
    TestStep::new(&format!("Select the rendered {name} row"))
        .with_click_on_saved_position_fn(move |app, window_id| {
            let index = sftp::sftp_browser_view(app, window_id).read(app, |view, _| {
                view.entries()
                    .iter()
                    .position(|entry| entry.name == name)
                    .expect("fixture row")
            });
            sftp::row_position_id(app, window_id, index)
        })
        .add_named_assertion(
            "The clicked file is the cursor target",
            move |app, window_id| {
                if !sftp::cursor_path(app, window_id).is_some_and(|path| path.ends_with(name)) {
                    return AssertionOutcome::failure(format!("Cursor did not select {name}"));
                }
                AssertionOutcome::Success
            },
        )
}

pub fn fill_dialog(value: &'static str) -> TestStep {
    TestStep::new("Click the visible dialog editor and type its new value")
        .with_click_on_saved_position_fn(|app, window_id| {
            sftp::sftp_browser_view(app, window_id).read(app, |view, _| {
                let editor = match view.dialog() {
                    Some(Dialog::Rename { .. }) => &view.rename_editor,
                    Some(Dialog::CreateFolder { .. }) => &view.new_folder_editor,
                    _ => panic!("expected rename or folder dialog"),
                };
                get_rich_content_position_id(&editor.id())
            })
        })
        .with_per_platform_keystroke(PerPlatformKeystroke {
            mac: "cmd-a",
            linux_and_windows: "ctrl-a",
        })
        .with_input_string(value, None)
        .add_named_assertion(
            "Real keyboard input reached the dialog",
            move |app, window_id| {
                sftp::sftp_browser_view(app, window_id).read(app, |view, ctx| {
                    let editor = match view.dialog() {
                        Some(Dialog::Rename { .. }) => &view.rename_editor,
                        Some(Dialog::CreateFolder { .. }) => &view.new_folder_editor,
                        _ => return AssertionOutcome::failure("Dialog closed before input".into()),
                    };
                    if editor.as_ref(ctx).buffer_text(ctx) != value {
                        return AssertionOutcome::failure(
                            "Dialog input differs from the typed value".into(),
                        );
                    }
                    AssertionOutcome::Success
                })
            },
        )
}

pub fn folder_created() -> TestStep {
    TestStep::new("Confirm folder creation without changing the original cursor target")
        .with_keystrokes(&["enter"])
        .add_named_assertion(
            "Folder exists and the original file remains selected",
            |app, window_id| {
                sftp::sftp_browser_view(app, window_id).read(app, |view, _| {
                    let cursor = view
                        .cursor_entry_index()
                        .and_then(|index| view.entries().get(index));
                    if view.dialog().is_some()
                        || cursor.is_none_or(|entry| entry.name != "ORIGINAL.txt")
                        || !view.current_path.join("CREATED").is_dir()
                    {
                        return AssertionOutcome::failure(
                            "Folder creation changed the cursor target or did not complete".into(),
                        );
                    }
                    AssertionOutcome::Success
                })
            },
        )
}

pub fn canceled_dialog() -> TestStep {
    TestStep::new("Cancel the typed folder name with Escape")
        .with_keystrokes(&["escape"])
        .add_named_assertion(
            "Cancel creates no folder and closes the dialog",
            |app, window_id| {
                sftp::sftp_browser_view(app, window_id).read(app, |view, _| {
                    if view.dialog().is_some() || view.current_path.join("CANCELED").exists() {
                        return AssertionOutcome::failure(
                            "Cancel left a dialog or created the folder".into(),
                        );
                    }
                    AssertionOutcome::Success
                })
            },
        )
}

pub fn enter_nested_after_cancel() -> TestStep {
    TestStep::new("Enter opens the next folder after Escape and Down, without another focus action")
        .with_keystrokes(&["enter"])
        .add_named_assertion(
            "Nested directory was opened through keyboard routing",
            |app, window_id| {
                sftp::sftp_browser_view(app, window_id).read(app, |view, _| {
                    if !view.current_path.ends_with("nested")
                        || !view.entries().iter().any(|entry| entry.name == "CHILD.txt")
                    {
                        return AssertionOutcome::failure(
                            "Keyboard did not open nested/CHILD.txt".into(),
                        );
                    }
                    AssertionOutcome::Success
                })
            },
        )
}

pub fn renamed_through_alias() -> TestStep {
    TestStep::new("Confirm the rename through the directory alias")
        .with_keystrokes(&["enter"])
        .add_named_assertion(
            "Rename preserves contents through the logical path",
            |app, window_id| {
                sftp::sftp_browser_view(app, window_id).read(app, |view, _| {
                    if !view.current_path.ends_with("native-alias")
                        || view.current_path.join("ORIGINAL.txt").exists()
                        || std::fs::read(view.current_path.join("RENAMED.txt"))
                            .ok()
                            .as_deref()
                            != Some(b"ALPHA")
                        || !view
                            .entries()
                            .iter()
                            .any(|entry| entry.name == "RENAMED.txt")
                    {
                        return AssertionOutcome::failure(
                            "Alias rename did not preserve the selected file".into(),
                        );
                    }
                    AssertionOutcome::Success
                })
            },
        )
}

pub fn details_tooltip() -> TestStep {
    TestStep::new("Hover the F3 Details action")
        .with_event_fn(|app, window_id| {
            let position_id = sftp::sftp_browser_view(app, window_id).read(app, |view, _| {
                view.layout_position_id_for_test("function-F3")
            });
            Event::MouseMoved {
                position: super::native_workspace::bounds(app, window_id, &position_id)
                    .expect("F3 action geometry")
                    .center(),
                cmd: false,
                shift: false,
                is_synthetic: false,
            }
        })
        .set_post_step_pause(Duration::from_millis(300))
        .add_named_assertion("Metadata tooltip is visible", |app, window_id| {
            let position_id = sftp::sftp_browser_view(app, window_id).read(app, |view, _| {
                view.layout_position_id_for_test("details-tooltip")
            });
            if super::native_workspace::bounds(app, window_id, &position_id).is_none() {
                return AssertionOutcome::failure("Metadata tooltip is missing".into());
            }
            AssertionOutcome::Success
        })
}

pub fn details_dialog() -> TestStep {
    TestStep::new("F3 opens metadata for the selected file")
        .with_keystrokes(&["f3"])
        .add_named_assertion("Details belong to the renamed five-byte file", |app, window_id| {
            sftp::sftp_browser_view(app, window_id).read(app, |view, _| {
                if !matches!(view.dialog(), Some(Dialog::FileDetails { entry }) if entry.name == "RENAMED.txt" && entry.size == 5) {
                    return AssertionOutcome::failure("F3 did not show the renamed file metadata".into());
                }
                AssertionOutcome::Success
            })
        })
}
