//! Fleet aggregation + Conductor tree (audit (d)#6 + #8).
//!
//! Given the CLI sessions discovered on each host, build a **Host ▸ Project ▸
//! AgentSession** tree — this IS the Agent-Inventory: the native hierarchy of
//! the cockpit. The *needs-me* count — sessions in [`SessionState::Waiting`],
//! i.e. the agent handed control back to you — bubbles up from session to
//! project to host. This is the "conductor" leit-view over the unified
//! inventory: at a glance, which host/project is waiting on you.
//!
//! Projects are keyed by the **repo** ([`SessionSnapshot::repo_root`]), so
//! sessions launched in different sub-directories — or in different linked
//! worktrees — of the same repo collapse into one project node. Each session
//! keeps its own tree in `project_root`; the worktree is an attribute of the
//! session, not a project of its own. Idle/Monitor/Active never count as needs-me.
//!
//! Pure aggregation (no IO, no remote calls): given per-host session lists it
//! yields the sorted tree. Fetching sessions cross-host (`list_sessions` over
//! the daemon) and rendering the tree build on top.

use crate::types::{SessionSnapshot, SessionState};
use std::collections::{BTreeMap, HashMap};

/// An agent-session in the inventory. Alias for the snapshot the spine already
/// produces — the leaf of the Host ▸ Project ▸ AgentSession tree.
pub type AgentSession = SessionSnapshot;

/// One host's contribution to the fleet: its label + the sessions found on it,
/// plus an explicit **local/remote** marker. `is_local` is authoritative — it
/// records which contribution came from *this* machine vs. a remote daemon at
/// fold time, so downstream routing (guardrail signals, attach) never has to
/// re-derive locality by comparing the display label against the local
/// hostname. That label comparison is unsafe: a remote daemon whose label
/// happens to equal the local hostname (SSH alias / matching `gethostname()`)
/// would be misclassified as local, sending a host-local `pid` signal to the
/// wrong machine.
pub struct HostSessions {
    pub host: String,
    /// `true` iff these sessions live on this machine (local `libc::kill`
    /// applies); `false` for every remote daemon's contribution.
    pub is_local: bool,
    /// Stable per-daemon host identity ([`crate::fleet::RemoteHost::host_id`]),
    /// `None` for the local contribution and `Some(daemon host id)` for each
    /// remote. Distinct from the display `host` label: two remote daemons can
    /// share a label (SSH alias / matching `gethostname()`), but never a
    /// `host_id`. Downstream guardrail routing resolves the target daemon by
    /// this id, never by the label, so a label collision can't send a
    /// host-local `pid` signal to the wrong machine.
    pub host_id: Option<String>,
    pub sessions: Vec<SessionSnapshot>,
}

/// A remote daemon's contribution to the fold: its display label plus the
/// stable per-daemon `host_id` the connection is keyed by. Carrying both keeps
/// the label for display while routing (guardrails / attach) by the id, so two
/// remote daemons that advertise the same label stay distinct and never
/// misroute a signal to each other's host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteHost {
    /// Human host label (SSH host name) — for display only.
    pub label: String,
    /// Stable, opaque per-daemon id (from the daemon's `InitializeResponse`).
    /// Unique per connected host even when labels collide.
    pub host_id: String,
}

