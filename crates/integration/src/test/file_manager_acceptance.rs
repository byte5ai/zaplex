//! Native regression evidence for file actions through a real directory alias.

use warp::integration_testing::{
    file_manager_acceptance as files, native_workspace as native,
    terminal::{
        execute_command, util::ExpectedExitStatus, wait_until_bootstrapped_single_pane_for_tab,
    },
};
use warpui::integration::TestStep;

use crate::Builder;

pub fn test_native_file_actions_evidence() -> Builder {
    file_actions_evidence("en")
}

pub fn test_native_file_actions_german_evidence() -> Builder {
    file_actions_evidence("de")
}

fn file_actions_evidence(locale: &'static str) -> Builder {
    warp::i18n::set_locale(locale);
    Builder::new()
        .with_real_display()
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(native::resize(1024., 640.))
        .with_step(native::theme(false))
        .with_step(execute_command(
            0,
            0,
            "mkdir -p native-physical/nested && printf ALPHA > native-physical/ORIGINAL.txt && printf BETA > native-physical/nested/CHILD.txt && ln -s native-physical native-alias && cd native-alias && ZAPLEX_PROOF=2111".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(native::open_file_manager_by_header())
        .with_step(files::select_entry("ORIGINAL.txt").with_take_screenshot("file-actions-selected.png"))
        .with_step(TestStep::new("Open Create Folder with F7").with_keystrokes(&["f7"]))
        .with_step(files::fill_dialog("CREATED"))
        .with_step(files::folder_created().with_take_screenshot("file-actions-created-cursor.png"))
        .with_step(files::select_entry("CREATED"))
        .with_step(TestStep::new("Open another Create Folder dialog").with_keystrokes(&["f7"]))
        .with_step(files::fill_dialog("CANCELED").with_take_screenshot("file-actions-cancel-before.png"))
        .with_step(files::canceled_dialog())
        .with_step(TestStep::new("Move to nested with the Down key after cancellation").with_keystrokes(&["down"]))
        .with_step(files::enter_nested_after_cancel().with_take_screenshot("file-actions-keyboard-after-cancel.png"))
        .with_step(TestStep::new("Return to the parent with Backspace").with_keystrokes(&["backspace"]))
        .with_step(files::select_entry("ORIGINAL.txt"))
        .with_step(TestStep::new("Open Rename with F2").with_keystrokes(&["f2"]))
        .with_step(files::fill_dialog("RENAMED.txt").with_take_screenshot("file-actions-rename-before.png"))
        .with_step(files::renamed_through_alias().with_take_screenshot("file-actions-rename-after.png"))
        .with_step(files::select_entry("RENAMED.txt"))
        .with_step(files::details_tooltip().with_take_screenshot("file-actions-details-tooltip.png"))
        .with_step(files::details_dialog().with_take_screenshot("file-actions-details-dialog.png"))
        .with_step(TestStep::new("Close File Details with Escape").with_keystrokes(&["escape"]))
        .with_step(TestStep::new("Return to the original shell with F10").with_keystrokes(&["f10"]))
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(execute_command(
            0,
            0,
            "test \"$ZAPLEX_PROOF\" = 2111 && test ! -e ORIGINAL.txt && test -d CREATED && test ! -e CANCELED && test \"$(cat RENAMED.txt)\" = ALPHA && printf 'FILE-ACTIONS-PASS\\n'".into(),
            ExpectedExitStatus::Success,
            "FILE-ACTIONS-PASS".to_string(),
        ).with_take_screenshot("file-actions-shell-preserved.png"))
}
