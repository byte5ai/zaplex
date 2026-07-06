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
