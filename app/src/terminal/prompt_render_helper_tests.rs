use remote_server::setup::{RemoteServerSetupState, UnsupportedReason};

use super::{remote_setup_prompt, RemoteSetupPrompt};

#[test]
fn failed_remote_setup_is_a_classic_ssh_fallback() {
    for state in [
        RemoteServerSetupState::Failed {
            error: "connection closed".to_string(),
        },
        RemoteServerSetupState::Unsupported {
            reason: UnsupportedReason::NonGlibc {
                name: "musl".to_string(),
            },
        },
    ] {
        assert_eq!(remote_setup_prompt(&state), RemoteSetupPrompt::ClassicSsh);
    }

    assert_eq!(
        remote_setup_prompt(&RemoteServerSetupState::Ready),
        RemoteSetupPrompt::Starting
    );
}

#[test]
fn active_remote_setup_preserves_stage_specific_prompt() {
    assert_eq!(
        remote_setup_prompt(&RemoteServerSetupState::Checking),
        RemoteSetupPrompt::Starting
    );
    assert_eq!(
        remote_setup_prompt(&RemoteServerSetupState::Installing {
            progress_percent: Some(42),
        }),
        RemoteSetupPrompt::Installing(Some(42))
    );
    assert_eq!(
        remote_setup_prompt(&RemoteServerSetupState::Installing {
            progress_percent: None,
        }),
        RemoteSetupPrompt::Installing(None)
    );
    assert_eq!(
        remote_setup_prompt(&RemoteServerSetupState::Updating),
        RemoteSetupPrompt::Updating
    );
    assert_eq!(
        remote_setup_prompt(&RemoteServerSetupState::Initializing),
        RemoteSetupPrompt::Initializing
    );
}
