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
    let jwt = fake_jwt(r#"{"email":"c@example.com","chatgpt_plan_type":"pro"}"#);
    write(
        &codex_home.join("auth.json"),
        &format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"account_id":"acc_1","id_token":"{jwt}","access_token":"SECRET","refresh_token":"SECRET"}}}}"#
        ),
    );

    let accounts = discover_accounts(codex_home);
    assert_eq!(accounts.len(), 1);
    let a = &accounts[0];
    assert_eq!(a.provider, Provider::Codex);
    assert_eq!(a.key, "codex:default");
    assert_eq!(a.email.as_deref(), Some("c@example.com"));
    assert_eq!(a.label, "c@example.com");
    // Provider ≠ plan (WS4 S2): auth_mode is not a plan tier; Codex exposes no plan.
    assert_eq!(a.plan_tier, None);
    assert!(a.is_default);
}

#[test]
fn missing_auth_json_yields_no_accounts() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(discover_accounts(tmp.path()).is_empty());
}

#[test]
fn parse_transcript_sums_per_turn_and_ignores_cumulative_envelope() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rollout-x.jsonl");
    write(
        &path,
        concat!(
            r#"{"type":"turn_context","model":"gpt-5-codex","timestamp":"2026-06-30T10:00:00Z"}"#,
            "\n",
            // last_token_usage nested under "info" — exercises the recursive finder.
            r#"{"type":"event_msg","timestamp":"2026-06-30T10:01:00Z","info":{"last_token_usage":{"input_tokens":200,"output_tokens":40,"cached_input_tokens":15,"reasoning_output_tokens":30}}}"#,
            "\n",
            // cumulative envelope must be ignored to avoid double-counting.
            r#"{"type":"event_msg","timestamp":"2026-06-30T10:02:00Z","total_token_usage":{"input_tokens":999,"output_tokens":999}}"#,
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
fn usage_subtracts_cached_tokens() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rollout-x.jsonl");
    write(
        &path,
        r#"{"type":"event_msg","last_token_usage":{"input_tokens":100,"cached_input_tokens":90,"output_tokens":10,"reasoning_output_tokens":5}}"#,
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
fn fixture_reports_total_115_work_25() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rollout-x.jsonl");
    write(
        &path,
        r#"{"type":"event_msg","last_token_usage":{"input_tokens":100,"cached_input_tokens":90,"output_tokens":10,"reasoning_output_tokens":5}}"#,
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
    assert_eq!(totals.total, 115);
    assert_eq!(totals.work, 25);
}

#[test]
fn cached_input_larger_than_input_saturates_at_zero() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rollout-x.jsonl");
    write(
        &path,
        r#"{"type":"event_msg","last_token_usage":{"input_tokens":10,"cached_input_tokens":90}}"#,
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
        r#"{"type":"event_msg","last_token_usage":{"cached_input_tokens":90,"reasoning_output_tokens":5}}"#,
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
            r#"{"type":"event_msg","timestamp":"2026-06-30T10:01:00Z","last_token_usage":{"input_tokens":10,"output_tokens":5}}"#,
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
                    "reasoning_output_tokens":5,"total_tokens":115}}}}),
        ),
    );

    let entries = parse_transcript(&path, DateTime::parse_from_rfc3339("2026-06-30T00:00:00Z").unwrap().with_timezone(&Utc));
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
                    "reasoning_output_tokens":5,"total_tokens":115}}}}),
        ),
    );

    let entries = parse_transcript(&path, DateTime::parse_from_rfc3339("2026-06-30T00:00:00Z").unwrap().with_timezone(&Utc));
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].session_id,
        crate::codex_sessions::session_id_from_path(&path),
        "both sides derive the fallback id from the same function"
    );
}
