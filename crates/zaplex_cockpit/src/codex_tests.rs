use super::*;
use base64::Engine;
use std::fs;

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn fake_jwt(payload_json: &str) -> String {
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
    format!("header.{payload}.signature")
}

#[test]
fn discovers_account_and_reads_email_from_id_token_without_storing_tokens() {
    let tmp = tempfile::tempdir().unwrap();
    let codex_home = tmp.path();
    let jwt = fake_jwt(
        r#"{"email":"c@example.com","https://api.openai.com/auth":{"chatgpt_plan_type":"pro"}}"#,
    );
    write(
        &codex_home.join("auth.json"),
        &format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"account_id":" acc_1 ","id_token":"{jwt}","access_token":"SECRET","refresh_token":"SECRET"}}}}"#
        ),
    );

    let accounts = discover_accounts(codex_home);
    assert_eq!(accounts.len(), 1);
    let a = &accounts[0];
    assert_eq!(a.provider, Provider::Codex);
    assert_eq!(a.key, "codex:default");
    assert_eq!(a.provider_account_id.as_deref(), Some("acc_1"));
    assert_eq!(a.email.as_deref(), Some("c@example.com"));
    assert_eq!(a.label, "c@example.com");
    // Provider ≠ plan (WS4 S2): the plan is the id_token claim, never auth_mode.
    assert_eq!(a.plan_tier.as_deref(), Some("Pro"));
    assert!(a.is_default);
}

#[test]
fn public_privacy_docs_match_auth_discovery() {
    let crate_docs = include_str!("lib.rs");
    let manifest = include_str!("../Cargo.toml");
    let codex_docs = include_str!("codex.rs");
    let normalize = |docs: &str| {
        docs.lines()
            .map(|line| line.strip_prefix("//!").unwrap_or(line))
            .flat_map(|line| line.split_whitespace())
            .collect::<Vec<_>>()
            .join(" ")
    };

    assert!(!crate_docs.contains("It never reads token strings or credentials"));
    assert!(!manifest.contains("never reads secrets"));
    for docs in [crate_docs, manifest, codex_docs] {
        assert!(docs.contains("auth.json"));
        assert!(docs.contains("id_token"));
    }
    for docs in [crate_docs, codex_docs] {
        assert!(
            normalize(docs).contains("never returned, persisted, logged, or sent over the network")
        );
    }
    assert!(manifest.contains("without exposing raw token strings"));
}

#[test]
fn missing_auth_json_yields_no_accounts() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(discover_accounts(tmp.path()).is_empty());
}

#[test]
fn legacy_api_key_auth_is_valid_but_not_a_subscription_account() {
    let tmp = tempfile::tempdir().unwrap();
    let credential = "sk-test-value-must-never-be-exposed";
    write(
        &tmp.path().join(".codex/auth.json"),
        &format!(r#"{{"OPENAI_API_KEY":"{credential}"}}"#),
    );

    let discovery = discover_account_roots(tmp.path(), None);

    assert!(discovery.issues.is_empty());
    assert!(discovery.accounts.is_empty());
    assert!(!format!("{discovery:?}").contains(credential));
}

#[test]
fn explicit_api_key_auth_is_valid_but_not_a_subscription_account() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join(".codex/auth.json"),
        r#"{"auth_mode":"apikey"}"#,
    );

    let discovery = discover_account_roots(tmp.path(), None);

    assert!(discovery.issues.is_empty());
    assert!(discovery.accounts.is_empty());
}

#[test]
fn api_key_root_does_not_block_another_subscription_account() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let pinned_root = tmp.path().join("accounts/work");
    write(
        &home.join(".codex/auth.json"),
        r#"{"OPENAI_API_KEY":"sk-test-only"}"#,
    );
    write(
        &pinned_root.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"work-account"}}"#,
    );

    let discovery = discover_account_roots(&home, Some(&pinned_root));

    assert!(discovery.issues.is_empty());
    assert_eq!(discovery.accounts.len(), 1);
    assert!(discovery.accounts[0].key.starts_with("codex:work:"));
}

#[test]
fn empty_legacy_api_key_is_still_malformed() {
    let tmp = tempfile::tempdir().unwrap();
    write(
        &tmp.path().join(".codex/auth.json"),
        r#"{"OPENAI_API_KEY":"  "}"#,
    );

    let discovery = discover_account_roots(tmp.path(), None);

    assert!(discovery.accounts.is_empty());
    assert_eq!(
        discovery.issues,
        vec!["Codex account sign-in file is malformed"]
    );
}

