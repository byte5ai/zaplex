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

use warpui::{AppContext, EntityId, SingletonEntity as _, ViewHandle};
use zaplex_cockpit::types::{SessionSnapshot, SessionState};

use crate::cockpit::agent_of;
use crate::cockpit::launch_registry::{self, BoundLaunchLookup};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::{CLIAgent, TerminalView};

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

/// How Zaplex can open one inventory row right now. This is the click path's
/// own decision ([`plan_session_open`] plus the remote daemon reattach), shared
/// with the Cockpit attention projection so that a row is only shown or counted
/// on the strength of the same evidence a click acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionReach {
    /// A Zaplex terminal hosts this exact agent session; opening focuses it.
    Terminal(EntityId),
    /// A remote live agent in a foreground daemon PTY on a connected daemon
    /// with a registry route; opening reattaches that PTY.
    DaemonReattach,
    /// Dormant; opening resumes it safely.
    Resume,
    /// Live, but Zaplex has no reliable pane or PTY locator for it.
    Unreachable,
}

impl SessionReach {
    pub fn is_openable(self) -> bool {
        !matches!(self, SessionReach::Unreachable)
    }

    pub fn terminal(self) -> Option<EntityId> {
        match self {
            SessionReach::Terminal(terminal_view_id) => Some(terminal_view_id),
            SessionReach::DaemonReattach | SessionReach::Resume | SessionReach::Unreachable => None,
        }
    }
}

/// Pure reachability decision from the facts the click path gathers: the
/// exact terminal hosting the session, if any, and whether the connected
/// daemon for a remote row offers a registry route for reattaching.
pub fn session_reach(
    session: &SessionSnapshot,
    is_local: bool,
    terminal_view_id: Option<EntityId>,
    daemon_route_available: bool,
) -> SessionReach {
    match plan_session_open(session, terminal_view_id.is_some()) {
        SessionOpenPlan::FocusExistingTerminal => terminal_view_id
            .map(SessionReach::Terminal)
            .unwrap_or(SessionReach::Unreachable),
        SessionOpenPlan::ResumeDormant => SessionReach::Resume,
        SessionOpenPlan::LiveSessionUnavailable => {
            if !is_local && daemon_route_available && daemon_reattach_target(session).is_some() {
                SessionReach::DaemonReattach
            } else {
                SessionReach::Unreachable
            }
        }
    }
}

/// Whether Zaplex launched or hosts this agent, so a momentarily unreachable
/// row (a reconnecting daemon, a PTY between foreground handovers, a pane
/// without an exact account binding) stays visible instead of flickering away.
/// Evidence: a daemon PTY binding — daemon PTYs exist only for Zaplex
/// terminals, so even a historical binding proves it — or an exact
/// launch-registry binding from the Spawn-Karte/hook bridge, a Zaplex terminal
/// whose hook reported this session id, or a local agent process that inherited
/// a Zaplex terminal's surface id (no hooks needed). Externally started agents
/// carry none of these.
pub fn zaplex_owned(session: &SessionSnapshot, zaplex_launch_or_terminal: bool) -> bool {
    session.pty_session_id.is_some() || zaplex_launch_or_terminal
}

/// The local Zaplex terminal whose PTY carries `surface_id` (the id injected as
/// `ZAPLEX_SURFACE_ID` into every local pane), in any window.
pub(crate) fn terminal_for_surface(surface_id: &str, ctx: &AppContext) -> Option<EntityId> {
    ctx.window_ids().find_map(|window_id| {
        ctx.views_of_type::<TerminalView>(window_id)
            .and_then(|terminal_views| {
                terminal_views.iter().find_map(|terminal_view| {
                    let terminal = terminal_view.as_ref(ctx);
                    terminal
                        .control_context()
                        .is_some_and(|context| context.surface_id() == surface_id)
                        .then(|| terminal.view_id())
                })
            })
    })
}

/// Launch-registry or terminal-hook evidence that Zaplex started or hosts
/// this row (see [`zaplex_owned`]).
pub(crate) fn zaplex_launch_or_terminal(
    session: &SessionSnapshot,
    is_local: bool,
    host_id: Option<&str>,
    ctx: &AppContext,
) -> bool {
    has_exact_launch_binding(session, is_local, host_id)
        || CLIAgentSessionsModel::as_ref(ctx).hosts_agent_session(
            agent_of(session.provider),
            &session.session_id,
            !is_local,
        )
}

/// Visibility rule for the Cockpit inventory: a row Zaplex can open, or one it
/// launched. An agent started outside Zaplex that can be neither focused,
/// reattached nor resumed is not shown at all. `origin_unknown` marks a local
/// agent whose process could not be inspected (platform, permissions, no
/// process found): such a row stays visible, because hiding is reserved for
/// positive evidence that the agent was started outside Zaplex.
pub fn session_visible(reach: SessionReach, zaplex_owned: bool, origin_unknown: bool) -> bool {
    reach.is_openable() || zaplex_owned || origin_unknown
}

