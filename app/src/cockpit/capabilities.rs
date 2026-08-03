//! What a given agent-session can actually be asked to do (spec v3 §2 F6).
//!
//! A session row offers verbs — stop, kill, fork, adopt, /compact, review. Not
//! every one of them is possible for every session, and the reasons are not
//! interchangeable:
//!
//! - **Codex records no pid** (`codex_sessions.rs`: there is no process registry
//!   to read one from), so a Codex session's `pid` is `0` and no signal can
//!   reach it. Stop/kill are not merely likely to fail — they cannot work.
//! - **Fork/resume** exist only for CLIs that have the mechanism. `CLIAgent`
//!   knows which; nothing else should re-decide it. Resume is additionally safe
//!   only for a dormant (`Idle`) session; live sessions must be focused/adopted.
//! - **Slash commands** use an existing pane when one is known, or resume an
//!   idle conversation first. Provider support is therefore independent of the
//!   session's current state; the opener decides the safe route.
//! - **Review** reads a git working tree. `project_root` is a path on the host
//!   that reported the session, so for a remote one it names a directory over
//!   there — reviewing it here would open the wrong tree, or nothing.
//!
//! Before this, each verb decided for itself, in its own way and in the middle
//! of rendering: fork asked `CLIAgent`, slash tested `provider == Claude` by
//! hand, review was gated at its call site — and stop/kill were not gated at
//! all, so a Codex row offered them and answered a click with an error toast.
//! Offering an action that cannot work is a lie the UI tells; the honest move is
//! not to offer it.
//!
//! This module answers those questions in one place, and **asks** wherever there
//! is something to ask:
//!
//! - `can_signal` **is** the predicate the signal path refuses on — the same
//!   function, so the verb and the action cannot disagree.
//! - `can_fork` asks `CLIAgent`; `can_resume` combines that CLI capability with
//!   the inventory's authoritative dormant state.
//! - `can_slash` asks the CLI. [`plan_session_open`] separately decides whether
//!   the command goes to an existing pane, an idle resume, or nowhere.
//! - `can_review` **restates** a rule of its own: reviewing needs the working
//!   tree to be here. There is no other holder of that fact to ask, so this is
//!   the one field that could drift from a caller who decided it differently —
//!   which is the reason for asking here rather than at the call site.
//!
//! It does not enforce anything: a caller that skips it can still render a verb
//! that fails. It is the one place to ask, not a gate around the actions.

use zaplex_cockpit::types::{SessionSnapshot, SessionState};

use crate::cockpit::agent_of;

/// Safe route for opening or addressing an agent session.
///
/// A known terminal always wins, even if a filesystem scan has briefly called
/// its transcript idle. Without one, only `Idle` may start a resume process;
/// every live state must wait for a reliable pane/PTY locator rather than
/// creating a second process for the same conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionOpenPlan {
    FocusExistingTerminal,
    ResumeDormant,
    LiveSessionUnavailable,
}

pub fn plan_session_open(
    session: &SessionSnapshot,
    has_existing_terminal: bool,
) -> SessionOpenPlan {
    if has_existing_terminal {
        return SessionOpenPlan::FocusExistingTerminal;
    }

    match session.state {
        SessionState::Idle => SessionOpenPlan::ResumeDormant,
        SessionState::Active | SessionState::Waiting | SessionState::Monitor => {
            SessionOpenPlan::LiveSessionUnavailable
        }
    }
}

/// Returns the only daemon PTY locator that may be attached for this row.
///
/// All three binding facts must agree: an id, a nonzero generation, and the
/// daemon's foreground marker. Historical rows stay visible but are never
/// attachable, and partial/legacy inventory fails closed.
pub fn daemon_reattach_target(session: &SessionSnapshot) -> Option<(&str, u64)> {
    let pty_session_id = session.pty_session_id.as_deref()?;
    let generation = session.pty_session_generation.filter(|value| *value != 0)?;
    session
        .pty_foreground
        .then_some((pty_session_id, generation))
}

/// Whether a terminal belongs to the fleet host named by an action. Locality is
/// explicit; remote hosts match only by the daemon's stable id, never by label.
pub fn session_host_matches(
    is_local: bool,
    expected_remote_host_id: Option<&str>,
    terminal_remote_host_id: Option<&str>,
) -> bool {
    if is_local {
        return terminal_remote_host_id.is_none();
    }

    match (expected_remote_host_id, terminal_remote_host_id) {
        (Some(expected), Some(actual)) => expected == actual,
        (Some(_), None) | (None, Some(_)) | (None, None) => false,
    }
}

/// Whether a provider/session id names at most one account route on its host.
/// The terminal tracker currently knows provider + conversation, but not the
/// account config directory; focusing it is safe only while every matching
/// inventory row agrees on that missing coordinate.
pub fn account_routes_are_unambiguous<'a>(
    routes: impl IntoIterator<Item = Option<&'a str>>,
) -> bool {
    let mut first: Option<Option<&'a str>> = None;
    let mut count = 0usize;
    for route in routes {
        count += 1;
        match first {
            None => first = Some(route),
            Some(expected) if expected == route => {}
            Some(_) => return false,
        }
    }
    count <= 1 || first.flatten().is_some()
}

/// Whether the row can truthfully offer an in-conversation slash command.
///
/// An exact live pane can receive it directly, and an idle conversation can be
/// resumed first. A live session without an exact pane cannot be duplicated
/// merely to make the menu item appear to work.
pub fn slash_action_available(session: &SessionSnapshot, has_exact_terminal: bool) -> bool {
    SessionCapabilities::of(session, true).can_slash
        && !matches!(
            plan_session_open(session, has_exact_terminal),
            SessionOpenPlan::LiveSessionUnavailable
        )
}

/// What this session supports. Every field is a fact about *this* session, not a
/// guess about its provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCapabilities {
    /// Stop (SIGINT) and kill (SIGKILL) can reach the process. False whenever
    /// discovery recorded no pid — always the case for Codex.
    pub can_signal: bool,
    /// The conversation can be forked into a divergent one.
    pub can_fork: bool,
    /// The conversation is dormant and the provider can resume it.
    pub can_resume: bool,
    /// In-conversation slash commands (`/compact`, `/clear`) can be sent.
    pub can_slash: bool,
    /// The session's working tree can be reviewed from here.
    pub can_review: bool,
}

impl SessionCapabilities {
    /// Derive from the session itself plus where it lives.
    ///
    /// `is_local` comes from the inventory's explicit marker, never from
    /// comparing host labels: a remote daemon can advertise the local hostname,
    /// and treating it as local would review this machine's identically-named
    /// directory instead of the session's.
    pub fn of(session: &SessionSnapshot, is_local: bool) -> Self {
        let agent = agent_of(session.provider);
        let can_resume = matches!(session.state, SessionState::Idle)
            && agent.resume_command(&session.session_id).is_some();
        Self {
            // The one honest test: a pid we can actually signal. Asked of the
            // same helper the signal path itself uses, so the verb we offer and
            // the action we'd take cannot disagree.
            can_signal: zaplex_cockpit::pid_signalable(session.pid)
                && session.process_fingerprint.is_some(),
            can_fork: agent.fork_command(&session.session_id).is_some(),
            can_resume,
            // A known live pane receives the command directly; an idle session
            // first resumes. Routing is handled by `plan_session_open`.
            can_slash: agent.supports_slash_commands(),
            can_review: is_local,
        }
    }
}

#[cfg(test)]
#[path = "capabilities_tests.rs"]
mod tests;
