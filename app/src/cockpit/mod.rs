//! Cockpit app integration: the `CockpitModel` singleton + file-watch/reconcile
//! wiring over the pure `zaplex_cockpit` data spine, plus scalar settings.
//!
//! Increment 1: data only (no UI). The account cards / heat bars / cost UI that
//! subscribe to `CockpitEvent::Updated` land in Increment 2 (`app/src/cockpit/…`).

pub mod ambient;
pub mod capabilities;
pub mod favorites;
pub mod github_flows;
pub mod launch_registry;
pub mod model;
pub mod oauth;
pub mod pane;
pub mod panel;
pub mod reviewed;
pub mod settings;
pub mod style;
pub mod tailscale;

pub use ambient::AttentionDriver;
pub use model::CockpitModel;
pub use pane::CockpitPaneView;
pub use panel::CockpitPanel;
pub use settings::CockpitSettings;

use std::path::Path;

use zaplex_cockpit::{Provider, SessionSnapshot};

use crate::terminal::cli_agent::CLIAgent;

/// Best-known reasoning **effort** for a Conductor session row (step 8), shared
/// by the pane and the sidebar so both render the same model·effort label.
///
/// Prefers the snapshot's own value (Codex records effort in its transcript),
/// else the launch registry's best-known intent for this session's launch
/// coordinates — Claude effort reaches no transcript (local *or* remote), so the
/// Spawn-Karte's launch record is its only source. The registry is keyed by the
/// launch's `(agent, host, cwd)`, where `host` is the **stable host identity**:
/// `None` for a local session, and the remote daemon's `host_id` for a remote
/// one. The launch records under that same `host_id` when the host is already
/// connected; when it is not, it records under the SSH `node_id` and migrates to
/// the `host_id` the moment the daemon connects (before the session can appear
/// in the inventory), so this lookup's `host_id` always matches the record — see
/// [`crate::workspace::view::Workspace::launch_routed_agent`] and
/// `rehost_launch_records_on_connect`. The `cwd` is likewise the *resolved*
/// launch dir (a local default-dir launch records `$HOME`, the dir the shell
/// actually starts in), so it equals the `session.cwd` reported here. Passing
/// the inventory node's `host_id` makes a remote Claude launch's effort resolve
/// instead of being dropped. `None` = honestly unknown; the label then omits the
/// effort rather than inventing one.
/// The CLI behind a discovered session's provider.
///
/// One mapping, so a caller cannot quietly disagree with another about which
/// binary a session belongs to — and so `CLIAgent` stays the single place that
/// knows what each CLI can do (see `capabilities`).
pub(crate) fn agent_of(provider: Provider) -> CLIAgent {
    match provider {
        Provider::Claude => CLIAgent::Claude,
        Provider::Codex => CLIAgent::Codex,
        Provider::Antigravity => CLIAgent::Antigravity,
    }
}

pub(crate) fn session_effort(
    session: &SessionSnapshot,
    is_local: bool,
    host_id: Option<&str>,
) -> Option<String> {
    if let Some(effort) = session.effort.clone() {
        return Some(effort);
    }
    let agent = agent_of(session.provider);
    // Local sessions are keyed with `host = None` (the launch recorded none);
    // remote sessions key on the daemon's stable `host_id`, the same identity
    // the launch resolved and stored. A remote node with no id (shouldn't
    // happen — the fold always sets it) yields the honest `None`.
    let host = if is_local { None } else { Some(host_id?) };
    launch_registry::lookup(agent, host, Some(Path::new(&session.cwd)))
        .and_then(|record| record.effort)
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod session_effort_tests;
