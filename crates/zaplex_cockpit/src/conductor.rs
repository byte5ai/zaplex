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
use crate::format::{context_fill, model_family};
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

/// Title-case a single lowercase display token: `"high"` -> `"High"`, `""` ->
/// `""`. ASCII-first-letter only — enough for the effort/model-family words the
/// Conductor renders.
fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Human word for a session state, used in the one-line attribute summary.
fn state_word(state: SessionState) -> &'static str {
    match state {
        SessionState::Waiting => "waiting",
        SessionState::Active | SessionState::Monitor => "working",
        SessionState::Idle => "idle",
    }
}

/// The compact **model·effort** label for a Conductor row.
///
/// A known Claude family (`opus`/`sonnet`/`haiku`/`fable`) is title-cased; any
/// other id (e.g. a Codex `gpt-5.5`) is shown verbatim. Effort — when known —
/// is appended, title-cased, after a `·`. The honest edges:
/// - **empty model** → `""` (nothing to show, never a placeholder), and
/// - **unknown effort** (`None`/blank) → the model alone (never an invented
///   effort — effort is absent from Claude transcripts entirely).
///
/// E.g. `("claude-opus-4-8", Some("high"))` -> `"Opus·High"`;
/// `("claude-opus-4-8", None)` -> `"Opus"`; `("gpt-5.5", Some("high"))` ->
/// `"gpt-5.5·High"`; `("", _)` -> `""`.
pub fn model_effort_label(model: &str, effort: Option<&str>) -> String {
    if model.trim().is_empty() {
        return String::new();
    }
    let fam = model_family(model);
    // `model_family` echoes the raw id when no Claude family matched; title-case
    // only the known family words, show any other id as-is.
    let model_disp = if ["opus", "sonnet", "haiku", "fable"].contains(&fam) {
        title_case(fam)
    } else {
        fam.to_string()
    };
    match effort {
        Some(e) if !e.trim().is_empty() => format!("{model_disp}·{}", title_case(e)),
        _ => model_disp,
    }
}

/// The always-visible per-row attributes (Step 8): the compact model·effort
/// label, the context-window fill, and the status glyph — the pure assembly the
/// pane and sidebar render (each field colored at the view layer). Keeping this
/// headless lets the "unknown effort / empty model / no-context-yet" edges be
/// unit-tested without a GPUI harness.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionAttrs {
    /// `"Opus·High"` / `"Opus"` / `""` — see [`model_effort_label`].
    pub model_effort: String,
    /// Rounded context-window fill percent of the latest turn; `None` when no
    /// context tokens are known yet (nothing to show — never a fake `0%`).
    pub ctx_pct: Option<u32>,
    /// Context fill fraction (0.0..) for coloring via
    /// [`crate::format::HeatLevel::from_fraction`].
    pub ctx_fill: f64,
    /// The status glyph (working/waiting/idle) — see [`session_glyph`].
    pub glyph: &'static str,
}

/// Assemble the always-visible row attributes from a session's model, effort,
/// context tokens, and state. Context fill is derived from the model's window
/// ([`context_fill`]) so Claude and Codex are treated identically.
pub fn session_attrs(
    model: &str,
    effort: Option<&str>,
    ctx_tokens: u64,
    state: SessionState,
) -> SessionAttrs {
    let ctx_fill = context_fill(model, ctx_tokens);
    let ctx_pct = (ctx_tokens > 0).then(|| (ctx_fill * 100.0).round() as u32);
    SessionAttrs {
        model_effort: model_effort_label(model, effort),
        ctx_pct,
        ctx_fill,
        glyph: session_glyph(state),
    }
}

