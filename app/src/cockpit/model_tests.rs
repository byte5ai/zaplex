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
        "remote",
        "host-remote",
        session("remote-session", zaplex_cockpit::SessionState::Active),
    );
    // A full scan starts while the remote host is still connected.
    let mut flight = RefreshSingleFlight::default();
    assert!(flight.request());
    let in_flight = flight.generation;
    let mut visible = FleetTree {
        hosts: vec![local, remote],
        needs_me: 0,
    };

    // The final session to that host closes before the scan returns. The
    // disconnect is a topology change, which the model answers by invalidating
    // the running generation and dropping the root synchronously.
    let disconnect = RemoteServerManagerEvent::HostDisconnected {
        host_id: warp_core::HostId::new("host-remote".to_string()),
    };
    assert_eq!(remote_refresh(&disconnect), RemoteRefresh::Topology);
    flight.invalidate();
    assert!(reconcile_live_daemon_roots(
        &mut visible,
        &mut ManagedFleetInventory::default(),
        "local",
        &[],
    ));

    // The late scan result belongs to the old topology and is discarded.
    assert!(
        !should_apply_refresh_result(flight.generation, in_flight),
        "a scan started before the disconnect must be ignored"
    );
    assert_eq!(visible.hosts.len(), 1);
    assert!(visible.hosts[0].is_local);
}

#[test]
fn connection_changes_invalidate_inflight_scans_but_inventory_changes_do_not() {
    let host_id = || warp_core::HostId::new("host-remote".to_string());
    assert_eq!(
        remote_refresh(&RemoteServerManagerEvent::HostConnected { host_id: host_id() }),
        RemoteRefresh::Topology,
        "a newly connected host must not be hidden by an older scan"
    );
    assert_eq!(
        remote_refresh(&RemoteServerManagerEvent::HostDisconnected { host_id: host_id() }),
        RemoteRefresh::Topology,
    );
    assert_eq!(
        remote_refresh(&RemoteServerManagerEvent::SessionInventoryChanged { host_id: host_id() }),
        RemoteRefresh::Inventory,
        "a managed Stop/Restart refreshes without discarding the running scan"
    );
}

