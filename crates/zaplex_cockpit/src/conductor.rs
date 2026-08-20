//! Conductor presentation helpers — the pure decisions behind the calm,
//! glanceable cross-host inventory UI (Step 4 "Conductor").
//!
//! These are the parts of the Conductor that carry a *rule* rather than a
//! layout, extracted here so they can be unit-tested without a GPUI harness and
//! shared by the roomy pane and the compact sidebar where their behavior is
//! meant to match (one consistent glyph language and one waiting-cycle order):
//!
//! - [`session_glyph`] — the single glyph vocabulary every surface renders.
//! - [`fleet_is_large`] / [`host_auto_collapsed`] — the roomy pane's
//!   *inverse-complexity* law: above a threshold, hosts with nothing waiting
//!   fold to a one-line [`host_summary`]; explicit sidebar expansion is kept
//!   separate.
//! - [`next_waiting`] — the `w`-jump order: cycle to the next Waiting agent
//!   across the whole fleet, in the tree's already-sorted (waiting-first) order.
//!
//! Pure — no IO, no rendering, no app types.

use crate::fleet::{FleetTree, HostNode};
use crate::format::{context_fill, model_family};
use crate::types::{Provider, SessionSnapshot, SessionState};
use std::collections::BTreeMap;

// Premium status dots: one uniform shape, meaning is carried by COLOR (the
// renderers color each glyph by state — green working · amber waiting · faint
// idle), not by an emoji. This keeps the Conductor calm and consistent instead
// of dropping a coloured emoji hand into an otherwise monochrome premium UI.
// Each state is distinguished by the dot's **shape**, not by colour alone, so
// the vocabulary survives red-green colour-blindness (a filled green dot and an
// amber one are otherwise indistinguishable). Colour reinforces the shape; in
// tables the state word is shown alongside.
/// Working: the agent is busy (Active) or mid tool-run / live job (Monitor) —
/// hands off. A **filled** dot (rendered green).
pub const GLYPH_WORKING: &str = "●";
/// Waiting: the agent handed control back — **this** is the attention state.
/// A **fisheye** dot (a filled centre inside a ring — a halo) rendered in the
/// amber attention colour: it stands out by shape *and* colour, so it reads as
/// "needs you" even without colour and without an emoji.
pub const GLYPH_WAITING: &str = "◉";
/// Idle: a resumable session with no live turn in flight. A **hollow** ring
/// (reads as "not active") in the faint colour.
pub const GLYPH_IDLE: &str = "○";

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
/// The state's word, for surfaces that spell it out beside the glyph (the
/// session table's Status column). One source, like [`session_glyph`] is for the
/// shape — a second `match` elsewhere is how a row ends up saying "working" next
/// to a waiting mark.
pub fn state_word(state: SessionState) -> &'static str {
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
/// `"gpt-5.5·High"`; a legacy estimate prefixed with `~` stays visibly
/// approximate (`"~high"` -> `"~High"`); `("", _)` -> `""`.
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
        Some(e) if !e.trim().is_empty() => {
            let effort = e
                .strip_prefix('~')
                .map(|estimated| format!("~{}", title_case(estimated)))
                .unwrap_or_else(|| title_case(e));
            format!("{model_disp}·{effort}")
        }
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

/// User-facing Session containers across the four-level Conductor tree.
/// Several Claude/Codex agents sharing one PTY generation count as one Session,
/// while agents without a PTY identity remain separate resumable sessions.
pub fn fleet_conductor_session_count(tree: &FleetTree) -> usize {
    tree.hosts.iter().map(host_conductor_session_count).sum()
}