/// A project grouping within a host, with its own needs-me tally. Sessions are
/// grouped by their **repo** (see [`SessionSnapshot::repo_root`]), so nested
/// sub-directory launches and parallel worktrees of one repo share a single node.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectNode {
    /// The sessions' shared **repo** root — the grouping key. For sessions in
    /// linked worktrees this is the main checkout, which none of them sits in;
    /// each session keeps its own tree in [`SessionSnapshot::project_root`], and
    /// anything acting on a session's files must use that.
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
    /// Whether this host is *this* machine. Carried explicitly from the fold
    /// (never re-derived from `host` label equality with the local hostname),
    /// so guardrail Stop/Kill routing and attach can trust it: `true` → local
    /// `libc::kill` / adopt-in-place; `false` → route over the daemon. A remote
    /// daemon whose label collides with the local hostname stays `false`.
    pub is_local: bool,
    /// Stable per-daemon host identity: `None` for the local host, `Some(daemon
    /// host id)` for a remote. Guardrail Stop/Kill and attach resolve the target
    /// [`ConnectedDaemon`](../../app/src/remote_server/manager.rs) by **this id**,
    /// not by the `host` label — so two remote daemons sharing a label (SSH
    /// alias / matching hostname) route to their own machine, never to each
    /// other's. The label is kept for display only.
    pub host_id: Option<String>,
    /// The SSH-registry `node.id` for this host, when it maps to a registered
    /// SSH host. Set for registry-only hosts merged into the tree (so the
    /// Conductor is the full host navigator — every registered host is a root,
    /// even with no live agent) and lets a host row act (open a terminal / scope
    /// a launch / manage) via the registry. `None` for the local host and for a
    /// daemon host not backed by a registry entry.
    pub registry_node_id: Option<String>,
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
            // Group by the REPO, not by each session's own tree (stable order
            // via BTreeMap on the key). Three worktrees of one repo are one
            // project with three sessions — they share a history and a name, and
            // splitting them into three headers said the opposite.
            //
            // A producer older than `repo_root` sends it empty; such a session
            // falls back to grouping by its own tree, i.e. exactly the old
            // per-worktree split. Degraded, never mis-grouped.
            let mut by_root: BTreeMap<String, Vec<SessionSnapshot>> = BTreeMap::new();
            for s in h.sessions {
                let key = if s.repo_root.is_empty() {
                    s.project_root.clone()
                } else {
                    s.repo_root.clone()
                };
                by_root.entry(key).or_default().push(s);
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
                    // All sessions in the group share a repo → share its label.
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
                is_local: h.is_local,
                host_id: h.host_id,
                // Session-derived hosts don't carry a registry id here; the app
                // layer sets it (and merges registry-only hosts) — see
                // `CockpitModel` inventory merge.
                registry_node_id: None,
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

/// Fold this machine's local sessions plus every connected daemon's
/// contribution into a single cross-host [`FleetTree`] — the unified
/// Agent-Inventory the Conductor renders and the attention ambient-bit reads.
///
/// `local_label` names the local host (the machine hostname, or `"local"` when
/// that is unavailable); `local` are its sessions. `remotes` is one
/// `(RemoteHost, sessions)` entry per connected daemon that advertised the
/// agent-inventory capability — a daemon without it (or one that errored)
/// simply contributes no entry, so a single unreachable host never fails the
/// whole fold. Each [`RemoteHost`] carries the daemon's display label **and**
/// its stable `host_id`, so guardrail routing can resolve the exact daemon by
/// id even when two remotes advertise the same label.
///
/// **Host namespacing.** Every host — local and each remote — becomes its own
/// [`HostSessions`], so two hosts that happen to share an absolute path (e.g.
/// `/home/me/proj` on both `local` and `devhost`) land in *separate* host
/// nodes and never collapse into one project. [`build_fleet_tree`] groups
/// sessions by `project_root` **within** a host only; identity is therefore
/// `(host, session_id)`, and a host-local `pid` is never assumed globally
/// unique. Empty hosts are dropped by `build_fleet_tree` (a host with no
/// sessions is not listed), so `fold_inventory` with an empty `remotes` list
/// yields exactly the local tree.
///
/// Pure — no IO, no remote calls. The live fetch that produces `remotes` lives
/// in the app's `CockpitModel`.
/// Merge registered SSH hosts into the tree as roots so the Conductor is the
/// FULL host navigator — every registered host appears even with no live agent.
/// `registered` is `(node_id, display_name)` pairs from the SSH registry
/// (`NodeKind::Server`). The display label is the only bridge between the SSH
/// registry and a live daemon host (which is namespaced by its daemon `HostId`,
/// not the registry `node_id`) — so this function is careful about **when that
/// bridge is trustworthy**:
///
/// - **Unambiguous label** (exactly one registry entry *and* exactly one live
///   host carry it) **and that host is still unbound**: the live host keeps its
///   sessions/`needs_me` and is **back-filled** with the registry `node_id`, so a
///   registered host that happens to have live agents can still be opened /
///   favorited / managed (it would otherwise carry `registry_node_id: None` and
///   lose those actions). A host already holding some *other* entry's id is left
///   alone and this entry is appended instead — re-pointing it would be the very
///   mis-binding this function exists to avoid.
/// - **Ambiguous label**, or no live host with it: the entry is appended as its
///   own root carrying its own `node_id` (`needs_me: 0`, so it sorts after the
///   active hosts). Never bound by guesswork — a wrong binding would point the
///   host row's open/manage/★ at someone else's SSH entry.
///
/// Consequence worth knowing: two registry entries sharing a label appear as two
/// identical-looking roots. That is honest — the data genuinely cannot tell them
/// apart — and strictly better than the previous behaviour, which showed only the
/// first and silently dropped the rest.
///
/// `needs_me` totals are unchanged (appended hosts contribute zero).
///
/// Idempotent: an entry already present in the tree (bound or standing as its own
/// root) is skipped, so merging twice yields the same tree as merging once.
pub fn merge_registered_hosts(tree: &mut FleetTree, registered: &[(String, String)]) {
    // How often each label occurs on either side. A label shared by several
    // registry entries — or by several live hosts — is **not a usable bridge**:
    // nothing in the data says which registry entry a given daemon belongs to.
    // Binding the first arbitrary match would point the host row's
    // open/manage/★ at the WRONG SSH entry, so ambiguous labels are never bound.
    let mut registry_uses: HashMap<&str, usize> = HashMap::new();
    for (_, name) in registered {
        *registry_uses.entry(name.as_str()).or_insert(0) += 1;
    }
    // Counted before the loop pushes anything, so appended registry-only roots
    // can't be mistaken for live hosts by a later iteration.
    let mut live_uses: HashMap<String, usize> = HashMap::new();
    for h in tree.hosts.iter() {
        *live_uses.entry(h.host.clone()).or_insert(0) += 1;
    }

    for (node_id, name) in registered {
        // Idempotence: this exact entry is already somewhere in the tree, bound to
        // a live host or standing as its own root. A previous merge placed it, so
        // re-running must leave it alone rather than append a second copy.
        if tree
            .hosts
            .iter()
            .any(|h| h.registry_node_id.as_deref() == Some(node_id.as_str()))
        {
            continue;
        }

        let unambiguous = registry_uses.get(name.as_str()).copied().unwrap_or(0) == 1
            && live_uses.get(name.as_str()).copied().unwrap_or(0) == 1;
        if unambiguous {
            // Exactly one registry entry and exactly one live host share this
            // label: the bridge is sound. Give the live host its registry identity
            // so host-row actions work. Only an unbound host is a candidate — one
            // that already carries a different entry's id must not be re-pointed.
            if let Some(existing) = tree
                .hosts
                .iter_mut()
                .find(|h| &h.host == name && h.registry_node_id.is_none())
            {
                existing.registry_node_id = Some(node_id.clone());
                continue;
            }
        }
        // Either the label is ambiguous, or there is no live host to bind to.
        // Append this entry as its own root: it keeps its OWN node_id (so its row
        // acts on the right SSH entry) and stays visible.
        //
        // This is what the previous implementation got wrong: it looked up the
        // label without the `registry_node_id.is_none()` guard, so a second
        // registry entry with the same name found the root the FIRST one had just
        // created, saw it was already bound, and `continue`d — silently dropping
        // that host from the Conductor entirely.
        tree.hosts.push(HostNode {
            host: name.clone(),
            is_local: false,
            host_id: None,
            registry_node_id: Some(node_id.clone()),
            needs_me: 0,
            projects: Vec::new(),
        });
    }
}

pub fn fold_inventory(
    local_label: impl Into<String>,
    local: Vec<SessionSnapshot>,
    remotes: Vec<(RemoteHost, Vec<SessionSnapshot>)>,
) -> FleetTree {
    let mut inputs = Vec::with_capacity(1 + remotes.len());
    // The local contribution is the ONLY one marked local — this is where the
    // authoritative local/remote bit is set. Every remote daemon's entry is
    // `is_local: false`, even if its label happens to equal `local_label`, so a
    // label collision can never route a signal to the wrong machine. The local
    // node carries no `host_id` (routing uses `is_local` for it).
    inputs.push(HostSessions {
        host: local_label.into(),
        is_local: true,
        host_id: None,
        sessions: local,
    });
    for (remote, sessions) in remotes {
        inputs.push(HostSessions {
            host: remote.label,
            is_local: false,
            host_id: Some(remote.host_id),
            sessions,
        });
    }
    build_fleet_tree(inputs)
}

/// One of an account's sessions, wherever in the fleet it runs.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountSession<'a> {
    /// The host it lives on — the session table's Host column.
    pub host: &'a str,
    /// `true` when that host is this machine.
    pub is_local: bool,
    /// The daemon identity to route through for a remote session (attach,
    /// resume, guardrails). `None` for the local host. The **label is never the
    /// route** — two daemons can share one.
    pub host_id: Option<&'a str>,
    pub session: &'a SessionSnapshot,
}

