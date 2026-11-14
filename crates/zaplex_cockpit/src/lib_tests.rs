use super::*;
use chrono::Utc;
use std::{fs, path::Path};
use walkdir::WalkDir;

/// A present-but-unreadable `auth.json` makes codex discovery return no account —
/// identical in shape to "Codex was never set up". The snapshot must report this
/// as *degraded* so the UI can say "couldn't read your account" (and offer a
/// retry) instead of the misleading "no accounts".
#[test]
fn a_malformed_codex_auth_json_degrades_the_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let codex_home = tmp.path().join("codex");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(codex_home.join("auth.json"), "{ this is not valid json").unwrap();

    let snap = build_snapshot(
        &home,
        &codex_home,
        None,
        Utc::now(),
        0,
        0,
        &PricingTable::default(),
    );
    assert!(
        matches!(snap.health, ScanHealth::Degraded(_)),
        "a present-but-unreadable codex auth.json must degrade, not read as empty: {:?}",
        snap.health,
    );
}

#[test]
fn a_malformed_claude_identity_degrades_the_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let codex_home = home.join(".codex");
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::write(home.join(".claude/.claude.json"), "{not valid json").unwrap();

    // Without process discovery: a Claude process running on the test host must
    // not add its account to this malformed setup.
    let snap = build_snapshot_with_claude_discovery(
        &home,
        &codex_home,
        None,
        Utc::now(),
        0,
        0,
        &PricingTable::default(),
        &mut TranscriptScanCache::default(),
        claude::discover_accounts_without_process_scan,
    );
    assert!(
        matches!(snap.health, ScanHealth::Degraded(_)),
        "a malformed Claude identity must degrade, not invent an account: {:?}",
        snap.health,
    );
    assert!(snap.accounts.is_empty());
}

#[test]
fn live_scan_reports_running_sessions_without_dormant_history() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let codex_home = home.join(".codex");
    let day = codex_home.join("sessions/2026/10/14");
    fs::create_dir_all(&day).unwrap();
    fs::write(
        codex_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"account-1"}}"#,
    )
    .unwrap();
    let now = Utc::now();
    let rollout = |session_id: &str, at: chrono::DateTime<Utc>| {
        [
            serde_json::json!({
                "type": "session_meta",
                "timestamp": at.to_rfc3339(),
                "payload": {"id": session_id, "cwd": "/tmp/project"}
            }),
            serde_json::json!({
                "type": "event_msg",
                "timestamp": at.to_rfc3339(),
                "payload": {"type": "task_started"}
            }),
        ]
        .map(|line| line.to_string() + "\n")
        .concat()
    };
    fs::write(day.join("rollout-running.jsonl"), rollout("running", now)).unwrap();
    let two_days_ago = now - chrono::Duration::days(2);
    let dormant = day.join("rollout-dormant.jsonl");
    fs::write(&dormant, rollout("dormant", two_days_ago)).unwrap();
    fs::File::options()
        .write(true)
        .open(&dormant)
        .unwrap()
        .set_modified(two_days_ago.into())
        .unwrap();

    let snapshot = build_snapshot_with_claude_discovery(
        &home,
        &codex_home,
        None,
        now,
        0,
        0,
        &PricingTable::default(),
        &mut TranscriptScanCache::default(),
        claude::discover_accounts_without_process_scan,
    );
    let live = scan_live_sessions_with_claude_discovery(
        &home,
        &codex_home,
        None,
        now,
        &mut TranscriptScanCache::default(),
        claude::discover_accounts_without_process_scan,
    );

    assert!(
        snapshot
            .accounts
            .iter()
            .flat_map(|account| &account.idle_sessions)
            .any(|session| session.session_id == "dormant"),
        "the full snapshot sees the dormant rollout"
    );
    let live_ids: Vec<&str> = live
        .sessions
        .iter()
        .map(|session| session.session_id.as_str())
        .collect();
    assert_eq!(live_ids, ["running"]);
    assert_eq!(
        live.accounts,
        snapshot
            .accounts
            .iter()
            .map(|account| account.account.clone())
            .collect::<Vec<_>>(),
        "both scans name the same accounts"
    );
}