/// The canonical one-line attribute summary — e.g. `"Opus·High · 42% ctx · ✋
/// waiting"`. The string form of [`session_attrs`], used for tests / tooltips /
/// accessibility; the view renders the same fields as individually-colored
/// spans. Empty pieces (unknown model, no context yet) are omitted so the line
/// never carries a placeholder.
pub fn session_attr_line(
    model: &str,
    effort: Option<&str>,
    ctx_tokens: u64,
    state: SessionState,
) -> String {
    let attrs = session_attrs(model, effort, ctx_tokens, state);
    let mut parts: Vec<String> = Vec::new();
    if !attrs.model_effort.is_empty() {
        parts.push(attrs.model_effort);
    }
    if let Some(pct) = attrs.ctx_pct {
        parts.push(format!("{pct}% ctx"));
    }
    parts.push(format!("{} {}", attrs.glyph, state_word(state)));
    parts.join(" · ")
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

/// Stable host-identity string for keying per-host UI state (collapse, hover,
/// the pre-scoped "+"). `local` for this machine, `daemon:<host_id>` for a
/// remote daemon — **never** the display label. Two remote daemons can advertise
/// the same label (SSH alias / matching `gethostname()`), and a label key would
/// then alias their UI state into one; `(is_local, host_id)` keeps them
/// distinct. Mirrors the identity used by [`WaitingTarget`] / [`next_waiting`].
pub fn host_ident(is_local: bool, host_id: Option<&str>) -> String {
    if is_local {
        // The local host carries no `host_id`; `is_local` alone identifies it,
        // and its key stays stable across reconciles regardless of hostname.
        "local".to_string()
    } else {
        match host_id {
            Some(id) => format!("daemon:{id}"),
            // A remote should always carry a stable id; if one is somehow
            // absent, use a fixed marker rather than the label so we never
            // reintroduce label aliasing (better one merged edge case than
            // silently crossing two hosts' state).
            None => "daemon:?".to_string(),
        }
    }
}

/// Composite `(host-identity, id)` key for per-`(host, session)` or
/// `(host, project)` UI state. Session and project ids are unique only within a
/// host, so they are scoped by the **stable** host identity — never the display
/// label. Use for every seed / lookup / retain of such maps so labels can never
/// key UI state again.
pub fn host_key(is_local: bool, host_id: Option<&str>, id: &str) -> String {
    format!("{}\u{0}{id}", host_ident(is_local, host_id))
}

/// A stable, host-identity-carrying pointer to one Waiting agent — the `w`-jump
/// target and cursor. It keeps the display `host_label` for the attach dispatch,
/// but **identity** is the stable `(is_local, host_id)` pair plus the
/// host-scoped `session_id`, never the label. Two remote daemons can advertise
/// the same label (SSH alias / matching `gethostname()`) and even share a
/// host-scoped `session_id`, yet stay distinct here because their `host_id`
/// differs — so the jump cycle visits both and never collapses them.
#[derive(Clone, Debug, PartialEq)]
pub struct WaitingTarget {
    /// Human host label (for display + the attach dispatch), not for identity.
    pub host_label: String,
    /// Stable per-daemon host id: `None` for the local host, `Some(daemon host
    /// id)` for a remote. Part of the identity key.
    pub host_id: Option<String>,
    /// `true` iff the target lives on this machine — part of the identity key
    /// (the local host carries no `host_id`, so `is_local` disambiguates it).
    pub is_local: bool,
    /// The host-scoped session id (unique only within one host).
    pub session_id: String,
}

impl WaitingTarget {
    /// Same waiting agent? Compared by **stable** host identity `(is_local,
    /// host_id)` + `session_id` — never the display label, so a label collision
    /// between two remote daemons can't alias two distinct agents into one.
    fn same_agent(&self, host: &HostNode, session: &SessionSnapshot) -> bool {
        host.is_local == self.is_local
            && host.host_id == self.host_id
            && session.session_id == self.session_id
    }
}

/// Every Waiting agent across the fleet, in the tree's canonical order (host,
/// then project, then session — all already waiting-first sorted). Each entry is
/// `(&host, &session)` so callers can read the host's **stable** identity
/// (`is_local`/`host_id`) alongside its display label; session identity is
/// host-scoped (`session_id` is unique only within a host).
pub fn waiting_sessions(tree: &FleetTree) -> Vec<(&HostNode, &SessionSnapshot)> {
    tree.hosts
        .iter()
        .flat_map(|h| {
            h.projects
                .iter()
                .flat_map(move |p| p.sessions.iter().map(move |s| (h, s)))
        })
        .filter(|(_, s)| s.state == SessionState::Waiting)
        .collect()
}

/// The next Waiting agent across the whole fleet after `current`, cycling back
/// to the first — the `w`-jump order. `current = None`, or a `current` that is
/// no longer waiting / no longer present, starts at the first waiting agent.
///
/// `current` and the returned [`WaitingTarget`] key on the **stable** host
/// identity `(is_local, host_id)` + `session_id`, never the display label: two
/// hosts sharing a label (and even a host-scoped `session_id`) stay distinct, so
/// the cycle visits both. `None` when nothing is waiting.
pub fn next_waiting(tree: &FleetTree, current: Option<&WaitingTarget>) -> Option<WaitingTarget> {
    let waiting = waiting_sessions(tree);
    if waiting.is_empty() {
        return None;
    }
    let here = current.and_then(|cur| waiting.iter().position(|(h, s)| cur.same_agent(h, s)));
    let next = match here {
        Some(i) => (i + 1) % waiting.len(),
        None => 0,
    };
    let (h, s) = waiting[next];
    Some(WaitingTarget {
        host_label: h.host.clone(),
        host_id: h.host_id.clone(),
        is_local: h.is_local,
        session_id: s.session_id.clone(),
    })
}

#[cfg(test)]
#[path = "conductor_tests.rs"]
mod tests;
