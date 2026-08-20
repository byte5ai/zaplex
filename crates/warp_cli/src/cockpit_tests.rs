use chrono::{TimeZone as _, Utc};
use serde_json::json;
use zaplex_cockpit::{
    Account, AccountStatus, AccountUsage, AgentInventoryStatus, CockpitSnapshot, FleetTree,
    HostAvailability, HostNode, ProjectNode, Provider, ScanHealth, SessionSnapshot, SessionState,
    TaskItem, TaskState, TaskStatus, UsageProvenance, WindowTotals,
};

use crate::control::ControlAuth;

use super::{
    COCKPIT_SNAPSHOT_PROTOCOL_VERSION, COCKPIT_SNAPSHOT_SCHEMA_VERSION, CockpitSnapshotDocument,
    CockpitSnapshotRequest, EXIT_HARD_ERROR, EXIT_PARTIAL, EXIT_SUCCESS, SnapshotStatus,
    SourceStatus,
};

fn generated_at() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 20, 10, 0, 0).unwrap()
}

fn session(id: &str, state: SessionState) -> SessionSnapshot {
    SessionSnapshot {
        session_id: id.to_string(),
        cwd: "/home/tester/private/project".to_string(),
        name: String::new(),
        state,
        provider: Provider::Claude,
        model: String::new(),
        effort: None,
        ctx_tokens: 0,
        project_root: "/home/tester/private/project".to_string(),
        repo_root: "/home/tester/private/project".to_string(),
        project_name: "project".to_string(),
        branch: Some("main".to_string()),
        worktree: None,
        config_dir: Some("/home/tester/.claude-secret".to_string()),
        account_email: Some("owner@example.test".to_string()),
        account_id: None,
        process_fingerprint: Some("private-process-fingerprint".to_string()),
        pty_session_id: Some("private-pty-id".to_string()),
        pty_session_generation: Some(7),
        pty_foreground: true,
        task_state: Some(TaskState {
            tasks: vec![TaskItem {
                id: "task-1".to_string(),
                title: "private transcript task title".to_string(),
                status: TaskStatus::InProgress,
            }],
        }),
        last_activity: generated_at(),
        pid: 4242,
    }
}

fn account_with(sessions: Vec<SessionSnapshot>) -> AccountUsage {
    account_at("/home/tester/.claude-secret", sessions)
}

fn account_at(config_dir: &str, sessions: Vec<SessionSnapshot>) -> AccountUsage {
    AccountUsage {
        account: Account {
            provider: Provider::Claude,
            key: "claude:duplicate-basename".to_string(),
            config_dir: config_dir.into(),
            label: "Claude account".to_string(),
            email: Some("owner@example.test".to_string()),
            org: None,
            role: None,
            plan_tier: Some("Max".to_string()),
            is_default: true,
        },
        block5h: WindowTotals::default(),
        today: WindowTotals::default(),
        today_by_session: Default::default(),
        week: WindowTotals::default(),
        reset5h: None,
        reset_week: None,
        heat: 0.0,
        heat_week: 0.0,
        heat_opus: None,
        heat_sonnet: None,
        sessions,
        idle_sessions: Vec::new(),
        status: AccountStatus::Live,
        provenance: UsageProvenance::Estimate,
    }
}

fn host(
    label: &str,
    is_local: bool,
    host_id: Option<&str>,
    inventory_status: AgentInventoryStatus,
    sessions: Vec<SessionSnapshot>,
) -> HostNode {
    HostNode {
        host: label.to_string(),
        is_local,
        host_id: host_id.map(str::to_string),
        availability: HostAvailability::Available,
        inventory_status,
        registry_node_id: None,
        needs_me: sessions
            .iter()
            .filter(|session| session.state == SessionState::Waiting)
            .count(),
        projects: vec![ProjectNode {
            root: "/private/root".to_string(),
            name: "project".to_string(),
            needs_me: 0,
            sessions,
        }],
    }
}

#[test]
fn hard_error_is_machine_readable_and_uses_failure_exit_code() {
    let document = CockpitSnapshotDocument::hard_error(
        generated_at(),
        "The current user's home directory could not be resolved".to_string(),
    );

    assert_eq!(document.status, SnapshotStatus::Error);
    assert_eq!(document.sources.local.status, SourceStatus::Error);
    assert_eq!(document.exit_code(), EXIT_HARD_ERROR);
    assert!(serde_json::to_string(&document).is_ok());
}

#[test]
fn empty_standalone_snapshot_has_stable_versioned_schema_and_partial_status() {
    let document = CockpitSnapshotDocument::from_local(CockpitSnapshot {
        accounts: Vec::new(),
        generated_at: generated_at(),
        health: ScanHealth::Loaded,
    });

    assert_eq!(document.schema_version, COCKPIT_SNAPSHOT_SCHEMA_VERSION);
    assert_eq!(document.status, SnapshotStatus::Degraded);
    assert_eq!(document.sources.local.status, SourceStatus::Loaded);
    assert_eq!(
        document.sources.remote_hosts.status,
        SourceStatus::Unavailable
    );
    assert_eq!(document.exit_code(), EXIT_PARTIAL);
    assert_eq!(
        serde_json::to_value(document).unwrap(),
        json!({
            "schema_version": 1,
            "generated_at": "2026-08-20T10:00:00Z",
            "status": "degraded",
            "sources": {
                "local": { "status": "loaded", "detail": null },
                "remote_hosts": {
                    "status": "unavailable",
                    "detail": "Connected remote hosts are available only from a running Zaplex surface"
                }
            },
            "accounts": [],
            "hosts": [{
                "id": "local",
                "label": "local",
                "kind": "local",
                "state": "connected",
                "session_ids": []
            }],
            "attention": []
        })
    );
}

