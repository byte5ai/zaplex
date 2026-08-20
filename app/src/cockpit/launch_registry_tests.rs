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
fn terminal_binding_separates_same_coordinate_launches() {
    let cwd = PathBuf::from("/tmp/launch-registry-test-exact");
    let first_terminal = EntityId::new();
    let second_terminal = EntityId::new();
    record_for_terminal(
        first_terminal,
        CLIAgent::Codex,
        None,
        Some(&cwd),
        Some(Path::new("/tmp/codex-a")),
        Some("a@example.com"),
        Some("gpt-5.6-sol".to_string()),
        Some("high".to_string()),
    );
    record_for_terminal(
        second_terminal,
        CLIAgent::Codex,
        None,
        Some(&cwd),
        Some(Path::new("/tmp/codex-b")),
        Some("b@example.com"),
        Some("gpt-5.6-terra".to_string()),
        Some("low".to_string()),
    );

    assert!(bind_terminal_session(first_terminal, "session-a"));
    assert!(bind_terminal_session(second_terminal, "session-b"));
    let first = lookup_bound_session(
        CLIAgent::Codex,
        None,
        Some(Path::new("/tmp/codex-a")),
        Some("a@example.com"),
        "session-a",
    );
    let second = lookup_bound_session(
        CLIAgent::Codex,
        None,
        Some(Path::new("/tmp/codex-b")),
        Some("b@example.com"),
        "session-b",
    );
    assert!(matches!(
        first,
        BoundLaunchLookup::Match(LaunchRecord {
            effort: Some(ref effort),
            ..
        }) if effort == "high"
    ));
    assert!(matches!(
        second,
        BoundLaunchLookup::Match(LaunchRecord {
            effort: Some(ref effort),
            ..
        }) if effort == "low"
    ));
}

#[test]
fn exact_lookup_fails_closed_for_another_account() {
    let terminal = EntityId::new();
    record_for_terminal(
        terminal,
        CLIAgent::Claude,
        Some("account-host"),
        Some(Path::new("/tmp/account-project")),
        Some(Path::new("/tmp/claude-account-a")),
        Some("a@example.com"),
        None,
        Some("high".to_string()),
    );
    assert!(bind_terminal_session(terminal, "shared-provider-session"));

    assert_eq!(
        lookup_bound_session(
            CLIAgent::Claude,
            Some("account-host"),
            Some(Path::new("/tmp/claude-account-b")),
            Some("b@example.com"),
            "shared-provider-session",
        ),
        BoundLaunchLookup::AccountMismatch,
    );
}

#[test]
fn opaque_account_id_disambiguates_same_session_without_email() {
    let first_terminal = EntityId::new();
    let second_terminal = EntityId::new();
    let first = begin_launch_with_account_id(
        CLIAgent::Codex,
        Some("daemon-1"),
        Some(Path::new("/tmp/project")),
        None,
        None,
        Some("opaque-a"),
        None,
        Some("high".to_string()),
    );
    let second = begin_launch_with_account_id(
        CLIAgent::Codex,
        Some("daemon-1"),
        Some(Path::new("/tmp/project")),
        None,
        None,
        Some("opaque-b"),
        None,
        Some("low".to_string()),
    );
    assert!(attach_terminal(first, first_terminal));
    assert!(attach_terminal(second, second_terminal));
    assert!(bind_terminal_session(first_terminal, "copied-session"));
    assert!(bind_terminal_session(second_terminal, "copied-session"));

    assert!(matches!(
        lookup_bound_session_with_account_id(
            CLIAgent::Codex,
            Some("daemon-1"),
            None,
            None,
            Some("opaque-a"),
            "copied-session",
        ),
        BoundLaunchLookup::Match(LaunchRecord {
            effort: Some(ref effort),
            ..
        }) if effort == "high"
    ));
    assert_eq!(
        lookup_bound_session_with_account_id(
            CLIAgent::Codex,
            Some("daemon-1"),
            None,
            None,
            Some("missing"),
            "copied-session",
        ),
        BoundLaunchLookup::AccountMismatch
    );
}

