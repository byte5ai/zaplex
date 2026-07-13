//! Tests for the fleet aggregation / Conductor tree.

use super::*;
use crate::types::{Provider, SessionSnapshot, SessionState};
use chrono::{DateTime, Utc};

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).expect("valid timestamp")
}

/// A session whose git root == its cwd (the common case in these tests).
fn session(id: &str, cwd: &str, state: SessionState, activity: i64) -> SessionSnapshot {
    session_in(id, cwd, cwd, state, activity)
}

/// A session with an explicit git `root` distinct from `cwd`.
fn session_in(
    id: &str,
    cwd: &str,
    root: &str,
    state: SessionState,
    activity: i64,
) -> SessionSnapshot {
    let name = root
        .trim_end_matches('/')
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(root)
        .to_string();
    SessionSnapshot {
        session_id: id.into(),
        cwd: cwd.into(),
        name: String::new(),
        state,
        provider: Provider::Claude,
        model: String::new(),
        effort: None,
        ctx_tokens: 0,
        project_root: root.into(),
        project_name: name,
        branch: None,
        worktree: None,
        config_dir: None,
        last_activity: at(activity),
        pid: 0,
    }
}

fn host(name: &str, sessions: Vec<SessionSnapshot>) -> HostSessions {
    HostSessions {
        host: name.into(),
        // Test helper default: locality is asserted explicitly by the tests
        // that care (see the fold + collision tests below).
        is_local: false,
        host_id: None,
        sessions,
    }
}

/// A `RemoteHost` whose display label and stable id are the given strings.
fn remote_host(label: &str, host_id: &str) -> RemoteHost {
    RemoteHost {
        label: label.into(),
        host_id: host_id.into(),
    }
}

#[test]
fn groups_by_cwd_and_labels_project_by_basename() {
    let tree = build_fleet_tree(vec![host(
        "devhost",
        vec![
            session("a", "/home/u/proj-x", SessionState::Active, 10),
            session("b", "/home/u/proj-x", SessionState::Active, 20),
            session("c", "/home/u/proj-y", SessionState::Active, 30),
        ],
    )]);
    assert_eq!(tree.hosts.len(), 1);
    let h = &tree.hosts[0];
    assert_eq!(h.projects.len(), 2, "two distinct cwds → two projects");
    let names: Vec<&str> = h.projects.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"proj-x") && names.contains(&"proj-y"));
}

#[test]
fn needs_me_bubbles_session_to_project_to_host_to_fleet() {
    let tree = build_fleet_tree(vec![
        host(
            "devhost",
            vec![
                session("a", "/p/one", SessionState::Waiting, 5),
                session("b", "/p/one", SessionState::Active, 6),
                session("c", "/p/two", SessionState::Waiting, 7),
            ],
        ),
        host(
            "macmini",
            vec![session("d", "/p/three", SessionState::Monitor, 8)],
        ),
    ]);
    // devhost: 2 waiting (one in /p/one, one in /p/two); macmini: 0.
    let dev = tree.hosts.iter().find(|h| h.host == "devhost").unwrap();
    assert_eq!(dev.needs_me, 2);
    let one = dev.projects.iter().find(|p| p.root == "/p/one").unwrap();
    assert_eq!(one.needs_me, 1);
    let mac = tree.hosts.iter().find(|h| h.host == "macmini").unwrap();
    assert_eq!(mac.needs_me, 0);
    assert_eq!(tree.needs_me, 2, "grand total across the fleet");
}

#[test]
fn hosts_and_projects_sort_needs_me_first() {
    let tree = build_fleet_tree(vec![
        host("quiet", vec![session("a", "/p/a", SessionState::Active, 1)]),
        host("busy", vec![session("b", "/p/b", SessionState::Waiting, 1)]),
    ]);
    // 'busy' has a waiting session → sorts before 'quiet' despite alpha order.
    assert_eq!(tree.hosts[0].host, "busy");
    assert_eq!(tree.hosts[1].host, "quiet");
}

