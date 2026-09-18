use settings::Setting;

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

    assert!(reconcile_live_daemon_roots(
        &mut visible,
        &mut ManagedFleetInventory::default(),
        "local",
        &[],
    ));
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

    let remotes = [connected_root("buildhost", "host-build", None)];
    assert!(reconcile_live_daemon_roots(
        &mut inventory,
        &mut ManagedFleetInventory::default(),
        "local",
        &remotes,
    ));
    assert_eq!(inventory.hosts.len(), 2);
    assert!(inventory.hosts.iter().any(|host| host.is_local));
    assert!(inventory
        .hosts
        .iter()
        .any(|host| host.host_id.as_deref() == Some("host-build")));
    assert_eq!(inventory.needs_me, 1);
    assert!(!reconcile_live_daemon_roots(
        &mut inventory,
        &mut ManagedFleetInventory::default(),
        "local",
        &remotes,
    ));
}

#[test]
fn registry_read_error_reuses_last_successful_snapshot() {
    let mut remote = remote_host(
        "daemon-label",
        "host-dev",
        session("waiting", zaplex_cockpit::SessionState::Waiting),
    );
    remote.registry_node_id = Some("node-dev".to_string());
    remote.needs_me = 1;
    remote.projects[0].needs_me = 1;
    let mut inventory = FleetTree {
        hosts: vec![remote],
        needs_me: 1,
    };
    let cached = vec![RegisteredHost {
        node_id: "node-dev".to_string(),
        label: "registry-label".to_string(),
        live_host_id: None,
    }];
    let live_hosts = vec![("node-dev".to_string(), "host-dev".to_string())];

    let read = resolve_registry_read(Err("database busy".to_string()), Some(&cached));
    reconcile_registry_read(&mut inventory, &read, &live_hosts);

    let host = &inventory.hosts[0];
    assert_eq!(host.host, "registry-label");
    assert_eq!(
        host.availability,
        zaplex_cockpit::HostAvailability::Available
    );
    assert_eq!(host.registry_node_id.as_deref(), Some("node-dev"));
    assert_eq!(inventory.needs_me, 1);
}

#[test]
fn registry_binding_preserves_multiple_daemons_for_one_node() {
    let registered = vec![RegisteredHost {
        node_id: "node-dev".to_string(),
        label: "devhost".to_string(),
        live_host_id: None,
    }];
    let live_hosts = vec![
        ("node-dev".to_string(), "daemon-current".to_string()),
        ("node-dev".to_string(), "daemon-old".to_string()),
    ];

    let bound = bind_live_registry_hosts(&registered, &live_hosts);

    assert_eq!(bound.len(), 2);
    assert!(bound
        .iter()
        .any(|host| host.live_host_id.as_deref() == Some("daemon-current")));
    assert!(bound
        .iter()
        .any(|host| host.live_host_id.as_deref() == Some("daemon-old")));
}

#[test]
fn first_registry_read_error_marks_bound_hosts_unverified() {
    let mut remote = remote_host(
        "daemon-label",
        "host-dev",
        session("waiting", zaplex_cockpit::SessionState::Waiting),
    );
    remote.registry_node_id = Some("node-dev".to_string());
    remote.needs_me = 1;
    remote.projects[0].needs_me = 1;
    let mut inventory = FleetTree {
        hosts: vec![remote],
        needs_me: 1,
    };

    let read = resolve_registry_read(Err("database busy".to_string()), None);
    reconcile_registry_read(&mut inventory, &read, &[]);

    let host = &inventory.hosts[0];
    assert_eq!(
        host.availability,
        zaplex_cockpit::HostAvailability::Unverified
    );
    assert_eq!(host.registry_node_id.as_deref(), Some("node-dev"));
    assert_eq!(inventory.needs_me, 0);
}

