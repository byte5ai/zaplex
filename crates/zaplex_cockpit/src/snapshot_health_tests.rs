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
