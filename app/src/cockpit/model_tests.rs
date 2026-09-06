use super::*;

#[test]
fn codex_home_uses_pinned_root_and_falls_back_to_default() {
    let home = Path::new("/test/home");
    assert_eq!(codex_home(home, None), home.join(".codex"));
    assert_eq!(
        codex_home(home, Some(std::ffi::OsString::from("/test/codex-work"))),
        PathBuf::from("/test/codex-work")
    );
    assert_eq!(
        codex_home(home, Some(std::ffi::OsString::new())),
        home.join(".codex")
    );
}

#[test]
fn initial_scan_state_is_loading_not_empty() {
    let snapshot = initial_snapshot();
    assert!(snapshot.accounts.is_empty());
    assert_eq!(snapshot.health, ScanHealth::Pending);
}

#[test]
fn stale_inventory_cannot_readd_disconnected_host() {
    assert!(should_apply_refresh_result(2, 2));
    assert!(
        !should_apply_refresh_result(2, 1),
        "a scan requested before the current generation must be ignored"
    );

    let local = HostNode {
        host: "local".to_string(),
        is_local: true,
        host_id: None,
        availability: zaplex_cockpit::HostAvailability::Available,
        inventory_status: zaplex_cockpit::AgentInventoryStatus::Ready,
        registry_node_id: None,
        projects: Vec::new(),
        needs_me: 0,
    };
    let remote = remote_host(
        "devhost",
        "host-dev",
        session("dev-session", zaplex_cockpit::SessionState::Active),
    );
    let stale_result = FleetTree {
        hosts: vec![local, remote],
        needs_me: 0,
    };
    let mut visible = stale_result.clone();

    assert!(remove_disconnected_host(&mut visible, "host-dev"));
    let current_generation = 2;
    let stale_generation = 1;
    if should_apply_refresh_result(current_generation, stale_generation) {
        visible = stale_result;
    }

    assert_eq!(visible.hosts.len(), 1);
    assert!(visible.hosts[0].is_local);
}

#[test]
fn blocked_build_coalesces_refresh_triggers_into_one_rerun() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    fn spawn_blocked_build(
        builds: Arc<AtomicUsize>,
        active: Arc<AtomicUsize>,
        max_active: Arc<AtomicUsize>,
        started: Arc<Barrier>,
        release: Arc<Barrier>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            builds.fetch_add(1, Ordering::SeqCst);
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            max_active.fetch_max(current, Ordering::SeqCst);
            started.wait();
            release.wait();
            active.fetch_sub(1, Ordering::SeqCst);
        })
    }

    let builds = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let max_active = Arc::new(AtomicUsize::new(0));
    let mut flight = RefreshSingleFlight::default();

    assert!(flight.request());
    let started = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let first = spawn_blocked_build(
        builds.clone(),
        active.clone(),
        max_active.clone(),
        started.clone(),
        release.clone(),
    );
    started.wait();

    assert!(!flight.request());
    assert!(!flight.request());
    assert!(!flight.request());
    release.wait();
    first.join().unwrap();

    assert!(
        flight.finish(),
        "all overlapping triggers reserve one rerun"
    );
    let rerun_started = Arc::new(Barrier::new(2));
    let rerun_release = Arc::new(Barrier::new(2));
    let rerun = spawn_blocked_build(
        builds.clone(),
        active.clone(),
        max_active.clone(),
        rerun_started.clone(),
        rerun_release.clone(),
    );
    rerun_started.wait();
    rerun_release.wait();
    rerun.join().unwrap();

    assert!(!flight.finish());
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    assert_eq!(max_active.load(Ordering::SeqCst), 1);
}

#[test]
fn disable_cancels_a_coalesced_refresh_rerun() {
    let mut flight = RefreshSingleFlight::default();
    assert!(flight.request());
    assert!(!flight.request());

    flight.cancel_rerun();

    assert!(!flight.finish());
    assert!(!flight.running);
    assert!(!flight.rerun_requested);
}

#[test]
fn coalesced_rerun_uses_the_latest_requested_generation() {
    let mut flight = RefreshSingleFlight::default();
    let mut generation = 1;
    assert!(flight.request());

    generation += 1;
    assert!(!flight.request());
    generation += 1;
    assert!(!flight.request());

    assert!(flight.finish());
    assert_eq!(generation, 3);
    assert!(should_apply_refresh_result(generation, 3));
    assert!(!flight.finish());
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
        inventory_status: zaplex_cockpit::AgentInventoryStatus::Ready,
        registry_node_id: None,
        projects: Vec::new(),
        needs_me: 0,
    });
    assert!(!is_blank(&empty_snapshot(), &inventory));
}

#[test]
fn last_open_remote_session_removes_host_root() {
    let local = HostNode {
        host: "local".to_string(),
        is_local: true,
        host_id: None,
        availability: zaplex_cockpit::HostAvailability::Available,
        inventory_status: zaplex_cockpit::AgentInventoryStatus::Ready,
        registry_node_id: None,
        projects: Vec::new(),
        needs_me: 0,
    };
    let mut devhost = remote_host(
        "devhost",
        "host-dev",
        session("dev-session", zaplex_cockpit::SessionState::Waiting),
    );
    devhost.needs_me = 2;
    let mut buildhost = remote_host(
        "buildhost",
        "host-build",
        session("build-session", zaplex_cockpit::SessionState::Waiting),
    );
    buildhost.needs_me = 1;
    let mut inventory = FleetTree {
        hosts: vec![local, devhost, buildhost],
        needs_me: 3,
    };

    assert!(remove_disconnected_host(&mut inventory, "host-dev"));
    assert_eq!(inventory.hosts.len(), 2);
    assert!(inventory.hosts.iter().any(|host| host.is_local));
    assert!(inventory
        .hosts
        .iter()
        .any(|host| host.host_id.as_deref() == Some("host-build")));
    assert_eq!(inventory.needs_me, 1);
    assert!(!remove_disconnected_host(&mut inventory, "host-dev"));
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
        account_id: None,
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
        inventory_status: zaplex_cockpit::AgentInventoryStatus::Ready,
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
        inventory_status: zaplex_cockpit::AgentInventoryStatus::Ready,
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
