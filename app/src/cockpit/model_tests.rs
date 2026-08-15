use super::*;

#[test]
fn initial_scan_state_is_loading_not_empty() {
    let snapshot = initial_snapshot();
    assert!(snapshot.accounts.is_empty());
    assert_eq!(snapshot.health, ScanHealth::Pending);
}

#[test]
fn older_scan_completion_cannot_replace_newer_snapshot() {
    assert!(should_apply_refresh_result(2, 2));
    assert!(
        !should_apply_refresh_result(2, 1),
        "a scan requested before the current generation must be ignored"
    );
}

fn empty_snapshot() -> CockpitSnapshot {
    CockpitSnapshot {
        accounts: Vec::new(),
        generated_at: Utc::now(),
        health: ScanHealth::Loaded,
    }
}

/// The freshly-disabled (or never-populated) state is blank — this is the
/// state `clear_for_disabled` settles into, and every disabled tick after
/// the first must see this and stay a no-op (no `Updated` spam).
#[test]
fn default_state_is_blank() {
    assert!(is_blank(&empty_snapshot(), &FleetTree::default()));
}

/// A nonzero waiting count (the exact staleness the Codex review flagged —
/// the badge stuck at an old count) must NOT read as blank, so
/// `clear_for_disabled` still clears it on the enabled→disabled
/// transition.
#[test]
fn nonzero_needs_me_is_not_blank() {
    let mut inventory = FleetTree::default();
    inventory.needs_me = 3;
    assert!(!is_blank(&empty_snapshot(), &inventory));
}

/// A populated host list is not blank even if nothing happens to be
/// waiting right now — the Conductor pane must also clear on disable, not
/// just the badge count.
#[test]
fn nonempty_hosts_is_not_blank() {
    let mut inventory = FleetTree::default();
    inventory.hosts.push(HostNode {
        host: "devbox".to_string(),
        is_local: true,
        host_id: None,
        availability: zaplex_cockpit::HostAvailability::Available,
        registry_node_id: None,
        projects: Vec::new(),
        needs_me: 0,
    });
    assert!(!is_blank(&empty_snapshot(), &inventory));
}

fn session(id: &str, state: zaplex_cockpit::SessionState) -> SessionSnapshot {
    SessionSnapshot {
        session_id: id.into(),
        cwd: "/w".into(),
        name: "job".into(),
        state,
        provider: Provider::Claude,
        model: "opus".into(),
        effort: None,
        ctx_tokens: 0,
        project_root: "/w".into(),
        repo_root: "/w".into(),
        project_name: "proj".into(),
        branch: None,
        worktree: None,
        config_dir: None,
        account_email: None,
        process_fingerprint: None,
        pty_session_id: None,
        pty_session_generation: None,
        pty_foreground: false,
        task_state: None,
        last_activity: Utc::now(),
        pid: 0,
    }
}

#[test]
fn pty_routes_require_daemon_binding_capability() {
    let mut legacy = session("legacy", zaplex_cockpit::SessionState::Active);
    legacy.pty_session_id = Some("pty-1".to_string());
    legacy.pty_session_generation = Some(7);
    legacy.pty_foreground = true;
    retain_negotiated_agent_pty_routes(
        &["agent-inventory".to_string()],
        std::slice::from_mut(&mut legacy),
    );
    assert!(legacy.pty_session_id.is_none());
    assert!(legacy.pty_session_generation.is_none());
    assert!(!legacy.pty_foreground);

    let mut v1_only = session("v1-only", zaplex_cockpit::SessionState::Active);
    v1_only.pty_session_id = Some("pty-v1".to_string());
    v1_only.pty_session_generation = Some(8);
    v1_only.pty_foreground = true;
    retain_negotiated_agent_pty_routes(
        &[
            "agent-inventory".to_string(),
            "agent-pty-binding".to_string(),
        ],
        std::slice::from_mut(&mut v1_only),
    );
    assert!(v1_only.pty_session_id.is_none());
    assert!(v1_only.pty_session_generation.is_none());
    assert!(!v1_only.pty_foreground);

    let mut capable = session("capable", zaplex_cockpit::SessionState::Active);
    capable.pty_session_id = Some("pty-2".to_string());
    capable.pty_session_generation = Some(8);
    capable.pty_foreground = true;
    retain_negotiated_agent_pty_routes(
        &[
            "agent-inventory".to_string(),
            "agent-pty-binding".to_string(),
            "agent-pty-binding-v2".to_string(),
        ],
        std::slice::from_mut(&mut capable),
    );
    assert_eq!(capable.pty_session_id.as_deref(), Some("pty-2"));
    assert_eq!(capable.pty_session_generation, Some(8));
    assert!(capable.pty_foreground);
}

