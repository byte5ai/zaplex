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
        last_activity: at(activity),
        pid: 0,
    }
}

fn host(name: &str, sessions: Vec<SessionSnapshot>) -> HostSessions {
    HostSessions {
        host: name.into(),
        sessions,
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