#[test]
fn successful_empty_registry_read_remains_authoritative() {
    let mut remote = remote_host(
        "daemon-label",
        "host-dev",
        session("waiting", zaplex_cockpit::SessionState::Waiting),
    );
    remote.registry_node_id = Some("node-dev".to_string());
    remote.needs_me = 1;
    remote.projects[0].needs_me = 1;
    let mut inventory = FleetTree {
        hosts: vec![remote],
        needs_me: 1,
    };
    let cached = vec![RegisteredHost {
        node_id: "node-dev".to_string(),
        label: "registry-label".to_string(),
        live_host_id: None,
    }];

    let read = resolve_registry_read(Ok(Vec::new()), Some(&cached));
    reconcile_registry_read(&mut inventory, &read, &[]);

    assert_eq!(
        inventory.hosts[0].availability,
        zaplex_cockpit::HostAvailability::Removed
    );
    assert_eq!(inventory.needs_me, 0);
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

fn connected_root(label: &str, host_id: &str, registry: Option<&str>) -> RemoteHost {
    RemoteHost {
        label: label.into(),
        host_id: host_id.into(),
        registry_node_id: registry.map(str::to_owned),
        inventory_status: AgentInventoryStatus::Pending,
    }
}

#[test]
fn repeated_ticks_cannot_starve_a_slow_refresh_result() {
    let mut flight = RefreshSingleFlight::default();
    assert!(flight.request());
    let started_generation = flight.generation;
    // A daemon request can span several reconcile ticks. Every completed
    // scan must remain publishable unless an authoritative input changed.
    for _tick in 0..8 {
        assert!(!flight.request());
    }
    assert!(should_apply_refresh_result(
        flight.generation,
        started_generation
    ));
    assert!(flight.finish());
    let rerun_generation = flight.generation;
    for _tick in 0..8 {
        assert!(!flight.request());
    }
    assert!(should_apply_refresh_result(
        flight.generation,
        rerun_generation
    ));
    assert!(flight.finish());
    assert!(!flight.finish());
}

#[test]
fn topology_change_rejects_old_result_but_ticks_allow_the_followup() {
    let mut flight = RefreshSingleFlight::default();
    assert!(flight.request());
    let before_disconnect = flight.generation;
    flight.invalidate();
    assert!(!flight.request());
    assert!(!should_apply_refresh_result(
        flight.generation,
        before_disconnect
    ));
    assert!(flight.finish());
    let after_disconnect = flight.generation;
    assert!(!flight.request());
    assert!(should_apply_refresh_result(
        flight.generation,
        after_disconnect
    ));
    assert!(flight.finish());
    assert!(!flight.finish());
}

#[test]
fn disabled_then_reenabled_cannot_publish_the_pre_disable_scan() {
    let mut flight = RefreshSingleFlight::default();
    assert!(flight.request());
    let old_generation = flight.generation;
    flight.invalidate();
    flight.cancel_rerun();
    assert!(!flight.request(), "re-enabling queues behind the old scan");
    assert!(!should_apply_refresh_result(
        flight.generation,
        old_generation
    ));
    assert!(flight.finish());
    assert!(should_apply_refresh_result(
        flight.generation,
        flight.generation
    ));
    assert!(!flight.finish());
}

#[test]
fn connected_host_is_visible_before_any_inventory_response() {
    let mut inventory = FleetTree::default();
    let mut managed = ManagedFleetInventory::default();
    let remotes = [connected_root("remote", "host-a", Some("node-a"))];
    assert!(reconcile_live_daemon_roots(
        &mut inventory,
        &mut managed,
        "laptop",
        &remotes
    ));
    assert_eq!(inventory.hosts.len(), 2);
    let local = inventory.hosts.iter().find(|host| host.is_local).unwrap();
    assert_eq!(local.host, "laptop");
    assert_eq!(local.inventory_status, AgentInventoryStatus::Pending);
    let remote = inventory.hosts.iter().find(|host| !host.is_local).unwrap();
    assert_eq!(remote.host_id.as_deref(), Some("host-a"));
    assert_eq!(remote.registry_node_id.as_deref(), Some("node-a"));
    assert_eq!(
        remote.availability,
        zaplex_cockpit::HostAvailability::Unverified
    );
    assert_eq!(remote.inventory_status, AgentInventoryStatus::Pending);
    assert!(remote.projects.is_empty());
    assert_eq!(inventory.needs_me, 0);
    assert!(!reconcile_live_daemon_roots(
        &mut inventory,
        &mut managed,
        "laptop",
        &remotes
    ));
}

#[test]
fn classic_ssh_host_without_registry_is_visible_and_same_labels_stay_distinct() {
    let mut inventory = FleetTree::default();
    let mut managed = ManagedFleetInventory::default();
    let remotes = [
        connected_root("same", "current-daemon", None),
        connected_root("same", "older-daemon", None),
    ];
    reconcile_live_daemon_roots(&mut inventory, &mut managed, "same", &remotes);
    assert_eq!(inventory.hosts.len(), 3);
    assert_eq!(
        inventory.hosts.iter().filter(|host| host.is_local).count(),
        1
    );
    for id in ["current-daemon", "older-daemon"] {
        let host = inventory
            .hosts
            .iter()
            .find(|host| host.host_id.as_deref() == Some(id))
            .unwrap();
        assert!(!host.is_local);
        assert_eq!(
            host.availability,
            zaplex_cockpit::HostAvailability::Available
        );
    }
}

#[test]
fn connection_events_preserve_inventory_for_other_live_hosts() {
    let mut existing = remote_host(
        "same",
        "host-a",
        session("agent-a", zaplex_cockpit::SessionState::Waiting),
    );
    existing.needs_me = 1;
    existing.projects[0].needs_me = 1;
    let mut inventory = fold_inventory(
        "laptop",
        vec![session("local-agent", zaplex_cockpit::SessionState::Active)],
        Vec::new(),
    );
    let local_before = inventory.hosts[0].clone();
    inventory.hosts.push(existing.clone());
    inventory.needs_me = 1;
    let mut managed = ManagedFleetInventory::default();
    reconcile_live_daemon_roots(
        &mut inventory,
        &mut managed,
        "laptop",
        &[
            connected_root("same", "host-a", None),
            connected_root("same", "host-b", None),
        ],
    );
    assert!(inventory.hosts.contains(&local_before));
    assert!(inventory.hosts.contains(&existing));
    assert_eq!(inventory.needs_me, 1);
    let new_host = inventory
        .hosts
        .iter()
        .find(|host| host.host_id.as_deref() == Some("host-b"))
        .unwrap();
    assert!(new_host.projects.is_empty());
}

#[test]
fn changed_registry_binding_is_unverified_until_current_registry_read() {
    let mut existing = remote_host(
        "remote",
        "host-a",
        session("agent-a", zaplex_cockpit::SessionState::Waiting),
    );
    existing.registry_node_id = Some("old-node".into());
    existing.needs_me = 1;
    let mut inventory = FleetTree {
        hosts: vec![existing],
        needs_me: 1,
    };
    let mut managed = ManagedFleetInventory::default();
    reconcile_live_daemon_roots(
        &mut inventory,
        &mut managed,
        "laptop",
        &[connected_root("remote", "host-a", Some("new-node"))],
    );
    let host = inventory.hosts.iter().find(|host| !host.is_local).unwrap();
    assert_eq!(host.registry_node_id.as_deref(), Some("new-node"));
    assert_eq!(
        host.availability,
        zaplex_cockpit::HostAvailability::Unverified
    );
    assert_eq!(inventory.needs_me, 0);
}

#[test]
fn last_disconnect_removes_host_and_managed_actions_before_scan_finishes() {
    use remote_server::proto::{ManagedSessionInfo, SessionInfo, SessionList};
    let mut inventory = FleetTree::default();
    let mut managed = ManagedFleetInventory::default();
    let remotes = [
        connected_root("remote", "host-a", None),
        connected_root("remote", "host-b", None),
    ];
    reconcile_live_daemon_roots(&mut inventory, &mut managed, "laptop", &remotes);
    for host_id in ["host-a", "host-b"] {
        managed.extend_session_list(
            host_id,
            "remote",
            None,
            SessionList {
                sessions: vec![SessionInfo {
                    session_id: "pty-1".into(),
                    generation: 1,
                    managed: Some(ManagedSessionInfo {
                        schema_version: 1,
                        provider: "claude".into(),
                        account_id: "account-a".into(),
                        project_root: "/work/project".into(),
                        launch_kind: "interactive-agent".into(),
                        launch_id: "launch-a".into(),
                        generation: 1,
                    }),
                    ..Default::default()
                }],
                ..Default::default()
            },
        );
    }
    assert_eq!(managed.sessions().len(), 2);
    reconcile_live_daemon_roots(&mut inventory, &mut managed, "laptop", &remotes[1..]);
    assert_eq!(inventory.hosts.len(), 2);
    assert_eq!(managed.sessions().len(), 1);
    assert_eq!(managed.sessions()[0].host_id, "host-b");
    reconcile_live_daemon_roots(&mut inventory, &mut managed, "laptop", &[]);
    assert_eq!(inventory.hosts.len(), 1);
    assert!(inventory.hosts[0].is_local);
    assert!(managed.sessions().is_empty());
}

#[test]
fn model_refresh_ticks_preserve_active_scan_and_disable_invalidates_it() {
    warpui::App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        app.add_singleton_model(RemoteServerManager::new);
        // Reserve an active scan without starting disk/network work. Calls below
        // exercise the real model scheduling path while that scan is blocked.
        let model = app.add_model(|_| CockpitModel {
            snapshot: initial_snapshot(),
            refresh_flight: RefreshSingleFlight {
                generation: 7,
                running: true,
                rerun_requested: false,
            },
            inventory: fold_inventory("laptop", Vec::new(), Vec::new()),
            managed_fleet: ManagedFleetInventory::default(),
            pricing: PricingTable::default(),
            oauth_cache: OauthCache::default(),
            transcript_cache: TranscriptScanCache::default(),
            registry_hosts: None,
            overrides: AccountOverrides::default(),
            local_label: "laptop".into(),
            selected_account: None,
        });
        model.update(&mut app, |model, ctx| {
            for _tick in 0..8 {
                model.spawn_refresh(ctx);
            }
            assert!(should_apply_refresh_result(
                model.refresh_flight.generation,
                7
            ));
            assert!(model.refresh_flight.running);
            assert!(model.refresh_flight.rerun_requested);
        });
        CockpitSettings::handle(&app).update(&mut app, |settings, ctx| {
            settings.enabled.set_value(false, ctx).unwrap();
        });
        model.update(&mut app, |model, ctx| {
            model.spawn_refresh(ctx);
            assert!(!should_apply_refresh_result(
                model.refresh_flight.generation,
                7
            ));
            assert!(!model.refresh_flight.rerun_requested);
            assert!(model.inventory.hosts.is_empty());
        });
    });
}

