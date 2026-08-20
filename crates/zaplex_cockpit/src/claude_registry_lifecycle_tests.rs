use std::fs;

use chrono::Utc;

use super::*;

fn fixture(pid: u32, proc_start: Option<&str>) -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let config = temp.path().join("claude");
    let sessions = config.join("sessions");
    let transcript_dir = config.join("projects/-work-zaplex");
    fs::create_dir_all(&sessions).unwrap();
    fs::create_dir_all(&transcript_dir).unwrap();
    let registry = sessions.join("entry.json");
    let mut value = serde_json::json!({
        "sessionId": "session-1",
        "kind": "cli",
        "cwd": "/work/zaplex",
        "pid": pid,
        "startedAt": Utc::now().timestamp_millis(),
    });
    if let Some(proc_start) = proc_start {
        value["procStart"] = Value::String(proc_start.to_string());
    }
    fs::write(&registry, serde_json::to_vec(&value).unwrap()).unwrap();
    fs::write(
        transcript_dir.join("session-1.jsonl"),
        "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n",
    )
    .unwrap();
    (temp, config, registry)
}

#[test]
fn cleanup_removes_only_registry_and_is_idempotent() {
    let (_temp, config, registry) = fixture(u32::MAX, None);
    let candidate = claude_stale_registry_candidate(&config, "session-1")
        .unwrap()
        .expect("dead process with history is stale");

    assert_eq!(
        cleanup_claude_stale_registry_entry(&candidate).unwrap(),
        ClaudeRegistryCleanupOutcome::Applied
    );
    assert!(!registry.exists());
    assert!(config
        .join("projects/-work-zaplex/session-1.jsonl")
        .exists());
    assert_eq!(
        cleanup_claude_stale_registry_entry(&candidate).unwrap(),
        ClaudeRegistryCleanupOutcome::AlreadyApplied
    );
}

#[test]
fn changed_registry_revision_fails_closed() {
    let (_temp, config, registry) = fixture(u32::MAX, None);
    let candidate = claude_stale_registry_candidate(&config, "session-1")
        .unwrap()
        .unwrap();
    let mut value: Value = serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    value["name"] = Value::String("changed".to_string());
    fs::write(&registry, serde_json::to_vec(&value).unwrap()).unwrap();

    assert!(matches!(
        cleanup_claude_stale_registry_entry(&candidate),
        Err(ClaudeRegistryLifecycleError::RegistryChanged)
    ));
    assert!(registry.exists());
}

#[test]
fn live_or_pid_reused_process_never_becomes_candidate() {
    let (_temp, config, registry) = fixture(std::process::id(), Some("definitely-wrong-start"));

    assert_eq!(
        claude_stale_registry_candidate(&config, "session-1").unwrap(),
        None
    );
    assert!(registry.exists());
}

#[test]
fn missing_transcript_is_not_a_cleanup_candidate() {
    let (_temp, config, registry) = fixture(u32::MAX, None);
    fs::remove_file(config.join("projects/-work-zaplex/session-1.jsonl")).unwrap();

    assert!(matches!(
        claude_stale_registry_candidate(&config, "session-1"),
        Err(ClaudeRegistryLifecycleError::MissingTranscript)
    ));
    assert!(registry.exists());
}

#[test]
fn malformed_process_identity_is_never_treated_as_dead() {
    let (_temp, config, registry) = fixture(u32::MAX, None);
    let mut value: Value = serde_json::from_slice(&fs::read(&registry).unwrap()).unwrap();
    value["pid"] = Value::from(u64::from(u32::MAX) + 1);
    fs::write(&registry, serde_json::to_vec(&value).unwrap()).unwrap();

    assert!(matches!(
        claude_stale_registry_candidate(&config, "session-1"),
        Err(ClaudeRegistryLifecycleError::UnsafeRegistryEntry)
    ));
    assert!(registry.exists());
}
