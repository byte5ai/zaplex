use super::*;
use std::fs;

fn write(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn discover_without_process(home: &Path, config_dir_env: Option<&str>) -> AccountDiscovery {
    discover_accounts_with_process_roots(home, config_dir_env, ProcessAccountDiscovery::default())
}

fn accounts_without_process(home: &Path, config_dir_env: Option<&str>) -> Vec<Account> {
    discover_without_process(home, config_dir_env).accounts
}

#[test]
fn unsupported_process_discovery_does_not_degrade_static_account_scan() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    write(
        &home.join(".claude-work/.claude.json"),
        r#"{"oauthAccount":{"emailAddress":"work@example.com"}}"#,
    );

    #[cfg(target_os = "linux")]
    let process_discovery = ProcessAccountDiscovery::default();
    #[cfg(not(target_os = "linux"))]
    let process_discovery = running_claude_config_dirs();
    let discovery = discover_accounts_with_process_roots(&home, None, process_discovery);

    assert_eq!(discovery.accounts.len(), 1);
    assert_eq!(discovery.accounts[0].key, "claude:work");
    assert!(discovery.issues.is_empty());
}

#[cfg(target_os = "linux")]
fn write_proc_process(proc_root: &Path, pid: u32, uid: u32, cmdline: &[u8], environ: &[u8]) {
    let process_root = proc_root.join(pid.to_string());
    fs::create_dir_all(&process_root).unwrap();
    fs::write(
        process_root.join("status"),
        format!("Name:\tprocess\nUid:\t{uid}\t{uid}\t{uid}\t{uid}\n"),
    )
    .unwrap();
    let mut stat_fields = vec!["S".to_string()];
    stat_fields.extend((4..=21).map(|_| "0".to_string()));
    stat_fields.push((u64::from(pid) * 100).to_string());
    fs::write(
        process_root.join("stat"),
        format!("{pid} (process) {}\n", stat_fields.join(" ")),
    )
    .unwrap();
    fs::write(process_root.join("cmdline"), cmdline).unwrap();
    fs::write(process_root.join("environ"), environ).unwrap();
}

#[cfg(target_os = "linux")]
fn scan_proc(proc_root: &Path) -> ProcessAccountDiscovery {
    running_claude_config_dirs_from_proc_with_reader(proc_root, &read_process_bytes)
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

    // Exclusion names are token/suffix matches, not arbitrary substrings.
    write(
        &home.join(".claude-attempt/.claude.json"),
        r#"{"oauthAccount":{"emailAddress":"attempt@example.com"}}"#,
    );

    // Backup dir must be excluded.
    write(
        &home.join(".claude-backup/.claude.json"),
        r#"{"oauthAccount":{"emailAddress":"nope@example.com"}}"#,
    );
    write(
        &home.join(".claude-memory/.claude.json"),
        r#"{"oauthAccount":{"emailAddress":"memory@example.com"}}"#,
    );

    let accounts = accounts_without_process(home, None);
    assert_eq!(accounts.len(), 3, "backup excluded: {accounts:?}");

    let default = accounts.iter().find(|a| a.is_default).unwrap();
    assert_eq!(default.key, "claude:default");
    assert_eq!(default.email.as_deref(), Some("me@example.com"));
    assert_eq!(default.org.as_deref(), Some("Acme"));
    assert_eq!(default.role.as_deref(), Some("admin"));
    assert_eq!(default.plan_tier.as_deref(), Some("Max 20x"));
    assert_eq!(default.label, "me@example.com");

    let work = accounts.iter().find(|a| a.key == "claude:work").unwrap();
    assert_eq!(work.key, "claude:work");
    assert_eq!(work.email.as_deref(), Some("work@example.com"));
    assert_eq!(work.plan_tier.as_deref(), Some("team")); // "claude_team" → "team"
}

#[test]
fn discovers_default_account_without_any_session_store() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    write(
        &home.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"quiet@example.com"}}"#,
    );

    let discovery = discover_without_process(home, None);
    assert!(discovery.issues.is_empty());
    assert_eq!(discovery.accounts.len(), 1);
    assert_eq!(
        discovery.accounts[0].email.as_deref(),
        Some("quiet@example.com")
    );
}

#[test]
fn default_history_without_an_identity_does_not_invent_an_account() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    fs::create_dir_all(home.join(".claude/projects/old-project")).unwrap();

    let discovery = discover_without_process(home, None);
    assert!(discovery.issues.is_empty());
    assert!(discovery.accounts.is_empty());
}

