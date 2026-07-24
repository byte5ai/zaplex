use chrono::Utc;
use zaplex_cockpit::types::{Provider, SessionSnapshot, SessionState};

use super::*;

fn session(provider: Provider, state: SessionState, pid: u32) -> SessionSnapshot {
    SessionSnapshot {
        session_id: "019f135f-7fcc-7d93-8a28-4835d98f8f0a".into(),
        cwd: "/tmp/proj".into(),
        name: String::new(),
        state,
        provider,
        model: String::new(),
        effort: None,
        ctx_tokens: 0,
        project_root: "/tmp/proj".into(),
        repo_root: "/tmp/proj".into(),
        project_name: "proj".into(),
        branch: None,
        worktree: None,
        config_dir: None,
        account_email: None,
        process_fingerprint: None,
        pty_session_id: None,
        pty_session_generation: None,
        pty_foreground: false,
        last_activity: Utc::now(),
        pid,
    }
}

/// The gap F6 exists to close: Codex has no process registry, so its sessions
/// carry no pid and no signal can reach them. The row must not offer stop/kill
/// and answer the click with an error toast.
#[test]
fn a_codex_session_cannot_be_signalled() {
    let caps = SessionCapabilities::of(
        &session(Provider::Codex, SessionState::Waiting, 0),
        true,
    );
    assert!(!caps.can_signal, "no pid, no signal — do not offer stop/kill");
    // Forking is intentionally a new conversation, so it remains available.
    assert!(caps.can_fork, "codex fork <id> exists");
    assert!(!caps.can_resume, "a live session must not be resumed twice");
}

#[test]
fn a_claude_session_with_only_a_pid_cannot_be_signalled() {
    let caps = SessionCapabilities::of(
        &session(Provider::Claude, SessionState::Active, 4242),
        true,
    );
    assert!(
        !caps.can_signal,
        "a pid without a process-identity fingerprint is not a safe signal target"
    );
    assert!(caps.can_fork);
    assert!(!caps.can_resume);
    assert!(caps.can_slash);
}

#[test]
fn a_fingerprinted_pid_is_signalable_independent_of_host_locality() {
    let mut session = session(Provider::Claude, SessionState::Active, 4242);
    session.process_fingerprint = Some("linux-v1:boot:start".to_string());

    assert!(SessionCapabilities::of(&session, true).can_signal);
    assert!(
        SessionCapabilities::of(&session, false).can_signal,
        "remote capability negotiation happens in the signal path, not here"
    );
}

/// pid 0 means "discovery recorded none", whoever the provider is — the test is
/// the pid, not the brand.
#[test]
fn a_claude_session_without_a_pid_cannot_be_signalled_either() {
    let caps = SessionCapabilities::of(
        &session(Provider::Claude, SessionState::Waiting, 0),
        true,
    );
    assert!(!caps.can_signal);
    // State, not pid availability, decides whether resuming is safe.
    assert!(!caps.can_resume);
    assert!(caps.can_slash);
}

/// A dormant Claude conversation can be resumed before staging its slash
/// command. Codex has no equivalent slash commands.
#[test]
fn dormant_slash_commands_follow_provider_support() {
    assert!(
        !SessionCapabilities::of(
            &session(Provider::Codex, SessionState::Idle, 0),
            true,
        )
        .can_slash
    );
    assert!(
        SessionCapabilities::of(
            &session(Provider::Claude, SessionState::Idle, 0),
            true,
        )
        .can_slash
    );
}

/// `project_root` is a path on the host that reported the session. Reviewing a
/// remote one from here would open this machine's identically-named directory —
/// the wrong tree, silently.
#[test]
fn a_remote_session_cannot_be_reviewed_from_here() {
    let session = session(Provider::Claude, SessionState::Waiting, 1);
    assert!(!SessionCapabilities::of(&session, false).can_review);
    assert!(SessionCapabilities::of(&session, true).can_review);
}

/// Being remote costs only the local-tree verb. A live conversation remains
/// forkable, but must be focused/adopted rather than resumed into a duplicate.
#[test]
fn a_remote_session_keeps_everything_that_does_not_need_this_machine() {
    let caps = SessionCapabilities::of(
        &session(Provider::Claude, SessionState::Monitor, 4242),
        false,
    );
    assert!(caps.can_fork);
    assert!(!caps.can_resume);
    assert!(caps.can_slash);
    assert!(
        !caps.can_signal,
        "a remote pid without a fingerprint must fail closed before host execution"
    );
    assert!(!caps.can_review);
}

#[test]
fn only_a_dormant_session_can_be_resumed() {
    for provider in [Provider::Claude, Provider::Codex] {
        for state in [
            SessionState::Active,
            SessionState::Waiting,
            SessionState::Monitor,
        ] {
            assert!(
                !SessionCapabilities::of(&session(provider, state, 1), true).can_resume,
                "{provider:?} {state:?} is live and must not be duplicated"
            );
        }

        assert!(
            SessionCapabilities::of(&session(provider, SessionState::Idle, 0), true).can_resume,
            "{provider:?} Idle has no live PTY and may be resumed"
        );
    }
}

#[test]
fn open_plan_focuses_a_known_terminal_for_every_session_state() {
    for state in [
        SessionState::Active,
        SessionState::Waiting,
        SessionState::Monitor,
        SessionState::Idle,
    ] {
        assert_eq!(
            plan_session_open(&session(Provider::Claude, state, 1), true),
            SessionOpenPlan::FocusExistingTerminal,
            "an existing pane is authoritative for {state:?}"
        );
    }
}