/// Codex heat comes from Codex's rate-limit report only: explicit token budgets
/// (the daemon passes the Claude defaults) must not turn transcript spend into a
/// Codex percentage, and a `~/.codex-*` sibling is an account in both the full
/// snapshot and the daemon's live scan.
#[test]
fn codex_accounts_ignore_token_budgets_and_include_home_siblings() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let codex_home = home.join(".codex");
    let now = Utc::now();
    let resets_at = (now + chrono::Duration::days(3)).timestamp();
    for (root, account_id, rate_limits) in [
        (codex_home.clone(), "acc-default", serde_json::Value::Null),
        (
            home.join(".codex-work"),
            "acc-work",
            serde_json::json!({
                "limit_id": "codex",
                "primary": {"used_percent": 29.0, "window_minutes": 10080, "resets_at": resets_at},
                "secondary": null,
            }),
        ),
    ] {
        fs::create_dir_all(root.join("sessions/2026/11/14")).unwrap();
        fs::write(
            root.join("auth.json"),
            format!(r#"{{"auth_mode":"chatgpt","tokens":{{"account_id":"{account_id}"}}}}"#),
        )
        .unwrap();
        let line = serde_json::json!({
            "type": "event_msg",
            "timestamp": now.to_rfc3339(),
            "payload": {
                "type": "token_count",
                "info": {"last_token_usage": {"input_tokens": 15_000_000, "output_tokens": 5}},
                "rate_limits": rate_limits,
            },
        });
        fs::write(
            root.join("sessions/2026/11/14/rollout-a.jsonl"),
            line.to_string() + "\n",
        )
        .unwrap();
    }

    let snapshot = build_snapshot_with_claude_discovery(
        &home,
        &codex_home,
        None,
        now,
        DEFAULT_BUDGET_5H,
        DEFAULT_BUDGET_WEEK,
        &PricingTable::default(),
        &mut TranscriptScanCache::default(),
        claude::discover_accounts_without_process_scan,
    );
    let live = scan_live_sessions_with_claude_discovery(
        &home,
        &codex_home,
        None,
        now,
        &mut TranscriptScanCache::default(),
        claude::discover_accounts_without_process_scan,
    );

    let keys: Vec<&str> = snapshot
        .accounts
        .iter()
        .map(|usage| usage.account.key.as_str())
        .collect();
    assert_eq!(keys, ["codex:default", "codex:work"]);
    let live_keys: Vec<&str> = live
        .accounts
        .iter()
        .map(|account| account.key.as_str())
        .collect();
    assert_eq!(live_keys, keys);

    let default = &snapshot.accounts[0];
    assert_eq!(default.provenance_5h, UsageProvenance::Unknown);
    assert_eq!(default.provenance_week, UsageProvenance::Unknown);
    assert_eq!(default.block5h.input, 15_000_000);
    let work = &snapshot.accounts[1];
    assert_eq!(work.provenance_5h, UsageProvenance::Unknown);
    assert_eq!(work.known_heat_week(), Some(0.29));
}

#[test]
fn non_test_sources_use_file_backed_test_modules() {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in WalkDir::new(source_dir) {
        let entry = entry.expect("crate source tree must be readable");
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !entry.file_type().is_file()
            || path.extension().and_then(|extension| extension.to_str()) != Some("rs")
            || file_name.ends_with("_tests.rs")
        {
            continue;
        }

        let source = fs::read_to_string(path).expect("Rust source must be readable");
        let mut after_test_cfg = false;
        for (line_number, line) in source.lines().enumerate() {
            let line = line.trim();
            if line == "#[cfg(test)]" {
                after_test_cfg = true;
                continue;
            }
            if !after_test_cfg || line.is_empty() || line.starts_with("#[") {
                continue;
            }
            assert!(
                !(line.starts_with("mod ") && line.ends_with('{')),
                "inline test module in {}:{}",
                path.display(),
                line_number + 1,
            );
            after_test_cfg = false;
        }
    }
}