#[test]
fn sessions_within_project_are_waiting_first_then_recent() {
    let tree = build_fleet_tree(vec![host(
        "h",
        vec![
            session("old-active", "/p", SessionState::Active, 100),
            session("waiting", "/p", SessionState::Waiting, 10),
            session("new-active", "/p", SessionState::Active, 200),
        ],
    )]);
    let p = &tree.hosts[0].projects[0];
    // Waiting first regardless of activity; then actives by most-recent.
    assert_eq!(p.sessions[0].session_id, "waiting");
    assert_eq!(p.sessions[1].session_id, "new-active");
    assert_eq!(p.sessions[2].session_id, "old-active");
}

#[test]
fn sessions_under_one_git_root_collapse_into_one_project() {
    // Two different cwds (sub-directories) that share a git root must land in a
    // single ProjectNode keyed + labelled by that root.
    let tree = build_fleet_tree(vec![host(
        "devhost",
        vec![
            session_in(
                "a",
                "/home/u/zaplex/app",
                "/home/u/zaplex",
                SessionState::Active,
                10,
            ),
            session_in(
                "b",
                "/home/u/zaplex/crates/x",
                "/home/u/zaplex",
                SessionState::Waiting,
                20,
            ),
        ],
    )]);
    let h = &tree.hosts[0];
    assert_eq!(h.projects.len(), 1, "one shared git root → one project");
    let p = &h.projects[0];
    assert_eq!(p.root, "/home/u/zaplex");
    assert_eq!(p.name, "zaplex");
    assert_eq!(p.sessions.len(), 2);
    assert_eq!(p.needs_me, 1);
}

#[test]
fn idle_sessions_never_count_as_needs_me() {
    let tree = build_fleet_tree(vec![host(
        "h",
        vec![
            session("idle", "/p/one", SessionState::Idle, 30),
            session("waiting", "/p/one", SessionState::Waiting, 10),
        ],
    )]);
    let p = &tree.hosts[0].projects[0];
    assert_eq!(p.needs_me, 1, "only Waiting counts; Idle does not");
    assert_eq!(tree.needs_me, 1);
    // Waiting sorts ahead of Idle despite Idle being more recent.
    assert_eq!(p.sessions[0].session_id, "waiting");
    assert_eq!(p.sessions[1].session_id, "idle");
}

#[test]
fn fold_empty_remote_list_is_exactly_the_local_tree() {
    // With no daemons, the fold must equal building a single-host tree from the
    // local sessions — nothing regresses when nothing is connected.
    let local = vec![
        session("a", "/home/me/proj", SessionState::Waiting, 10),
        session("b", "/home/me/other", SessionState::Active, 20),
    ];
    let folded = fold_inventory("local", local.clone(), vec![]);
    let expected = build_fleet_tree(vec![HostSessions {
        host: "local".into(),
        is_local: true, // fold marks the local contribution local
        host_id: None,  // the local node carries no daemon id
        sessions: local,
    }]);
    assert_eq!(folded, expected);
    assert_eq!(folded.hosts.len(), 1);
    assert_eq!(folded.hosts[0].host, "local");
    assert_eq!(folded.needs_me, 1);
}

#[test]
fn fold_two_hosts_sharing_a_path_stay_isolated() {
    // The SAME absolute path on two different hosts must NOT collapse into one
    // project node — projects are namespaced by host.
    let shared = "/home/me/proj";
    let local = vec![session("l", shared, SessionState::Waiting, 10)];
    let remote = vec![session("r", shared, SessionState::Active, 20)];
    let tree = fold_inventory(
        "local",
        local,
        vec![(remote_host("devhost", "devhost-id"), remote)],
    );

    assert_eq!(
        tree.hosts.len(),
        2,
        "two hosts, never merged by shared path"
    );
    let local_host = tree.hosts.iter().find(|h| h.host == "local").unwrap();
    let dev_host = tree.hosts.iter().find(|h| h.host == "devhost").unwrap();
    // Each host keeps its own single project rooted at the shared path.
    assert_eq!(local_host.projects.len(), 1);
    assert_eq!(dev_host.projects.len(), 1);
    assert_eq!(local_host.projects[0].root, shared);
    assert_eq!(dev_host.projects[0].root, shared);
    // The waiting session is local's; devhost's identical path is a separate node.
    assert_eq!(local_host.projects[0].sessions[0].session_id, "l");
    assert_eq!(dev_host.projects[0].sessions[0].session_id, "r");
}

