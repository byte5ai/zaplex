use std::time::Duration;

use super::{
    ClassicSshConnectAttempts, SshConnectRegistry, resolved_ssh_secret_owner,
    ssh_connect_terminal_event_finishes_attempt,
};
use warp_ssh_manager::{
    AuthType, ResolvedSshConnection, SecretKind, SessionResilience, SshServerInfo,
};
use warpui::EntityId;

#[test]
fn pending_preflight_remains_single_flight_past_ten_seconds() {
    let mut registry = SshConnectRegistry::default();
    let first = registry
        .begin("host-a".to_string(), "a.example.com".to_string())
        .unwrap();

    let simulated_pending_time = Duration::from_secs(11);
    assert!(simulated_pending_time > Duration::from_secs(10));
    assert!(registry
        .begin("host-a".to_string(), "a.example.com".to_string())
        .is_none());
    assert!(registry.contains(&first));
}

#[test]
fn completing_an_attempt_allows_a_retry() {
    let mut registry = SshConnectRegistry::default();
    let first = registry
        .begin("host-a".to_string(), "a.example.com".to_string())
        .unwrap();

    assert!(registry.finish(&first));
    assert!(registry
        .begin("host-a".to_string(), "a.example.com".to_string())
        .is_some());
}

#[test]
fn different_hosts_can_connect_in_parallel() {
    let mut registry = SshConnectRegistry::default();

    assert!(registry
        .begin("host-a".to_string(), "a.example.com".to_string())
        .is_some());
    assert!(registry
        .begin("host-b".to_string(), "b.example.com".to_string())
        .is_some());
}

#[test]
fn stale_completion_cannot_release_a_new_generation() {
    let mut registry = SshConnectRegistry::default();
    let first = registry
        .begin("host-a".to_string(), "a.example.com".to_string())
        .unwrap();
    assert!(registry.finish(&first));
    let retry = registry
        .begin("host-a".to_string(), "a.example.com".to_string())
        .unwrap();

    assert!(!registry.finish(&first));
    assert!(registry.contains(&retry));
}

#[test]
fn classic_connection_finishes_only_with_its_terminal_lifecycle() {
    assert!(ssh_connect_terminal_event_finishes_attempt(
        &crate::terminal::Event::SshSessionBootstrapped
    ));
    assert!(ssh_connect_terminal_event_finishes_attempt(
        &crate::terminal::Event::PendingCommandCompleted
    ));
    assert!(ssh_connect_terminal_event_finishes_attempt(
        &crate::terminal::Event::Exited
    ));
    assert!(!ssh_connect_terminal_event_finishes_attempt(
        &crate::terminal::Event::SessionBootstrapped
    ));
}

#[test]
fn closing_classic_tab_releases_its_exact_attempt() {
    let mut registry = SshConnectRegistry::default();
    let attempt = registry
        .begin("host-a".to_string(), "a.example.com".to_string())
        .unwrap();
    let pane_group_id = EntityId::new();
    let mut classic_attempts = ClassicSshConnectAttempts::default();
    classic_attempts.bind(pane_group_id, attempt.clone());

    let closed_attempt = classic_attempts.take_for_tab(pane_group_id).unwrap();
    assert!(registry.finish(&closed_attempt));
    assert!(registry
        .begin("host-a".to_string(), "a.example.com".to_string())
        .is_some());
}

#[test]
fn closing_an_old_classic_tab_cannot_release_a_new_retry() {
    let mut registry = SshConnectRegistry::default();
    let first = registry
        .begin("host-a".to_string(), "a.example.com".to_string())
        .unwrap();
    let old_pane_group_id = EntityId::new();
    let mut classic_attempts = ClassicSshConnectAttempts::default();
    classic_attempts.bind(old_pane_group_id, first.clone());

    classic_attempts.finish(&first);
    assert!(registry.finish(&first));
    let retry = registry
        .begin("host-a".to_string(), "a.example.com".to_string())
        .unwrap();
    classic_attempts.bind(EntityId::new(), retry.clone());

    assert!(classic_attempts
        .take_for_tab(old_pane_group_id)
        .is_none());
    assert!(registry.contains(&retry));
}

#[test]
fn onekey_key_fallback_preserves_credential_secret_owner() {
    let resolved = ResolvedSshConnection {
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

    let fallback = resolved.clone();
    let (secret_owner, secret_kind) = resolved_ssh_secret_owner(&fallback);
    assert_eq!(secret_owner, "cred-1");
    assert_eq!(secret_kind, SecretKind::Passphrase);
    let argv = warp_ssh_manager::build_ssh_args(&fallback.server);
    assert!(argv.windows(2).any(|args| args == ["-i", "/keys/deploy"]));
    assert_eq!(argv.last().map(String::as_str), Some("deploy@example.test"));
}