#[test]
fn pinned_root_is_non_default_and_routes_back_to_that_root() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let pinned = tmp.path().join("accounts/work");
    fs::create_dir_all(&home).unwrap();
    write(
        &pinned.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"work@example.com"}}"#,
    );

    let discovery = discover_without_process(&home, pinned.to_str());
    assert!(discovery.issues.is_empty());
    assert_eq!(discovery.accounts.len(), 1);
    let account = &discovery.accounts[0];
    assert!(!account.is_default);
    assert_eq!(account.config_dir, fs::canonicalize(&pinned).unwrap());
    assert_eq!(
        account.config_dir_pin(),
        Some(account.config_dir.to_string_lossy().into_owned())
    );
}

#[test]
fn duplicate_stable_claude_identity_is_emitted_once() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    for suffix in ["a", "b"] {
        write(
            &home.join(format!(".claude-{suffix}/.claude.json")),
            r#"{"oauthAccount":{"accountUuid":"account-1","emailAddress":"same@example.com"}}"#,
        );
    }

    let discovery = discover_without_process(home, None);
    assert!(discovery.issues.is_empty());
    assert_eq!(discovery.accounts.len(), 1);
    assert_eq!(discovery.accounts[0].key, "claude:a");
}

#[cfg(unix)]
#[test]
fn canonical_root_alias_is_emitted_once() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let root = home.join(".claude-work");
    write(
        &root.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"work@example.com"}}"#,
    );
    symlink(&root, home.join(".claude-alias")).unwrap();

    let discovery = discover_without_process(home, None);
    assert!(discovery.issues.is_empty());
    assert_eq!(discovery.accounts.len(), 1);
    assert_eq!(
        discovery.accounts[0].config_dir,
        fs::canonicalize(root).unwrap()
    );
}

#[test]
fn malformed_identity_is_degraded_without_inventing_an_account() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    write(&home.join(".claude/.claude.json"), "{not valid json");
    fs::create_dir_all(home.join(".claude/projects")).unwrap();

    let discovery = discover_without_process(home, None);
    assert!(discovery.accounts.is_empty());
    assert_eq!(
        discovery.issues,
        vec!["Claude account identity is malformed"]
    );
}

#[test]
fn unreadable_identity_source_is_degraded_without_inventing_an_account() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    fs::create_dir_all(home.join(".claude/.claude.json")).unwrap();

    let discovery = discover_without_process(home, None);
    assert!(discovery.accounts.is_empty());
    assert_eq!(
        discovery.issues,
        vec!["Claude account identity is unreadable"]
    );
}

