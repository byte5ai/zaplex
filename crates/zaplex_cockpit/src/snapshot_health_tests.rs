use std::fs;

use chrono::Utc;
use serde_json::json;

use super::*;

#[test]
fn codex_session_discovery_failure_alone_degrades_snapshot_once() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let codex_home = tmp.path().join("codex");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(codex_home.join("sessions/2026/09/06")).unwrap();
    fs::write(
        codex_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"account-1"}}"#,
    )
    .unwrap();
    let now = Utc::now();
    let rollout = [
        json!({
            "type": "session_meta",
            "timestamp": now.to_rfc3339(),
            "payload": {"id": "session-1", "cwd": "/tmp/project"}
        }),
        json!({
            "type": "event_msg",
            "timestamp": now.to_rfc3339(),
            "payload": {"type": "task_started"}
        }),
    ]
    .into_iter()
    .map(|line| serde_json::to_string(&line).unwrap())
    .collect::<Vec<_>>()
    .join("\n");
    fs::write(
        codex_home.join("sessions/2026/09/06/rollout-session-1.jsonl"),
        format!("{rollout}\n"),
    )
    .unwrap();
    let mut cache = TranscriptScanCache::default();
    cache.codex_rollouts.fail_next_parse();

    let snapshot = build_snapshot_with_cache(
        &home,
        &codex_home,
        None,
        now,
        0,
        0,
        &PricingTable::default(),
        &mut cache,
    );

    let ScanHealth::Degraded(reason) = snapshot.health else {
        panic!("an incomplete session scan must degrade the snapshot");
    };
    assert_eq!(reason.matches("transcript history unreadable").count(), 1);
    assert_eq!(
        snapshot.accounts.len(),
        1,
        "the readable account remains visible"
    );
}

#[test]
fn unreadable_usage_content_degrades_both_providers_and_prevents_auto_routing() {
    for provider in [Provider::Claude, Provider::Codex] {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let codex_home = home.join(".codex");
        let transcript = match provider {
            Provider::Claude => {
                fs::write(
                    home.join(".claude.json"),
                    r#"{"oauthAccount":{"emailAddress":"usage@example.com"}}"#,
                )
                .unwrap();
                home.join(".claude/projects/project/session.jsonl")
            }
            Provider::Codex => {
                fs::create_dir_all(&codex_home).unwrap();
                fs::write(
                    codex_home.join("auth.json"),
                    r#"{"auth_mode":"chatgpt","tokens":{"account_id":"account-1"}}"#,
                )
                .unwrap();
                codex_home.join("sessions/2026/09/27/rollout-session.jsonl")
            }
            Provider::Antigravity => unreachable!("no usage scanner for Antigravity"),
        };
        fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        // The directory walk and metadata remain readable; UTF-8 decoding fails
        // independently of OS permissions and whether the test runs as root.
        fs::write(&transcript, [0xff, b'\n']).unwrap();
        let snapshot = build_snapshot(
            &home,
            &codex_home,
            None,
            Utc::now(),
            0,
            0,
            &PricingTable::default(),
        );
        let ScanHealth::Degraded(reason) = &snapshot.health else {
            panic!("unreadable usage must not look like an unused account");
        };
        let expected = match provider {
            Provider::Claude => "usage history unreadable",
            Provider::Codex => "transcript history unreadable",
            Provider::Antigravity => unreachable!("no usage scanner for Antigravity"),
        };
        assert!(reason.contains(expected), "{provider:?}: {reason}");
        assert!(snapshot
            .accounts
            .iter()
            .any(|a| a.account.provider == provider));
        assert!(pick_freest_checked(provider, &snapshot).is_none());
    }
}
