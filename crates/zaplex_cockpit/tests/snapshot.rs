//! Integration test: `build_snapshot` over fixture Claude + Codex homes, via the
//! crate's public API only.

use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use zaplex_cockpit::{build_snapshot, PricingTable, Provider, DEFAULT_BUDGET_5H};

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn ts(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

#[test]
fn build_snapshot_aggregates_both_providers() {
    let home_tmp = tempfile::tempdir().unwrap();
    let home = home_tmp.path();
    write(
        &home.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"me@example.com",
            "organizationRateLimitTier":"max_20x","organizationType":"claude_max"}}"#,
    );
    write(
        &home.join(".claude/projects/p/s.jsonl"),
        concat!(
            r#"{"type":"assistant","timestamp":"2026-06-30T10:00:00Z","message":{"model":"claude-opus-4-8","usage":{"input_tokens":1000,"output_tokens":100}}}"#,
            "\n",
        ),
    );

    let codex_tmp = tempfile::tempdir().unwrap();
    let codex_home = codex_tmp.path();
    // No JWT id_token here (base64 isn't a dev-dep); the JWT-email path is covered by
    // the crate's unit tests. The account is still discovered from auth_mode.
    write(
        &codex_home.join("auth.json"),
        r#"{"auth_mode":"chatgpt","tokens":{"account_id":"acc_1"}}"#,
    );
    write(
        &codex_home.join("sessions/2026/06/30/rollout-a.jsonl"),
        concat!(
            r#"{"type":"turn_context","model":"gpt-5-codex","timestamp":"2026-06-30T10:30:00Z"}"#,
            "\n",
            r#"{"type":"event_msg","timestamp":"2026-06-30T10:31:00Z","last_token_usage":{"input_tokens":200,"output_tokens":50}}"#,
            "\n",
        ),
    );

    let now = ts("2026-06-30T12:00:00Z");
    let pricing = PricingTable::default();
    let snap = build_snapshot(home, codex_home, None, now, DEFAULT_BUDGET_5H, &pricing);

    assert_eq!(snap.generated_at, now);
    assert_eq!(snap.accounts.len(), 2, "one Claude + one Codex account");

    let claude = snap
        .accounts
        .iter()
        .find(|a| a.account.provider == Provider::Claude)
        .unwrap();
    assert_eq!(claude.account.plan_tier.as_deref(), Some("Max 20x"));
    assert_eq!(claude.block5h.messages, 1);
    assert_eq!(claude.block5h.input, 1000);
    assert!(claude.block5h.cost_usd > 0.0);

    let codex = snap
        .accounts
        .iter()
        .find(|a| a.account.provider == Provider::Codex)
        .unwrap();
    assert_eq!(codex.account.label, "chatgpt");
    assert_eq!(codex.block5h.messages, 1);
    assert_eq!(codex.block5h.output, 50);
    assert_eq!(codex.block5h.cache_read, 0);
}
