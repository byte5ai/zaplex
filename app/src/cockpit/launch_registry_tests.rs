use super::*;
use chrono::TimeZone;

#[test]
fn record_and_lookup_roundtrips() {
    let cwd = PathBuf::from("/tmp/launch-registry-test-a");
    record(
        CLIAgent::Claude,
        None,
        Some(&cwd),
        Some("opus".to_string()),
        Some("high".to_string()),
    );
    let got = lookup(CLIAgent::Claude, None, Some(&cwd)).expect("recorded launch");
    assert_eq!(got.model.as_deref(), Some("opus"));
    assert_eq!(got.effort.as_deref(), Some("high"));
    assert_eq!(got.agent, CLIAgent::Claude);
}

#[test]
fn newest_launch_supersedes() {
    let cwd = PathBuf::from("/tmp/launch-registry-test-b");
    let t0 = Utc.with_ymd_and_hms(2026, 7, 6, 10, 0, 0).unwrap();
    let t1 = Utc.with_ymd_and_hms(2026, 7, 6, 11, 0, 0).unwrap();
    record_at(
        CLIAgent::Codex,
        None,
        Some(&cwd),
        Some("gpt-5".to_string()),
        Some("low".to_string()),
        t0,
    );
    record_at(
        CLIAgent::Codex,
        None,
        Some(&cwd),
        Some("gpt-5-codex".to_string()),
        Some("high".to_string()),
        t1,
    );
    let got = lookup(CLIAgent::Codex, None, Some(&cwd)).expect("recorded launch");
    assert_eq!(got.model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(got.effort.as_deref(), Some("high"));
    assert_eq!(got.launched_at, t1);
}

#[test]
fn distinct_coordinates_do_not_collide() {
    let cwd = PathBuf::from("/tmp/launch-registry-test-c");
    record(
        CLIAgent::Claude,
        Some("devhost"),
        Some(&cwd),
        Some("sonnet".to_string()),
        None,
    );
    // Same cwd + agent but local host is a different key → no record.
    assert!(lookup(CLIAgent::Claude, None, Some(&cwd)).is_none());
    let got = lookup(CLIAgent::Claude, Some("devhost"), Some(&cwd)).expect("host-scoped launch");
    assert_eq!(got.model.as_deref(), Some("sonnet"));
    assert_eq!(got.effort, None);
}

#[test]
fn missing_lookup_is_none() {
    let cwd = PathBuf::from("/tmp/launch-registry-test-never-recorded");
    assert!(lookup(CLIAgent::Gemini, None, Some(&cwd)).is_none());
}

#[test]
fn rehost_migrates_node_id_key_to_host_id() {
    // A remote launch recorded under the SSH node_id (daemon not yet connected).
    let cwd = PathBuf::from("/tmp/launch-registry-test-rehost");
    record(
        CLIAgent::Claude,
        Some("ssh-node-42"),
        Some(&cwd),
        Some("opus".to_string()),
        Some("high".to_string()),
    );
    // Before the daemon connects, a lookup by the eventual host_id misses.
    assert!(lookup(CLIAgent::Claude, Some("daemon-host-42"), Some(&cwd)).is_none());
    // The daemon connects → the workspace migrates node_id → host_id.
    rehost("ssh-node-42", "daemon-host-42");
    // The old node_id key is gone; the record now resolves under host_id — the
    // identity the Conductor inventory / session_effort look it up by.
    assert!(lookup(CLIAgent::Claude, Some("ssh-node-42"), Some(&cwd)).is_none());
    let got = lookup(CLIAgent::Claude, Some("daemon-host-42"), Some(&cwd))
        .expect("record migrated to host_id");
    assert_eq!(got.model.as_deref(), Some("opus"));
    assert_eq!(got.effort.as_deref(), Some("high"));
    assert_eq!(got.host.as_deref(), Some("daemon-host-42"));
}

#[test]
fn rehost_is_noop_when_ids_match() {
    // An already-connected host records directly under host_id; a rehost with
    // equal ids must leave it untouched (and never wipe it).
    let cwd = PathBuf::from("/tmp/launch-registry-test-rehost-noop");
    record(
        CLIAgent::Codex,
        Some("daemon-host-noop"),
        Some(&cwd),
        Some("gpt-5".to_string()),
        Some("low".to_string()),
    );
    rehost("daemon-host-noop", "daemon-host-noop");
    let got = lookup(CLIAgent::Codex, Some("daemon-host-noop"), Some(&cwd))
        .expect("record untouched by no-op rehost");
    assert_eq!(got.effort.as_deref(), Some("low"));
}

#[test]
fn rehost_leaves_other_hosts_untouched() {
    // Only records under the source host migrate; a same-cwd record on another
    // host must not be moved or clobbered.
    let cwd = PathBuf::from("/tmp/launch-registry-test-rehost-isolation");
    record(
        CLIAgent::Claude,
        Some("node-src"),
        Some(&cwd),
        None,
        Some("high".to_string()),
    );
    record(
        CLIAgent::Claude,
        Some("host-other"),
        Some(&cwd),
        None,
        Some("medium".to_string()),
    );
    rehost("node-src", "host-dst");
    // Source moved to dst.
    assert_eq!(
        lookup(CLIAgent::Claude, Some("host-dst"), Some(&cwd))
            .and_then(|r| r.effort)
            .as_deref(),
        Some("high"),
    );
    // Unrelated host is unaffected.
    assert_eq!(
        lookup(CLIAgent::Claude, Some("host-other"), Some(&cwd))
            .and_then(|r| r.effort)
            .as_deref(),
        Some("medium"),
    );
}
