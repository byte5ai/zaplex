//! Conductor presentation helpers — the pure decisions behind the calm,
//! glanceable cross-host inventory UI (Step 4 "Conductor").
//!
//! These are the parts of the Conductor that carry a *rule* rather than a
//! layout, extracted here so they can be unit-tested without a GPUI harness and
//! shared verbatim between the roomy pane and the compact sidebar (one
//! consistent glyph language, one collapse law, one waiting-cycle order):
//!
//! - [`session_glyph`] — the single glyph vocabulary every surface renders.
//! - [`fleet_is_large`] / [`host_auto_collapsed`] — the *inverse-complexity*
//!   law: more hosts/agents ⇒ **calmer**. Above a threshold, hosts with nothing
//!   waiting fold to a one-line [`host_summary`]; hosts that need you stay open.
//! - [`next_waiting`] — the `w`-jump order: cycle to the next Waiting agent
//!   across the whole fleet, in the tree's already-sorted (waiting-first) order.
//!
//! Pure — no IO, no rendering, no app types.

use crate::fleet::{FleetTree, HostNode};
use crate::types::{SessionSnapshot, SessionState};

/// Working: the agent is busy (Active) or mid tool-run / live job (Monitor) —
/// hands off.
pub const GLYPH_WORKING: &str = "●";
/// Waiting: the agent handed control back — **this** is the attention state.
pub const GLYPH_WAITING: &str = "✋";
/// Idle: a resumable session with no live turn in flight.
pub const GLYPH_IDLE: &str = "◦";

/// The one consistent status glyph for a session, used identically on every
/// Conductor surface (pane, sidebar, and — later — the ambient bit). Active and
/// Monitor collapse to a single "working" glyph on purpose: the calm view cares
/// about *working vs. waiting vs. idle*, not the busy sub-states.
pub fn session_glyph(state: SessionState) -> &'static str {
    match state {
        SessionState::Active | SessionState::Monitor => GLYPH_WORKING,
        SessionState::Waiting => GLYPH_WAITING,
        SessionState::Idle => GLYPH_IDLE,
    }
}

/// Agent-sessions on a single host (across all its projects).
pub fn host_session_count(host: &HostNode) -> usize {
    host.projects.iter().map(|p| p.sessions.len()).sum()
}

/// Agent-sessions across the whole fleet.
pub fn fleet_session_count(tree: &FleetTree) -> usize {
    tree.hosts.iter().map(host_session_count).sum()
}

/// Above this many total agents (or more than this many hosts) the fleet counts
/// as "large" and the inverse-complexity collapsing kicks in. Tuned so a normal
/// single-host day (a handful of agents) never auto-collapses, but a busy
/// multi-host fleet does.
pub const LARGE_FLEET_AGENTS: usize = 8;
/// Hosts above this count also trip the "large" rule, independent of the agent
/// total — many machines is itself a reason to summarize.
pub const LARGE_FLEET_HOSTS: usize = 2;

/// Is the fleet large enough that we default to quiet (auto-collapsed) hosts?
pub fn fleet_is_large(tree: &FleetTree) -> bool {
    fleet_session_count(tree) > LARGE_FLEET_AGENTS || tree.hosts.len() > LARGE_FLEET_HOSTS
}

/// The inverse-complexity law, per host: when the fleet is large, a host with
/// **nothing waiting** folds to a one-line summary (calm by default); a host
/// that needs you stays expanded so the attention state is never hidden. Small
/// fleets never auto-collapse (there's room to show everything).
pub fn host_auto_collapsed(host: &HostNode, fleet_is_large: bool) -> bool {
    fleet_is_large && host.needs_me == 0
}

/// One-line summary for a collapsed host, e.g. `"devhost · 5 agents · 1
/// waiting"`. The waiting clause is omitted when zero — the summary of a calm
/// host stays calm.
pub fn host_summary(host: &HostNode) -> String {
    let agents = host_session_count(host);
    let mut s = format!(
        "{} · {} agent{}",
        host.host,
        agents,
        if agents == 1 { "" } else { "s" }
    );
    if host.needs_me > 0 {
        s.push_str(&format!(" · {} waiting", host.needs_me));
    }
    s
}

/// Every Waiting agent across the fleet, in the tree's canonical order (host,
/// then project, then session — all already waiting-first sorted). Each entry is
/// `(host, &session)`; identity is `(host, session_id)` because session ids are
/// unique only within a host.
pub fn waiting_sessions(tree: &FleetTree) -> Vec<(&str, &SessionSnapshot)> {
    tree.hosts
        .iter()
        .flat_map(|h| {
            h.projects
                .iter()
                .flat_map(move |p| p.sessions.iter().map(move |s| (h.host.as_str(), s)))
        })
        .filter(|(_, s)| s.state == SessionState::Waiting)
        .collect()
}

/// The next Waiting agent across the whole fleet after `current`, cycling back
/// to the first — the `w`-jump order. `current = None`, or a `current` that is
/// no longer waiting / no longer present, starts at the first waiting agent.
/// Returns owned `(host, session_id)`; `None` when nothing is waiting.
pub fn next_waiting(tree: &FleetTree, current: Option<(&str, &str)>) -> Option<(String, String)> {
    let waiting = waiting_sessions(tree);
    if waiting.is_empty() {
        return None;
    }
    let here = current.and_then(|(h, id)| {
        waiting
            .iter()
            .position(|(wh, ws)| *wh == h && ws.session_id == id)
    });
    let next = match here {
        Some(i) => (i + 1) % waiting.len(),
        None => 0,
    };
    let (h, s) = waiting[next];
    Some((h.to_string(), s.session_id.clone()))
}

#[cfg(test)]
#[path = "conductor_tests.rs"]
mod tests;