#[test]
fn provider_event_can_arrive_before_terminal_attach() {
    let terminal = EntityId::new();
    let launch_id = begin_launch(
        CLIAgent::Codex,
        None,
        Some(Path::new("/tmp/reordered-project")),
        Some(Path::new("/tmp/reordered-account")),
        Some("reordered@example.com"),
        Some("gpt-5.6-sol".to_string()),
        Some("xhigh".to_string()),
    );

    assert!(!bind_terminal_session(terminal, "event-first-session"));
    assert!(attach_terminal(launch_id, terminal));
    assert!(matches!(
        lookup_bound_session(
            CLIAgent::Codex,
            None,
            Some(Path::new("/tmp/reordered-account")),
            Some("reordered@example.com"),
            "event-first-session",
        ),
        BoundLaunchLookup::Match(LaunchRecord {
            effort: Some(ref effort),
            ..
        }) if effort == "xhigh"
    ));
    assert!(lookup(
        CLIAgent::Codex,
        None,
        Some(Path::new("/tmp/reordered-project"))
    )
    .is_none());
}

#[test]
fn remote_transport_then_inventory_promotes_exact_launch() {
    let terminal = EntityId::new();
    let cwd = Path::new("/tmp/remote-transport-first");
    let launch = begin_launch_with_account_id(
        CLIAgent::Codex,
        Some("remote-host-transport-first"),
        Some(cwd),
        None,
        None,
        Some("account-transport-first"),
        Some("gpt-5.6-sol".to_string()),
        Some("xhigh".to_string()),
    );
    assert!(attach_terminal(launch, terminal));
    assert!(!attach_remote_terminal(
        terminal,
        "remote-host-transport-first",
        "pty-transport-first",
        7,
    ));
    assert!(bind_remote_pty_session(
        "remote-host-transport-first",
        "pty-transport-first",
        7,
        CLIAgent::Codex,
        "account-transport-first",
        "provider-session-transport-first",
        cwd,
        cwd,
    ));
    assert!(matches!(
        lookup_bound_session_with_account_id(
            CLIAgent::Codex,
            Some("remote-host-transport-first"),
            None,
            None,
            Some("account-transport-first"),
            "provider-session-transport-first",
        ),
        BoundLaunchLookup::Match(LaunchRecord {
            effort: Some(ref effort),
            ..
        }) if effort == "xhigh"
    ));
}

#[test]
fn remote_inventory_then_transport_promotes_exact_launch() {
    let terminal = EntityId::new();
    let cwd = Path::new("/tmp/remote-inventory-first");
    let launch = begin_launch_with_account_id(
        CLIAgent::Claude,
        Some("remote-host-inventory-first"),
        Some(cwd),
        None,
        None,
        Some("account-inventory-first"),
        Some("opus".to_string()),
        Some("high".to_string()),
    );
    assert!(attach_terminal(launch, terminal));
    assert!(!bind_remote_pty_session(
        "remote-host-inventory-first",
        "pty-inventory-first",
        3,
        CLIAgent::Claude,
        "account-inventory-first",
        "provider-session-inventory-first",
        cwd,
        cwd,
    ));
    assert!(attach_remote_terminal(
        terminal,
        "remote-host-inventory-first",
        "pty-inventory-first",
        3,
    ));
    assert!(matches!(
        lookup_bound_session_with_account_id(
            CLIAgent::Claude,
            Some("remote-host-inventory-first"),
            None,
            None,
            Some("account-inventory-first"),
            "provider-session-inventory-first",
        ),
        BoundLaunchLookup::Match(_)
    ));
}

