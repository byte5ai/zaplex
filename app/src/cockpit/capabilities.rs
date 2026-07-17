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
//!   knows which; nothing else should re-decide it.
//! - **Slash commands** are typed into a resumed conversation, so they need both
//!   a CLI that has them and a session that can be resumed.
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
//! - `can_fork` / `can_resume` ask `CLIAgent`, where what-a-CLI-can-do lives.
//! - `can_slash` composes two of those facts (a CLI that has slash commands,
//!   and a session that can be resumed).
//! - `can_review` **restates** a rule of its own: reviewing needs the working
//!   tree to be here. There is no other holder of that fact to ask, so this is
//!   the one field that could drift from a caller who decided it differently —
//!   which is the reason for asking here rather than at the call site.
//!
//! It does not enforce anything: a caller that skips it can still render a verb
//! that fails. It is the one place to ask, not a gate around the actions.

use zaplex_cockpit::types::SessionSnapshot;

use crate::cockpit::agent_of;

/// What this session supports. Every field is a fact about *this* session, not a
/// guess about its provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCapabilities {
    /// Stop (SIGINT) and kill (SIGKILL) can reach the process. False whenever
    /// discovery recorded no pid — always the case for Codex.
    pub can_signal: bool,
    /// The conversation can be forked into a divergent one.
    pub can_fork: bool,
    /// The conversation can be resumed/adopted in place.
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
        let can_resume = agent.resume_command(&session.session_id).is_some();
        Self {
            // The one honest test: a pid we can actually signal. Asked of the
            // same helper the signal path itself uses, so the verb we offer and
            // the action we'd take cannot disagree.
            can_signal: zaplex_cockpit::pid_signalable(session.pid),
            can_fork: agent.fork_command(&session.session_id).is_some(),
            can_resume,
            // Both halves are needed: the command is typed into a session that
            // has been resumed.
            can_slash: agent.supports_slash_commands() && can_resume,
            can_review: is_local,
        }
    }
}

#[cfg(test)]
#[path = "capabilities_tests.rs"]
mod tests;