/// User-facing PTY Session containers below one host in the four-level tree.
pub fn host_conductor_session_count(host: &HostNode) -> usize {
    host.projects
        .iter()
        .map(|project| {
            group_project_sessions(host.is_local, host.host_id.as_deref(), &project.sessions).len()
        })
        .sum()
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

/// Composite `(host-identity, id)` key for project UI state and legacy favorite
/// compatibility. The id is scoped by the **stable** host identity — never the
/// display label. Session rows must use [`session_key`] instead because a
/// conversation id can also collide between provider accounts on one host.
pub fn host_key(is_local: bool, host_id: Option<&str>, id: &str) -> String {
    format!("{}\u{0}{id}", host_ident(is_local, host_id))
}

/// Complete identity of one agent session for UI state and action routing.
///
/// A session id is not globally unique: a transcript can be copied to another
/// provider account, and the same id can exist on several hosts. The account
/// email is the durable account coordinate; `config_dir` is the exact host-local
/// route. Keeping both makes the key fail closed for stale or copied account
/// configurations instead of letting either coordinate silently override the
/// other.
pub fn session_identity_key(
    is_local: bool,
    host_id: Option<&str>,
    provider: Provider,
    config_dir: Option<&str>,
    account_email: Option<&str>,
    session_id: &str,
) -> String {
    session_identity_key_with_account_id(
        is_local,
        host_id,
        provider,
        config_dir,
        account_email,
        None,
        session_id,
    )
}

pub fn session_identity_key_with_account_id(
    is_local: bool,
    host_id: Option<&str>,
    provider: Provider,
    config_dir: Option<&str>,
    account_email: Option<&str>,
    account_id: Option<&str>,
    session_id: &str,
) -> String {
    let account = match (account_id, account_email) {
        (Some(account_id), _) => format!("opaque:{account_id}"),
        (None, Some(email)) => format!("email:{email}"),
        (None, None) => "unknown".to_string(),
    };
    let config = match (account_id, config_dir) {
        (Some(_), _) => "opaque".to_string(),
        (None, Some(config_dir)) => format!("config:{config_dir}"),
        (None, None) => "default".to_string(),
    };
    format!(
        "{}\u{0}{}\u{0}{account}\u{0}{config}\u{0}{session_id}",
        host_ident(is_local, host_id),
        provider.as_str(),
    )
}

/// Complete identity key for an observed session snapshot.
pub fn session_key(is_local: bool, host_id: Option<&str>, session: &SessionSnapshot) -> String {
    session_identity_key_with_account_id(
        is_local,
        host_id,
        session.provider,
        session.config_dir.as_deref(),
        session.account_email.as_deref(),
        session.account_id.as_deref(),
        &session.session_id,
    )
}

/// One terminal/PTY session in the four-level Conductor presentation. The
/// authoritative inventory remains the flat agent list on `ProjectNode`; this
/// borrowed projection supplies the missing Session container without copying
/// or independently mutating agent state.
#[derive(Clone, Debug)]
pub struct ConductorSession<'a> {
    /// Stable, host-scoped key for expansion state.
    pub key: String,
    /// Foreground agent when known, otherwise the most recently active child.
    pub representative: &'a SessionSnapshot,
    /// Waiting-first, then most-recent child agents.
    pub agents: Vec<&'a SessionSnapshot>,
    /// Aggregate state for the Session row.
    pub state: SessionState,
    /// Number of child agents waiting for the user.
    pub needs_me: usize,
}

fn aggregate_session_state(agents: &[&SessionSnapshot]) -> SessionState {
    if agents
        .iter()
        .any(|agent| agent.state == SessionState::Waiting)
    {
        SessionState::Waiting
    } else if agents
        .iter()
        .any(|agent| agent.state == SessionState::Monitor)
    {
        SessionState::Monitor
    } else if agents
        .iter()
        .any(|agent| agent.state == SessionState::Active)
    {
        SessionState::Active
    } else {
        SessionState::Idle
    }
}