#[test]
fn parallel_remote_launches_same_project_bind_by_account_and_pty() {
    let cwd = Path::new("/tmp/parallel-remote-project");
    let first_terminal = EntityId::new();
    let second_terminal = EntityId::new();
    let first = begin_launch_with_account_id(
        CLIAgent::Codex,
        Some("parallel-remote-host"),
        Some(cwd),
        None,
        None,
        Some("parallel-account-a"),
        Some("model-a".to_string()),
        Some("high".to_string()),
    );
    let second = begin_launch_with_account_id(
        CLIAgent::Codex,
        Some("parallel-remote-host"),
        Some(cwd),
        None,
        None,
        Some("parallel-account-b"),
        Some("model-b".to_string()),
        Some("low".to_string()),
    );
    assert!(attach_terminal(first, first_terminal));
    assert!(attach_terminal(second, second_terminal));
    assert!(!attach_remote_terminal(
        first_terminal,
        "parallel-remote-host",
        "parallel-pty-a",
        1,
    ));
    assert!(!attach_remote_terminal(
        second_terminal,
        "parallel-remote-host",
        "parallel-pty-b",
        1,
    ));
    assert!(bind_remote_pty_session(
        "parallel-remote-host",
        "parallel-pty-b",
        1,
        CLIAgent::Codex,
        "parallel-account-b",
        "parallel-provider-b",
        cwd,
        cwd,
    ));
    assert!(bind_remote_pty_session(
        "parallel-remote-host",
        "parallel-pty-a",
        1,
        CLIAgent::Codex,
        "parallel-account-a",
        "parallel-provider-a",
        cwd,
        cwd,
    ));
    let first = lookup_bound_session_with_account_id(
        CLIAgent::Codex,
        Some("parallel-remote-host"),
        None,
        None,
        Some("parallel-account-a"),
        "parallel-provider-a",
    );
    let second = lookup_bound_session_with_account_id(
        CLIAgent::Codex,
        Some("parallel-remote-host"),
        None,
        None,
        Some("parallel-account-b"),
        "parallel-provider-b",
    );
    assert!(matches!(
        first,
        BoundLaunchLookup::Match(LaunchRecord {
            model: Some(ref model),
            ..
        }) if model == "model-a"
    ));
    assert!(matches!(
        second,
        BoundLaunchLookup::Match(LaunchRecord {
            model: Some(ref model),
            ..
        }) if model == "model-b"
    ));
}

#[test]
fn remote_binding_fails_closed_on_wrong_provider_account_or_project() {
    let terminal = EntityId::new();
    let cwd = Path::new("/tmp/remote-fail-closed");
    let launch = begin_launch_with_account_id(
        CLIAgent::Claude,
        Some("remote-host-fail-closed"),
        Some(cwd),
        None,
        None,
        Some("expected-account"),
        None,
        Some("high".to_string()),
    );
    assert!(attach_terminal(launch, terminal));
    assert!(!attach_remote_terminal(
        terminal,
        "remote-host-fail-closed",
        "pty-fail-closed",
        11,
    ));
    for (agent, account, project) in [
        (CLIAgent::Codex, "expected-account", cwd),
        (CLIAgent::Claude, "wrong-account", cwd),
        (
            CLIAgent::Claude,
            "expected-account",
            Path::new("/tmp/wrong-project"),
        ),
    ] {
        assert!(!bind_remote_pty_session(
            "remote-host-fail-closed",
            "pty-fail-closed",
            11,
            agent,
            account,
            "wrong-provider-session",
            project,
            project,
        ));
    }
    assert_eq!(
        lookup_bound_session_with_account_id(
            CLIAgent::Claude,
            Some("remote-host-fail-closed"),
            None,
            None,
            Some("expected-account"),
            "wrong-provider-session",
        ),
        BoundLaunchLookup::Unbound,
    );
}

#[test]
fn reordered_exact_bindings_keep_the_newest_launch_intent() {
    let cwd = Path::new("/tmp/reordered-exact-project");
    let config = Path::new("/tmp/reordered-exact-account");
    let older_terminal = EntityId::new();
    let newer_terminal = EntityId::new();
    let older = begin_launch_at(
        CLIAgent::Claude,
        None,
        Some(cwd),
        Some(config),
        Some("same@example.com"),
        None,
        Some("low".to_string()),
        Utc.with_ymd_and_hms(2026, 8, 20, 9, 0, 0).unwrap(),
    );
    let newer = begin_launch_at(
        CLIAgent::Claude,
        None,
        Some(cwd),
        Some(config),
        Some("same@example.com"),
        None,
        Some("xhigh".to_string()),
        Utc.with_ymd_and_hms(2026, 8, 20, 10, 0, 0).unwrap(),
    );
    assert!(attach_terminal(older, older_terminal));
    assert!(attach_terminal(newer, newer_terminal));

    assert!(bind_terminal_session(newer_terminal, "same-session"));
    assert!(bind_terminal_session(older_terminal, "same-session"));

    assert!(matches!(
        lookup_bound_session(
            CLIAgent::Claude,
            None,
            Some(config),
            Some("same@example.com"),
            "same-session",
        ),
        BoundLaunchLookup::Match(LaunchRecord {
            effort: Some(ref effort),
            ..
        }) if effort == "xhigh"
    ));
}