#[test]
fn full_inventory_registry_fold_stays_stable_on_following_tick() {
    let mut remote = connected_root("daemon-label", "host-a", Some("node-a"));
    remote.inventory_status = AgentInventoryStatus::Ready;
    let mut inventory = fold_inventory(
        "laptop",
        vec![session("local-agent", zaplex_cockpit::SessionState::Active)],
        vec![(
            remote,
            vec![session(
                "remote-agent",
                zaplex_cockpit::SessionState::Waiting,
            )],
        )],
    );
    zaplex_cockpit::reconcile_connected_hosts(
        &mut inventory,
        &[RegisteredHost {
            node_id: "node-a".into(),
            label: "registry-label".into(),
            live_host_id: Some("host-a".into()),
        }],
    );
    let completed = inventory.clone();
    let mut managed = ManagedFleetInventory::default();
    assert!(!reconcile_live_daemon_roots(
        &mut inventory,
        &mut managed,
        "laptop",
        &[connected_root("daemon-label", "host-a", Some("node-a"))],
    ));
    assert_eq!(inventory, completed);
}

#[test]
fn removed_registry_host_does_not_invalidate_each_following_refresh() {
    let mut remote = connected_root("remote", "host-a", Some("deleted-node"));
    remote.inventory_status = AgentInventoryStatus::Ready;
    let mut inventory = fold_inventory(
        "laptop",
        Vec::new(),
        vec![(
            remote,
            vec![session("waiting", zaplex_cockpit::SessionState::Waiting)],
        )],
    );
    zaplex_cockpit::reconcile_connected_hosts(&mut inventory, &[]);
    let removed = inventory.hosts.iter().find(|host| !host.is_local).unwrap();
    assert_eq!(removed.availability, HostAvailability::Removed);
    assert_eq!(removed.registry_node_id, None);
    assert_eq!(inventory.needs_me, 0);
    let accepted = inventory.clone();
    let mut managed = ManagedFleetInventory::default();
    for _tick in 0..8 {
        assert!(!reconcile_live_daemon_roots(
            &mut inventory,
            &mut managed,
            "laptop",
            &[connected_root("remote", "host-a", Some("deleted-node"))],
        ));
        assert_eq!(inventory, accepted);
    }
}