#[test]
fn discovers_default_and_pinned_roots_with_distinct_routing() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let default_root = home.join(".codex");
    let pinned_root = tmp.path().join("accounts/work");
    write(
        &default_root.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"default-account"}}"#,
    );
    write(
        &pinned_root.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"work-account"}}"#,
    );

    let discovery = discover_account_roots(&home, Some(&pinned_root));
    assert!(discovery.issues.is_empty());
    assert_eq!(discovery.accounts.len(), 2);
    let default = discovery
        .accounts
        .iter()
        .find(|account| account.is_default)
        .unwrap();
    assert_eq!(default.key, "codex:default");
    assert_eq!(default.config_dir_pin(), None);
    let pinned = discovery
        .accounts
        .iter()
        .find(|account| !account.is_default)
        .unwrap();
    assert!(pinned.key.starts_with("codex:work:"));
    assert_eq!(
        pinned.config_dir_pin(),
        Some(
            fs::canonicalize(pinned_root)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        )
    );
}

#[test]
fn duplicate_stable_codex_identity_is_emitted_once() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let default_root = home.join(".codex");
    let pinned_root = tmp.path().join("accounts/work");
    for root in [&default_root, &pinned_root] {
        write(
            &root.join("auth.json"),
            r#"{"auth_mode":"chatgpt","tokens":{"account_id":"same-account"}}"#,
        );
    }

    let discovery = discover_account_roots(&home, Some(&pinned_root));
    assert!(discovery.issues.is_empty());
    assert_eq!(discovery.accounts.len(), 1);
    assert!(discovery.accounts[0].is_default);
}

#[test]
fn reserved_external_codex_root_has_a_stable_distinct_key() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let default_root = home.join(".codex");
    let external_root = tmp.path().join("accounts/default");
    write(
        &default_root.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"default-account"}}"#,
    );
    write(
        &external_root.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"external-account"}}"#,
    );

    let discovery = discover_account_roots(&home, Some(&external_root));
    assert!(discovery.issues.is_empty());
    assert_eq!(discovery.accounts.len(), 2);
    let external_key = discovery
        .accounts
        .iter()
        .find(|account| !account.is_default)
        .unwrap()
        .key
        .clone();
    assert_eq!(discovery.accounts[0].key, "codex:default");
    assert!(external_key.starts_with("codex:default:"));

    fs::remove_file(default_root.join("auth.json")).unwrap();
    let without_default = discover_account_roots(&home, Some(&external_root));
    assert!(without_default.issues.is_empty());
    assert_eq!(without_default.accounts.len(), 1);
    assert_eq!(without_default.accounts[0].key, external_key);
}

#[test]
fn direct_codex_home_sibling_keeps_legacy_friendly_key() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let sibling_root = home.join(".codex-work");
    fs::create_dir_all(&home).unwrap();
    write(
        &sibling_root.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"work-account"}}"#,
    );

    let discovery = discover_account_roots(&home, Some(&sibling_root));

    assert!(discovery.issues.is_empty());
    assert_eq!(discovery.accounts.len(), 1);
    assert_eq!(discovery.accounts[0].key, "codex:work");
}

#[cfg(unix)]
#[test]
fn canonical_codex_root_alias_is_emitted_once() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let default_root = home.join(".codex");
    write(
        &default_root.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"account-1"}}"#,
    );
    let alias = tmp.path().join("codex-link");
    symlink(&default_root, &alias).unwrap();

    let discovery = discover_account_roots(&home, Some(&alias));
    assert!(discovery.issues.is_empty());
    assert_eq!(discovery.accounts.len(), 1);
    assert_eq!(
        discovery.accounts[0].config_dir,
        fs::canonicalize(default_root).unwrap()
    );
}

#[test]
fn parse_transcript_uses_per_turn_usage_from_token_count_events() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rollout-x.jsonl");
    write(
        &path,
        concat!(
            r#"{"type":"turn_context","model":"gpt-5-codex","timestamp":"2026-06-30T10:00:00Z"}"#,
            "\n",
            r#"{"type":"event_msg","timestamp":"2026-06-30T10:01:00Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":200,"output_tokens":40,"cached_input_tokens":15,"reasoning_output_tokens":30},"total_token_usage":{"input_tokens":200,"output_tokens":40,"cached_input_tokens":15,"reasoning_output_tokens":30,"total_tokens":240}}}}"#,
            "\n",
            // Usage attached to a different event type is not a completed turn.
            r#"{"type":"event_msg","timestamp":"2026-06-30T10:02:00Z","payload":{"type":"rate_limits","info":{"last_token_usage":{"input_tokens":999,"output_tokens":999},"total_token_usage":{"input_tokens":999,"output_tokens":999}}}}"#,
            "\n",
        ),
    );
    let file_date = DateTime::parse_from_rfc3339("2026-06-30T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);

    let entries = parse_transcript(&path, file_date);
    assert_eq!(entries.len(), 1, "only the per-turn usage line counts");
    let e = &entries[0];
    assert_eq!(e.model, "gpt-5-codex");
    assert_eq!(e.input, 185, "cached input is excluded from uncached input");
    assert_eq!(e.output, 40);
    assert_eq!(e.cache_read, 15);
    assert_eq!(e.reasoning, 30);
    assert_eq!(e.cache_create, 0);
    assert_eq!(
        e.ts,
        DateTime::parse_from_rfc3339("2026-06-30T10:01:00Z")
            .unwrap()
            .with_timezone(&Utc)
    );
}

