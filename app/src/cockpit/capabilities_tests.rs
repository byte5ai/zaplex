use chrono::Utc;
use zaplex_cockpit::types::{Provider, SessionSnapshot, SessionState};

use super::*;

fn session(provider: Provider, pid: u32) -> SessionSnapshot {
    SessionSnapshot {
        session_id: "019f135f-7fcc-7d93-8a28-4835d98f8f0a".into(),
        cwd: "/tmp/proj".into(),
        name: String::new(),
        state: SessionState::Waiting,
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
        last_activity: Utc::now(),
        pid,
    }
}

/// The gap F6 exists to close: Codex has no process registry, so its sessions
/// carry no pid and no signal can reach them. The row must not offer stop/kill
/// and answer the click with an error toast.
#[test]
fn a_codex_session_cannot_be_signalled() {
    let caps = SessionCapabilities::of(&session(Provider::Codex, 0), true);
    assert!(!caps.can_signal, "no pid, no signal — do not offer stop/kill");
    // Everything else Codex genuinely does keep.
    assert!(caps.can_fork, "codex fork <id> exists");
    assert!(caps.can_resume, "codex resume <id> exists");
}

#[test]
fn a_claude_session_with_a_live_pid_can_be_signalled() {
    let caps = SessionCapabilities::of(&session(Provider::Claude, 4242), true);
    assert!(caps.can_signal);
    assert!(caps.can_fork);
    assert!(caps.can_resume);
    assert!(caps.can_slash);
}

/// pid 0 means "discovery recorded none", whoever the provider is — the test is
/// the pid, not the brand.
#[test]
fn a_claude_session_without_a_pid_cannot_be_signalled_either() {
    let caps = SessionCapabilities::of(&session(Provider::Claude, 0), true);
    assert!(!caps.can_signal);
    // …but it is still a conversation: the rest stands.
    assert!(caps.can_resume);
    assert!(caps.can_slash);
}

/// Slash commands are typed into a resumed conversation. Codex has no `/compact`,
/// so the pair is what decides — not the provider on its own.
#[test]
fn slash_commands_need_both_a_cli_that_has_them_and_a_resumable_session() {
    assert!(!SessionCapabilities::of(&session(Provider::Codex, 0), true).can_slash);
    assert!(SessionCapabilities::of(&session(Provider::Claude, 1), true).can_slash);
}

/// `project_root` is a path on the host that reported the session. Reviewing a
/// remote one from here would open this machine's identically-named directory —
/// the wrong tree, silently.
#[test]
fn a_remote_session_cannot_be_reviewed_from_here() {
    assert!(!SessionCapabilities::of(&session(Provider::Claude, 1), false).can_review);
    assert!(SessionCapabilities::of(&session(Provider::Claude, 1), true).can_review);
}

/// Being remote costs only the local-tree verb: the conversation itself is
/// still forkable and resumable through the daemon.
#[test]
fn a_remote_session_keeps_everything_that_does_not_need_this_machine() {
    let caps = SessionCapabilities::of(&session(Provider::Claude, 4242), false);
    assert!(caps.can_fork);
    assert!(caps.can_resume);
    assert!(caps.can_slash);
    assert!(caps.can_signal, "a remote pid is signalled via the daemon");
    assert!(!caps.can_review);
}

/// The no-regression guarantee, stated as a test rather than as a claim: the
/// verb we offer and the signal path's own refusal must agree on every pid, or
/// gating the row either hides a working action or keeps a dead one.
#[test]
fn can_signal_is_exactly_what_the_signal_path_accepts() {
    for pid in [0u32, 1, 2, 4242, u32::MAX] {
        for provider in [Provider::Claude, Provider::Codex] {
            for is_local in [true, false] {
                let caps = SessionCapabilities::of(&session(provider, pid), is_local);
                assert_eq!(
                    caps.can_signal,
                    zaplex_cockpit::pid_signalable(pid),
                    "pid {pid}, {provider:?}, local={is_local}: the row must offer \
                     stop/kill exactly when the handler would carry them out"
                );
            }
        }
    }
}
