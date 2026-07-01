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
    assert_eq!(a.plan_tier.as_deref(), Some("chatgpt")); // auth_mode, best-effort
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
    assert_eq!(e.input, 200);
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
    let entries = usage_for_account(&account, since);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].input, 10);
    assert_eq!(entries[0].provider, Provider::Codex);
}