#[test]
fn open_plan_refuses_to_duplicate_an_unlocated_live_session() {
    for state in [
        SessionState::Active,
        SessionState::Waiting,
        SessionState::Monitor,
    ] {
        assert_eq!(
            plan_session_open(&session(Provider::Codex, state, 0), false),
            SessionOpenPlan::LiveSessionUnavailable,
            "an unlocated {state:?} session must not be resumed"
        );
    }
}

#[test]
fn open_plan_resumes_an_unlocated_dormant_session() {
    assert_eq!(
        plan_session_open(
            &session(Provider::Claude, SessionState::Idle, 0),
            false,
        ),
        SessionOpenPlan::ResumeDormant,
    );
}

#[test]
fn reattach_uses_id_without_cwd_guessing() {
    let mut live = session(Provider::Codex, SessionState::Active, 0);
    live.cwd = "/a/path/that/must/not-be-used-as-an-id".to_string();
    live.pty_session_id = Some("daemon-pty-7".to_string());
    live.pty_session_generation = Some(42);
    live.pty_foreground = true;

    assert_eq!(daemon_reattach_target(&live), Some(("daemon-pty-7", 42)));

    live.pty_foreground = false;
    assert_eq!(daemon_reattach_target(&live), None);
    live.pty_foreground = true;
    live.pty_session_generation = None;
    assert_eq!(daemon_reattach_target(&live), None);
}

#[test]
fn terminal_host_matching_never_crosses_local_or_remote_host_boundaries() {
    assert!(session_host_matches(true, None, None));
    assert!(!session_host_matches(true, None, Some("remote-a")));
    assert!(session_host_matches(
        false,
        Some("remote-a"),
        Some("remote-a")
    ));
    assert!(!session_host_matches(
        false,
        Some("remote-a"),
        Some("remote-b")
    ));
    assert!(!session_host_matches(false, Some("remote-a"), None));
    assert!(!session_host_matches(false, None, Some("remote-a")));
    assert!(!session_host_matches(false, None, None));
}

#[test]
fn session_identity_matching_never_crosses_provider_or_account_boundaries() {
    let mut claude_default = session(Provider::Claude, SessionState::Idle, 0);
    claude_default.config_dir = None;
    claude_default.account_email = Some("default@example.com".to_string());
    let mut claude_work = claude_default.clone();
    claude_work.config_dir = Some("/accounts/claude-work".to_string());
    claude_work.account_email = Some("work@example.com".to_string());
    let mut stale_email = claude_work.clone();
    stale_email.account_email = Some("old-work@example.com".to_string());
    let mut codex_default = claude_default.clone();
    codex_default.provider = Provider::Codex;

    assert!(session_identity_matches(
        &claude_default,
        Provider::Claude,
        None,
        Some("default@example.com")
    ));
    assert!(!session_identity_matches(
        &claude_work,
        Provider::Claude,
        None,
        Some("default@example.com")
    ));
    assert!(session_identity_matches(
        &claude_work,
        Provider::Claude,
        Some("/accounts/claude-work"),
        Some("work@example.com")
    ));
    assert!(!session_identity_matches(
        &stale_email,
        Provider::Claude,
        Some("/accounts/claude-work"),
        Some("work@example.com")
    ));
    assert!(!session_identity_matches(
        &codex_default,
        Provider::Claude,
        None,
        Some("default@example.com")
    ));
}

#[test]
fn terminal_account_routing_is_safe_only_when_every_match_agrees() {
    assert!(account_routes_are_unambiguous(std::iter::empty()));
    assert!(account_routes_are_unambiguous([None]));
    assert!(
        !account_routes_are_unambiguous([None, None]),
        "two sessions without a stamped account route are not distinguishable"
    );
    assert!(account_routes_are_unambiguous([
        Some("/accounts/work"),
        Some("/accounts/work"),
    ]));
    assert!(!account_routes_are_unambiguous([
        None,
        Some("/accounts/work"),
    ]));
    assert!(!account_routes_are_unambiguous([
        Some("/accounts/work"),
        Some("/accounts/personal"),
    ]));
}

#[test]
fn live_session_without_an_exact_terminal_does_not_advertise_slash_actions() {
    for state in [
        SessionState::Active,
        SessionState::Waiting,
        SessionState::Monitor,
    ] {
        let session = session(Provider::Claude, state, 4242);
        assert_eq!(
            plan_session_open(&session, false),
            SessionOpenPlan::LiveSessionUnavailable
        );
        assert!(
            !slash_action_available(&session, false),
            "a {state:?} row without an exact terminal must not offer an action \
             that can only end in an unavailable toast"
        );
        assert!(
            slash_action_available(&session, true),
            "a {state:?} Claude session with an exact terminal may stage its slash command"
        );
    }
}

/// A PID alone never proves which process currently owns that number. Until the
/// snapshot carries a verifiable process-identity fingerprint, no row may offer
/// a destructive signal action.
#[test]
fn session_without_process_identity_never_advertises_signal() {
    for pid in [0u32, 1, 2, 4242, u32::MAX] {
        for provider in [Provider::Claude, Provider::Codex] {
            for is_local in [true, false] {
                let caps = SessionCapabilities::of(
                    &session(provider, SessionState::Waiting, pid),
                    is_local,
                );
                assert!(
                    !caps.can_signal,
                    "pid {pid}, {provider:?}, local={is_local}: an unverified pid \
                     must never expose stop/kill"
                );
            }
        }
    }
}