/// Every session in the fleet that belongs to `account`, local and remote.
///
/// This is the account pane's view of the inventory, and it is a **query over
/// the tree** rather than a second copy of it: the tree already holds every
/// host's sessions, so copying them onto the account would leave two sets that
/// can disagree — and the tree fold would then count the local ones twice.
///
/// The join is `(provider, account_email)`. Not `config_dir`: that is a path on
/// whichever host produced it, so it neither matches this machine's spelling of
/// the same account nor exists at all for a default account. Joining on it would
/// hang every host's default sessions on the first default account and hide every
/// plexed one. An email is the only thing a subscription is called the same on
/// every host — and since one address can own both a Claude and a Codex account,
/// the provider has to match too.
///
/// A session whose account is unknown (`account_email: None` — an older daemon,
/// or a host with no OAuth identity) joins nothing: it stays visible under its
/// host in the Conductor rather than being claimed by an account it may not
/// belong to.
/// The result borrows from `tree` alone — `account` is read, never held — so a
/// caller may pass a temporary without keeping it alive to use the rows.
pub fn sessions_of_account<'a>(
    tree: &'a FleetTree,
    account: &crate::types::Account,
) -> Vec<AccountSession<'a>> {
    let Some(email) = account.email.as_deref() else {
        // An account we cannot name cannot claim anything.
        return Vec::new();
    };
    tree.hosts
        .iter()
        .flat_map(|h| {
            h.projects
                .iter()
                .flat_map(|p| p.sessions.iter())
                .filter(move |s| {
                    s.provider == account.provider
                        && s.account_email
                            .as_deref()
                            // Case-insensitive. Both hosts read the address from
                            // the provider's own token, so it ought to match byte
                            // for byte — but if it ever didn't, this join would
                            // fail *silently*: the sessions simply absent, with
                            // nothing on screen to explain why. Not noticing a
                            // difference in capitalisation costs nothing, and no
                            // two real accounts differ by case alone.
                            //
                            // ASCII-only on purpose: a domain is ASCII by
                            // definition, and full Unicode folding of a local
                            // part is a worse bet than no folding (dotted vs
                            // dotless i, and providers do not vary it anyway).
                            .is_some_and(|e| e.eq_ignore_ascii_case(email))
                })
                .map(move |session| AccountSession {
                    host: &h.host,
                    is_local: h.is_local,
                    host_id: h.host_id.as_deref(),
                    session,
                })
        })
        .collect()
}

#[cfg(test)]
#[path = "fleet_tests.rs"]
mod tests;
