//! Guardrails (cockpit step 7): pause / stop / kill any agent, plus a
//! fleet-wide stop-all — so the user trusts nothing runs unattended out of
//! control. This module carries only the **pure decisions**: which POSIX
//! signal a verb sends, whether a session's host executes it locally or over
//! the daemon, and the exact confirm-dialog / toast text. The process-identity
//! module owns verified local signalling; the app wires these decisions to it
//! and to the remote-server client; see `app/src/workspace/view.rs`.

use crate::types::SessionSnapshot;

/// The two signals the cockpit ever sends from a guardrail verb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GuardrailSignal {
    /// Graceful interrupt (Ctrl-C) — the row's "pause" verb and stop-all.
    Interrupt,
    /// Forced kill — the row's "kill" verb, always behind a confirmation.
    Kill,
}

impl GuardrailSignal {
    /// The POSIX signal number (`SIGINT` = 2, `SIGKILL` = 9).
    pub fn signal_number(self) -> i32 {
        match self {
            GuardrailSignal::Interrupt => 2,
            GuardrailSignal::Kill => 9,
        }
    }
}

/// Where a guardrail signal for a given host executes: `Local` uses verified
/// in-process signalling; `Remote` asks that host's daemon to perform the same
/// identity verification before signalling.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardrailTarget {
    Local,
    Remote(String),
}

/// Decide which host executes a guardrail signal, driven by the **explicit**
/// local/remote marker carried through the inventory ([`crate::fleet::HostNode::is_local`]),
/// NOT by comparing display labels.
///
/// PIDs are host-local: a remote daemon whose label collides with the local
/// hostname (SSH alias, or a remote `gethostname()` matching ours) must still
/// route over the daemon — routing it `Local` would send `SIGKILL`/`SIGINT` to
/// a same-numbered, unrelated *local* process. So locality comes from
/// `is_local` (set once at fold time from which contribution is this machine's),
/// and the label is only carried along in `Remote` for the daemon lookup.
pub fn guardrail_target(is_local: bool, host_label: &str) -> GuardrailTarget {
    if is_local {
        GuardrailTarget::Local
    } else {
        GuardrailTarget::Remote(host_label.to_string())
    }
}

/// A session whose pid can't be honestly signaled: unknown (`0`, discovery
/// never recorded one) or outside the positive signed process-id range. The
/// caller must surface a clear toast rather than a silent no-op (design
/// invariant: never hide a failure as a no-op).
///
/// POSIX reserves non-positive `pid_t` values for process groups and broad
/// targets. Validate the unsigned inventory value before any caller casts it
/// to `pid_t`, so `u32::MAX` can never become `-1` and signal every permitted
/// process.
pub fn pid_signalable(pid: u32) -> bool {
    i32::try_from(pid).is_ok_and(|pid| pid > 0)
}

/// The Conductor row's own label rule (`name — dir`, or just `dir` when
/// unnamed), reused verbatim so confirm-dialog / toast text names the agent
/// exactly as its row does.
pub fn session_label(session: &SessionSnapshot) -> String {
    let dir = std::path::Path::new(&session.cwd)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| session.cwd.clone());
    if session.name.is_empty() {
        dir
    } else {
        format!("{} — {dir}", session.name)
    }
}

/// Title + body for the kill-confirmation dialog ("Kill \"<agent>\"? …").
/// Pure builder so the copy is unit-testable without a GPUI harness.
pub fn kill_confirm_message(session_label: &str, project_name: &str) -> (String, String) {
    let title = format!("Kill \u{201c}{session_label}\u{201d}?");
    let body = if project_name.trim().is_empty() {
        "This sends SIGKILL and ends the process immediately — unsaved work in \
         the agent's current turn is lost."
            .to_string()
    } else {
        format!(
            "This sends SIGKILL and ends the process immediately in {} — \
             unsaved work in the agent's current turn is lost.",
            project_name.trim()
        )
    };
    (title, body)
}

/// Title + body for the fleet-wide stop-all confirmation dialog. `n` is the
/// number of live agents that will be interrupted (callers gate on `n > 0`
/// before offering the control at all, so `n == 0` never renders — this stays
/// grammatically correct regardless).
pub fn stop_all_confirm_message(n: usize) -> (String, String) {
    let title = format!("Stop all {n} agent{}?", if n == 1 { "" } else { "s" });
    let body = "Sends an interrupt (Ctrl-C) to every live agent across the \
                fleet — the same graceful stop as each row's pause verb."
        .to_string();
    (title, body)
}

/// Toast text for a session whose process can't be signaled honestly (unknown,
/// unverified, or already dead) — used by both the pause and kill verbs.
pub fn unsignalable_toast(session_label: &str) -> String {
    format!(
        "\u{201c}{session_label}\u{201d} has no verified live process (already exited?) — nothing to signal."
    )
}

/// Toast text confirming a signal was sent.
pub fn sent_toast(session_label: &str, signal: GuardrailSignal) -> String {
    match signal {
        GuardrailSignal::Interrupt => format!("Sent interrupt to \u{201c}{session_label}\u{201d}."),
        GuardrailSignal::Kill => format!("Killed \u{201c}{session_label}\u{201d}."),
    }
}

/// Toast text for a signal that failed — always shown, never swallowed.
pub fn failed_toast(session_label: &str, signal: GuardrailSignal, error: &str) -> String {
    let verb = match signal {
        GuardrailSignal::Interrupt => "interrupt",
        GuardrailSignal::Kill => "kill",
    };
    format!("Could not {verb} \u{201c}{session_label}\u{201d}: {error}")
}

/// Toast text when a remote host has no live daemon connection to route the
/// signal through — the honest degradation the remote path reports instead of
/// silently doing nothing.
pub fn no_remote_connection_toast(session_label: &str, host: &str) -> String {
    format!(
        "\u{201c}{session_label}\u{201d} is on {host}, which has no live connection right now — \
         could not send the signal. Open a tab on that host and retry."
    )
}

/// Toast text when the target host's daemon is too old to run a session-less
/// host command (no `host-exec` capability) — the honest degradation instead of
/// a misleading "could not kill" failure. Distinct from a lost connection: the
/// host is reachable, but its daemon predates the guardrail command path.
pub fn remote_unsupported_toast(session_label: &str, host: &str) -> String {
    format!(
        "\u{201c}{session_label}\u{201d} is on {host}, whose daemon is too old for remote \
         guardrails — update the zaplex daemon on that host, then retry."
    )
}

/// Fleet-wide stop-all summary toast, given how many signals succeeded vs.
/// failed (including unreachable remote hosts) — always reports the outcome,
/// never silently completes.
pub fn stop_all_summary_toast(sent: usize, failed: usize) -> String {
    if failed == 0 {
        format!("Stopped {sent} agent{}.", if sent == 1 { "" } else { "s" })
    } else {
        format!(
            "Stopped {sent} agent{}, {failed} could not be reached.",
            if sent == 1 { "" } else { "s" },
        )
    }
}

#[cfg(test)]
#[path = "guardrails_tests.rs"]
mod tests;