#[test]
fn parse_transcript_skips_rate_limit_rebroadcast_without_dropping_equal_real_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rollout-x.jsonl");
    let token_count = |timestamp: &str, total_tokens: u64, rate_limit_update: bool| {
        serde_json::json!({
            "type": "event_msg",
            "timestamp": timestamp,
            "payload": {
                "type": "token_count",
                "info": {
                    "last_token_usage": {
                        "input_tokens": 100,
                        "output_tokens": 20,
                        "cached_input_tokens": 10,
                        "reasoning_output_tokens": 5
                    },
                    "total_token_usage": {
                        "input_tokens": total_tokens - 20,
                        "output_tokens": 20,
                        "cached_input_tokens": 10,
                        "reasoning_output_tokens": 5,
                        "total_tokens": total_tokens
                    },
                    "rate_limits": rate_limit_update.then_some(serde_json::json!({
                        "primary": {"used_percent": 42.0}
                    }))
                }
            }
        })
    };
    write(
        &path,
        &[
            token_count("2026-06-30T10:01:00Z", 120, false),
            token_count("2026-06-30T10:01:01Z", 120, true),
            token_count("2026-06-30T10:02:00Z", 240, false),
            token_count("2026-06-30T10:03:00Z", 360, false),
        ]
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n"),
    );

    let entries = parse_transcript(
        &path,
        DateTime::parse_from_rfc3339("2026-06-30T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    );

    assert_eq!(entries.len(), 3);
    assert!(entries.iter().all(|entry| entry.input == 90));
    assert!(entries.iter().all(|entry| entry.output == 20));
    assert!(entries.iter().all(|entry| entry.cache_read == 10));
    assert!(entries.iter().all(|entry| entry.reasoning == 5));
}

#[test]
fn parse_transcript_uses_cumulative_delta_when_last_usage_is_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rollout-x.jsonl");
    write(
        &path,
        &[
            serde_json::json!({
                "type": "event_msg",
                "timestamp": "2026-06-30T10:01:00Z",
                "payload": {"type": "token_count", "info": {
                    "total_token_usage": {
                        "input_tokens": 100,
                        "output_tokens": 10,
                        "cached_input_tokens": 20,
                        "total_tokens": 110
                    }
                }}
            }),
            serde_json::json!({
                "type": "event_msg",
                "timestamp": "2026-06-30T10:02:00Z",
                "payload": {"type": "token_count", "info": {
                    "total_token_usage": {
                        "input_tokens": 160,
                        "output_tokens": 15,
                        "cached_input_tokens": 30,
                        "total_tokens": 175
                    }
                }}
            }),
        ]
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n"),
    );

    let entries = parse_transcript(
        &path,
        DateTime::parse_from_rfc3339("2026-06-30T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    );
    assert_eq!(entries.len(), 2);
    assert_eq!((entries[0].input, entries[0].output), (80, 10));
    assert_eq!(entries[0].cache_read, 20);
    assert_eq!((entries[1].input, entries[1].output), (50, 5));
    assert_eq!(entries[1].cache_read, 10);
}

#[test]
fn parse_transcript_rebaselines_after_cumulative_reset() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rollout-x.jsonl");
    let token_count = |timestamp: &str, total_tokens: u64, input: u64, output: u64| {
        serde_json::json!({
            "type": "event_msg",
            "timestamp": timestamp,
            "payload": {"type": "token_count", "info": {
                "last_token_usage": {
                    "input_tokens": input,
                    "output_tokens": output
                },
                "total_token_usage": {
                    "input_tokens": total_tokens,
                    "output_tokens": 0,
                    "total_tokens": total_tokens
                }
            }}
        })
    };
    write(
        &path,
        &[
            token_count("2026-06-30T10:01:00Z", 100, 80, 20),
            token_count("2026-06-30T10:02:00Z", 10, 999, 999),
            token_count("2026-06-30T10:03:00Z", 30, 15, 5),
        ]
        .into_iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n"),
    );

    let entries = parse_transcript(
        &path,
        DateTime::parse_from_rfc3339("2026-06-30T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    );
    assert_eq!(entries.len(), 2);
    assert_eq!((entries[0].input, entries[0].output), (80, 20));
    assert_eq!((entries[1].input, entries[1].output), (15, 5));
}

#[test]
fn legacy_token_count_without_totals_keeps_equal_real_turns() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rollout-x.jsonl");
    let line = r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":90,"output_tokens":10}}}}"#;
    write(&path, &format!("{line}\n{line}\n"));

    let entries = parse_transcript(
        &path,
        DateTime::parse_from_rfc3339("2026-06-30T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    );
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|entry| entry.input == 10));
    assert!(entries.iter().all(|entry| entry.cache_read == 90));
}

#[test]
fn codex_usage_subtracts_cached_tokens_from_input_total() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rollout-x.jsonl");
    write(
        &path,
        r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":90,"output_tokens":10,"reasoning_output_tokens":5}}}}"#,
    );

    let entries = parse_transcript(
        &path,
        DateTime::parse_from_rfc3339("2026-06-30T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].input, 10, "input excludes cached input");
    assert_eq!(entries[0].cache_read, 90);
}

#[test]
fn codex_usage_fixture_reports_total_110_and_work_20() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rollout-x.jsonl");
    write(
        &path,
        r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":90,"output_tokens":10,"reasoning_output_tokens":5}}}}"#,
    );
    let entries = parse_transcript(
        &path,
        DateTime::parse_from_rfc3339("2026-06-30T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    );
    let mut totals = crate::types::WindowTotals::default();
    for entry in entries {
        totals.add(&entry, &crate::pricing::PricingTable::default());
    }

    assert_eq!(totals.input, 10);
    assert_eq!(totals.cache_read, 90);
    assert_eq!(totals.total, 110);
    assert_eq!(totals.work, 20);
}

#[test]
fn cached_input_larger_than_input_saturates_at_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rollout-x.jsonl");
    write(
        &path,
        r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"cached_input_tokens":90}}}}"#,
    );

    let entries = parse_transcript(
        &path,
        DateTime::parse_from_rfc3339("2026-06-30T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    );
    assert_eq!(entries[0].input, 0);
    assert_eq!(entries[0].cache_read, 90);
}

#[test]
fn missing_usage_fields_are_zero_without_dropping_present_tokens() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rollout-x.jsonl");
    write(
        &path,
        r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"cached_input_tokens":90,"reasoning_output_tokens":5}}}}"#,
    );

    let entries = parse_transcript(
        &path,
        DateTime::parse_from_rfc3339("2026-06-30T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].input, 0);
    assert_eq!(entries[0].output, 0);
    assert_eq!(entries[0].cache_read, 90);
    assert_eq!(entries[0].reasoning, 5);
}

#[test]
fn usage_for_account_walks_sessions_tree() {
    let tmp = tempfile::tempdir().unwrap();
    let codex_home = tmp.path();
    let jwt = fake_jwt(r#"{"email":"c@example.com"}"#);
    write(
        &codex_home.join("auth.json"),
        &format!(r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"{jwt}"}}}}"#),
    );
    write(
        &codex_home.join("sessions/2026/06/30/rollout-abc.jsonl"),
        concat!(
            r#"{"type":"turn_context","model":"gpt-5-codex","timestamp":"2026-06-30T10:00:00Z"}"#,
            "\n",
            r#"{"type":"event_msg","timestamp":"2026-06-30T10:01:00Z","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":10,"output_tokens":5}}}}"#,
            "\n",
        ),
    );

    let account = discover_accounts(codex_home).into_iter().next().unwrap();
    let since = DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let (entries, io_error) = usage_for_account(&account, since);
    assert!(!io_error, "a readable rollout tree is not an I/O error");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].input, 10);
    assert_eq!(entries[0].provider, Provider::Codex);
}

