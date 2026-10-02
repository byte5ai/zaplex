//! Runs only through script/run-daemon-terminal-acceptance with its isolated SSH fixture.
//! The scenario is registered in the Builder runner, not the environment-free Rust suite.

use crate::Builder;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use warp::integration_testing::{
    daemon_terminal as daemon,
    input::{assert_autosuggestion_state, input_contains_string, AutosuggestionState},
    native_workspace::resize,
    terminal::{
        assert_active_block_received_precmd, assert_command_executed, execute_command,
        util::{ExactLine, ExpectedExitStatus},
        wait_until_bootstrapped_pane, wait_until_bootstrapped_single_pane_for_tab,
    },
};
use warpui::integration::TestStep;

pub fn test_daemon_terminal_acceptance() -> Builder {
    let state = Arc::new(Mutex::new(daemon::State::default()));
    Builder::new()
        .use_tmp_filesystem_for_test_root_directory()
        .with_user_defaults(HashMap::from([(
            "UndoCloseEnabled".to_string(),
            false.to_string(),
        )]))
        .with_timeout(Duration::from_secs(240))
        .with_real_display()
        .with_setup(|_| {
            warp::i18n::init(Some("en"));
            daemon::stage_binary();
        })
        .with_step(wait_until_bootstrapped_single_pane_for_tab(0))
        .with_step(resize(1180., 760.))
        .with_step(daemon::prepare_host_key(Arc::clone(&state)))
        .with_step(daemon::open(Arc::clone(&state)))
        .with_step(daemon::ready(Arc::clone(&state)))
        // The daemon route is ready before the PTY shell bootstraps; Enter is
        // rejected until then. A reattach below resumes an already bootstrapped shell.
        .with_step(wait_until_bootstrapped_pane(1, 0))
        .with_step(daemon::terminal_input())
        .with_step(execute_command(
            1,
            0,
            concat!(
                "mkdir -p \"acceptance/projects/FM space's directory\" && ",
                "printf 'remote FM fixture\\n' > \"acceptance/projects/FM space's directory/remote-marker.txt\" && ",
                "cd acceptance && ",
                "export ZAPLEX_ACCEPTANCE_MARKER=daemon-pty-survived"
            )
            .into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            TestStep::new("Complete the remote directory before matching history exists")
                .with_typed_characters(&["cd pro"])
                .with_keystrokes(&["tab"])
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion(
                    "Tab expands the only remote prefix candidate",
                    input_contains_string(1, "cd projects/".into()),
                )
                .with_take_screenshot("linux-daemon-tab-completion.png"),
        )
        .with_step(
            TestStep::new("Execute the completed directory command")
                .with_keystrokes(&["enter"])
                .add_named_assertion(
                    "The completed command actually ran",
                    assert_command_executed(1, 0, "cd projects/".into()),
                )
                .add_named_assertion(
                    "The next remote prompt is ready",
                    assert_active_block_received_precmd(1, 0),
                ),
        )
        .with_step(execute_command(
            1,
            0,
            "pwd".into(),
            ExpectedExitStatus::Success,
            ExactLine::from(daemon::projects_path()),
        ))
        .with_step(execute_command(
            1,
            0,
            "cd ..".into(),
            ExpectedExitStatus::Success,
            (),
        ))
        .with_step(
            TestStep::new("Show the real daemon-session history ghost text")
                .with_typed_characters(&["cd pro"])
                .set_timeout(Duration::from_secs(30))
                .add_named_assertion(
                    "The expected suffix is visible",
                    assert_autosuggestion_state(
                        1,
                        AutosuggestionState::ActiveWithText("jects/".into()),
                    ),
                )
                .with_take_screenshot("linux-daemon-ghost-text.png"),
        )
        .with_step(
            TestStep::new("Accept the ghost text through the real keybinding")
                .with_keystrokes(&["right"])
                .add_named_assertion(
                    "Accepted text is in the editor",
                    input_contains_string(1, "cd projects/".into()),
                ),
        )
        .with_step(
            TestStep::new("Clear the unexecuted command before detaching")
                .with_keystrokes(&["ctrl-u"]),
        )
        .with_step(daemon::inventory(Arc::clone(&state), false))
        .with_step(daemon::open_remote_file_manager(Arc::clone(&state)))
        .with_step(daemon::enter_remote_file_manager_directory())
        .with_step(daemon::close_remote_file_manager(Arc::clone(&state)))
        .with_step(TestStep::new("Clear the preserved, unexecuted draft").with_keystrokes(&["ctrl-u"]))
        .with_step(execute_command(
            1,
            0,
            "pwd".into(),
            ExpectedExitStatus::Success,
            ExactLine::from(daemon::remote_file_manager_path()),
        ))
        .with_step(daemon::inventory(Arc::clone(&state), false))
        .with_step(daemon::detach(Arc::clone(&state)))
        .with_step(daemon::detached_inventory(Arc::clone(&state)))
        .with_step(daemon::reattach(Arc::clone(&state)))
        .with_step(daemon::ready(Arc::clone(&state)))
        .with_step(daemon::terminal_input())
        .with_step(execute_command(
            1,
            0,
            "printf '%s\\n' \"$ZAPLEX_ACCEPTANCE_MARKER\"".into(),
            ExpectedExitStatus::Success,
            ExactLine::from("daemon-pty-survived"),
        ))
        .with_step(
            daemon::inventory(Arc::clone(&state), true)
                .with_take_screenshot("linux-daemon-reattached.png"),
        )
        .with_step(daemon::write_identity_evidence(state))
}
