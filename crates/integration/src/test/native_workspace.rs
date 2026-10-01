//! Pixel and interaction evidence from the native app, not the HTML reference.
//! Remote inventory in the first scenario is explicitly fixture data. Local
//! panes and file operations in the other scenarios use real isolated shells.

use warp::integration_testing::{
    native_workspace as native,
    terminal::{
        execute_command, util::ExpectedExitStatus, wait_until_bootstrapped_pane,
        wait_until_bootstrapped_single_pane_for_tab,
    },
};
use warpui::integration::TestStep;

use crate::Builder;

// A 640px content height fits the macOS CI display below its menu bar and Dock.

pub fn test_native_workspace_connections_evidence() -> Builder {
    let mut builder = Builder::new()
        .with_setup(|_| warp::i18n::init(Some("en")))
        .with_real_display()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(native::resize(1440., 640.))
        .with_step(native::seed_connections())
        .with_step(native::connection_layout(250., true))
        .with_step(native::seed_session_inventory());
    for (light, name) in [(false, "dark"), (true, "light")] {
        builder = builder.with_step(native::theme(light));
        for width in [250, 320, 480] {
            builder = builder
                .with_step(native::connection_layout(width as f32, true))
                .with_step(native::hover_first_connection())
                .with_step(native::connection_layout(width as f32, false))
                .with_step(
                    native::session_layout()
                        .with_take_screenshot(format!("native-connections-{name}-{width}.png")),
                );
        }
    }
    builder
        .with_step(native::resize(900., 640.))
        .with_step(native::connection_layout(250., true))
        .with_step(native::open_favorites())
        .with_step(native::favorite_flyout().with_take_screenshot("native-favorite-flyout.png"))
        .with_step(native::dismiss_flyout())
}

pub fn test_native_workspace_panes_evidence() -> Builder {
    Builder::new()
        .with_setup(|_| warp::i18n::init(Some("en")))
        .with_real_display()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(native::resize(1180., 640.))
        .with_step(execute_command(
            0,
            0,
            "mkdir -p native-acceptance-projects && cd native-acceptance-projects".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(native::assert_focused_directory_title(
            "native-acceptance-projects",
        ))
        .with_step(native::split_local_right())
        .with_step(wait_until_bootstrapped_pane(0, 0))
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(
            TestStep::new("Keep a real unexecuted draft in the newly split pane")
                .with_typed_characters(&["printf retained-draft"]),
        )
        .with_step(
            native::assert_split_and_save_identity("printf retained-draft")
                .with_take_screenshot("native-panes-side-by-side.png"),
        )
        .with_step(native::custom_tab_title())
        .with_steps(native::drag_first_pane_below_second())
        .with_step(
            TestStep::new("Capture the actual mouse-drop result")
                .with_take_screenshot("native-panes-after-drag.png"),
        )
}

pub fn test_native_workspace_file_managers_evidence() -> Builder {
    Builder::new()
        .with_setup(|_| warp::i18n::init(Some("en")))
        .with_real_display()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(native::resize(1180., 640.))
        .with_step(native::split_local_right())
        .with_step(wait_until_bootstrapped_pane(0, 0))
        .with_step(wait_until_bootstrapped_pane(0, 1))
        .with_step(
            TestStep::new("Retain a shell draft while browsing files")
                .with_typed_characters(&["printf file-manager-draft"]),
        )
        .with_step(native::assert_split_and_save_identity(
            "printf file-manager-draft",
        ))
        .with_step(native::open_local_file_manager(0))
        .with_step(native::open_local_file_manager(1))
        .with_step(
            native::file_manager_layout().with_take_screenshot("native-file-managers-normal.png"),
        )
        .with_step(native::resize(760., 640.))
        .with_step(
            native::file_manager_layout().with_take_screenshot("native-file-managers-narrow.png"),
        )
        .with_step(native::inactive_file_manager_action())
        .with_step(
            native::focused_file_manager_action()
                .with_take_screenshot("native-file-manager-focused-action.png"),
        )
        .with_step(TestStep::new("Cancel the Create Folder dialog").with_keystrokes(&["escape"]))
        .with_step(native::close_file_manager(1))
        .with_step(native::focus_remaining_file_manager())
        .with_step(native::close_file_manager(0))
        .with_step(
            native::assert_retained_terminals()
                .with_take_screenshot("native-file-managers-restored-terminals.png"),
        )
}
