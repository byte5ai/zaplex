use super::{
    ProcessLocation, SubscriptionAgent, legacy_ssh_candidates, remote_candidates_for_resolved_ssh,
};
use crate::terminal::ssh::util::InteractiveSshCommand;
use warp_ssh_manager::{
    AuthType, ResolvedSshConnection, SecretKind, SessionResilience, SshServerInfo,
};

#[test]
fn legacy_ssh_candidates_run_both_agents_on_the_active_host() {
    let candidates = legacy_ssh_candidates(&InteractiveSshCommand {
        host: Some("developer@ssh.example.test".to_string()),
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
            "legacy-ssh:developer@ssh.example.test:2222"
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
                    "developer@ssh.example.test".to_string(),
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

#[test]
fn remote_onekey_key_uses_shared_credential() {
    let connection = ResolvedSshConnection {
        server: SshServerInfo {
            node_id: "host-1".to_string(),
            host: "example.test".to_string(),
            port: 22,
            username: "deploy".to_string(),
            auth_type: AuthType::Key,
            key_path: Some("/keys/deploy".to_string()),
            credential_id: Some("cred-1".to_string()),
            startup_command: None,
            notes: None,
            last_connected_at: None,
            session_resilience: SessionResilience::PersistOnly,
            ring_ceiling_mb: 0,
        },
        secret_lookup_id: "cred-1".to_string(),
        secret_kind: SecretKind::Passphrase,
    };

    let candidates = remote_candidates_for_resolved_ssh("daemon-1", "edge", &connection);
    for candidate in candidates {
        let ProcessLocation::Remote { ssh_argv } = candidate.location else {
            panic!("resolved OneKey host must remain remote");
        };
        assert!(
            ssh_argv
                .windows(2)
                .any(|args| args == ["-i", "/keys/deploy"])
        );
        assert_eq!(
            ssh_argv.last().map(String::as_str),
            Some("deploy@example.test")
        );
    }
}