fn token_count_line(input_tokens: u64) -> String {
    serde_json::json!({
        "type": "event_msg",
        "timestamp": "2026-06-30T10:01:00Z",
        "payload": {
            "type": "token_count",
            "info": {"last_token_usage": {"input_tokens": input_tokens, "output_tokens": 5}},
        },
    })
    .to_string()
        + "\n"
}

fn usage_cache_fixture(codex_home: &Path) -> (Account, PathBuf, DateTime<Utc>) {
    write(
        &codex_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"account-1"}}"#,
    );
    let rollout = codex_home.join("sessions/2026/06/30/rollout-abc.jsonl");
    write(&rollout, &token_count_line(10));
    let account = discover_accounts(codex_home).into_iter().next().unwrap();
    let since = DateTime::parse_from_rfc3339("2026-06-01T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    (account, rollout, since)
}

#[test]
fn codex_usage_cache_parses_unchanged_rollouts_once_and_follows_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let (account, rollout, since) = usage_cache_fixture(tmp.path());
    let mut cache = CodexUsageCache::default();

    let first = usage_for_account_with_cache(&account, since, &mut cache);
    let unchanged = usage_for_account_with_cache(&account, since, &mut cache);
    assert_eq!(first.entries, unchanged.entries);
    assert_eq!(first.entries.len(), 1);
    assert_eq!(cache.parse_count, 1);

    write(&rollout, &(token_count_line(10) + &token_count_line(20)));
    let changed = usage_for_account_with_cache(&account, since, &mut cache).entries;
    assert_eq!(
        changed.iter().map(|entry| entry.input).collect::<Vec<_>>(),
        [10, 20]
    );
    assert_eq!(cache.parse_count, 2);

    fs::remove_file(&rollout).unwrap();
    let deleted = usage_for_account_with_cache(&account, since, &mut cache);
    assert!(!deleted.io_error);
    assert!(deleted.entries.is_empty());
    assert!(cache.entries.is_empty());
}