#[test]
fn fold_marks_only_the_local_contribution_local() {
    // The local contribution is the only `is_local == true` node; every remote
    // daemon's node is `is_local == false`.
    let local = vec![session("l", "/p/a", SessionState::Active, 10)];
    let remote = vec![session("r", "/p/b", SessionState::Active, 20)];
    let tree = fold_inventory(
        "local",
        local,
        vec![(remote_host("devhost", "devhost-id"), remote)],
    );
    let local_host = tree.hosts.iter().find(|h| h.host == "local").unwrap();
    let dev_host = tree.hosts.iter().find(|h| h.host == "devhost").unwrap();
    assert!(local_host.is_local, "the local host must be marked local");
    assert!(!dev_host.is_local, "a remote host must be marked remote");
    // The local node carries no daemon id; the remote node carries the id the
    // connection advertised (guardrails route by it, not by the label).
    assert_eq!(local_host.host_id, None);
    assert_eq!(dev_host.host_id.as_deref(), Some("devhost-id"));
}

#[test]
fn fold_remote_host_label_colliding_with_local_is_still_remote() {
    // The exact P1 collision: a remote daemon advertises the SAME label as the
    // local host (SSH alias / matching hostname). Both nodes exist separately;
    // the local one is `is_local`, the remote one is NOT — so guardrail routing
    // (which reads `is_local`, never the label) can never signal the remote
    // agent's host-local pid on the local machine.
    let local = vec![session("l", "/p/a", SessionState::Active, 10)];
    let remote = vec![session("r", "/p/b", SessionState::Active, 20)];
    let tree = fold_inventory(
        "devhost",
        local,
        vec![(remote_host("devhost", "devhost-id"), remote)],
    );
    // Two distinct host nodes despite the shared label.
    let colliding: Vec<&HostNode> = tree.hosts.iter().filter(|h| h.host == "devhost").collect();
    assert_eq!(colliding.len(), 2, "shared label → still two host nodes");
    // Exactly one is local; the one holding the remote session is NOT local.
    let local_node = colliding
        .iter()
        .find(|h| h.is_local)
        .expect("one local node");
    let remote_node = colliding
        .iter()
        .find(|h| !h.is_local)
        .expect("one remote node");
    assert_eq!(local_node.projects[0].sessions[0].session_id, "l");
    assert_eq!(remote_node.projects[0].sessions[0].session_id, "r");
    assert!(
        !remote_node.is_local,
        "the remote contribution stays remote even when its label equals the local label"
    );
}

#[test]
fn fold_two_remotes_sharing_a_label_carry_distinct_host_ids() {
    // The remote↔remote collision: two connected daemons advertise the SAME
    // label but different stable host ids. Both become separate host nodes, and
    // each keeps its own `host_id` — so guardrail routing (which resolves the
    // target daemon by id, never by the shared label) can never signal one
    // remote's host-local pid on the other remote's machine.
    let local = vec![session("l", "/p/a", SessionState::Active, 1)];
    let box_a = vec![session("a", "/p/x", SessionState::Waiting, 10)];
    let box_b = vec![session("b", "/p/y", SessionState::Waiting, 20)];
    let tree = fold_inventory(
        "local",
        local,
        vec![
            (remote_host("prod", "prod-a-id"), box_a),
            (remote_host("prod", "prod-b-id"), box_b),
        ],
    );
    // Three nodes total: one local + two remotes sharing the "prod" label.
    let prod_nodes: Vec<&HostNode> = tree.hosts.iter().filter(|h| h.host == "prod").collect();
    assert_eq!(prod_nodes.len(), 2, "shared label → still two remote nodes");
    // Each remote node carries its own id and holds only its own session — the
    // ids are the routing key that keeps them apart.
    let node_a = prod_nodes
        .iter()
        .find(|h| h.host_id.as_deref() == Some("prod-a-id"))
        .expect("node with prod-a-id");
    let node_b = prod_nodes
        .iter()
        .find(|h| h.host_id.as_deref() == Some("prod-b-id"))
        .expect("node with prod-b-id");
    assert!(!node_a.is_local && !node_b.is_local, "both are remote");
    assert_eq!(node_a.projects[0].sessions[0].session_id, "a");
    assert_eq!(node_b.projects[0].sessions[0].session_id, "b");
}