/// Whether the launch registry holds an exact binding for this row, keyed the
/// same way the launch recorded it (`host = None` locally, the daemon's stable
/// `host_id` remotely).
pub fn has_exact_launch_binding(
    session: &SessionSnapshot,
    is_local: bool,
    host_id: Option<&str>,
) -> bool {
    let host = if is_local {
        None
    } else {
        match host_id {
            Some(host_id) => Some(host_id),
            None => return false,
        }
    };
    matches!(
        launch_registry::lookup_bound_session_with_account_id(
            agent_of(session.provider),
            host,
            session.config_dir.as_deref().map(std::path::Path::new),
            session.account_email.as_deref(),
            session.account_id.as_deref(),
            &session.session_id,
        ),
        BoundLaunchLookup::Match(_)
    )
}

/// Finds a terminal view by id in any Zaplex window.
pub(crate) fn terminal_view_handle(
    terminal_view_id: EntityId,
    ctx: &AppContext,
) -> Option<ViewHandle<TerminalView>> {
    ctx.window_ids().find_map(|window_id| {
        ctx.views_of_type::<TerminalView>(window_id)
            .and_then(|terminal_views| {
                terminal_views.iter().find_map(|terminal_view| {
                    (terminal_view.as_ref(ctx).view_id() == terminal_view_id)
                        .then(|| terminal_view.clone())
                })
            })
    })
}

/// Resolves a CLI conversation to the terminal on the exact fleet host.
/// Provider/session ids alone are insufficient: copied sessions may retain
/// the same id on local and multiple remote hosts.
#[allow(clippy::too_many_arguments)]
pub(crate) fn terminal_view_id_for_agent_session(
    agent: CLIAgent,
    session_id: &str,
    config_dir: Option<&str>,
    account_email: Option<&str>,
    account_id: Option<&str>,
    host_id: Option<&str>,
    is_local: bool,
    ctx: &AppContext,
) -> Option<EntityId> {
    CLIAgentSessionsModel::as_ref(ctx).terminal_view_id_for_agent_session_matching_with_id(
        agent,
        session_id,
        config_dir,
        account_email,
        account_id,
        |terminal_view_id, session| {
            if (is_local && session.is_remote()) || (!is_local && !session.is_remote()) {
                return false;
            }
            let Some(terminal_view) = terminal_view_handle(terminal_view_id, ctx) else {
                return false;
            };
            #[cfg(feature = "local_tty")]
            let remote_host_id = terminal_view.as_ref(ctx).active_session_remote_host_id(ctx);
            #[cfg(not(feature = "local_tty"))]
            let remote_host_id: Option<warp_core::HostId> = {
                let _ = terminal_view;
                None
            };
            session_host_matches(
                is_local,
                host_id,
                remote_host_id.as_ref().map(warp_core::HostId::as_str),
            )
        },
    )
}

/// The exact terminal hosting one inventory row, if any.
pub(crate) fn terminal_for_inventory_session(
    session: &SessionSnapshot,
    is_local: bool,
    host_id: Option<&str>,
    ctx: &AppContext,
) -> Option<EntityId> {
    terminal_view_id_for_agent_session(
        agent_of(session.provider),
        &session.session_id,
        session.config_dir.as_deref(),
        session.account_email.as_deref(),
        session.account_id.as_deref(),
        host_id,
        is_local,
        ctx,
    )
}

/// Resolves an SSH-registry node to the connection a daemon reattach uses.
#[cfg(unix)]
pub(crate) fn resolved_daemon_connection(
    node_id: &str,
) -> Option<warp_ssh_manager::ResolvedSshConnection> {
    warp_ssh_manager::with_conn(|database| {
        Ok(warp_ssh_manager::SshRepository::get_server_with_resolved_auth(database, node_id)?)
    })
    .ok()
    .flatten()
}

/// The route the click path reattaches a remote live agent through: the
/// connected daemon for `host_id` plus its resolved registry connection.
#[cfg(unix)]
pub(crate) fn daemon_reattach_route(
    host_id: Option<&str>,
    ctx: &AppContext,
) -> Option<(
    warp_ssh_manager::ResolvedSshConnection,
    Option<remote_server::transport::DaemonRuntimeRoute>,
)> {
    let host_id = host_id?;
    let daemon = crate::remote_server::manager::RemoteServerManager::as_ref(ctx)
        .connected_daemons()
        .into_iter()
        .find(|daemon| daemon.host_id == host_id)?;
    let connection = daemon
        .registry_node_id
        .as_deref()
        .and_then(resolved_daemon_connection)?;
    Some((connection, daemon.daemon_runtime))
}

/// Whether [`daemon_reattach_route`] currently resolves for `host_id`.
pub(crate) fn daemon_reattach_route_available(host_id: Option<&str>, ctx: &AppContext) -> bool {
    #[cfg(unix)]
    {
        daemon_reattach_route(host_id, ctx).is_some()
    }
    #[cfg(not(unix))]
    {
        let _ = (host_id, ctx);
        false
    }
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
            // Local signals require a process-bound platform primitive. Remote
            // sessions rely on the daemon that supplied the fingerprint and
            // perform this support check on that host.
            can_signal: zaplex_cockpit::pid_signalable(session.pid)
                && session.process_fingerprint.is_some()
                && (!is_local || zaplex_cockpit::local_process_signalling_supported()),
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