#[test]
fn dormant_account_history_never_enters_local_tree() {
    let live = raw_row("live", zaplex_cockpit::SessionState::Active);
    let dormant = raw_row("dormant", zaplex_cockpit::SessionState::Idle);
    let account = zaplex_cockpit::AccountUsage {
        account: zaplex_cockpit::Account {
            provider: Provider::Claude,
            key: "claude".to_string(),
            config_dir: PathBuf::from("/accounts/claude"),
            label: "Claude".to_string(),
            provider_account_id: None,
            email: None,
            org: None,
            role: None,
            plan_tier: None,
            is_default: true,
        },
        block5h: zaplex_cockpit::WindowTotals::default(),
        today: zaplex_cockpit::WindowTotals::default(),
        today_by_session: Default::default(),
        week: zaplex_cockpit::WindowTotals::default(),
        reset5h: None,
        reset_week: None,
        heat: 0.0,
        heat_week: 0.0,
        heat_opus: None,
        heat_sonnet: None,
        sessions: vec![live],
        idle_sessions: vec![dormant],
        status: zaplex_cockpit::AccountStatus::Live,
        provenance: zaplex_cockpit::UsageProvenance::Estimate,
    };
    let resumable = raw_row("antigravity-resume", zaplex_cockpit::SessionState::Idle);

    let local = local_tree_sessions(std::slice::from_ref(&account), vec![resumable]);

    let ids: Vec<&str> = local
        .iter()
        .map(|session| session.session_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["live", "antigravity-resume"],
        "dormant Claude/Codex history stays in the account detail, not the live tree"
    );
    assert_eq!(account.idle_sessions.len(), 1, "the history itself is kept");
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

/// Fixture rows model the published inventory: a Waiting row carries the
/// unseen-turn attention the projection stamps on openable rows.
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
        awaiting_input: false,
        turn_id: None,
        attention: (state == zaplex_cockpit::SessionState::Waiting)
            .then_some(Attention::UnseenTurn),
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
    new_personal.attention = Some(Attention::UnseenTurn);
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
        let model = app.add_model(|ctx| {
            CockpitModel::subscribe_to_settings(ctx);
            CockpitModel {
                raw_snapshot: initial_snapshot(),
                snapshot: initial_snapshot(),
                refresh_flight: RefreshSingleFlight {
                    generation: 7,
                    running: true,
                    rerun_requested: false,
                },
                raw_inventory: fold_inventory("laptop", Vec::new(), Vec::new()),
                inventory: fold_inventory("laptop", Vec::new(), Vec::new()),
                seen_turns: SeenTurns::new(Utc::now()),
                local_terminal_links: HashMap::new(),
                focused_terminals: HashMap::new(),
                daemon_route_cache: HashMap::new(),
                managed_fleet: ManagedFleetInventory::default(),
                pricing: PricingTable::default(),
                oauth_cache: OauthCache::default(),
                transcript_cache: TranscriptScanCache::default(),
                registry_hosts: None,
                overrides: AccountOverrides::default(),
                local_label: "laptop".into(),
                selected_account: None,
            }
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
        model.update(&mut app, |model, _ctx| {
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

#[test]
fn partial_local_refresh_preserves_pending_remote_roots_and_their_attention_counts() {
    let remote_a = connected_root("same-label", "host-a", None);
    let remote_b = connected_root("same-label", "host-b", None);
    let mut visible = fold_inventory(
        "laptop",
        vec![session("old-local", zaplex_cockpit::SessionState::Active)],
        vec![
            (
                remote_a,
                vec![session("remote-a", zaplex_cockpit::SessionState::Waiting)],
            ),
            (
                remote_b,
                vec![session("remote-b", zaplex_cockpit::SessionState::Active)],
            ),
        ],
    );
    let previous_remotes: Vec<_> = visible
        .hosts
        .iter()
        .filter(|host| !host.is_local)
        .cloned()
        .collect();
    let updated = fold_inventory(
        "laptop",
        vec![session("new-local", zaplex_cockpit::SessionState::Waiting)],
        Vec::new(),
    );
    let expected_local = updated
        .hosts
        .iter()
        .find(|host| host.is_local)
        .unwrap()
        .clone();

    replace_inventory_roots(&mut visible, updated);

    assert_eq!(
        visible.hosts.iter().find(|host| host.is_local),
        Some(&expected_local)
    );
    assert_eq!(
        visible
            .hosts
            .iter()
            .filter(|host| !host.is_local)
            .cloned()
            .collect::<Vec<_>>(),
        previous_remotes
    );
    assert_eq!(visible.needs_me, 2);
}

#[test]
fn failed_partial_host_refresh_clears_only_that_exact_host() {
    let mut visible = fold_inventory(
        "laptop",
        vec![session("local", zaplex_cockpit::SessionState::Active)],
        vec![
            (
                connected_root("same-label", "host-a", None),
                vec![session("a", zaplex_cockpit::SessionState::Waiting)],
            ),
            (
                connected_root("same-label", "host-b", None),
                vec![session("b", zaplex_cockpit::SessionState::Waiting)],
            ),
        ],
    );
    let untouched: Vec<_> = visible
        .hosts
        .iter()
        .filter(|host| host.host_id.as_deref() != Some("host-a"))
        .cloned()
        .collect();
    let mut failed = connected_root("same-label", "host-a", None);
    failed.inventory_status = AgentInventoryStatus::Unavailable;
    let mut completed = fold_inventory("", Vec::new(), vec![(failed, Vec::new())]);
    completed.hosts.retain(|host| !host.is_local);

    replace_inventory_roots(&mut visible, completed);

    let failed = visible
        .hosts
        .iter()
        .find(|host| host.host_id.as_deref() == Some("host-a"))
        .unwrap();
    assert_eq!(failed.inventory_status, AgentInventoryStatus::Unavailable);
    assert!(failed.projects.is_empty());
    assert_eq!(
        visible
            .hosts
            .iter()
            .filter(|host| host.host_id.as_deref() != Some("host-a"))
            .cloned()
            .collect::<Vec<_>>(),
        untouched
    );
    assert_eq!(visible.needs_me, 1);
}

#[test]
fn obsolete_partial_refresh_cannot_restore_a_disconnected_root() {
    let mut flight = RefreshSingleFlight::default();
    assert!(flight.request());
    let completed_generation = flight.generation;
    let mut visible = fold_inventory(
        "laptop",
        Vec::new(),
        vec![
            (
                connected_root("same-label", "host-a", None),
                vec![session("a", zaplex_cockpit::SessionState::Waiting)],
            ),
            (
                connected_root("same-label", "host-b", None),
                vec![session("b", zaplex_cockpit::SessionState::Active)],
            ),
        ],
    );
    let completed = FleetTree {
        hosts: visible
            .hosts
            .iter()
            .filter(|host| host.host_id.as_deref() == Some("host-a"))
            .cloned()
            .collect(),
        needs_me: 1,
    };
    assert!(reconcile_live_daemon_roots(
        &mut visible,
        &mut ManagedFleetInventory::default(),
        "laptop",
        &[connected_root("same-label", "host-b", None)],
    ));
    flight.invalidate();
    assert!(!flight.request());
    let after_disconnect = visible.clone();
    if should_apply_refresh_result(flight.generation, completed_generation) {
        replace_inventory_roots(&mut visible, completed);
    }
    assert_eq!(visible, after_disconnect);
    assert!(
        flight.finish(),
        "one refresh must remain queued for the new topology"
    );
}

#[cfg(not(target_family = "wasm"))]
#[tokio::test]
async fn remote_refresh_publishes_a_healthy_host_while_another_rpc_hangs() {
    use crate::remote_server::client::RemoteServerClient;
    use crate::remote_server::proto::{
        client_message, server_message, AgentSessionInfo, AgentSessionList, ServerMessage,
    };
    use crate::remote_server::protocol;
    use futures::FutureExt as _;
    use std::sync::Arc;
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
    use warpui::r#async::executor;

    let executor = executor::Background::default();
    let mut daemons = Vec::new();
    let mut servers = Vec::new();
    let (seen_sender, mut seen_receiver) = tokio::sync::mpsc::unbounded_channel();
    for (host_id, respond) in [("hung-host", false), ("healthy-host", true)] {
        let (client_stream, server_stream) = tokio::io::duplex(4096);
        let (client_read, client_write) = tokio::io::split(client_stream);
        let (server_read, server_write) = tokio::io::split(server_stream);
        let (client, _events) = RemoteServerClient::new(
            TokioAsyncReadCompatExt::compat(client_read),
            client_write.compat_write(),
            &executor,
        );
        daemons.push(ConnectedDaemon {
            host_label: "same-label".to_string(),
            host_id: host_id.to_string(),
            registry_node_id: None,
            daemon_runtime: None,
            client: Arc::new(client),
            features: vec![FEATURE_AGENT_INVENTORY.to_string()],
        });
        let seen_sender = seen_sender.clone();
        servers.push(tokio::spawn(async move {
            let mut reader = TokioAsyncReadCompatExt::compat(server_read);
            let mut writer = server_write.compat_write();
            let request = protocol::read_client_message(&mut reader).await.unwrap();
            assert!(matches!(
                request.message,
                Some(client_message::Message::ListAgentSessions(_))
            ));
            seen_sender.send(host_id).unwrap();
            if respond {
                protocol::write_server_message(
                    &mut writer,
                    &ServerMessage {
                        request_id: request.request_id,
                        message: Some(server_message::Message::AgentSessionList(
                            AgentSessionList {
                                sessions: vec![AgentSessionInfo {
                                    session_id: "fresh-session".to_string(),
                                    provider: "codex".to_string(),
                                    state: "active".to_string(),
                                    cwd: "/work/project".to_string(),
                                    project_root: "/work/project".to_string(),
                                    effort: "high".to_string(),
                                    ..Default::default()
                                }],
                                ..Default::default()
                            },
                        )),
                    },
                )
                .await
                .unwrap();
            }
            // Keep both transports open. The hung peer never answers its RPC.
            std::future::pending::<()>().await;
        }));
    }
    let mut pending: FuturesUnordered<_> = daemons.into_iter().map(refresh_remote_host).collect();
    let ((host, sessions), fleet) = tokio::time::timeout(Duration::from_secs(2), pending.next())
        .await
        .expect("healthy host must finish independently")
        .unwrap();
    assert_eq!(host.host_id, "healthy-host");
    assert_eq!(host.inventory_status, AgentInventoryStatus::Ready);
    assert_eq!(sessions[0].session_id, "fresh-session");
    assert!(fleet.sessions().is_empty());
    let mut seen = Vec::new();
    for _request in 0..2 {
        seen.push(
            tokio::time::timeout(Duration::from_secs(2), seen_receiver.recv())
                .await
                .unwrap()
                .unwrap(),
        );
    }
    seen.sort_unstable();
    assert_eq!(seen, vec!["healthy-host", "hung-host"]);
    assert!(pending.next().now_or_never().is_none());

    // A disconnect completes only the stalled host with an honest empty error result.
    servers[0].abort();
    let ((failed, sessions), fleet) = tokio::time::timeout(Duration::from_secs(2), pending.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.host_id, "hung-host");
    assert_eq!(failed.inventory_status, AgentInventoryStatus::Unavailable);
    assert!(sessions.is_empty());
    assert!(fleet.sessions().is_empty());
    servers[1].abort();
}

#[test]
fn local_scan_failure_cannot_become_an_authoritative_empty_inventory() {
    let remote = connected_root("remote", "host-a", None);
    let mut inventory = fold_inventory("local", Vec::new(), vec![(remote, Vec::new())]);
    let remote_before = inventory
        .hosts
        .iter()
        .find(|host| !host.is_local)
        .unwrap()
        .clone();
    for (health, expected) in [
        (ScanHealth::Pending, AgentInventoryStatus::Pending),
        (
            ScanHealth::Degraded("unreadable account".into()),
            AgentInventoryStatus::Unavailable,
        ),
        (ScanHealth::Loaded, AgentInventoryStatus::Ready),
    ] {
        apply_local_scan_health(&mut inventory, &health);
        assert_eq!(
            inventory
                .hosts
                .iter()
                .find(|host| host.is_local)
                .unwrap()
                .inventory_status,
            expected
        );
        assert_eq!(
            inventory.hosts.iter().find(|host| !host.is_local).unwrap(),
            &remote_before
        );
    }
}

// ── Attention projection (owner decisions on the attention indicator) ───────

fn raw_row(id: &str, state: zaplex_cockpit::SessionState) -> SessionSnapshot {
    let mut row = session(id, state);
    row.attention = None;
    row.last_activity = Utc::now();
    row
}

fn facts_for(
    tree: &FleetTree,
    mut decide: impl FnMut(&SessionSnapshot) -> RowFacts,
) -> HashMap<String, RowFacts> {
    tree.hosts
        .iter()
        .flat_map(|host| {
            host.projects
                .iter()
                .flat_map(|project| &project.sessions)
                .map(move |session| (host, session))
        })
        .map(|(host, session)| {
            (
                session_key(host.is_local, host.host_id.as_deref(), session),
                decide(session),
            )
        })
        .collect()
}

fn reach(reach: SessionReach) -> RowFacts {
    RowFacts {
        reach,
        zaplex_owned: false,
        origin_unknown: false,
        hook_blocked: false,
    }
}

fn all_rows(tree: &FleetTree) -> Vec<&SessionSnapshot> {
    tree.hosts
        .iter()
        .flat_map(|host| host.projects.iter().flat_map(|project| &project.sessions))
        .collect()
}

/// An external agent Zaplex can neither focus, reattach nor resume is not shown
/// anywhere: not in the tree, not in the count (pulse/Dock/sidebar), and not
/// in the inbox or jump order — even with an unseen finished turn.
#[test]
fn external_unreachable_session_is_excluded_from_tree_count_and_inbox() {
    use zaplex_cockpit::SessionState;
    let raw = fold_inventory(
        "laptop",
        vec![
            raw_row("external", SessionState::Waiting),
            raw_row("in-zaplex", SessionState::Waiting),
        ],
        Vec::new(),
    );
    let pane = EntityId::new();
    let facts = facts_for(&raw, |session| match session.session_id.as_str() {
        "external" => reach(SessionReach::Unreachable),
        _ => reach(SessionReach::Terminal(pane)),
    });
    let mut seen = SeenTurns::new(Utc::now() - chrono::Duration::hours(1));

    let projected = project_inventory_attention(&raw, &facts, None, &mut seen);

    let ids: Vec<&str> = all_rows(&projected)
        .into_iter()
        .map(|session| session.session_id.as_str())
        .collect();
    assert_eq!(ids, vec!["in-zaplex"]);
    assert_eq!(projected.needs_me, 1);
    let inbox: Vec<&str> = zaplex_cockpit::waiting_sessions(&projected)
        .into_iter()
        .map(|(_, session)| session.session_id.as_str())
        .collect();
    assert_eq!(inbox, vec!["in-zaplex"]);
}

/// A Zaplex-launched agent whose daemon PTY is momentarily not reattachable
/// (reconnect, foreground handover) stays visible instead of flickering away,
/// but is not counted until a click can actually open it.
#[test]
fn zaplex_owned_session_stays_visible_while_momentarily_unreachable() {
    use zaplex_cockpit::SessionState;
    let mut owned = raw_row("owned", SessionState::Waiting);
    owned.pty_session_id = Some("pty-1".to_string());
    owned.pty_session_generation = Some(3);
    let mut raw = fold_inventory(
        "laptop",
        Vec::new(),
        vec![(connected_root("devhost", "daemon-1", None), vec![owned])],
    );
    for host in &mut raw.hosts {
        host.inventory_status = AgentInventoryStatus::Ready;
    }
    let facts = facts_for(&raw, |session| RowFacts {
        reach: SessionReach::Unreachable,
        zaplex_owned: capabilities::zaplex_owned(session, false),
        origin_unknown: false,
        hook_blocked: false,
    });
    let mut seen = SeenTurns::new(Utc::now() - chrono::Duration::hours(1));

    let projected = project_inventory_attention(&raw, &facts, None, &mut seen);

    let rows = all_rows(&projected);
    assert_eq!(rows.len(), 1, "the owned row survives the reconnect");
    assert_eq!(rows[0].attention, None);
    assert_eq!(projected.needs_me, 0);
    assert_eq!(zaplex_cockpit::next_waiting(&projected, None), None);
}

/// Looking at an agent's pane (the active window's focused terminal) marks its
/// finished turn as seen; the agent's next finished turn counts again.
#[test]
fn viewing_the_pane_clears_the_unseen_turn_and_a_new_turn_counts_again() {
    use zaplex_cockpit::SessionState;
    let pane = EntityId::new();
    let mut done = raw_row("agent", SessionState::Waiting);
    done.turn_id = Some("turn-1".to_string());
    let raw = fold_inventory("laptop", vec![done.clone()], Vec::new());
    let facts = facts_for(&raw, |_| reach(SessionReach::Terminal(pane)));
    let mut seen = SeenTurns::new(Utc::now() - chrono::Duration::hours(1));

    assert_eq!(
        project_inventory_attention(&raw, &facts, None, &mut seen).needs_me,
        1,
        "an unseen finished turn counts"
    );
    assert_eq!(
        project_inventory_attention(&raw, &facts, Some(pane), &mut seen).needs_me,
        0,
        "viewing its pane clears it"
    );
    assert_eq!(
        project_inventory_attention(&raw, &facts, None, &mut seen).needs_me,
        0,
        "an idle, seen session stays quiet after the user looks away"
    );

    let mut next_turn = done;
    next_turn.turn_id = Some("turn-2".to_string());
    let raw = fold_inventory("laptop", vec![next_turn], Vec::new());
    let facts = facts_for(&raw, |_| reach(SessionReach::Terminal(pane)));
    assert_eq!(
        project_inventory_attention(&raw, &facts, None, &mut seen).needs_me,
        1,
        "the next finished turn is unread again"
    );
}

/// A hook-reported permission prompt in a Zaplex terminal counts even though
/// discovery still reads a tool run, and viewing does not clear it.
#[test]
fn hook_blocked_terminal_counts_until_answered_even_while_viewed() {
    use zaplex_cockpit::SessionState;
    let pane = EntityId::new();
    let raw = fold_inventory(
        "laptop",
        vec![raw_row("asking", SessionState::Monitor)],
        Vec::new(),
    );
    let facts = facts_for(&raw, |_| RowFacts {
        reach: SessionReach::Terminal(pane),
        zaplex_owned: true,
        origin_unknown: false,
        hook_blocked: true,
    });
    let mut seen = SeenTurns::new(Utc::now());

    let projected = project_inventory_attention(&raw, &facts, Some(pane), &mut seen);
    assert_eq!(projected.needs_me, 1);
    assert_eq!(
        all_rows(&projected)[0].presented_state(),
        SessionState::Waiting
    );
}

/// Startup baseline: a turn that finished before the app started is history,
/// not unread mail.
#[test]
fn startup_does_not_count_turns_finished_before_the_app_started() {
    use zaplex_cockpit::SessionState;
    let started = Utc::now();
    let mut historical = raw_row("old", SessionState::Waiting);
    historical.last_activity = started - chrono::Duration::hours(3);
    historical.turn_id = Some("old-turn".to_string());
    let raw = fold_inventory("laptop", vec![historical], Vec::new());
    let facts = facts_for(&raw, |_| reach(SessionReach::Terminal(EntityId::new())));
    let mut seen = SeenTurns::new(started);

    let projected = project_inventory_attention(&raw, &facts, None, &mut seen);
    assert_eq!(projected.needs_me, 0);
    assert_eq!(all_rows(&projected).len(), 1, "still listed, just quiet");
}

/// The projection's verdict reaches the account snapshot rows the dashboard
/// reads, so the same session never shows attention on one surface only.
#[test]
fn account_rows_hide_external_sessions_and_carry_the_projected_attention() {
    use zaplex_cockpit::SessionState;
    let row = raw_row("agent", SessionState::Waiting);
    let external = raw_row("external", SessionState::Waiting);
    let raw = fold_inventory("laptop", vec![row.clone(), external.clone()], Vec::new());
    let pane = EntityId::new();
    let facts = facts_for(&raw, |session| match session.session_id.as_str() {
        "external" => reach(SessionReach::Unreachable),
        _ => reach(SessionReach::Terminal(pane)),
    });
    let mut seen = SeenTurns::new(Utc::now() - chrono::Duration::hours(1));
    let projected = project_inventory_attention(&raw, &facts, None, &mut seen);

    let mut snapshot = empty_snapshot();
    snapshot.accounts.push(zaplex_cockpit::AccountUsage {
        account: zaplex_cockpit::Account {
            provider: Provider::Claude,
            key: "claude".to_string(),
            config_dir: PathBuf::from("/home/me/.claude"),
            label: "Claude".to_string(),
            provider_account_id: None,
            email: None,
            org: None,
            role: None,
            plan_tier: None,
            is_default: true,
        },
        block5h: zaplex_cockpit::WindowTotals::default(),
        today: zaplex_cockpit::WindowTotals::default(),
        today_by_session: Default::default(),
        week: zaplex_cockpit::WindowTotals::default(),
        reset5h: None,
        reset_week: None,
        heat: 0.0,
        heat_week: 0.0,
        heat_opus: None,
        heat_sonnet: None,
        sessions: vec![row, external],
        idle_sessions: Vec::new(),
        status: zaplex_cockpit::AccountStatus::Live,
        provenance: zaplex_cockpit::UsageProvenance::Estimate,
    });
    snapshot.accounts[0].today.messages = 7;

    let published = published_snapshot(&snapshot, &projected);
    let rows: Vec<&str> = published.accounts[0]
        .sessions
        .iter()
        .map(|session| session.session_id.as_str())
        .collect();
    assert_eq!(rows, vec!["agent"], "no row for the external session");
    assert_eq!(
        published.accounts[0].sessions[0].attention,
        Some(Attention::UnseenTurn)
    );
    assert_eq!(
        published.accounts[0].today.messages, 7,
        "account usage still describes the whole account"
    );
    assert_eq!(snapshot.accounts[0].sessions.len(), 2, "discovery is kept");
}

/// Without any hook bridge, an agent started in a Zaplex pane inherits that
/// pane's surface id: it stays visible, is counted, and the pane is its click
/// target. A truly external agent (readable environment, no Zaplex surface)
/// stays hidden; one whose process cannot be inspected is never hidden.
#[test]
fn hookless_agent_in_a_zaplex_pane_is_visible_counted_and_openable() {
    use zaplex_cockpit::SessionState;
    let pane = EntityId::new();
    let raw = fold_inventory(
        "laptop",
        vec![
            raw_row("hookless", SessionState::Waiting),
            raw_row("external", SessionState::Waiting),
            raw_row("uninspectable", SessionState::Waiting),
        ],
        Vec::new(),
    );
    // Facts as `attention_row_facts` derives them from the process links: the
    // hookless agent's linked pane makes it reachable and owned.
    let facts = facts_for(&raw, |session| match session.session_id.as_str() {
        "hookless" => RowFacts {
            reach: capabilities::session_reach(session, true, Some(pane), false),
            zaplex_owned: capabilities::zaplex_owned(session, true),
            origin_unknown: false,
            hook_blocked: false,
        },
        "external" => reach(SessionReach::Unreachable),
        _ => RowFacts {
            reach: SessionReach::Unreachable,
            zaplex_owned: false,
            origin_unknown: true,
            hook_blocked: false,
        },
    });
    let mut seen = SeenTurns::new(Utc::now() - chrono::Duration::hours(1));

    let projected = project_inventory_attention(&raw, &facts, None, &mut seen);

    let mut ids: Vec<&str> = all_rows(&projected)
        .into_iter()
        .map(|session| session.session_id.as_str())
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec!["hookless", "uninspectable"]);
    assert_eq!(
        projected.needs_me, 1,
        "only the openable hookless agent counts"
    );
    let target = zaplex_cockpit::next_waiting(&projected, None).expect("a target");
    assert_eq!(target.session_id, "hookless");
}

/// The local process link is computed per discovered account row and keyed
/// exactly like the inventory rows the projection looks up.
#[test]
fn local_terminal_links_are_keyed_like_inventory_rows() {
    use zaplex_cockpit::SessionState;
    let mut codex = raw_row("codex-session", SessionState::Waiting);
    codex.provider = Provider::Codex;
    let mut snapshot = empty_snapshot();
    snapshot.accounts.push(zaplex_cockpit::AccountUsage {
        account: zaplex_cockpit::Account {
            provider: Provider::Codex,
            key: "codex".to_string(),
            config_dir: PathBuf::from("/home/me/.codex"),
            label: "Codex".to_string(),
            provider_account_id: None,
            email: None,
            org: None,
            role: None,
            plan_tier: None,
            is_default: true,
        },
        block5h: zaplex_cockpit::WindowTotals::default(),
        today: zaplex_cockpit::WindowTotals::default(),
        today_by_session: Default::default(),
        week: zaplex_cockpit::WindowTotals::default(),
        reset5h: None,
        reset_week: None,
        heat: 0.0,
        heat_week: 0.0,
        heat_opus: None,
        heat_sonnet: None,
        sessions: vec![codex.clone()],
        idle_sessions: Vec::new(),
        status: zaplex_cockpit::AccountStatus::Live,
        provenance: zaplex_cockpit::UsageProvenance::Estimate,
    });

    let links = local_terminal_links(&snapshot);
    let raw = fold_inventory("laptop", vec![codex], Vec::new());
    let row = all_rows(&raw)[0];
    // No process holds this fixture's rollout: no evidence, never "external".
    assert_eq!(
        links.get(&session_key(true, None, row)),
        Some(&TerminalLink::Unknown)
    );
}
