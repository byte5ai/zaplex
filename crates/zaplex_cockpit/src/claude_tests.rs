use super::*;
use std::fs;

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn discovers_default_and_alt_accounts_excludes_backups() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();

    // Default account: ~/.claude.json (home-level) + ~/.claude/projects/…
    write(
        &home.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"me@example.com","displayName":"Me",
            "organizationName":"Acme","organizationRole":"admin",
            "organizationType":"claude_max","organizationRateLimitTier":"max_20x"},
            "accessToken":"SHOULD-NEVER-BE-SURFACED"}"#,
    );
    fs::create_dir_all(home.join(".claude/projects")).unwrap();

    // Alt account: ~/.claude-work/.claude.json
    write(
        &home.join(".claude-work/.claude.json"),
        r#"{"oauthAccount":{"emailAddress":"work@example.com","organizationType":"claude_team"}}"#,
    );

    // Backup dir must be excluded.
    write(
        &home.join(".claude-backup/.claude.json"),
        r#"{"oauthAccount":{"emailAddress":"nope@example.com"}}"#,
    );

    let accounts = discover_accounts(home, None);
    assert_eq!(accounts.len(), 2, "default + work, backup excluded: {accounts:?}");

    let default = accounts.iter().find(|a| a.is_default).unwrap();
    assert_eq!(default.key, "claude:default");
    assert_eq!(default.email.as_deref(), Some("me@example.com"));
    assert_eq!(default.org.as_deref(), Some("Acme"));
    assert_eq!(default.role.as_deref(), Some("admin"));
    assert_eq!(default.plan_tier.as_deref(), Some("Max 20x"));
    assert_eq!(default.label, "me@example.com");

    let work = accounts.iter().find(|a| !a.is_default).unwrap();
    assert_eq!(work.key, "claude:work");
    assert_eq!(work.email.as_deref(), Some("work@example.com"));
    assert_eq!(work.plan_tier.as_deref(), Some("team")); // "claude_team" → "team"
}

#[test]
fn parse_transcript_extracts_only_assistant_usage() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("session.jsonl");
    write(
        &path,
        concat!(
            r#"{"type":"user","timestamp":"2026-06-30T09:59:00Z"}"#,
            "\n",
            r#"{"type":"assistant","timestamp":"2026-06-30T10:00:00Z","message":{"model":"claude-opus-4-8","usage":{"input_tokens":100,"output_tokens":20,"cache_creation_input_tokens":10,"cache_read_input_tokens":5}}}"#,
            "\n",
            "not json at all\n",
            r#"{"type":"assistant","timestamp":"2026-06-30T10:05:00Z","message":{"model":"claude-sonnet-4-6","usage":{"input_tokens":50,"output_tokens":10}}}"#,
            "\n",
        ),
    );

    let entries = parse_transcript(&path);
    assert_eq!(entries.len(), 2, "two assistant turns, user + junk skipped");

    let first = &entries[0];
    assert_eq!(first.model, "claude-opus-4-8");
    assert_eq!(first.input, 100);
    assert_eq!(first.output, 20);
    assert_eq!(first.cache_create, 10);
    assert_eq!(first.cache_read, 5);
    assert_eq!(first.reasoning, 0);
    assert_eq!(first.provider, Provider::Claude);

    let second = &entries[1];
    assert_eq!(second.model, "claude-sonnet-4-6");
    assert_eq!(second.cache_create, 0); // missing fields default to 0
}

#[test]
fn usage_for_account_respects_the_since_cutoff() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    write(
        &home.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"me@example.com"}}"#,
    );
    write(
        &home.join(".claude/projects/p/s.jsonl"),
        concat!(
            r#"{"type":"assistant","timestamp":"2026-06-01T10:00:00Z","message":{"model":"claude-opus-4-8","usage":{"input_tokens":1}}}"#,
            "\n",
            r#"{"type":"assistant","timestamp":"2026-06-30T10:00:00Z","message":{"model":"claude-opus-4-8","usage":{"input_tokens":2}}}"#,
            "\n",
        ),
    );

    let account = discover_accounts(home, None)
        .into_iter()
        .find(|a| a.is_default)
        .unwrap();
    let since = DateTime::parse_from_rfc3339("2026-06-15T00:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let (entries, io_error) = usage_for_account(&account, since);
    assert!(!io_error, "a readable transcript tree is not an I/O error");
    assert_eq!(entries.len(), 1, "only the 06-30 entry passes the cutoff");
    assert_eq!(entries[0].input, 2);
}

/// The join that makes per-session spend usable: the id `parse_transcript`
/// stamps must be the very id discovery gives the session, or the table's "today
/// $" column looks up a key no row has.
#[test]
fn parsed_spend_carries_the_same_session_id_discovery_uses() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("projects").join("-tmp-proj");
    std::fs::create_dir_all(&dir).unwrap();
    let id = "a9b3a0e6-9067-41a0-b9fd-dcbee7ad5c01";
    std::fs::write(
        dir.join(format!("{id}.jsonl")),
        serde_json::to_string(&serde_json::json!({
            "type": "assistant",
            "timestamp": "2026-06-30T10:00:00Z",
            "sessionId": id,
            "message": {
                "model": "claude-opus-4-8",
                "usage": {"input_tokens": 100, "output_tokens": 10}
            }
        }))
        .unwrap()
            + "\n",
    )
    .unwrap();

    let entries = parse_transcript(&dir.join(format!("{id}.jsonl")));
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].session_id, id,
        "spend must be stamped with the transcript's own session id"
    );
    // And that id is exactly the key discovery indexes transcripts by.
    assert_eq!(
        crate::sessions::transcript_path(tmp.path(), id),
        Some(dir.join(format!("{id}.jsonl"))),
        "the stamped id is the key `transcripts_by_id` resolves"
    );
}
