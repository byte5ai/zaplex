//! Tests for the Conductor presentation helpers (glyph vocabulary, the
//! inverse-complexity collapse law, and the `w`-jump cycle order).

use super::*;
use crate::fleet::{build_fleet_tree, HostSessions};
use crate::types::{Provider, SessionSnapshot, SessionState};
use chrono::{DateTime, Utc};

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).expect("valid timestamp")
}

fn session(id: &str, cwd: &str, state: SessionState, activity: i64) -> SessionSnapshot {
    let name = cwd
        .trim_end_matches('/')
        .rsplit('/')
        .find(|s| !s.is_empty())
        .unwrap_or(cwd)
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
        project_root: cwd.into(),
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
fn glyph_vocabulary_is_working_waiting_idle() {
    assert_eq!(session_glyph(SessionState::Active), GLYPH_WORKING);
    assert_eq!(session_glyph(SessionState::Monitor), GLYPH_WORKING);
    assert_eq!(session_glyph(SessionState::Waiting), GLYPH_WAITING);
    assert_eq!(session_glyph(SessionState::Idle), GLYPH_IDLE);
}

#[test]
fn small_fleet_is_not_large_and_never_auto_collapses() {
    let tree = build_fleet_tree(vec![host(
        "devhost",
        vec![
            session("a", "/p/x", SessionState::Active, 10),
            session("b", "/p/y", SessionState::Idle, 20),
        ],
    )]);
    assert!(!fleet_is_large(&tree));
    for h in &tree.hosts {
        assert!(!host_auto_collapsed(h, fleet_is_large(&tree)));
    }
}

#[test]
fn large_by_agent_count() {
    // 9 agents on one host > LARGE_FLEET_AGENTS (8).
    let sessions: Vec<_> = (0..9)
        .map(|i| {
            session(
                &format!("s{i}"),
                &format!("/p/{i}"),
                SessionState::Active,
                i,
            )
        })
        .collect();
    let tree = build_fleet_tree(vec![host("devhost", sessions)]);
    assert!(fleet_is_large(&tree));
}

#[test]
fn large_by_host_count() {
    let tree = build_fleet_tree(vec![
        host("h1", vec![session("a", "/p/a", SessionState::Active, 1)]),
        host("h2", vec![session("b", "/p/b", SessionState::Active, 1)]),
        host("h3", vec![session("c", "/p/c", SessionState::Active, 1)]),
    ]);
    assert!(fleet_is_large(&tree));
}

#[test]
fn large_fleet_collapses_calm_hosts_keeps_waiting_hosts_open() {
    // Three hosts (large): only h2 has a waiting agent.
    let tree = build_fleet_tree(vec![
        host("h1", vec![session("a", "/p/a", SessionState::Active, 1)]),
        host("h2", vec![session("b", "/p/b", SessionState::Waiting, 1)]),
        host("h3", vec![session("c", "/p/c", SessionState::Idle, 1)]),
    ]);
    assert!(fleet_is_large(&tree));
    let large = fleet_is_large(&tree);
    for h in &tree.hosts {
        let collapsed = host_auto_collapsed(h, large);
        if h.host == "h2" {
            assert!(!collapsed, "the waiting host must stay expanded");
        } else {
            assert!(collapsed, "a calm host in a large fleet folds up");
        }
    }
}

#[test]
fn host_summary_formats_counts_and_omits_zero_waiting() {
    let tree = build_fleet_tree(vec![host(
        "devhost",
        vec![
            session("a", "/p/a", SessionState::Active, 1),
            session("b", "/p/b", SessionState::Waiting, 1),
            session("c", "/p/c", SessionState::Idle, 1),
        ],
    )]);
    let h = &tree.hosts[0];
    assert_eq!(host_summary(h), "devhost · 3 agents · 1 waiting");

    let calm = build_fleet_tree(vec![host(
        "mac",
        vec![session("x", "/p/x", SessionState::Active, 1)],
    )]);
    assert_eq!(host_summary(&calm.hosts[0]), "mac · 1 agent");
}

#[test]
fn next_waiting_cycles_in_tree_order() {
    // Two hosts, mixed states. build_fleet_tree sorts waiting-first and hosts by
    // needs-me desc, so the waiting order is deterministic.
    let tree = build_fleet_tree(vec![
        host(
            "h1",
            vec![
                session("a", "/p/a", SessionState::Waiting, 1),
                session("b", "/p/b", SessionState::Active, 1),
            ],
        ),
        host("h2", vec![session("c", "/p/c", SessionState::Waiting, 1)]),
    ]);

    // From nothing → first waiting.
    let first = next_waiting(&tree, None).expect("some waiting");
    // Cycle through all waiting and back to the first.
    let mut seen = vec![first.clone()];
    let mut cur = first.clone();
    loop {
        let nxt = next_waiting(&tree, Some((&cur.0, &cur.1))).expect("still waiting");
        if nxt == first {
            break;
        }
        assert!(!seen.contains(&nxt), "no repeats before a full cycle");
        seen.push(nxt.clone());
        cur = nxt;
    }
    assert_eq!(seen.len(), 2, "exactly the two waiting agents, once each");
}

#[test]
fn next_waiting_none_when_nothing_waits() {
    let tree = build_fleet_tree(vec![host(
        "h1",
        vec![session("a", "/p/a", SessionState::Active, 1)],
    )]);
    assert_eq!(next_waiting(&tree, None), None);
}

#[test]
fn next_waiting_from_stale_cursor_restarts_at_first() {
    let tree = build_fleet_tree(vec![host(
        "h1",
        vec![session("a", "/p/a", SessionState::Waiting, 1)],
    )]);
    // A cursor pointing at a session that is no longer waiting/present.
    let got = next_waiting(&tree, Some(("h1", "gone")));
    assert_eq!(got, Some(("h1".to_string(), "a".to_string())));
}
