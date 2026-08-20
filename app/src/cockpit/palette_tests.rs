use super::*;
use chrono::Utc;
use zaplex_cockpit::{
    Account, AccountStatus, AccountUsage, HostAvailability, HostNode, ProjectNode, UsageProvenance,
    WindowTotals,
};

fn session(
    id: &str,
    state: SessionState,
    provider: Provider,
    config_dir: Option<&str>,
    email: Option<&str>,
) -> SessionSnapshot {
    SessionSnapshot {
        session_id: id.to_string(),
        cwd: "/projects/zaplex".to_string(),
        name: "session".to_string(),
        state,
        provider,
        model: "model".to_string(),
        effort: None,
        ctx_tokens: 0,
        project_root: "/projects/zaplex".to_string(),
        repo_root: "/projects/zaplex".to_string(),
        project_name: "zaplex".to_string(),
        branch: Some("feat/palette".to_string()),
        worktree: Some("palette".to_string()),
        config_dir: config_dir.map(str::to_owned),
        account_email: email.map(str::to_owned),
        account_id: None,
        process_fingerprint: None,
        pty_session_id: None,
        pty_session_generation: None,
        pty_foreground: false,
        task_state: None,
        last_activity: Utc::now(),
        pid: if state == SessionState::Idle { 0 } else { 10 },
    }
}

fn account(
    key: &str,
    label: &str,
    provider: Provider,
    config_dir: &str,
    email: &str,
    idle_sessions: Vec<SessionSnapshot>,
) -> AccountUsage {
    AccountUsage {
        account: Account {
            provider,
            key: key.to_string(),
            config_dir: config_dir.into(),
            label: label.to_string(),
            email: Some(email.to_string()),
            org: None,
            role: None,
            plan_tier: Some("Max".to_string()),
            is_default: false,
        },
        block5h: WindowTotals::default(),
        today: WindowTotals::default(),
        today_by_session: Default::default(),
        week: WindowTotals::default(),
        reset5h: None,
        reset_week: None,
        heat: 0.,
        heat_week: 0.,
        heat_opus: None,
        heat_sonnet: None,
        sessions: Vec::new(),
        idle_sessions,
        status: AccountStatus::Offline,
        provenance: UsageProvenance::Real,
    }
}

fn snapshot(accounts: Vec<AccountUsage>) -> CockpitSnapshot {
    CockpitSnapshot {
        accounts,
        generated_at: Utc::now(),
        health: ScanHealth::Loaded,
    }
}

fn host(
    label: &str,
    host_id: Option<&str>,
    is_local: bool,
    availability: HostAvailability,
    inventory_status: AgentInventoryStatus,
    sessions: Vec<SessionSnapshot>,
) -> HostNode {
    HostNode {
        host: label.to_string(),
        is_local,
        host_id: host_id.map(str::to_owned),
        availability,
        inventory_status,
        registry_node_id: host_id.map(|id| format!("node-{id}")),
        needs_me: sessions
            .iter()
            .filter(|session| session.state == SessionState::Waiting)
            .count(),
        projects: vec![ProjectNode {
            root: "/projects/zaplex".to_string(),
            name: "zaplex".to_string(),
            needs_me: sessions
                .iter()
                .filter(|session| session.state == SessionState::Waiting)
                .count(),
            sessions,
        }],
    }
}

#[test]
fn duplicate_labels_and_session_ids_keep_stable_distinct_targets() {
    let local_session = session(
        "copied",
        SessionState::Waiting,
        Provider::Claude,
        Some("/secret/claude-a"),
        Some("a@example.com"),
    );
    let remote_session = session(
        "copied",
        SessionState::Active,
        Provider::Claude,
        Some("/remote/secret/claude-b"),
        Some("b@example.com"),
    );
    let snapshot = snapshot(vec![
        account(
            "claude:a",
            "Same label",
            Provider::Claude,
            "/secret/claude-a",
            "a@example.com",
            Vec::new(),
        ),
        account(
            "claude:b",
            "Same label",
            Provider::Claude,
            "/secret/claude-b",
            "b@example.com",
            Vec::new(),
        ),
    ]);
    let fleet = FleetTree {
        hosts: vec![
            host(
                "mac",
                None,
                true,
                HostAvailability::Available,
                AgentInventoryStatus::Ready,
                vec![local_session],
            ),
            host(
                "devhost",
                Some("daemon-b"),
                false,
                HostAvailability::Available,
                AgentInventoryStatus::Ready,
                vec![remote_session],
            ),
        ],
        needs_me: 1,
    };

    let records = build_palette_index(&snapshot, &fleet, None);
    let accounts: Vec<_> = records
        .iter()
        .filter(|record| record.kind == CockpitPaletteKind::Account)
        .collect();
    assert_eq!(accounts.len(), 2);
    assert_ne!(accounts[0].stable_key(), accounts[1].stable_key());
    let sessions: Vec<_> = records
        .iter()
        .filter(|record| record.kind == CockpitPaletteKind::Session)
        .collect();
    assert_eq!(sessions.len(), 2);
    assert_ne!(sessions[0].stable_key(), sessions[1].stable_key());
}