#[test]
fn unavailable_pinned_root_is_degraded_not_successfully_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let missing = home.join("outside/missing");

    let discovery = discover_without_process(home, missing.to_str());
    assert!(discovery.accounts.is_empty());
    assert_eq!(
        discovery.issues,
        vec!["Claude pinned account root is unavailable"]
    );

    let default_root = home.join(".claude");
    let default_discovery = discover_without_process(home, default_root.to_str());
    assert_eq!(
        default_discovery.issues,
        vec!["Claude pinned account root is unavailable"]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn discovers_account_from_same_uid_live_claude_process() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let account_root = tmp.path().join("accounts/external");
    let proc_root = tmp.path().join("proc");
    fs::create_dir_all(&home).unwrap();
    write(
        &account_root.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"external@example.com"}}"#,
    );
    write(
        &proc_root.join("self/status"),
        "Name:\tzaplex\nUid:\t1000\t1000\t1000\t1000\n",
    );
    write_proc_process(
        &proc_root,
        101,
        1000,
        b"/usr/bin/node\0/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js\0",
        format!(
            "PRIVATE_ENV_VALUE=must-not-leak\0CLAUDE_CONFIG_DIR={}\0",
            account_root.display()
        )
        .as_bytes(),
    );

    let process_discovery = scan_proc(&proc_root);
    assert!(process_discovery.issues.is_empty());
    let discovery = discover_accounts_with_process_roots(&home, None, process_discovery);

    assert!(discovery.issues.is_empty());
    assert_eq!(discovery.accounts.len(), 1);
    assert_eq!(
        discovery.accounts[0].config_dir,
        fs::canonicalize(account_root).unwrap()
    );
    assert_eq!(
        discovery.accounts[0].email.as_deref(),
        Some("external@example.com")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn process_scan_ignores_foreign_uid_and_non_claude_processes() {
    let tmp = tempfile::tempdir().unwrap();
    let proc_root = tmp.path().join("proc");
    write(
        &proc_root.join("self/status"),
        "Name:\tzaplex\nUid:\t1000\t1000\t1000\t1000\n",
    );
    write_proc_process(
        &proc_root,
        201,
        1000,
        b"/usr/bin/claudeplex\0",
        b"CLAUDE_CONFIG_DIR=/accounts/not-claude\0",
    );
    write_proc_process(
        &proc_root,
        202,
        2000,
        b"/usr/bin/claude\0",
        b"CLAUDE_CONFIG_DIR=/accounts/foreign\0",
    );

    let non_claude_environ = proc_root.join("201/environ");
    let foreign_cmdline = proc_root.join("202/cmdline");
    let discovery = running_claude_config_dirs_from_proc_with_reader(&proc_root, &|path| {
        assert_ne!(path, non_claude_environ);
        assert_ne!(path, foreign_cmdline);
        fs::read(path)
    });

    assert!(discovery.issues.is_empty());
    assert!(discovery.roots.is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn denied_process_environment_degrades_health_without_leaking_contents() {
    let tmp = tempfile::tempdir().unwrap();
    let proc_root = tmp.path().join("proc");
    write(
        &proc_root.join("self/status"),
        "Name:\tzaplex\nUid:\t1000\t1000\t1000\t1000\n",
    );
    write_proc_process(
        &proc_root,
        301,
        1000,
        b"/usr/bin/claude\0",
        b"CLAUDE_CONFIG_DIR=/accounts/private-fixture\0",
    );

    let denied_environment = proc_root.join("301/environ");
    let discovery = running_claude_config_dirs_from_proc_with_reader(&proc_root, &|path| {
        if path == denied_environment {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "PRIVATE_ENV_VALUE=fixture-sensitive CLAUDE_CONFIG_DIR=/accounts/private-fixture",
            ));
        }
        fs::read(path)
    });

    assert!(discovery.roots.is_empty());
    assert_eq!(discovery.issues, vec![PROCESS_DISCOVERY_PERMISSION_DENIED]);
    assert!(discovery
        .issues
        .iter()
        .all(|issue| !issue.contains("fixture-sensitive") && !issue.contains("private-fixture")));
}

#[cfg(target_os = "linux")]
#[test]
fn process_scan_rejects_pid_reuse_even_when_command_is_identical() {
    use std::cell::Cell;

    let tmp = tempfile::tempdir().unwrap();
    let proc_root = tmp.path().join("proc");
    write(
        &proc_root.join("self/status"),
        "Name:\tzaplex\nUid:\t1000\t1000\t1000\t1000\n",
    );
    write_proc_process(
        &proc_root,
        351,
        1000,
        b"/usr/bin/claude\0",
        b"CLAUDE_CONFIG_DIR=/accounts/reused-pid\0",
    );

    let stat_path = proc_root.join("351/stat");
    let stat_reads = Cell::new(0usize);
    let discovery = running_claude_config_dirs_from_proc_with_reader(&proc_root, &|path| {
        let bytes = fs::read(path)?;
        if path != stat_path {
            return Ok(bytes);
        }
        let read = stat_reads.get();
        stat_reads.set(read + 1);
        if read == 0 {
            return Ok(bytes);
        }
        let mut fields = std::str::from_utf8(&bytes)
            .unwrap()
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        *fields.last_mut().unwrap() = "999999".to_string();
        Ok(format!("{}\n", fields.join(" ")).into_bytes())
    });

    assert!(discovery.issues.is_empty());
    assert!(discovery.roots.is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn process_and_explicit_aliases_deduplicate_with_sibling_root() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let account_root = home.join(".claude-work");
    let process_alias = tmp.path().join("aliases/live-account");
    let proc_root = tmp.path().join("proc");
    write(
        &account_root.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"work@example.com"}}"#,
    );
    fs::create_dir_all(process_alias.parent().unwrap()).unwrap();
    symlink(&account_root, &process_alias).unwrap();
    write(
        &proc_root.join("self/status"),
        "Name:\tzaplex\nUid:\t1000\t1000\t1000\t1000\n",
    );
    write_proc_process(
        &proc_root,
        401,
        1000,
        b"/usr/bin/claude\0",
        format!("CLAUDE_CONFIG_DIR={}\0", process_alias.display()).as_bytes(),
    );

    let discovery =
        discover_accounts_with_process_roots(&home, account_root.to_str(), scan_proc(&proc_root));

    assert!(discovery.issues.is_empty());
    assert_eq!(discovery.accounts.len(), 1);
    assert_eq!(
        discovery.accounts[0].config_dir,
        fs::canonicalize(account_root).unwrap()
    );
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

    let account = accounts_without_process(home, None)
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
