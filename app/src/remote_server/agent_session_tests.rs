use chrono::{TimeZone, Utc};
use zaplex_cockpit::types::{
    Provider, SessionSnapshot, SessionState, TaskItem, TaskState, TaskStatus,
};

use super::*;

fn sample(state: SessionState, provider: Provider, effort: Option<String>) -> SessionSnapshot {
    SessionSnapshot {
        session_id: "sess-1".to_string(),
        cwd: "/home/me/proj/src".to_string(),
        name: "my-session".to_string(),
        state,
        provider,
        model: "claude-opus-4-8".to_string(),
        effort,
        ctx_tokens: 123_456,
        project_root: "/home/me/proj".to_string(),
        repo_root: "/home/me/proj".to_string(),
        project_name: "proj".to_string(),
        // Present values so the proto round-trip exercises the new identity
        // fields (Some → wire string → Some).
        branch: Some("rc/master-plan".to_string()),
        worktree: Some("wt-rc".to_string()),
        config_dir: Some("/home/me/.codex-work".to_string()),
        account_email: Some("me@example.de".to_string()),
        process_fingerprint: Some("linux-v1:boot-id:12345".to_string()),
        pty_session_id: Some("pty-7".to_string()),
        pty_session_generation: Some(42),
        pty_foreground: true,
        task_state: Some(TaskState {
            tasks: vec![
                TaskItem {
                    id: "0".to_string(),
                    title: "Inspect".to_string(),
                    status: TaskStatus::Completed,
                },
                TaskItem {
                    id: "1".to_string(),
                    title: "Implement".to_string(),
                    status: TaskStatus::InProgress,
                },
            ],
        }),
        // Millisecond-precise so the epoch-millis round-trip is exact.
        last_activity: Utc
            .timestamp_millis_opt(1_720_000_000_123)
            .single()
            .unwrap(),
        pid: 4242,
    }
}

/// Every field survives a SessionSnapshot → AgentSessionInfo → SessionSnapshot
/// round-trip, including the state/provider string mapping and a present effort.
#[test]
fn snapshot_round_trips_through_proto() {
    let original = sample(
        SessionState::Waiting,
        Provider::Codex,
        Some("high".to_string()),
    );
    let proto = snapshot_to_proto(&original);

    // Spot-check the wire encoding.
    assert_eq!(proto.state, "waiting");
    assert_eq!(proto.provider, "codex");
    assert_eq!(proto.effort, "high");
    assert_eq!(proto.process_fingerprint, "linux-v1:boot-id:12345");
    assert_eq!(proto.pty_session_id, "pty-7");
    assert_eq!(proto.pty_session_generation, 42);
    assert!(proto.pty_foreground);
    assert!(proto.has_task_state);
    assert_eq!(proto.task_items.len(), 2);
    assert_eq!(proto.task_items[1].status, "in_progress");
    assert_eq!(proto.last_activity_epoch_millis, 1_720_000_000_123);

    let back = proto_to_snapshot(&proto);
    assert_eq!(back, original);
}

#[test]
fn agent_inventory_round_trips_optional_pty_session_id() {
    let original = sample(SessionState::Active, Provider::Codex, None);
    let wire = snapshot_to_proto(&original);
    let decoded = proto_to_snapshot(&wire);

    assert_eq!(decoded.pty_session_id.as_deref(), Some("pty-7"));
    assert_eq!(decoded.pty_session_generation, Some(42));
    assert!(decoded.pty_foreground);
}

#[test]
fn dormant_agent_without_pty_round_trips_as_valid_state() {
    let mut original = sample(SessionState::Idle, Provider::Codex, None);
    original.pty_session_id = None;
    original.pty_session_generation = None;
    original.pty_foreground = false;

    let decoded = proto_to_snapshot(&snapshot_to_proto(&original));

    assert_eq!(decoded, original);
    assert_eq!(decoded.state, SessionState::Idle);
    assert_eq!(decoded.pty_session_id, None);
    assert_eq!(decoded.pty_session_generation, None);
    assert!(!decoded.pty_foreground);
}

#[test]
fn agent_row_attach_identity_keeps_all_routing_dimensions() {
    let original = sample(SessionState::Active, Provider::Codex, None);
    let identity = snapshot_agent_identity(&original);

    assert_eq!(identity.session_id, original.session_id);
    assert_eq!(identity.provider, "codex");
    assert_eq!(
        identity.account_email,
        original.account_email.unwrap_or_default()
    );
    assert_eq!(identity.config_dir, original.config_dir.unwrap_or_default());
}

