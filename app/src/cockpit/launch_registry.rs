//! Launch-time (model, effort) registry — the honest half of "effort tracking".
//!
//! Effort is **not** present in any transcript (Claude Code has no effort CLI
//! flag at all, and Codex records only token counts), so the Conductor cannot
//! recover the effort a session was started with by parsing session files. The
//! Spawn-Karte is the only place that *knows* the chosen (model, effort), so it
//! records them here at launch. A stable launch id is created before the
//! provider process can start, associated with its terminal once that transport
//! exists, then promoted to an exact `(agent, host, account, session-id)`
//! binding as soon as the native hook bridge reports the provider session id.
//!
//! [`lookup`] remains as a compatibility fallback for externally started or
//! pre-hook sessions. Exact lookup is account-aware and fails closed when the
//! session id exists under another account route, so credentials and launch
//! intent cannot silently bleed between accounts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};
use warpui::EntityId;

use crate::terminal::CLIAgent;

const MAX_RETAINED_LAUNCH_RECORDS: usize = 4_096;

/// A single recorded launch intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRecord {
    pub agent: CLIAgent,
    /// `None` = local host.
    pub host: Option<String>,
    /// `None` = the launcher's default dir.
    pub cwd: Option<PathBuf>,
    /// Provider-specific configuration root selected for this launch.
    pub config_dir: Option<String>,
    /// Provider account selected for this launch.
    pub account_email: Option<String>,
    /// Opaque daemon-local account identity for routing-capable remote peers.
    pub account_id: Option<String>,
    /// The chosen model (`None` = provider default).
    pub model: Option<String>,
    /// The chosen thinking-effort (`None` = provider default). Recorded even for
    /// Claude, whose effort never reaches the command line.
    pub effort: Option<String>,
    pub launched_at: DateTime<Utc>,
}

/// Coordinates a launch is keyed by. Normalized so lookups match records.
type LaunchKey = (CLIAgent, Option<String>, Option<PathBuf>);

/// Stable provider identity for an exact, post-hook binding.
type SessionKey = (
    CLIAgent,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

/// Process-local identity assigned before a provider process can start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LaunchId(u64);

impl LaunchId {
    fn next() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(1);
        Self(NEXT_ID.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Default)]
struct LaunchStore {
    coordinates: HashMap<LaunchKey, LaunchRecord>,
    launches: HashMap<LaunchId, LaunchRecord>,
    terminal_launches: HashMap<EntityId, LaunchId>,
    observed_sessions: HashMap<EntityId, String>,
    bound_terminals: HashMap<EntityId, String>,
    sessions: HashMap<SessionKey, LaunchRecord>,
}

/// Result of resolving an exact launch binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoundLaunchLookup {
    Match(LaunchRecord),
    /// The provider session id is known, but only for a different account.
    AccountMismatch,
    Unbound,
}

fn key(agent: CLIAgent, host: Option<&str>, cwd: Option<&Path>) -> LaunchKey {
    (agent, host.map(str::to_owned), cwd.map(Path::to_path_buf))
}

fn session_key(record: &LaunchRecord, session_id: &str) -> SessionKey {
    (
        record.agent,
        record.host.clone(),
        record.config_dir.clone(),
        record.account_email.clone(),
        record.account_id.clone(),
        session_id.to_owned(),
    )
}

fn store() -> &'static Mutex<LaunchStore> {
    static STORE: OnceLock<Mutex<LaunchStore>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(LaunchStore::default()))
}

fn evict_oldest_coordinate(store: &mut LaunchStore) {
    while store.coordinates.len() > MAX_RETAINED_LAUNCH_RECORDS {
        let Some(oldest) = store
            .coordinates
            .iter()
            .min_by_key(|(_, record)| record.launched_at)
            .map(|(key, _)| key.clone())
        else {
            return;
        };
        store.coordinates.remove(&oldest);
    }
}

fn evict_oldest_pending_launch(store: &mut LaunchStore) {
    while store.launches.len() > MAX_RETAINED_LAUNCH_RECORDS {
        let Some(oldest) = store
            .launches
            .iter()
            .min_by_key(|(_, record)| record.launched_at)
            .map(|(launch_id, _)| *launch_id)
        else {
            return;
        };
        store.launches.remove(&oldest);
        store
            .terminal_launches
            .retain(|_, launch_id| *launch_id != oldest);
    }
}

