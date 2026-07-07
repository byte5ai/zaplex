//! Cockpit app integration: the `CockpitModel` singleton + file-watch/reconcile
//! wiring over the pure `zaplex_cockpit` data spine, plus scalar settings.
//!
//! Increment 1: data only (no UI). The account cards / heat bars / cost UI that
//! subscribe to `CockpitEvent::Updated` land in Increment 2 (`app/src/cockpit/…`).

pub mod ambient;
pub mod github_flows;
pub mod launch_registry;
pub mod model;
pub mod oauth;
pub mod pane;
pub mod tailscale;
pub mod panel;
pub mod settings;

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
/// else — for a **local** session only — the launch registry's best-known intent
/// for this `(agent, local host, cwd)` (Claude effort reaches no transcript, so
/// the Spawn-Karte's launch record is its only source). `None` = honestly
/// unknown; the label then omits the effort rather than inventing one. Remote
/// sessions use only what the wire snapshot carried.
pub(crate) fn session_effort(session: &SessionSnapshot, is_local: bool) -> Option<String> {
    if let Some(effort) = session.effort.clone() {
        return Some(effort);
    }
    if !is_local {
        return None;
    }
    let agent = match session.provider {
        Provider::Claude => CLIAgent::Claude,
        Provider::Codex => CLIAgent::Codex,
    };
    launch_registry::lookup(agent, None, Some(Path::new(&session.cwd)))
        .and_then(|record| record.effort)
}