/// All four states map to their lowercase strings and back.
#[test]
fn all_states_map_both_directions() {
    for state in [
        SessionState::Active,
        SessionState::Waiting,
        SessionState::Monitor,
        SessionState::Idle,
    ] {
        let s = state_to_str(state);
        assert_eq!(state_from_str(s), state);
    }
    assert_eq!(state_to_str(SessionState::Active), "active");
    assert_eq!(state_to_str(SessionState::Waiting), "waiting");
    assert_eq!(state_to_str(SessionState::Monitor), "monitor");
    assert_eq!(state_to_str(SessionState::Idle), "idle");
}

/// Every supported provider maps to its lowercase string and back.
#[test]
fn providers_map_both_directions() {
    assert_eq!(
        provider_from_str(Provider::Claude.as_str()),
        Provider::Claude
    );
    assert_eq!(provider_from_str(Provider::Codex.as_str()), Provider::Codex);
    assert_eq!(
        provider_from_str(Provider::Antigravity.as_str()),
        Provider::Antigravity
    );
    assert_eq!(Provider::Claude.as_str(), "claude");
    assert_eq!(Provider::Codex.as_str(), "codex");
    assert_eq!(Provider::Antigravity.as_str(), "antigravity");
}

/// An unknown/future state string folds to Idle (never a false attention
/// signal); an unknown provider folds to Claude.
#[test]
fn unknown_state_and_provider_fold_to_safe_defaults() {
    assert_eq!(state_from_str("teleporting"), SessionState::Idle);
    assert_eq!(state_from_str(""), SessionState::Idle);
    assert_eq!(provider_from_str("gemini"), Provider::Claude);
    assert_eq!(provider_from_str(""), Provider::Claude);
}

/// An unknown (None) effort encodes as the empty string and decodes back to
/// None — not Some("").
#[test]
fn unknown_effort_round_trips_to_none() {
    let original = sample(SessionState::Idle, Provider::Claude, None);
    let proto = snapshot_to_proto(&original);
    assert_eq!(proto.effort, "");

    let back = proto_to_snapshot(&proto);
    assert_eq!(back.effort, None);
    assert_eq!(back, original);
}

/// The account identity has to survive the wire, or a remote session cannot be
/// joined to the account it belongs to.
#[test]
fn the_account_email_round_trips() {
    let original = sample(SessionState::Active, Provider::Claude, None);
    let proto = snapshot_to_proto(&original);
    assert_eq!(proto.account_email, "me@example.de");
    assert_eq!(
        proto_to_snapshot(&proto).account_email,
        original.account_email
    );
}

/// A daemon older than this field sends nothing. That must read as "unknown"
/// rather than as an account named "" — an empty-string identity would collide
/// with every other silent daemon and pull all their sessions onto one account.
#[test]
fn a_daemon_that_sends_no_account_decodes_as_unknown() {
    let mut proto = snapshot_to_proto(&sample(SessionState::Active, Provider::Claude, None));
    proto.account_email = String::new();
    assert_eq!(proto_to_snapshot(&proto).account_email, None);
}

#[test]
fn a_daemon_that_sends_no_process_fingerprint_decodes_as_unsignalable() {
    let mut proto = snapshot_to_proto(&sample(SessionState::Active, Provider::Claude, None));
    proto.process_fingerprint = String::new();

    assert_eq!(proto_to_snapshot(&proto).process_fingerprint, None);
}

#[test]
fn an_older_daemon_without_task_fields_decodes_as_no_task_state() {
    let mut proto = snapshot_to_proto(&sample(SessionState::Active, Provider::Claude, None));
    proto.has_task_state = false;
    proto.task_items.clear();

    assert_eq!(proto_to_snapshot(&proto).task_state, None);
}

#[test]
fn an_explicit_empty_task_state_survives_the_wire() {
    let mut original = sample(SessionState::Active, Provider::Codex, None);
    original.task_state = Some(TaskState { tasks: Vec::new() });

    assert_eq!(
        proto_to_snapshot(&snapshot_to_proto(&original)).task_state,
        Some(TaskState { tasks: Vec::new() })
    );
}

/// The routing pin and the identity are different questions: a session can name
/// its account while carrying no pin at all (a default account), and that is the
/// exact case a config_dir-based join would get wrong.
#[test]
fn a_default_account_session_has_an_identity_but_no_pin() {
    let mut s = sample(SessionState::Active, Provider::Claude, None);
    s.config_dir = None;
    let proto = snapshot_to_proto(&s);
    assert_eq!(proto.config_dir, "", "no pin for a default account");
    assert_eq!(
        proto.account_email, "me@example.de",
        "but it still says whose it is"
    );

    let back = proto_to_snapshot(&proto);
    assert_eq!(back.config_dir, None);
    assert_eq!(back.account_email, Some("me@example.de".to_string()));
}