#[test]
fn terminal_reuse_can_bind_a_new_provider_session() {
    let terminal = EntityId::new();
    let first = begin_launch(
        CLIAgent::Codex,
        None,
        Some(Path::new("/tmp/reused-terminal")),
        None,
        None,
        None,
        Some("low".to_string()),
    );
    assert!(attach_terminal(first, terminal));
    assert!(bind_terminal_session(terminal, "first-session"));

    clear_terminal_session_binding(terminal);
    let second = begin_launch(
        CLIAgent::Codex,
        None,
        Some(Path::new("/tmp/reused-terminal")),
        None,
        None,
        None,
        Some("high".to_string()),
    );
    assert!(attach_terminal(second, terminal));
    assert!(bind_terminal_session(terminal, "second-session"));
    assert!(matches!(
        lookup_bound_session(CLIAgent::Codex, None, None, None, "second-session"),
        BoundLaunchLookup::Match(LaunchRecord {
            effort: Some(ref effort),
            ..
        }) if effort == "high"
    ));

    forget_terminal(terminal);
    assert!(!bind_terminal_session(terminal, "third-session"));
}

#[test]
fn reconnect_is_idempotent_and_rehosts_the_exact_binding() {
    let terminal = EntityId::new();
    record_for_terminal(
        terminal,
        CLIAgent::Claude,
        Some("reconnect-node"),
        Some(Path::new("/tmp/reconnect-project")),
        None,
        None,
        Some("opus".to_string()),
        Some("high".to_string()),
    );
    assert!(bind_terminal_session(terminal, "reconnect-session"));
    assert!(bind_terminal_session(terminal, "reconnect-session"));

    rehost("reconnect-node", "reconnect-host");
    rehost("reconnect-node", "reconnect-host");
    assert!(bind_terminal_session(terminal, "reconnect-session"));
    assert_eq!(
        lookup_bound_session(
            CLIAgent::Claude,
            Some("reconnect-node"),
            None,
            None,
            "reconnect-session",
        ),
        BoundLaunchLookup::Unbound,
    );
    assert!(matches!(
        lookup_bound_session(
            CLIAgent::Claude,
            Some("reconnect-host"),
            None,
            None,
            "reconnect-session",
        ),
        BoundLaunchLookup::Match(_)
    ));
}

#[test]
fn rehost_migrates_pending_and_exact_bindings() {
    let pending_terminal = EntityId::new();
    let exact_terminal = EntityId::new();
    for terminal in [pending_terminal, exact_terminal] {
        record_for_terminal(
            terminal,
            CLIAgent::Claude,
            Some("binding-node"),
            Some(Path::new("/tmp/binding-project")),
            None,
            None,
            None,
            Some("medium".to_string()),
        );
    }
    assert!(bind_terminal_session(exact_terminal, "exact-before-rehost"));

    rehost("binding-node", "binding-host");
    assert!(bind_terminal_session(
        pending_terminal,
        "pending-after-rehost"
    ));
    for session_id in ["exact-before-rehost", "pending-after-rehost"] {
        assert!(matches!(
            lookup_bound_session(
                CLIAgent::Claude,
                Some("binding-host"),
                None,
                None,
                session_id,
            ),
            BoundLaunchLookup::Match(_)
        ));
    }
}

#[test]
fn missing_lookup_is_none() {
    let cwd = PathBuf::from("/tmp/launch-registry-test-never-recorded");
    assert!(lookup(CLIAgent::Gemini, None, Some(&cwd)).is_none());
}