/// Codex names its session inside the rollout, so spend must be stamped from
/// `session_meta` — and with the same id discovery reads, or the table's
/// "today $" column looks up a key no row has.
#[test]
fn parsed_spend_carries_the_session_id_from_session_meta() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp
        .path()
        .join("sessions/2026/06/30/rollout-2026-06-30T10-00-00-abc.jsonl");
    let id = "a9b3a0e6-9067-41a0-b9fd-dcbee7ad5c01";
    write(
        &path,
        &format!(
            "{}\n{}\n",
            serde_json::json!({"type":"session_meta","timestamp":"2026-06-30T10:00:00Z",
                "payload":{"id":id,"cwd":"/tmp/proj"}}),
            serde_json::json!({"type":"event_msg","timestamp":"2026-06-30T10:01:00Z",
                "payload":{"type":"token_count","info":{"last_token_usage":{
                    "input_tokens":100,"cached_input_tokens":0,"output_tokens":10,
                    "reasoning_output_tokens":5,"total_tokens":110}}}}),
        ),
    );

    let entries = parse_transcript(
        &path,
        DateTime::parse_from_rfc3339("2026-06-30T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].session_id, id,
        "the rollout's own id wins over the file name"
    );
}

/// Without a `session_meta` line both sides fall back to the file name — and
/// must fall back to the *same* string, which is why one shared function derives
/// it. Diverging fallbacks would silently unattribute the spend.
#[test]
fn the_file_name_fallback_matches_the_one_discovery_uses() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp
        .path()
        .join("sessions/2026/06/30/rollout-2026-06-30T10-00-00-abc.jsonl");
    write(
        &path,
        &format!(
            "{}\n",
            serde_json::json!({"type":"event_msg","timestamp":"2026-06-30T10:01:00Z",
                "payload":{"type":"token_count","info":{"last_token_usage":{
                    "input_tokens":100,"cached_input_tokens":0,"output_tokens":10,
                    "reasoning_output_tokens":5,"total_tokens":110}}}}),
        ),
    );

    let entries = parse_transcript(
        &path,
        DateTime::parse_from_rfc3339("2026-06-30T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc),
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].session_id,
        crate::codex_sessions::session_id_from_path(&path),
        "both sides derive the fallback id from the same function"
    );
}

fn subscription_auth(account_id: &str, email: &str, plan_type: &str) -> String {
    let jwt = fake_jwt(
        &serde_json::json!({
            "email": email,
            "https://api.openai.com/auth": {"chatgpt_plan_type": plan_type},
        })
        .to_string(),
    );
    format!(
        r#"{{"auth_mode":"chatgpt","tokens":{{"account_id":"{account_id}","id_token":"{jwt}","access_token":"SECRET"}}}}"#
    )
}

#[test]
fn codex_home_siblings_are_discovered_in_stable_order() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write(
        &home.join(".codex/auth.json"),
        &subscription_auth("acc-default", "me@example.com", "pro"),
    );
    // Two workspaces of one login: same email, distinct account ids and plans.
    write(
        &home.join(".codex-zeta/auth.json"),
        &subscription_auth("acc-zeta", "me@example.com", "self_serve_business_prolite"),
    );
    write(
        &home.join(".codex-alpha/auth.json"),
        &subscription_auth("acc-alpha", "other@example.com", "prolite"),
    );

    let discovery = discover_account_roots(&home, None);

    assert!(discovery.issues.is_empty(), "{:?}", discovery.issues);
    let found: Vec<(&str, Option<&str>, Option<&str>)> = discovery
        .accounts
        .iter()
        .map(|account| {
            (
                account.key.as_str(),
                account.email.as_deref(),
                account.plan_tier.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            ("codex:default", Some("me@example.com"), Some("Pro")),
            ("codex:alpha", Some("other@example.com"), Some("Pro Lite")),
            ("codex:zeta", Some("me@example.com"), Some("Business")),
        ]
    );
    // Every sibling is pinned through its own CODEX_HOME; the default is not.
    let pins: Vec<Option<String>> = discovery
        .accounts
        .iter()
        .map(Account::config_dir_pin)
        .collect();
    let canonical = |name: &str| {
        Some(
            fs::canonicalize(home.join(name))
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        )
    };
    assert_eq!(
        pins,
        [None, canonical(".codex-alpha"), canonical(".codex-zeta")]
    );
}