fn evict_oldest_bound_session(store: &mut LaunchStore) {
    while store.sessions.len() > MAX_RETAINED_LAUNCH_RECORDS {
        let Some(oldest) = store
            .sessions
            .iter()
            .min_by_key(|(_, record)| record.launched_at)
            .map(|(key, _)| key.clone())
        else {
            return;
        };
        store.sessions.remove(&oldest);
    }
}

/// Record the (model, effort) an agent was launched with. Called by the launch
/// path right before the routed command is executed. The newest record for a
/// given `(agent, host, cwd)` wins (a re-launch supersedes the prior intent).
pub fn record(
    agent: CLIAgent,
    host: Option<&str>,
    cwd: Option<&Path>,
    model: Option<String>,
    effort: Option<String>,
) {
    record_at(agent, host, cwd, model, effort, Utc::now());
}

/// [`record`] with an explicit timestamp, for deterministic tests.
pub fn record_at(
    agent: CLIAgent,
    host: Option<&str>,
    cwd: Option<&Path>,
    model: Option<String>,
    effort: Option<String>,
    launched_at: DateTime<Utc>,
) {
    let record = LaunchRecord {
        agent,
        host: host.map(str::to_owned),
        cwd: cwd.map(Path::to_path_buf),
        config_dir: None,
        account_email: None,
        account_id: None,
        model,
        effort,
        launched_at,
    };
    if let Ok(mut store) = store().lock() {
        store.coordinates.insert(key(agent, host, cwd), record);
        evict_oldest_coordinate(&mut store);
    }
}

/// Associate launch intent with the terminal that will execute it. The hook
/// bridge later calls [`bind_terminal_session`] once the provider session id is
/// known. A coordinate entry is also retained for sessions without hook data.
#[allow(clippy::too_many_arguments)]
pub fn record_for_terminal(
    terminal_view_id: EntityId,
    agent: CLIAgent,
    host: Option<&str>,
    cwd: Option<&Path>,
    config_dir: Option<&Path>,
    account_email: Option<&str>,
    model: Option<String>,
    effort: Option<String>,
) {
    let launch_id = begin_launch(agent, host, cwd, config_dir, account_email, model, effort);
    attach_terminal(launch_id, terminal_view_id);
}