/// One remote host with `host_id` carrying a single session in `state`,
/// under the shared display `label`.
fn remote_host(label: &str, host_id: &str, session: SessionSnapshot) -> HostNode {
    HostNode {
        host: label.into(),
        is_local: false,
        host_id: Some(host_id.into()),
        availability: zaplex_cockpit::HostAvailability::Available,
        registry_node_id: None,
        projects: vec![zaplex_cockpit::ProjectNode {
            root: "/w".into(),
            name: "proj".into(),
            needs_me: 0,
            sessions: vec![session],
        }],
        needs_me: 0,
    }
}

/// Finding 2: two remote daemons sharing a display label, each with a session
/// under the SAME host-scoped id but DISTINCT `host_id`. A working→Waiting
/// transition on one must not be masked by the other's old state. A
/// label-keyed diff would alias both into one map entry (one overwriting the
/// other); keying by the stable host identity keeps them distinct.
#[test]
fn same_label_hosts_do_not_mask_each_others_waiting_transition() {
    use zaplex_cockpit::SessionState;
    // Both hosts labelled "box", same session id "s1", different host_id.
    let old = FleetTree {
        hosts: vec![
            remote_host("box", "host-A", session("s1", SessionState::Active)),
            remote_host("box", "host-B", session("s1", SessionState::Active)),
        ],
        needs_me: 0,
    };
    // Host A's session flips to Waiting; host B keeps working.
    let new = FleetTree {
        hosts: vec![
            remote_host("box", "host-A", session("s1", SessionState::Waiting)),
            remote_host("box", "host-B", session("s1", SessionState::Active)),
        ],
        needs_me: 1,
    };
    let transitions = fleet_transitions_to_waiting(&old, &new);
    // Exactly one transition fires — host A's — and it isn't masked by host
    // B's identical (label, session id).
    assert_eq!(transitions, vec!["box — job".to_string()]);

    // And symmetrically: a transition on B alone also fires (not swallowed by
    // A's old Active state under the shared label).
    let new_b = FleetTree {
        hosts: vec![
            remote_host("box", "host-A", session("s1", SessionState::Active)),
            remote_host("box", "host-B", session("s1", SessionState::Waiting)),
        ],
        needs_me: 1,
    };
    assert_eq!(
        fleet_transitions_to_waiting(&old, &new_b),
        vec!["box — job".to_string()]
    );
}

#[test]
fn removed_host_does_not_emit_an_actionable_waiting_transition() {
    use zaplex_cockpit::{HostAvailability, SessionState};

    let old = FleetTree {
        hosts: vec![remote_host(
            "box",
            "host-A",
            session("s1", SessionState::Active),
        )],
        needs_me: 0,
    };
    let mut removed = remote_host("box", "host-A", session("s1", SessionState::Waiting));
    removed.availability = HostAvailability::Removed;
    let new = FleetTree {
        hosts: vec![removed],
        needs_me: 0,
    };

    assert!(fleet_transitions_to_waiting(&old, &new).is_empty());
}

#[test]
fn same_host_and_session_id_in_different_accounts_do_not_mask_waiting_transition() {
    use zaplex_cockpit::SessionState;

    let mut old_personal = session("copied", SessionState::Active);
    old_personal.account_email = Some("personal@example.com".to_string());
    let mut old_work = session("copied", SessionState::Waiting);
    old_work.account_email = Some("work@example.com".to_string());

    let mut new_personal = old_personal.clone();
    new_personal.state = SessionState::Waiting;
    let new_work = old_work.clone();

    let host = |sessions| HostNode {
        host: "box".to_string(),
        is_local: false,
        host_id: Some("host-A".to_string()),
        availability: zaplex_cockpit::HostAvailability::Available,
        registry_node_id: None,
        projects: vec![zaplex_cockpit::ProjectNode {
            root: "/w".to_string(),
            name: "proj".to_string(),
            needs_me: 0,
            sessions,
        }],
        needs_me: 0,
    };
    let old = FleetTree {
        hosts: vec![host(vec![old_personal, old_work])],
        needs_me: 1,
    };
    let new = FleetTree {
        hosts: vec![host(vec![new_personal, new_work])],
        needs_me: 2,
    };

    assert_eq!(
        fleet_transitions_to_waiting(&old, &new),
        vec!["box — job".to_string()],
        "the already-waiting account must not overwrite the other account's old state"
    );
}