/// Group a project's agent conversations into terminal/PTY sessions.
///
/// PTY id + generation is the authoritative container identity when the daemon
/// negotiated it. A conversation without PTY metadata gets its own fallback
/// container keyed by the complete agent identity, so unrelated conversations
/// can never collapse merely because they share a project or display label.
pub fn group_project_sessions<'a>(
    is_local: bool,
    host_id: Option<&str>,
    agents: &'a [SessionSnapshot],
) -> Vec<ConductorSession<'a>> {
    let mut grouped: BTreeMap<String, Vec<&SessionSnapshot>> = BTreeMap::new();
    for agent in agents {
        let key = match agent.pty_session_id.as_deref() {
            Some(pty_id) => format!(
                "{}\0pty\0{pty_id}\0{}",
                host_ident(is_local, host_id),
                agent
                    .pty_session_generation
                    .map(|generation| generation.to_string())
                    .unwrap_or_else(|| "legacy".to_string())
            ),
            None => format!("agent\0{}", session_key(is_local, host_id, agent)),
        };
        grouped.entry(key).or_default().push(agent);
    }

    let mut sessions: Vec<ConductorSession<'a>> = grouped
        .into_iter()
        .map(|(key, mut agents)| {
            agents.sort_by(|a, b| {
                (b.state == SessionState::Waiting)
                    .cmp(&(a.state == SessionState::Waiting))
                    .then_with(|| b.last_activity.cmp(&a.last_activity))
                    .then_with(|| {
                        session_key(is_local, host_id, a).cmp(&session_key(is_local, host_id, b))
                    })
            });
            let representative = agents
                .iter()
                .copied()
                .min_by(|a, b| {
                    b.pty_foreground
                        .cmp(&a.pty_foreground)
                        .then_with(|| b.last_activity.cmp(&a.last_activity))
                        .then_with(|| {
                            session_key(is_local, host_id, a)
                                .cmp(&session_key(is_local, host_id, b))
                        })
                })
                .expect("a grouped Conductor session always has an agent");
            let state = aggregate_session_state(&agents);
            let needs_me = agents
                .iter()
                .filter(|agent| agent.state == SessionState::Waiting)
                .count();
            ConductorSession {
                key,
                representative,
                agents,
                state,
                needs_me,
            }
        })
        .collect();
    sessions.sort_by(|a, b| {
        (b.state == SessionState::Waiting)
            .cmp(&(a.state == SessionState::Waiting))
            .then_with(|| {
                b.agents
                    .iter()
                    .map(|agent| &agent.last_activity)
                    .max()
                    .expect("a grouped Conductor session always has an agent")
                    .cmp(
                        a.agents
                            .iter()
                            .map(|agent| &agent.last_activity)
                            .max()
                            .expect("a grouped Conductor session always has an agent"),
                    )
            })
            .then_with(|| a.key.cmp(&b.key))
    });
    sessions
}

/// Inverse of [`host_key`]: split a key back into `(host_ident, id)`. The
/// `host_ident` is `"local"` for the local host or `"daemon:<id>"` for a remote.
/// Used to route a host-scoped favorite: a `"local"` key always launches locally
/// (durable), a remote key resolves against the live inventory. Returns `None`
/// for a string that is not a `host_key` (no separator).
pub fn split_host_key(key: &str) -> Option<(&str, &str)> {
    key.split_once('\u{0}')
}

/// Whether a [`host_key`] targets the local host.
pub fn host_key_is_local(key: &str) -> bool {
    matches!(split_host_key(key), Some(("local", _)))
}

/// A stable, host-identity-carrying pointer to one Waiting agent — the `w`-jump
/// target and cursor. It keeps the display `host_label` for the attach dispatch,
/// but **identity** is the stable `(is_local, host_id)` pair plus provider,
/// opaque/legacy account route and `session_id`, never the label. This also
/// keeps two local accounts carrying a copied conversation id distinct.
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
    /// Provider plus opaque (or legacy email/config) account identity complete
    /// the identity within a host: session ids can survive a copy between accounts.
    pub provider: Provider,
    pub account_email: Option<String>,
    pub account_id: Option<String>,
    /// Host-local launch route carried for the eventual attach dispatch and
    /// included in the exact routing identity.
    pub config_dir: Option<String>,
}

impl WaitingTarget {
    /// Same waiting agent? Compared by **stable** host identity `(is_local,
    /// host_id)` plus provider, account route, and `session_id` —
    /// never the display label.
    fn same_agent(&self, host: &HostNode, session: &SessionSnapshot) -> bool {
        session_key(host.is_local, host.host_id.as_deref(), session)
            == session_identity_key_with_account_id(
                self.is_local,
                self.host_id.as_deref(),
                self.provider,
                self.config_dir.as_deref(),
                self.account_email.as_deref(),
                self.account_id.as_deref(),
                &self.session_id,
            )
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
        .filter(|h| h.is_available())
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
/// identity `(is_local, host_id)` plus provider, account route,
/// and session id, never the display label. Host-label and account collisions
/// therefore stay distinct. `None` when nothing is waiting.
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
        provider: s.provider,
        account_email: s.account_email.clone(),
        account_id: s.account_id.clone(),
        config_dir: s.config_dir.clone(),
    })
}

#[cfg(test)]
#[path = "conductor_tests.rs"]
mod tests;