#[test]
fn api_key_empty_and_backup_codex_siblings_yield_no_account() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write(
        &home.join(".codex-api/auth.json"),
        r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-test-only"}"#,
    );
    fs::create_dir_all(home.join(".codex-empty")).unwrap();
    write(
        &home.join(".codex-backup/auth.json"),
        &subscription_auth("acc-backup", "me@example.com", "pro"),
    );
    write(&home.join(".codex-file"), "not a directory");

    let discovery = discover_account_roots(&home, None);

    assert!(discovery.issues.is_empty(), "{:?}", discovery.issues);
    assert!(discovery.accounts.is_empty(), "{:?}", discovery.accounts);
}

#[test]
fn codex_home_pinned_to_a_sibling_is_one_account_with_its_pin() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let sibling = home.join(".codex-work");
    write(
        &sibling.join("auth.json"),
        &subscription_auth("acc-work", "me@example.com", "team"),
    );

    let discovery = discover_account_roots(&home, Some(&sibling));

    assert!(discovery.issues.is_empty(), "{:?}", discovery.issues);
    assert_eq!(discovery.accounts.len(), 1);
    let account = &discovery.accounts[0];
    assert_eq!(account.key, "codex:work");
    assert_eq!(account.plan_tier.as_deref(), Some("Team"));
    assert_eq!(
        account.config_dir_pin(),
        Some(
            fs::canonicalize(&sibling)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        )
    );
}

#[cfg(unix)]
#[test]
fn codex_sibling_symlink_to_the_default_root_is_deduplicated() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write(
        &home.join(".codex/auth.json"),
        &subscription_auth("acc-default", "me@example.com", "pro"),
    );
    symlink(home.join(".codex"), home.join(".codex-link")).unwrap();

    let discovery = discover_account_roots(&home, None);

    assert!(discovery.issues.is_empty(), "{:?}", discovery.issues);
    assert_eq!(discovery.accounts.len(), 1);
    assert_eq!(discovery.accounts[0].key, "codex:default");
}