#[test]
fn config_directories_never_enter_search_or_accessibility_text() {
    let secret_path = "/Users/christian/private/.claude-account";
    let mut live = session(
        "session-1",
        SessionState::Active,
        Provider::Claude,
        Some(secret_path),
        Some("me@example.com"),
    );
    let worktree_path = "/Users/christian/private/worktrees/feature-one";
    live.name.clear();
    live.branch = None;
    live.worktree = Some(worktree_path.to_string());
    let snapshot = snapshot(vec![account(
        "claude:private",
        "Work",
        Provider::Claude,
        secret_path,
        "me@example.com",
        Vec::new(),
    )]);
    let fleet = FleetTree {
        hosts: vec![host(
            "mac",
            None,
            true,
            HostAvailability::Available,
            AgentInventoryStatus::Ready,
            vec![live],
        )],
        needs_me: 0,
    };
    for record in build_palette_index(&snapshot, &fleet, None) {
        assert!(!record.search_text.contains(secret_path));
        assert!(!record.primary.contains(secret_path));
        assert!(!record.secondary.contains(secret_path));
        assert!(!record.accessibility_label().contains(secret_path));
        assert!(!record.search_text.contains(worktree_path));
        assert!(!record.primary.contains(worktree_path));
    }
}

#[test]
fn dormant_duplicate_is_deduped_against_live_session() {
    let live = session(
        "same",
        SessionState::Active,
        Provider::Codex,
        Some("/secret/codex"),
        Some("me@example.com"),
    );
    let mut dormant = live.clone();
    dormant.state = SessionState::Idle;
    dormant.pid = 0;
    let snapshot = snapshot(vec![account(
        "codex:work",
        "Codex work",
        Provider::Codex,
        "/secret/codex",
        "me@example.com",
        vec![dormant],
    )]);
    let fleet = FleetTree {
        hosts: vec![host(
            "mac",
            None,
            true,
            HostAvailability::Available,
            AgentInventoryStatus::Ready,
            vec![live],
        )],
        needs_me: 0,
    };
    let sessions = build_palette_index(&snapshot, &fleet, None)
        .into_iter()
        .filter(|record| record.kind == CockpitPaletteKind::Session)
        .count();
    assert_eq!(sessions, 1);
}

#[test]
fn copied_dormant_session_ids_on_distinct_accounts_do_not_collapse() {
    let first = session(
        "copied",
        SessionState::Idle,
        Provider::Claude,
        Some("/secret/claude-a"),
        Some("a@example.com"),
    );
    let second = session(
        "copied",
        SessionState::Idle,
        Provider::Claude,
        Some("/secret/claude-b"),
        Some("b@example.com"),
    );
    let snapshot = snapshot(vec![
        account(
            "claude:a",
            "Work",
            Provider::Claude,
            "/secret/claude-a",
            "a@example.com",
            vec![first],
        ),
        account(
            "claude:b",
            "Personal",
            Provider::Claude,
            "/secret/claude-b",
            "b@example.com",
            vec![second],
        ),
    ]);
    let sessions: Vec<_> = build_palette_index(&snapshot, &FleetTree::default(), None)
        .into_iter()
        .filter(|record| record.kind == CockpitPaletteKind::Session)
        .collect();
    assert_eq!(sessions.len(), 2);
    assert_ne!(sessions[0].stable_key(), sessions[1].stable_key());
}

#[test]
fn removed_and_inventory_unavailable_hosts_do_not_expose_routing_children() {
    let remote_session = session(
        "remote",
        SessionState::Active,
        Provider::Claude,
        None,
        Some("me@example.com"),
    );
    let fleet = FleetTree {
        hosts: vec![
            host(
                "removed",
                Some("removed-id"),
                false,
                HostAvailability::Removed,
                AgentInventoryStatus::Ready,
                vec![remote_session.clone()],
            ),
            host(
                "degraded",
                Some("degraded-id"),
                false,
                HostAvailability::Available,
                AgentInventoryStatus::Unavailable,
                vec![remote_session],
            ),
        ],
        needs_me: 0,
    };
    let records = build_palette_index(&snapshot(Vec::new()), &fleet, None);
    assert!(records.iter().all(|record| record.primary != "removed"));
    assert!(records
        .iter()
        .any(|record| { record.kind == CockpitPaletteKind::Host && record.primary == "degraded" }));
    assert!(records.iter().all(|record| {
        !matches!(
            record.kind,
            CockpitPaletteKind::Project | CockpitPaletteKind::Session
        )
    }));
}

#[test]
fn github_flows_are_repo_scoped_and_use_stable_flow_keys() {
    let repository = RepositoryContext {
        slug: "byte5ai/zaplex".to_string(),
        worktree: "/projects/zaplex/.worktrees/feat".into(),
        display_label: "zaplex feat".to_string(),
    };
    let records = build_palette_index(
        &snapshot(Vec::new()),
        &FleetTree::default(),
        Some(&repository),
    );
    let flows: Vec<_> = records
        .iter()
        .filter(|record| record.kind == CockpitPaletteKind::GitHubFlow)
        .collect();
    assert_eq!(flows.len(), flow_keys().len());
    assert!(flows.iter().all(|record| {
        record.stable_key().contains("byte5ai/zaplex")
            && record
                .stable_key()
                .contains("/projects/zaplex/.worktrees/feat")
    }));
    assert!(
        build_palette_index(&snapshot(Vec::new()), &FleetTree::default(), None)
            .iter()
            .all(|record| record.kind != CockpitPaletteKind::GitHubFlow)
    );
}

#[test]
fn pending_snapshot_exposes_no_partially_loaded_results() {
    let snapshot = CockpitSnapshot {
        accounts: Vec::new(),
        generated_at: Utc::now(),
        health: ScanHealth::Pending,
    };
    assert!(build_palette_index(&snapshot, &FleetTree::default(), None).is_empty());
}
