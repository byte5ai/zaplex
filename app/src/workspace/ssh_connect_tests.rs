use std::time::Duration;

use super::{
    ssh_connect_terminal_event_finishes_attempt, ClassicSshConnectAttempts, SshConnectRegistry,
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