/// Create a launch intent before any provider command can execute.
#[allow(clippy::too_many_arguments)]
pub fn begin_launch(
    agent: CLIAgent,
    host: Option<&str>,
    cwd: Option<&Path>,
    config_dir: Option<&Path>,
    account_email: Option<&str>,
    model: Option<String>,
    effort: Option<String>,
) -> LaunchId {
    begin_launch_with_account_id(
        agent,
        host,
        cwd,
        config_dir,
        account_email,
        None,
        model,
        effort,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn begin_launch_with_account_id(
    agent: CLIAgent,
    host: Option<&str>,
    cwd: Option<&Path>,
    config_dir: Option<&Path>,
    account_email: Option<&str>,
    account_id: Option<&str>,
    model: Option<String>,
    effort: Option<String>,
) -> LaunchId {
    begin_launch_at_with_account_id(
        agent,
        host,
        cwd,
        config_dir,
        account_email,
        account_id,
        model,
        effort,
        Utc::now(),
    )
}

#[allow(clippy::too_many_arguments)]
fn begin_launch_at(
    agent: CLIAgent,
    host: Option<&str>,
    cwd: Option<&Path>,
    config_dir: Option<&Path>,
    account_email: Option<&str>,
    model: Option<String>,
    effort: Option<String>,
    launched_at: DateTime<Utc>,
) -> LaunchId {
    begin_launch_at_with_account_id(
        agent,
        host,
        cwd,
        config_dir,
        account_email,
        None,
        model,
        effort,
        launched_at,
    )
}

#[allow(clippy::too_many_arguments)]
fn begin_launch_at_with_account_id(
    agent: CLIAgent,
    host: Option<&str>,
    cwd: Option<&Path>,
    config_dir: Option<&Path>,
    account_email: Option<&str>,
    account_id: Option<&str>,
    model: Option<String>,
    effort: Option<String>,
    launched_at: DateTime<Utc>,
) -> LaunchId {
    let launch_id = LaunchId::next();
    let record = LaunchRecord {
        agent,
        host: host.map(str::to_owned),
        cwd: cwd.map(Path::to_path_buf),
        config_dir: account_id
            .is_none()
            .then(|| config_dir.map(|path| path.to_string_lossy().into_owned()))
            .flatten(),
        account_email: account_email.map(str::to_owned),
        account_id: account_id.map(str::to_owned),
        model,
        effort,
        launched_at,
    };
    if let Ok(mut store) = store().lock() {
        store.launches.insert(launch_id, record);
        evict_oldest_pending_launch(&mut store);
    }
    launch_id
}

fn promote_terminal_binding(store: &mut LaunchStore, terminal_view_id: EntityId) -> bool {
    let Some(session_id) = store.observed_sessions.remove(&terminal_view_id) else {
        return false;
    };
    let Some(launch_id) = store.terminal_launches.get(&terminal_view_id).copied() else {
        store.observed_sessions.insert(terminal_view_id, session_id);
        return false;
    };
    let Some(record) = store.launches.remove(&launch_id) else {
        return false;
    };
    store.terminal_launches.remove(&terminal_view_id);
    let coordinate_key = key(record.agent, record.host.as_deref(), record.cwd.as_deref());
    if store.coordinates.get(&coordinate_key) == Some(&record) {
        store.coordinates.remove(&coordinate_key);
    }
    let exact_key = session_key(&record, &session_id);
    let should_replace = store
        .sessions
        .get(&exact_key)
        .is_none_or(|existing| record.launched_at > existing.launched_at);
    if should_replace {
        store.sessions.insert(exact_key, record);
        evict_oldest_bound_session(store);
    }
    store.bound_terminals.insert(terminal_view_id, session_id);
    true
}

/// Attach a pre-created launch identity to its transport terminal. If the hook
/// event arrived first, this immediately completes the exact binding.
pub fn attach_terminal(launch_id: LaunchId, terminal_view_id: EntityId) -> bool {
    let Ok(mut store) = store().lock() else {
        return false;
    };
    let Some(record) = store.launches.get(&launch_id).cloned() else {
        return false;
    };
    store.coordinates.insert(
        key(record.agent, record.host.as_deref(), record.cwd.as_deref()),
        record,
    );
    store.terminal_launches.insert(terminal_view_id, launch_id);
    evict_oldest_coordinate(&mut store);
    promote_terminal_binding(&mut store, terminal_view_id);
    true
}

/// Clear only the provider-session relation for a reusable terminal. A pending
/// launch intent remains attached so an `Ended` event emitted while replacing
/// the old session cannot erase the new launch before its `Started` event.
pub fn clear_terminal_session_binding(terminal_view_id: EntityId) {
    if let Ok(mut store) = store().lock() {
        store.observed_sessions.remove(&terminal_view_id);
        store.bound_terminals.remove(&terminal_view_id);
    }
}

/// Forget all transport-scoped state when a terminal no longer hosts an agent.
/// Exact session records remain available for dormant history until bounded LRU
/// eviction, but no stale terminal relation can block a later reuse.
pub fn forget_terminal(terminal_view_id: EntityId) {
    if let Ok(mut store) = store().lock() {
        store.observed_sessions.remove(&terminal_view_id);
        store.bound_terminals.remove(&terminal_view_id);
        if let Some(launch_id) = store.terminal_launches.remove(&terminal_view_id) {
            if let Some(record) = store.launches.remove(&launch_id) {
                let coordinate_key =
                    key(record.agent, record.host.as_deref(), record.cwd.as_deref());
                if store.coordinates.get(&coordinate_key) == Some(&record) {
                    store.coordinates.remove(&coordinate_key);
                }
            }
        }
    }
}

/// Promote a terminal-scoped launch intent to an exact provider session id.
/// Returns `false` when there is no pending intent or the id is empty.
pub fn bind_terminal_session(terminal_view_id: EntityId, session_id: &str) -> bool {
    if session_id.is_empty() {
        return false;
    }
    let Ok(mut store) = store().lock() else {
        return false;
    };
    if let Some(bound_session_id) = store.bound_terminals.get(&terminal_view_id) {
        return bound_session_id == session_id;
    }
    store
        .observed_sessions
        .insert(terminal_view_id, session_id.to_owned());
    promote_terminal_binding(&mut store, terminal_view_id)
}

/// Resolve a launch by provider session id and its complete account route.
pub fn lookup_bound_session(
    agent: CLIAgent,
    host: Option<&str>,
    config_dir: Option<&Path>,
    account_email: Option<&str>,
    session_id: &str,
) -> BoundLaunchLookup {
    lookup_bound_session_with_account_id(agent, host, config_dir, account_email, None, session_id)
}

pub fn lookup_bound_session_with_account_id(
    agent: CLIAgent,
    host: Option<&str>,
    config_dir: Option<&Path>,
    account_email: Option<&str>,
    account_id: Option<&str>,
    session_id: &str,
) -> BoundLaunchLookup {
    let Ok(store) = store().lock() else {
        return BoundLaunchLookup::Unbound;
    };
    let config_dir = account_id
        .is_none()
        .then(|| config_dir.map(|path| path.to_string_lossy().into_owned()))
        .flatten();
    let exact_key = (
        agent,
        host.map(str::to_owned),
        config_dir,
        account_email.map(str::to_owned),
        account_id.map(str::to_owned),
        session_id.to_owned(),
    );
    if let Some(record) = store.sessions.get(&exact_key) {
        return BoundLaunchLookup::Match(record.clone());
    }
    if store
        .sessions
        .keys()
        .any(|(bound_agent, bound_host, _, _, _, bound_session_id)| {
            *bound_agent == agent && bound_host.as_deref() == host && bound_session_id == session_id
        })
    {
        return BoundLaunchLookup::AccountMismatch;
    }
    BoundLaunchLookup::Unbound
}

/// Best-known launch intent for a session at these coordinates, or `None` when
/// nothing was recorded (e.g. a session started outside the Spawn-Karte). See
/// the module docs for why this is a heuristic, not a guaranteed binding.
pub fn lookup(agent: CLIAgent, host: Option<&str>, cwd: Option<&Path>) -> Option<LaunchRecord> {
    store()
        .lock()
        .ok()?
        .coordinates
        .get(&key(agent, host, cwd))
        .cloned()
}

/// Migrate every record keyed by host `old_host` over to host `new_host`,
/// preserving the `(agent, cwd)` half of each key.
///
/// ## Why this exists — the launch/lookup id-space bridge
/// A remote launch keys its record by the SSH `node_id` it knows at launch. The
/// Conductor inventory, however, keys hosts by the daemon's stable `host_id`,
/// which is only learned once the daemon finishes its initialize handshake —
/// often *after* the launch fired (the spawn card can target a host that has no
/// live daemon yet). Until then the record sits under `node_id` while
/// [`crate::cockpit::session_effort`] looks it up under `host_id`, so the effort
/// would be dropped. When the daemon connects and its `host_id` becomes known,
/// the workspace calls this to move the record onto the same `host_id` the
/// inventory uses, so record coordinates == lookup coordinates from then on. The
/// inventory only surfaces a remote session once its daemon connects, so no
/// lookup runs against the pre-migration `node_id` key before this fires.
///
/// A no-op when `old_host == new_host` (an already-connected host recorded under
/// `host_id` directly). If a `new_host` key already exists for a given
/// `(agent, cwd)` — a re-launch after reconnect — the newer timestamp wins.
pub fn rehost(old_host: &str, new_host: &str) {
    if old_host == new_host {
        return;
    }
    if let Ok(mut store) = store().lock() {
        let moved: Vec<(LaunchKey, LaunchRecord)> = store
            .coordinates
            .keys()
            .filter(|(_, host, _)| host.as_deref() == Some(old_host))
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|k| {
                store.coordinates.remove(&k).map(|mut rec| {
                    rec.host = Some(new_host.to_owned());
                    ((k.0, Some(new_host.to_owned()), k.2), rec)
                })
            })
            .collect();
        for (new_key, rec) in moved {
            let should_replace = match store.coordinates.get(&new_key) {
                Some(existing) => rec.launched_at > existing.launched_at,
                None => true,
            };
            if should_replace {
                store.coordinates.insert(new_key, rec);
            }
        }
        for record in store.launches.values_mut() {
            if record.host.as_deref() == Some(old_host) {
                record.host = Some(new_host.to_owned());
            }
        }
        let moved_sessions = store
            .sessions
            .keys()
            .filter(|(_, host, _, _, _, _)| host.as_deref() == Some(old_host))
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|old_key| {
                store.sessions.remove(&old_key).map(|mut record| {
                    record.host = Some(new_host.to_owned());
                    (session_key(&record, &old_key.5), record)
                })
            })
            .collect::<Vec<_>>();
        for (new_key, record) in moved_sessions {
            let should_replace = match store.sessions.get(&new_key) {
                Some(existing) => record.launched_at > existing.launched_at,
                None => true,
            };
            if should_replace {
                store.sessions.insert(new_key, record);
            }
        }
    }
}

#[cfg(test)]
#[path = "launch_registry_tests.rs"]
mod tests;
