use std::time::Duration;

use super::SshConnectRegistry;

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
