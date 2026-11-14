//! Cockpit app integration: the `CockpitModel` singleton + file-watch/reconcile
//! wiring over the pure `zaplex_cockpit` data spine, plus scalar settings.
//!
//! Increment 1: data only (no UI). The account cards / heat bars / cost UI that
//! subscribe to `CockpitEvent::Updated` land in Increment 2 (`app/src/cockpit/…`).

pub mod accounts_panel;
pub mod ambient;
pub mod capabilities;
pub mod favorites;
pub(crate) mod fleet_details;
pub(crate) mod github_flow_dialog;
pub mod github_flows;
pub mod launch_registry;
pub(crate) mod localize;
pub mod model;
pub mod oauth;
pub mod palette;
pub mod pane;
pub mod panel;
pub mod reviewed;
pub(crate) mod session_lifecycle;
pub(crate) mod session_names;
pub mod settings;
pub mod style;
pub mod tailscale;
pub mod transcript_view;

pub use accounts_panel::CockpitAccountsPanel;
pub use ambient::AttentionDriver;
pub use model::CockpitModel;
pub use pane::CockpitPaneView;
pub use panel::CockpitPanel;
pub use settings::CockpitSettings;

use std::path::Path;

use zaplex_cockpit::{Account, Provider, SessionSnapshot};

use crate::terminal::cli_agent::CLIAgent;

/// Best-known reasoning **effort** for a Conductor session row (step 8), shared
/// by the pane and the sidebar so both render the same model·effort label.
///
/// Prefers the snapshot's own value (Codex records effort in its transcript),
/// else the exact launch record bound to `(host, provider, account, session-id)`.
/// Claude effort reaches no transcript (local *or* remote), so the Spawn-Karte's
/// launch record is its only source. Host identity is the daemon `host_id`; a
/// launch made before the daemon handshake is rehosted from SSH `node_id` once
/// that stable identity arrives. Externally started and pre-hook sessions retain
/// a marked compatibility path through `(agent, host, cwd)`. An account mismatch
/// fails closed and never uses that coordinate fallback. `None` is honestly
/// unknown, so the label omits effort rather than inventing it.
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

/// Placeholder for an unknown plan, so the plan slot never collapses and a
/// missing plan reads as unknown rather than as absent.
pub(crate) const UNKNOWN_PLAN: &str = "—";

/// One identity hierarchy shared by compact and large Cockpit account cards.
/// Provider is the headline; account and plan are two separate slots of one
/// quieter subordinate line, so an email used as the label is never rendered
/// twice and provider and plan never share a field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AccountIdentityPresentation {
    pub provider: Provider,
    pub provider_label: &'static str,
    /// The user's alias when set, else the email (or the discovery label).
    pub account: String,
    /// The provider's plan name, or [`UNKNOWN_PLAN`].
    pub plan: String,
    /// The email behind an alias, for the account slot's tooltip; `None` when
    /// the account slot already shows it.
    pub email_hint: Option<String>,
}

pub(crate) fn account_identity(account: &Account) -> AccountIdentityPresentation {
    let email = account
        .email
        .as_deref()
        .map(str::trim)
        .filter(|email| !email.is_empty());
    let label = account.label.trim();
    let account_value = if label.is_empty() {
        email.unwrap_or_default()
    } else {
        label
    };
    let plan = account
        .plan_tier
        .as_deref()
        .map(str::trim)
        .filter(|plan| !plan.is_empty())
        .unwrap_or(UNKNOWN_PLAN);
    AccountIdentityPresentation {
        provider: account.provider,
        provider_label: style::provider_label(account.provider),
        account: account_value.to_string(),
        plan: plan.to_string(),
        email_hint: email
            .filter(|email| *email != account_value)
            .map(str::to_string),
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
    match launch_registry::lookup_bound_session_with_account_id(
        agent,
        host,
        session.config_dir.as_deref().map(Path::new),
        session.account_email.as_deref(),
        session.account_id.as_deref(),
        &session.session_id,
    ) {
        launch_registry::BoundLaunchLookup::Match(record) => return record.effort,
        launch_registry::BoundLaunchLookup::AccountMismatch => return None,
        launch_registry::BoundLaunchLookup::Unbound => {}
    }
    launch_registry::lookup(agent, host, Some(Path::new(&session.cwd)))
        .and_then(|record| record.effort)
        .map(|effort| format!("~{effort}"))
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod session_effort_tests;