#[test]
fn fold_needs_me_bubbles_across_the_whole_fleet() {
    let local = vec![session("a", "/p/one", SessionState::Waiting, 5)];
    let dev = vec![
        session("b", "/p/two", SessionState::Waiting, 6),
        session("c", "/p/two", SessionState::Active, 7),
    ];
    let mac = vec![session("d", "/p/three", SessionState::Monitor, 8)];
    let tree = fold_inventory(
        "local",
        local,
        vec![
            (remote_host("devhost", "devhost-id"), dev),
            (remote_host("macmini", "macmini-id"), mac),
        ],
    );
    // Grand total spans every host: 1 (local) + 1 (devhost) + 0 (macmini).
    assert_eq!(tree.needs_me, 2);
    assert_eq!(tree.hosts.len(), 3);
    // Hosts with waiting work sort ahead of the quiet one.
    assert_eq!(tree.hosts[0].needs_me, 1);
    assert_eq!(tree.hosts[1].needs_me, 1);
    assert_eq!(
        tree.hosts
            .iter()
            .find(|h| h.host == "macmini")
            .unwrap()
            .needs_me,
        0
    );
}

#[test]
fn fold_identity_is_host_scoped_session_id() {
    // The same session_id on two hosts must remain two distinct leaves — id is
    // unique only within a host.
    let local = vec![session("dup", "/p/a", SessionState::Waiting, 10)];
    let remote = vec![session("dup", "/p/b", SessionState::Waiting, 20)];
    let tree = fold_inventory(
        "local",
        local,
        vec![(remote_host("devhost", "devhost-id"), remote)],
    );
    assert_eq!(tree.needs_me, 2, "same id on two hosts counts twice");
    let total_sessions: usize = tree
        .hosts
        .iter()
        .flat_map(|h| h.projects.iter())
        .map(|p| p.sessions.len())
        .sum();
    assert_eq!(total_sessions, 2);
}

#[test]
fn empty_hosts_are_dropped_and_empty_fleet_is_zero() {
    let tree = build_fleet_tree(vec![
        host("idle", vec![]),
        host("live", vec![session("a", "/p", SessionState::Active, 1)]),
    ]);
    assert_eq!(tree.hosts.len(), 1, "the idle host is dropped");
    assert_eq!(tree.hosts[0].host, "live");

    let empty = build_fleet_tree(vec![]);
    assert!(empty.hosts.is_empty());
    assert_eq!(empty.needs_me, 0);
}

#[test]
fn merge_registered_adds_agentless_hosts_and_dedups_by_label() {
    // A tree with one session-backed host ("devhost").
    let mut tree = build_fleet_tree(vec![host(
        "devhost",
        vec![session("a", "/p/x", SessionState::Active, 10)],
    )]);
    // Registry: devhost (already present, connected) + agenthost (registered, no
    // live agent — build_fleet_tree would have dropped it).
    let registered = vec![
        ("node-dev".to_string(), "devhost".to_string()),
        ("node-agent".to_string(), "agenthost".to_string()),
    ];
    merge_registered_hosts(&mut tree, &registered);

    assert_eq!(tree.hosts.len(), 2, "agenthost added, devhost not duplicated");
    let dev = tree.hosts.iter().find(|h| h.host == "devhost").unwrap();
    assert_eq!(dev.projects.len(), 1, "the connected host keeps its sessions");
    assert_eq!(
        dev.registry_node_id.as_deref(),
        Some("node-dev"),
        "a live registered host is back-filled with its registry id so host-row \
         actions (open/manage/star) work"
    );
    let agent = tree.hosts.iter().find(|h| h.host == "agenthost").unwrap();
    assert!(agent.projects.is_empty(), "a registry-only host has no sessions");
    assert_eq!(agent.registry_node_id.as_deref(), Some("node-agent"));
    assert!(!agent.is_local);
    assert_eq!(agent.needs_me, 0);
    assert_eq!(tree.needs_me, 0, "an agentless host adds no needs-me");
}
