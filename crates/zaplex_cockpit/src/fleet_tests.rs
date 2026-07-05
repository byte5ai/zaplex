//! Tests for the fleet aggregation / Conductor tree.

use super::*;
use crate::types::{SessionSnapshot, SessionState};
use chrono::{DateTime, Utc};

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).expect("valid timestamp")
}

fn session(id: &str, cwd: &str, state: SessionState, activity: i64) -> SessionSnapshot {
    SessionSnapshot {
        session_id: id.into(),
        cwd: cwd.into(),
        name: String::new(),
        state,
        model: String::new(),
        ctx_tokens: 0,
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
    let one = dev.projects.iter().find(|p| p.cwd == "/p/one").unwrap();
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
