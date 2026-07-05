//! Fleet aggregation + Conductor tree (audit (d)#6 + #8).
//!
//! Given the CLI sessions discovered on each host, build a **Host ▸ Project ▸
//! Session** tree in which the *needs-me* count — sessions in
//! [`SessionState::Waiting`], i.e. the agent handed control back to you — bubbles
//! up from session to project to host. This is the "conductor" leit-view over
//! the unified inventory: at a glance, which host/project is waiting on you.
//!
//! Pure aggregation (no IO, no remote calls): given per-host session lists it
//! yields the sorted tree. Fetching sessions cross-host (`list_sessions` over
//! the daemon) and rendering the tree build on top.

use crate::types::{SessionSnapshot, SessionState};
use std::collections::BTreeMap;

/// One host's contribution to the fleet: its label + the sessions found on it.
pub struct HostSessions {
    pub host: String,
    pub sessions: Vec<SessionSnapshot>,
}

/// A project (working directory) grouping within a host, with its own
/// needs-me tally.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectNode {
    /// The sessions' shared working directory (the grouping key).
    pub cwd: String,
    /// Short label for display (final path component of `cwd`).
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

fn project_name(cwd: &str) -> String {
    cwd.trim_end_matches('/')
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(cwd)
        .to_string()
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
            // Group sessions by cwd (stable order via BTreeMap on the key).
            let mut by_cwd: BTreeMap<String, Vec<SessionSnapshot>> = BTreeMap::new();
            for s in h.sessions {
                by_cwd.entry(s.cwd.clone()).or_default().push(s);
            }
            let mut projects: Vec<ProjectNode> = by_cwd
                .into_iter()
                .map(|(cwd, mut sessions)| {
                    // Waiting first, then most-recent activity.
                    sessions.sort_by(|a, b| {
                        is_waiting(b)
                            .cmp(&is_waiting(a))
                            .then_with(|| b.last_activity.cmp(&a.last_activity))
                    });
                    let needs_me = sessions.iter().filter(|s| is_waiting(s)).count();
                    let name = project_name(&cwd);
                    ProjectNode {
                        cwd,
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
