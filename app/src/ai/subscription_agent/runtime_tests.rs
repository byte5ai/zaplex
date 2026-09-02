use super::{legacy_ssh_candidates, ProcessLocation, SubscriptionAgent};
use crate::terminal::ssh::util::InteractiveSshCommand;

#[test]
fn legacy_ssh_candidates_run_both_agents_on_the_active_host() {
    let candidates = legacy_ssh_candidates(&InteractiveSshCommand {
        host: Some("cwendler@devhost".to_string()),
        port: Some("2222".to_string()),
    })
    .unwrap();

    assert_eq!(candidates.len(), 2);
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.installation.agent)
            .collect::<Vec<_>>(),
        vec![SubscriptionAgent::ClaudeCode, SubscriptionAgent::Codex],
    );
    for candidate in candidates {
        assert_eq!(
            candidate.installation.host.id,
            "legacy-ssh:cwendler@devhost:2222"
        );
        assert_eq!(
            candidate.location,
            ProcessLocation::Remote {
                ssh_argv: vec![
                    "ssh".to_string(),
                    "-o".to_string(),
                    "StrictHostKeyChecking=ask".to_string(),
                    "-p".to_string(),
                    "2222".to_string(),
                    "--".to_string(),
                    "cwendler@devhost".to_string(),
                ],
            }
        );
    }
}

#[test]
fn legacy_ssh_candidates_require_a_reusable_host() {
    let error = legacy_ssh_candidates(&InteractiveSshCommand::default())
        .err()
        .expect("missing SSH host must fail closed");

    assert_eq!(
        error.to_string(),
        "the active SSH session has no reusable host"
    );
}
