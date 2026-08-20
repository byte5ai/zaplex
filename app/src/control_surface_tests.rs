use super::*;

fn context() -> ControlPtyContext {
    ControlPtyContext {
        socket: "/tmp/control.sock".to_string(),
        token: "fixed-secret".to_string(),
        surface_id: "surface-a".to_string(),
        tab_id: "tab-a".to_string(),
        codewhale_audit_log: None,
    }
}

fn auth(token: &str, surface: &str, tab: &str) -> ControlAuth {
    ControlAuth {
        token: token.to_string(),
        caller_surface_id: surface.to_string(),
        caller_tab_id: tab.to_string(),
    }
}

#[test]
fn token_is_bound_to_the_exact_surface_and_tab() {
    let context = context();
    assert!(context.authenticates(&auth("fixed-secret", "surface-a", "tab-a")));
    assert!(!context.authenticates(&auth("wrong", "surface-a", "tab-a")));
    assert!(!context.authenticates(&auth("fixed-secret", "surface-b", "tab-a")));
    assert!(!context.authenticates(&auth("fixed-secret", "surface-a", "tab-b")));
}

#[test]
fn control_context_injects_only_the_documented_trust_boundary() {
    let mut context = context();
    let mut env = HashMap::new();
    context.inject_env(&mut env);

    assert_eq!(env.len(), 4);
    assert_eq!(env[OsStr::new(CONTROL_SOCKET_ENV)], "/tmp/control.sock");
    assert_eq!(env[OsStr::new(CONTROL_TOKEN_ENV)], "fixed-secret");
    assert_eq!(env[OsStr::new(SURFACE_ID_ENV)], "surface-a");
    assert_eq!(env[OsStr::new(TAB_ID_ENV)], "tab-a");
}

#[test]
fn codewhale_audit_destination_is_pane_scoped_and_user_override_is_preserved() {
    let directory = tempfile::tempdir().unwrap();
    let pane_log = directory.path().join("pane.jsonl");
    std::fs::write(&pane_log, "").unwrap();
    let mut context = context();
    context.codewhale_audit_log = Some(Arc::new(CodeWhaleAuditLog {
        path: pane_log.clone(),
    }));
    let mut env = HashMap::new();

    context.inject_env(&mut env);

    assert_eq!(
        env[OsStr::new(CODEWHALE_TOOL_AUDIT_LOG_ENV)],
        pane_log.as_os_str()
    );
    assert_eq!(context.codewhale_audit_log_path(), Some(pane_log.as_path()));

    let user_log = directory.path().join("user.jsonl");
    env.insert(
        OsString::from(CODEWHALE_TOOL_AUDIT_LOG_ENV),
        user_log.as_os_str().to_os_string(),
    );
    context.inject_env(&mut env);

    assert_eq!(
        env[OsStr::new(CODEWHALE_TOOL_AUDIT_LOG_ENV)],
        user_log.as_os_str()
    );
    assert!(context.codewhale_audit_log_path().is_none());
}

#[test]
fn independently_created_surfaces_do_not_share_bearer_credentials() {
    let first = ControlPtyContext::new("tab-a".to_string());
    let second = ControlPtyContext::new("tab-a".to_string());

    assert_ne!(first.token, second.token);
    assert_ne!(first.surface_id, second.surface_id);
    assert_eq!(first.socket, second.socket);
}

#[cfg(unix)]
#[test]
fn control_endpoint_is_a_local_unix_socket_path() {
    let address = control_address();
    assert!(address.starts_with("/tmp/zplx-ctl-"));
    assert!(address.ends_with(".sock"));
    assert!(!address.contains("://"));
}

#[test]
fn worktree_directory_never_interprets_branch_as_a_path() {
    let repo = PathBuf::from("/repo");
    let name = worktree_directory_name(&repo, "../../feature/control surface");
    assert!(!name.contains('/'));
    assert!(!name.contains('\\'));
    assert!(!name.contains(".."));
    assert_eq!(
        name,
        worktree_directory_name(&repo, "../../feature/control surface")
    );
    assert_ne!(
        name,
        worktree_directory_name(&repo, "../feature/control surface")
    );
    assert_ne!(
        name,
        worktree_directory_name(
            &PathBuf::from("/other/repo"),
            "../../feature/control surface"
        )
    );
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn remote_account_results_require_the_exact_live_connection() {
    let original = Arc::new(());
    let replacement = Arc::new(());

    assert!(same_connection(
        "daemon-a", &original, "daemon-a", &original
    ));
    assert!(!same_connection(
        "daemon-a", &original, "daemon-b", &original
    ));
    assert!(!same_connection(
        "daemon-a",
        &original,
        "daemon-a",
        &replacement
    ));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn remote_account_projection_preserves_only_path_free_inventory_fields() {
    let inventory = AgentAccountInventory {
        schema_version: 1,
        accounts: vec![remote_server::proto::AgentAccountInfo {
            provider: "claude".to_string(),
            account_id: "opaque-account".to_string(),
            display_label: "Claude account".to_string(),
            email: "owner@example.test".to_string(),
            organization: "Example".to_string(),
            plan_tier: "Max".to_string(),
            is_default: true,
            capacity_5h: 0.75,
            capacity_week: 0.5,
            capacity_known: true,
            health: "loaded".to_string(),
            usage_provenance: "estimate".to_string(),
        }],
        health: "loaded".to_string(),
        health_message: "must not cross the export boundary".to_string(),
    };

    let projected = remote_account_inventory_snapshot(
        "daemon-a",
        &RemoteAccountFetchOutcome::Inventory(inventory),
    );

    assert_eq!(projected.host_id, "daemon-a");
    assert_eq!(projected.status, RemoteAccountInventoryStatus::Loaded);
    assert_eq!(projected.accounts.len(), 1);
    assert_eq!(projected.accounts[0].account_id, "opaque-account");
    assert_eq!(projected.accounts[0].capacity_5h, 0.75);
    assert!(!format!("{projected:?}").contains("must not cross"));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn unknown_remote_account_schema_is_invalid_not_empty_loaded() {
    let inventory = AgentAccountInventory {
        schema_version: 2,
        accounts: Vec::new(),
        health: "loaded".to_string(),
        health_message: String::new(),
    };

    let projected = remote_account_inventory_snapshot(
        "daemon-a",
        &RemoteAccountFetchOutcome::Inventory(inventory),
    );

    assert_eq!(projected.status, RemoteAccountInventoryStatus::Invalid);
}
