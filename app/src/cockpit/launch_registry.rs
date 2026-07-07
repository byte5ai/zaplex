//! Launch-time (model, effort) registry — the honest half of "effort tracking".
//!
//! Effort is **not** present in any transcript (Claude Code has no effort CLI
//! flag at all, and Codex records only token counts), so the Conductor cannot
//! recover the effort a session was started with by parsing session files. The
//! Spawn-Karte is the only place that *knows* the chosen (model, effort), so it
//! records them here at launch, keyed by the launch coordinates it does know:
//! `(host, cwd, agent)`.
//!
//! ## Honest scope / the binding gap
//! This binds a launch to its **coordinates**, not yet to a concrete
//! agent-session-id: the provider mints the session id only *after* the CLI
//! starts, and we have no reliable, race-free way to read it back at launch
//! time. So a lookup here answers "the most recent launch for this host + cwd +
//! agent", which is correct for the overwhelmingly common case (one fresh agent
//! per project dir) but is a heuristic, not a guaranteed 1:1 binding. Step 8,
//! which surfaces model·effort·context on Conductor rows, should read via
//! [`lookup`] and treat a hit as "best-known launch intent", falling back to the
//! model parsed from the transcript when there is no record. When a robust
//! launch→session-id binding exists (e.g. the daemon echoing the new id back),
//! this registry gains a real key and the heuristic can be dropped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};

use crate::terminal::CLIAgent;

/// A single recorded launch intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRecord {
    pub agent: CLIAgent,
    /// `None` = local host.
    pub host: Option<String>,
    /// `None` = the launcher's default dir.
    pub cwd: Option<PathBuf>,
    /// The chosen model (`None` = provider default).
    pub model: Option<String>,
    /// The chosen thinking-effort (`None` = provider default). Recorded even for
    /// Claude, whose effort never reaches the command line.
    pub effort: Option<String>,
    pub launched_at: DateTime<Utc>,
}

/// Coordinates a launch is keyed by. Normalized so lookups match records.
type LaunchKey = (CLIAgent, Option<String>, Option<PathBuf>);

fn key(agent: CLIAgent, host: Option<&str>, cwd: Option<&Path>) -> LaunchKey {
    (agent, host.map(str::to_owned), cwd.map(Path::to_path_buf))
}

fn store() -> &'static Mutex<HashMap<LaunchKey, LaunchRecord>> {
    static STORE: OnceLock<Mutex<HashMap<LaunchKey, LaunchRecord>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
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
        model,
        effort,
        launched_at,
    };
    if let Ok(mut map) = store().lock() {
        map.insert(key(agent, host, cwd), record);
    }
}

/// Best-known launch intent for a session at these coordinates, or `None` when
/// nothing was recorded (e.g. a session started outside the Spawn-Karte). See
/// the module docs for why this is a heuristic, not a guaranteed binding.
pub fn lookup(agent: CLIAgent, host: Option<&str>, cwd: Option<&Path>) -> Option<LaunchRecord> {
    store().lock().ok()?.get(&key(agent, host, cwd)).cloned()
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
/// `(agent, cwd)` — a re-launch after reconnect — the migrated record replaces
/// it (the migrated one is at least as recent).
pub fn rehost(old_host: &str, new_host: &str) {
    if old_host == new_host {
        return;
    }
    if let Ok(mut map) = store().lock() {
        let moved: Vec<(LaunchKey, LaunchRecord)> = map
            .keys()
            .filter(|(_, host, _)| host.as_deref() == Some(old_host))
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|k| {
                map.remove(&k).map(|mut rec| {
                    rec.host = Some(new_host.to_owned());
                    ((k.0, Some(new_host.to_owned()), k.2), rec)
                })
            })
            .collect();
        for (new_key, rec) in moved {
            map.insert(new_key, rec);
        }
    }
}

#[cfg(test)]
#[path = "launch_registry_tests.rs"]
mod tests;
