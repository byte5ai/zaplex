//! Fleet aggregation + Conductor tree (audit (d)#6 + #8).
//!
//! Given the CLI sessions discovered on each host, build a **Host ▸ Project ▸
//! AgentSession** tree — this IS the Agent-Inventory: the native hierarchy of
//! the cockpit. The *needs-me* count — sessions in [`SessionState::Waiting`],
//! i.e. the agent handed control back to you — bubbles up from session to
//! project to host. This is the "conductor" leit-view over the unified
//! inventory: at a glance, which host/project is waiting on you.
//!
//! Projects are keyed by **git root** ([`SessionSnapshot::project_root`]), so
//! sessions launched in different sub-directories of the same repo collapse
//! into one project node. Idle/Monitor/Active sessions never count as needs-me.
//!
//! Pure aggregation (no IO, no remote calls): given per-host session lists it
//! yields the sorted tree. Fetching sessions cross-host (`list_sessions` over
//! the daemon) and rendering the tree build on top.

use crate::types::{SessionSnapshot, SessionState};
use std::collections::BTreeMap;

/// An agent-session in the inventory. Alias for the snapshot the spine already
/// produces — the leaf of the Host ▸ Project ▸ AgentSession tree.
pub type AgentSession = SessionSnapshot;

/// One host's contribution to the fleet: its label + the sessions found on it.
pub struct HostSessions {
    pub host: String,
    pub sessions: Vec<SessionSnapshot>,
}

/// A project grouping within a host, with its own needs-me tally. Sessions are
/// grouped by their git **root** (see [`SessionSnapshot::project_root`]), so
/// nested sub-directory launches of one repo share a single node.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectNode {
    /// The sessions' shared git root — the grouping key.
    pub root: String,
    /// Human repo label (from [`SessionSnapshot::project_name`]).
    pub name: String,
    /// Count of [`SessionState::Waiting`] sessions in this project.
    pub needs_me: usize,
    pub sessions: Vec<SessionSnapshot>,
}

/// A host node with its projects and an aggregated needs-me tally.
#[derive(Debug, Clone, PartialEq)]
pub struct HostNode {
    pub host: String,
    /// Sum of the projects' needs-me counts.
    pub needs_me: usize,
    pub projects: Vec<ProjectNode>,
}

/// The whole fleet, with a grand needs-me total.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FleetTree {
    pub hosts: Vec<HostNode>,
    /// Sum of every host's needs-me count — the number the badge shows.
    pub needs_me: usize,
}

fn is_waiting(s: &SessionSnapshot) -> bool {
    matches!(s.state, SessionState::Waiting)
}

/// Build the Host ▸ Project ▸ Session tree with needs-me bubbling.
///
/// Ordering makes the things that want you rise to the top:
/// - hosts by needs-me **descending**, then host name;
/// - projects within a host by needs-me descending, then name;
/// - sessions within a project: **waiting first**, then most-recent activity.
///
/// Empty hosts (no sessions) are dropped — a conductor lists live work, not
/// idle machines (host discovery is a separate surface).
pub fn build_fleet_tree(inputs: Vec<HostSessions>) -> FleetTree {
    let mut hosts: Vec<HostNode> = inputs
        .into_iter()
        .filter(|h| !h.sessions.is_empty())
        .map(|h| {
            // Group sessions by git root (stable order via BTreeMap on the key).
            let mut by_root: BTreeMap<String, Vec<SessionSnapshot>> = BTreeMap::new();
            for s in h.sessions {
                by_root.entry(s.project_root.clone()).or_default().push(s);
            }
            let mut projects: Vec<ProjectNode> = by_root
                .into_iter()
                .map(|(root, mut sessions)| {
                    // Waiting first, then most-recent activity.
                    sessions.sort_by(|a, b| {
                        is_waiting(b)
                            .cmp(&is_waiting(a))
                            .then_with(|| b.last_activity.cmp(&a.last_activity))
                    });
                    let needs_me = sessions.iter().filter(|s| is_waiting(s)).count();
                    // All sessions in the group share a root → share the label.
                    let name = sessions
                        .first()
                        .map(|s| s.project_name.clone())
                        .unwrap_or_default();
                    ProjectNode {
                        root,
                        name,
                        needs_me,
                        sessions,
                    }
                })
                .collect();
            projects.sort_by(|a, b| {
                b.needs_me
                    .cmp(&a.needs_me)
                    .then_with(|| a.name.cmp(&b.name))
            });
            let needs_me = projects.iter().map(|p| p.needs_me).sum();
            HostNode {
                host: h.host,
                needs_me,
                projects,
            }
        })
        .collect();
    hosts.sort_by(|a, b| {
        b.needs_me
            .cmp(&a.needs_me)
            .then_with(|| a.host.cmp(&b.host))
    });
    let needs_me = hosts.iter().map(|h| h.needs_me).sum();
    FleetTree { hosts, needs_me }
}

#[cfg(test)]
#[path = "fleet_tests.rs"]
mod tests;