#[test]
fn degraded_scan_uses_unknown_usage_and_omits_private_collection_data() {
    let document = CockpitSnapshotDocument::from_local(CockpitSnapshot {
        accounts: vec![account_with(vec![
            session("session-z", SessionState::Active),
            session("session-a", SessionState::Waiting),
        ])],
        generated_at: generated_at(),
        health: ScanHealth::Degraded("usage history unreadable".to_string()),
    });
    let encoded = serde_json::to_string(&document).unwrap();

    assert_eq!(document.accounts[0].usage_provenance, "unknown");
    assert_eq!(document.accounts[0].usage, None);
    assert!(!encoded.contains("/home/tester"));
    assert!(!encoded.contains("private-process-fingerprint"));
    assert!(!encoded.contains("private-pty-id"));
    assert!(!encoded.contains("private transcript task title"));
    assert_eq!(document.hosts[0].session_ids.len(), 2);
    assert_ne!(
        document.hosts[0].session_ids[0],
        document.hosts[0].session_ids[1]
    );
    assert_eq!(document.attention.len(), 1);
    assert!(document.attention[0].session_id.starts_with("session:"));
    assert_eq!(document.accounts[0].sessions[0].model, None);
    assert_eq!(document.accounts[0].sessions[0].context_tokens, None);
}

#[test]
fn account_ids_use_the_full_private_identity_without_exposing_it() {
    let document = CockpitSnapshotDocument::from_local(CockpitSnapshot {
        accounts: vec![
            account_at("/home/one/accounts/work", Vec::new()),
            account_at("/home/two/accounts/work", Vec::new()),
        ],
        generated_at: generated_at(),
        health: ScanHealth::Loaded,
    });
    let encoded = serde_json::to_string(&document).unwrap();

    assert_ne!(document.accounts[0].id, document.accounts[1].id);
    assert!(!encoded.contains("/home/one"));
    assert!(!encoded.contains("/home/two"));
}

#[test]
fn runtime_export_uses_loaded_fleet_and_unique_host_scoped_session_ids() {
    let snapshot = CockpitSnapshot {
        accounts: vec![account_with(Vec::new())],
        generated_at: generated_at(),
        health: ScanHealth::Loaded,
    };
    let local = session("shared-session", SessionState::Active);
    let mut remote = session("shared-session", SessionState::Waiting);
    remote.config_dir = Some("/remote/private/.claude".to_string());
    let fleet = FleetTree {
        hosts: vec![
            host(
                "this machine",
                true,
                None,
                AgentInventoryStatus::Ready,
                vec![local],
            ),
            host(
                "devhost",
                false,
                Some("daemon-stable-id"),
                AgentInventoryStatus::Ready,
                vec![remote],
            ),
        ],
        needs_me: 1,
    };

    let document = CockpitSnapshotDocument::from_runtime(&snapshot, &fleet);
    let encoded = serde_json::to_string(&document).unwrap();
    let remote_host = document
        .hosts
        .iter()
        .find(|host| host.kind == "remote")
        .unwrap();

    assert_eq!(document.status, SnapshotStatus::Loaded);
    assert_eq!(document.sources.remote_hosts.status, SourceStatus::Loaded);
    assert_eq!(document.exit_code(), EXIT_SUCCESS);
    assert_eq!(remote_host.label, "devhost");
    assert_eq!(document.accounts.len(), 1);
    assert!(document.accounts[0].usage.is_some());
    assert_eq!(document.accounts[0].status, "working");
    assert_eq!(document.accounts[0].sessions.len(), 2);
    assert_ne!(
        document.accounts[0].sessions[0].id,
        document.accounts[0].sessions[1].id
    );
    assert_eq!(document.attention.len(), 1);
    assert_eq!(document.attention[0].host_id, remote_host.id);
    assert!(!encoded.contains("/remote/private"));
}

#[test]
fn incomplete_connected_host_keeps_its_root_and_degrades_exit() {
    let snapshot = CockpitSnapshot {
        accounts: Vec::new(),
        generated_at: generated_at(),
        health: ScanHealth::Loaded,
    };
    let fleet = FleetTree {
        hosts: vec![host(
            "old-daemon",
            false,
            Some("old-daemon-id"),
            AgentInventoryStatus::Unsupported,
            Vec::new(),
        )],
        needs_me: 0,
    };

    let document = CockpitSnapshotDocument::from_runtime(&snapshot, &fleet);

    assert_eq!(document.status, SnapshotStatus::Degraded);
    assert_eq!(document.sources.remote_hosts.status, SourceStatus::Degraded);
    assert_eq!(document.exit_code(), EXIT_PARTIAL);
    assert!(
        document
            .hosts
            .iter()
            .any(|host| host.label == "old-daemon" && host.state == "unsupported")
    );
}

#[test]
fn snapshot_request_is_versioned_and_revalidates_auth() {
    let request = CockpitSnapshotRequest::new(ControlAuth {
        token: "secret".to_string(),
        caller_surface_id: "surface".to_string(),
        caller_tab_id: "tab".to_string(),
    })
    .unwrap();

    assert_eq!(request.version, COCKPIT_SNAPSHOT_PROTOCOL_VERSION);
    assert!(request.validate().is_ok());
    assert!(serde_json::to_string(&request).is_ok());
}
