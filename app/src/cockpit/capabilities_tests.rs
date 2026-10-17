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
        account_id: None,
        process_fingerprint: None,
        pty_session_id: None,
        pty_session_generation: None,
        pty_foreground: false,
        task_state: None,
        last_activity: Utc::now(),
        pid,
        awaiting_input: false,
        turn_id: None,
        attention: None,
    }
}

#[test]
fn missing_process_fingerprint_disables_signal_fail_closed() {
    let caps =
        SessionCapabilities::of(&session(Provider::Claude, SessionState::Active, 4242), true);
    assert!(
        !caps.can_signal,
        "a pid without a process-identity fingerprint is not a safe signal target"
    );
    assert!(caps.can_fork);
    assert!(!caps.can_resume);
    assert!(caps.can_slash);
}

/// A dormant Claude conversation can be resumed before staging its slash
/// command. Codex has no equivalent slash commands.
#[test]
fn dormant_slash_commands_follow_provider_support() {
    assert!(
        !SessionCapabilities::of(&session(Provider::Codex, SessionState::Idle, 0), true,).can_slash
    );
    assert!(
        SessionCapabilities::of(&session(Provider::Claude, SessionState::Idle, 0), true,).can_slash
    );
}

#[test]
fn only_a_dormant_session_can_be_resumed() {
    for provider in [Provider::Claude, Provider::Codex, Provider::Antigravity] {
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
fn open_plan_focuses_known_terminal_for_every_session_state() {
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
        plan_session_open(&session(Provider::Claude, SessionState::Idle, 0), false,),
        SessionOpenPlan::ResumeDormant,
    );
}

#[test]
fn session_node_dispatches_shared_session_open_plan() {
    let active = session(Provider::Codex, SessionState::Active, 0);
    let dormant = session(Provider::Claude, SessionState::Idle, 0);

    assert_eq!(
        plan_session_open(&active, true),
        SessionOpenPlan::FocusExistingTerminal
    );
    assert_eq!(
        plan_session_open(&active, false),
        SessionOpenPlan::LiveSessionUnavailable
    );
    assert_eq!(
        plan_session_open(&dormant, false),
        SessionOpenPlan::ResumeDormant
    );
}

#[test]
fn reattach_uses_pty_id_without_cwd_guessing() {
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
fn missing_pty_binding_never_starts_duplicate_session() {
    let mut live = session(Provider::Codex, SessionState::Active, 0);
    assert_eq!(daemon_reattach_target(&live), None);
    assert_eq!(
        plan_session_open(&live, false),
        SessionOpenPlan::LiveSessionUnavailable
    );

    live.pty_session_id = Some("daemon-pty-7".to_string());
    live.pty_session_generation = Some(42);
    assert_eq!(
        daemon_reattach_target(&live),
        None,
        "a non-foreground binding is not attachable"
    );
    assert_eq!(
        plan_session_open(&live, false),
        SessionOpenPlan::LiveSessionUnavailable,
        "a partial PTY binding must not fall back to resume"
    );
}

#[test]
fn terminal_host_matching_never_crosses_local_or_remote_boundaries() {
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

    let default_key = zaplex_cockpit::session_key(true, None, &claude_default);
    assert_ne!(
        default_key,
        zaplex_cockpit::session_key(true, None, &claude_work)
    );
    assert_ne!(
        zaplex_cockpit::session_key(true, None, &claude_work),
        zaplex_cockpit::session_key(true, None, &stale_email)
    );
    assert_ne!(
        default_key,
        zaplex_cockpit::session_key(true, None, &codex_default)
    );
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

/// The attention projection shows and counts a row on the same evidence the
/// click path acts on: a known terminal, a dormant resume, or a reattachable
/// remote daemon PTY.
#[test]
fn reachability_mirrors_the_click_path() {
    let terminal = warpui::EntityId::new();
    let live = session(Provider::Claude, SessionState::Waiting, 4242);
    assert_eq!(
        session_reach(&live, true, Some(terminal), false),
        SessionReach::Terminal(terminal)
    );
    assert_eq!(
        session_reach(&live, true, None, true),
        SessionReach::Unreachable,
        "a local live session without a pane cannot be opened"
    );

    let dormant = session(Provider::Claude, SessionState::Idle, 0);
    assert_eq!(
        session_reach(&dormant, true, None, false),
        SessionReach::Resume
    );

    let mut remote = session(Provider::Codex, SessionState::Waiting, 0);
    remote.pty_session_id = Some("pty-9".to_string());
    remote.pty_session_generation = Some(2);
    remote.pty_foreground = true;
    assert_eq!(
        session_reach(&remote, false, None, true),
        SessionReach::DaemonReattach
    );
    assert_eq!(
        session_reach(&remote, false, None, false),
        SessionReach::Unreachable,
        "without a registry route the reattach cannot happen"
    );
    remote.pty_foreground = false;
    assert_eq!(
        session_reach(&remote, false, None, true),
        SessionReach::Unreachable,
        "a historical PTY binding is not attachable"
    );
}

/// External agents are hidden; Zaplex-launched ones stay visible while they
/// are momentarily unreachable. A daemon PTY binding proves Zaplex ownership
/// even when it is not the foreground one.
#[test]
fn only_openable_or_zaplex_owned_rows_are_visible() {
    let external = session(Provider::Claude, SessionState::Waiting, 4242);
    assert!(!zaplex_owned(&external, false));
    assert!(!session_visible(SessionReach::Unreachable, false, false));
    assert!(session_visible(SessionReach::Resume, false, false));
    assert!(session_visible(
        SessionReach::Terminal(warpui::EntityId::new()),
        false,
        false
    ));

    let mut detached = session(Provider::Codex, SessionState::Waiting, 0);
    detached.pty_session_id = Some("pty-9".to_string());
    assert!(zaplex_owned(&detached, false));
    assert!(
        zaplex_owned(&external, true),
        "launch, hook or surface evidence"
    );
    assert!(session_visible(SessionReach::Unreachable, true, false));
    assert!(
        session_visible(SessionReach::Unreachable, false, true),
        "an origin that cannot be inspected is never hidden"
    );
}

/// The process environment link and the pane registry use the same variable.
#[test]
fn process_link_reads_the_variable_zaplex_injects_into_panes() {
    assert_eq!(
        zaplex_cockpit::ZAPLEX_SURFACE_ENV,
        warp_cli::control::SURFACE_ID_ENV
    );
}