#[test]
fn rehost_migrates_node_id_key_to_host_id() {
    // A remote launch recorded under the SSH node_id (daemon not yet connected).
    let cwd = PathBuf::from("/tmp/launch-registry-test-rehost");
    record(
        CLIAgent::Claude,
        Some("ssh-node-42"),
        Some(&cwd),
        Some("opus".to_string()),
        Some("high".to_string()),
    );
    // Before the daemon connects, a lookup by the eventual host_id misses.
    assert!(lookup(CLIAgent::Claude, Some("daemon-host-42"), Some(&cwd)).is_none());
    // The daemon connects → the workspace migrates node_id → host_id.
    rehost("ssh-node-42", "daemon-host-42");
    // The old node_id key is gone; the record now resolves under host_id — the
    // identity the Conductor inventory / session_effort look it up by.
    assert!(lookup(CLIAgent::Claude, Some("ssh-node-42"), Some(&cwd)).is_none());
    let got = lookup(CLIAgent::Claude, Some("daemon-host-42"), Some(&cwd))
        .expect("record migrated to host_id");
    assert_eq!(got.model.as_deref(), Some("opus"));
    assert_eq!(got.effort.as_deref(), Some("high"));
    assert_eq!(got.host.as_deref(), Some("daemon-host-42"));
}

#[test]
fn rehost_is_noop_when_ids_match() {
    // An already-connected host records directly under host_id; a rehost with
    // equal ids must leave it untouched (and never wipe it).
    let cwd = PathBuf::from("/tmp/launch-registry-test-rehost-noop");
    record(
        CLIAgent::Codex,
        Some("daemon-host-noop"),
        Some(&cwd),
        Some("gpt-5".to_string()),
        Some("low".to_string()),
    );
    rehost("daemon-host-noop", "daemon-host-noop");
    let got = lookup(CLIAgent::Codex, Some("daemon-host-noop"), Some(&cwd))
        .expect("record untouched by no-op rehost");
    assert_eq!(got.effort.as_deref(), Some("low"));
}

#[test]
fn rehost_leaves_other_hosts_untouched() {
    // Only records under the source host migrate; a same-cwd record on another
    // host must not be moved or clobbered.
    let cwd = PathBuf::from("/tmp/launch-registry-test-rehost-isolation");
    record(
        CLIAgent::Claude,
        Some("node-src"),
        Some(&cwd),
        None,
        Some("high".to_string()),
    );
    record(
        CLIAgent::Claude,
        Some("host-other"),
        Some(&cwd),
        None,
        Some("medium".to_string()),
    );
    rehost("node-src", "host-dst");
    // Source moved to dst.
    assert_eq!(
        lookup(CLIAgent::Claude, Some("host-dst"), Some(&cwd))
            .and_then(|r| r.effort)
            .as_deref(),
        Some("high"),
    );
    // Unrelated host is unaffected.
    assert_eq!(
        lookup(CLIAgent::Claude, Some("host-other"), Some(&cwd))
            .and_then(|r| r.effort)
            .as_deref(),
        Some("medium"),
    );
}

#[test]
fn rehost_does_not_overwrite_a_newer_destination_launch() {
    let cwd = PathBuf::from("/tmp/launch-registry-test-rehost-newer");
    let older = Utc.with_ymd_and_hms(2026, 8, 20, 10, 0, 0).unwrap();
    let newer = Utc.with_ymd_and_hms(2026, 8, 20, 11, 0, 0).unwrap();
    record_at(
        CLIAgent::Codex,
        Some("old-host-for-newer-test"),
        Some(&cwd),
        None,
        Some("low".to_string()),
        older,
    );
    record_at(
        CLIAgent::Codex,
        Some("new-host-for-newer-test"),
        Some(&cwd),
        None,
        Some("high".to_string()),
        newer,
    );

    rehost("old-host-for-newer-test", "new-host-for-newer-test");
    assert_eq!(
        lookup(CLIAgent::Codex, Some("new-host-for-newer-test"), Some(&cwd),)
            .and_then(|record| record.effort)
            .as_deref(),
        Some("high"),
    );
}