#[test]
fn plan_claim_is_read_namespaced_or_top_level_and_missing_stays_unknown() {
    let plan_of = |payload: &str| {
        let tmp = tempfile::tempdir().unwrap();
        let jwt = fake_jwt(payload);
        write(
            &tmp.path().join("auth.json"),
            &format!(r#"{{"auth_mode":"chatgpt","tokens":{{"id_token":"{jwt}"}}}}"#),
        );
        discover_accounts(tmp.path())
            .into_iter()
            .next()
            .unwrap()
            .plan_tier
    };

    assert_eq!(
        plan_of(r#"{"email":"a@example.com","https://api.openai.com/auth":{"chatgpt_plan_type":"plus"}}"#)
            .as_deref(),
        Some("Plus")
    );
    assert_eq!(
        plan_of(r#"{"email":"a@example.com","chatgpt_plan_type":"enterprise"}"#).as_deref(),
        Some("Enterprise")
    );
    assert_eq!(plan_of(r#"{"email":"a@example.com"}"#), None);
    assert_eq!(
        plan_of(
            r#"{"email":"a@example.com","https://api.openai.com/auth":{"chatgpt_plan_type":" "}}"#
        ),
        None
    );
}

#[test]
fn plan_labels_are_readable() {
    let cases = [
        ("pro", "Pro"),
        ("plus", "Plus"),
        ("prolite", "Pro Lite"),
        ("team", "Team"),
        ("business", "Business"),
        ("self_serve_business", "Business"),
        ("self_serve_business_prolite", "Business"),
        ("enterprise", "Enterprise"),
        ("edu", "Edu"),
        ("free", "Free"),
        ("PRO", "Pro"),
        ("free_workspace", "Free Workspace"),
    ];
    for (raw, label) in cases {
        assert_eq!(plan_label(raw), label, "{raw}");
    }
}

fn rate_limit_line(
    timestamp: &str,
    primary: serde_json::Value,
    secondary: serde_json::Value,
) -> String {
    serde_json::json!({
        "type": "event_msg",
        "timestamp": timestamp,
        "payload": {
            "type": "token_count",
            "info": {"last_token_usage": {"input_tokens": 50_000_000, "output_tokens": 5}},
            "rate_limits": {
                "limit_id": "codex",
                "primary": primary,
                "secondary": secondary,
                "plan_type": "pro",
            },
        },
    })
    .to_string()
        + "\n"
}

fn ts(raw: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(raw)
        .unwrap()
        .with_timezone(&Utc)
}

fn codex_account(root: &Path) -> Account {
    write(
        &root.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"account-1"}}"#,
    );
    discover_accounts(root).into_iter().next().unwrap()
}

/// Usage of `account` at `now`, through the same scan and rate-limit merge the
/// snapshot builder uses.
fn codex_usage_at(account: &Account, now: DateTime<Utc>) -> AccountUsage {
    let scan = usage_for_account_with_cache(
        account,
        now - chrono::Duration::days(7),
        &mut CodexUsageCache::default(),
    );
    let mut usage = crate::windows::build_account_usage(
        account.clone(),
        scan.entries,
        now,
        0,
        0,
        &crate::pricing::PricingTable::default(),
    );
    apply_rate_limits(&mut usage, scan.rate_limits.as_ref(), now);
    usage
}

fn set_mtime(path: &Path, at: DateTime<Utc>) {
    fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(at.into())
        .unwrap();
}

#[test]
fn weekly_primary_window_is_real_and_missing_5h_window_is_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let account = codex_account(tmp.path());
    let now = ts("2026-11-14T12:00:00Z");
    let resets_at = ts("2026-11-18T09:00:00Z");
    let rollout = tmp.path().join("sessions/2026/11/14/rollout-a.jsonl");
    write(
        &rollout,
        &rate_limit_line(
            "2026-11-14T11:00:00Z",
            serde_json::json!({"used_percent": 29.0, "window_minutes": 10080, "resets_at": resets_at.timestamp()}),
            serde_json::Value::Null,
        ),
    );
    set_mtime(&rollout, now);

    let usage = codex_usage_at(&account, now);

    assert_eq!(usage.provenance_week, UsageProvenance::Real);
    assert!((usage.heat_week - 0.29).abs() < 1e-9, "{}", usage.heat_week);
    assert_eq!(usage.reset_week, Some(resets_at));
    assert_eq!(usage.provenance_5h, UsageProvenance::Unknown);
    assert_eq!(usage.known_heat_5h(), None);
    assert_eq!(usage.reset5h, None);
    assert_eq!(usage.heat_opus, None);
    assert_eq!(usage.heat_sonnet, None);
    // The spend is still counted; it just no longer sizes a heat.
    assert_eq!(usage.week.input, 50_000_000);
}

#[test]
fn both_windows_map_by_window_minutes_regardless_of_slot() {
    let tmp = tempfile::tempdir().unwrap();
    let account = codex_account(tmp.path());
    let now = ts("2026-11-14T12:00:00Z");
    let rollout = tmp.path().join("sessions/2026/11/14/rollout-a.jsonl");
    write(
        &rollout,
        &rate_limit_line(
            "2026-11-14T11:00:00Z",
            serde_json::json!({"used_percent": 80.0, "window_minutes": 10080, "resets_at": ts("2026-11-18T09:00:00Z").timestamp()}),
            serde_json::json!({"used_percent": 12.5, "window_minutes": 300, "resets_at": ts("2026-11-14T14:00:00Z").timestamp()}),
        ),
    );
    set_mtime(&rollout, now);

    let usage = codex_usage_at(&account, now);

    assert_eq!(usage.provenance_5h, UsageProvenance::Real);
    assert!((usage.heat - 0.125).abs() < 1e-9);
    assert_eq!(usage.reset5h, Some(ts("2026-11-14T14:00:00Z")));
    assert_eq!(usage.provenance_week, UsageProvenance::Real);
    assert!((usage.heat_week - 0.80).abs() < 1e-9);
}

#[test]
fn rollout_without_rate_limits_leaves_both_windows_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let account = codex_account(tmp.path());
    let now = ts("2026-11-14T12:00:00Z");
    let rollout = tmp.path().join("sessions/2026/11/14/rollout-a.jsonl");
    // Heavy spend, but no rate-limit report: no token budget may turn it into heat.
    let line = serde_json::json!({
        "type": "event_msg",
        "timestamp": "2026-11-14T11:00:00Z",
        "payload": {
            "type": "token_count",
            "info": {"last_token_usage": {"input_tokens": 50_000_000, "output_tokens": 5}},
            "rate_limits": null,
        },
    });
    write(&rollout, &(line.to_string() + "\n"));
    set_mtime(&rollout, now);

    let usage = codex_usage_at(&account, now);

    assert_eq!(usage.provenance_5h, UsageProvenance::Unknown);
    assert_eq!(usage.provenance_week, UsageProvenance::Unknown);
    assert_eq!(usage.known_heat_5h(), None);
    assert_eq!(usage.known_heat_week(), None);
    assert_eq!(usage.block5h.input, 50_000_000);
}

#[test]
fn rate_limit_window_past_its_reset_is_unknown() {
    let tmp = tempfile::tempdir().unwrap();
    let account = codex_account(tmp.path());
    let now = ts("2026-11-14T12:00:00Z");
    let rollout = tmp.path().join("sessions/2026/11/14/rollout-a.jsonl");
    write(
        &rollout,
        &rate_limit_line(
            "2026-11-14T06:00:00Z",
            serde_json::json!({"used_percent": 90.0, "window_minutes": 300, "resets_at": ts("2026-11-14T10:00:00Z").timestamp()}),
            // No reset at all: just as unusable.
            serde_json::json!({"used_percent": 40.0, "window_minutes": 10080}),
        ),
    );
    set_mtime(&rollout, now);

    let usage = codex_usage_at(&account, now);

    assert_eq!(usage.provenance_5h, UsageProvenance::Unknown);
    assert_eq!(usage.provenance_week, UsageProvenance::Unknown);
    assert_eq!(usage.reset5h, None);
    assert_eq!(usage.reset_week, None);
}

#[test]
fn latest_rate_limit_report_wins_across_rollouts_by_line_time() {
    let tmp = tempfile::tempdir().unwrap();
    let account = codex_account(tmp.path());
    let now = ts("2026-11-14T12:00:00Z");
    let week = |used: f64| serde_json::json!({"used_percent": used, "window_minutes": 10080, "resets_at": ts("2026-11-18T09:00:00Z").timestamp()});
    // The newer report sits in the rollout that sorts and is touched first.
    let newer = tmp.path().join("sessions/2026/11/13/rollout-a.jsonl");
    write(
        &newer,
        &rate_limit_line("2026-11-14T11:30:00Z", week(31.0), serde_json::Value::Null),
    );
    set_mtime(&newer, now - chrono::Duration::hours(2));
    let older = tmp.path().join("sessions/2026/11/14/rollout-b.jsonl");
    write(
        &older,
        &(rate_limit_line("2026-11-14T08:00:00Z", week(20.0), serde_json::Value::Null)
            + &rate_limit_line("2026-11-14T09:00:00Z", week(25.0), serde_json::Value::Null)),
    );
    set_mtime(&older, now);

    let usage = codex_usage_at(&account, now);

    assert!((usage.heat_week - 0.31).abs() < 1e-9, "{}", usage.heat_week);
    // The walk order of the two rollouts is the file system's; the merge must
    // not depend on it.
    let report = |at: &str| RateLimitSnapshot {
        observed_at: ts(at),
        windows: Vec::new(),
    };
    let (old, new) = (
        report("2026-11-14T08:00:00Z"),
        report("2026-11-14T11:30:00Z"),
    );
    assert_eq!(
        latest_rate_limits(Some(old.clone()), Some(new.clone())),
        Some(new.clone())
    );
    assert_eq!(latest_rate_limits(Some(new.clone()), Some(old)), Some(new));
}

#[test]
fn rate_limits_of_another_limit_id_are_ignored() {
    let tmp = tempfile::tempdir().unwrap();
    let account = codex_account(tmp.path());
    let now = ts("2026-11-14T12:00:00Z");
    let rollout = tmp.path().join("sessions/2026/11/14/rollout-a.jsonl");
    let account_wide = rate_limit_line(
        "2026-11-14T10:00:00Z",
        serde_json::json!({"used_percent": 40.0, "window_minutes": 10080, "resets_at": ts("2026-11-18T09:00:00Z").timestamp()}),
        serde_json::Value::Null,
    );
    let other_limit = serde_json::json!({
        "type": "event_msg",
        "timestamp": "2026-11-14T11:00:00Z",
        "payload": {
            "type": "token_count",
            "info": null,
            "rate_limits": {
                "limit_id": "codex_other_model",
                "primary": {"used_percent": 99.0, "window_minutes": 10080, "resets_at": ts("2026-11-18T09:00:00Z").timestamp()},
                "secondary": null,
            },
        },
    });
    write(&rollout, &(account_wide + &other_limit.to_string() + "\n"));
    set_mtime(&rollout, now);

    let usage = codex_usage_at(&account, now);

    assert!((usage.heat_week - 0.40).abs() < 1e-9, "{}", usage.heat_week);
}
