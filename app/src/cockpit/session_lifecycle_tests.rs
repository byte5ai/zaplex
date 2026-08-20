use chrono::Utc;
use zaplex_cockpit::{Provider, SessionSnapshot, SessionState};

use super::*;

fn snapshot(provider: Provider) -> SessionSnapshot {
    SessionSnapshot {
        session_id: "session-1".to_string(),
        cwd: "/work/zaplex".to_string(),
        name: "main".to_string(),
        state: SessionState::Active,
        provider,
        model: "opus".to_string(),
        effort: Some("high".to_string()),
        ctx_tokens: 0,
        project_root: "/work/zaplex".to_string(),
        repo_root: "/work/zaplex".to_string(),
        project_name: "zaplex".to_string(),
        branch: Some("main".to_string()),
        worktree: None,
        config_dir: Some("/accounts/work".to_string()),
        account_email: Some("work@example.com".to_string()),
        account_id: None,
        process_fingerprint: Some("process-1".to_string()),
        pty_session_id: None,
        pty_session_generation: None,
        pty_foreground: false,
        task_state: None,
        last_activity: Utc::now(),
        pid: 42,
    }
}

fn record(route: &SessionRoute) -> LaunchRecord {
    let (host, config_dir, account_email, account_id) = match (&route.host, &route.account) {
        (
            SessionHostRoute::Local,
            SessionAccountRoute::Local {
                config_dir,
                account_email,
            },
        ) => (
            None,
            config_dir
                .as_deref()
                .map(|path| path.to_string_lossy().into_owned()),
            account_email.clone(),
            None,
        ),
        (
            SessionHostRoute::Remote { host_id, .. },
            SessionAccountRoute::Remote {
                account_id,
                account_email,
            },
        ) => (
            Some(host_id.clone()),
            None,
            account_email.clone(),
            Some(account_id.clone()),
        ),
        (SessionHostRoute::Local, SessionAccountRoute::Remote { .. })
        | (SessionHostRoute::Remote { .. }, SessionAccountRoute::Local { .. }) => {
            unreachable!("tests construct coherent routes")
        }
    };
    LaunchRecord {
        agent: agent_of(route.provider),
        host,
        cwd: Some(route.cwd.clone()),
        config_dir,
        account_email,
        account_id,
        model: Some("opus".to_string()),
        effort: Some("high".to_string()),
        launched_at: Utc::now(),
    }
}

#[test]
fn local_restart_preserves_conversation_account_and_intent() {
    let route = SessionRoute::from_snapshot(&snapshot(Provider::Claude), true, None, None).unwrap();
    let plan = plan_restart(
        route.clone(),
        RestartPresence::VerifiedProcess,
        &record(&route),
    )
    .expect("an exact process-bound route is restartable");

    assert_eq!(
        plan.termination,
        RestartTermination::VerifiedProcess {
            pid: 42,
            fingerprint: "process-1".to_string(),
        }
    );
    let ResumeInvocation::LocalShell { command } = plan.resume else {
        panic!("local restart must stay local");
    };
    assert!(command.contains("CLAUDE_CONFIG_DIR=/accounts/work"));
    assert!(command.contains("--model opus"));
    assert!(command.contains("--resume session-1"));
    assert!(command.contains("unset ANTHROPIC_API_KEY"));
}

#[test]
fn remote_restart_uses_only_stable_host_and_opaque_account_ids() {
    let mut session = snapshot(Provider::Codex);
    session.config_dir = None;
    session.account_id = Some("codex:work".to_string());
    let route =
        SessionRoute::from_snapshot(&session, false, Some("daemon-host-7"), Some("fleet-node-7"))
            .unwrap();
    let plan = plan_restart(route.clone(), RestartPresence::Dormant, &record(&route)).unwrap();
    assert_eq!(
        plan.resume,
        ResumeInvocation::RemoteDaemon {
            host_id: "daemon-host-7".to_string(),
            node_id: "fleet-node-7".to_string(),
            account_id: "codex:work".to_string(),
            provider: Provider::Codex,
            session_id: "session-1".to_string(),
            cwd: PathBuf::from("/work/zaplex"),
            model: Some("opus".to_string()),
            effort: Some("high".to_string()),
        }
    );
}

#[test]
fn restart_rejects_account_drift_and_pid_reuse() {
    let route = SessionRoute::from_snapshot(&snapshot(Provider::Claude), true, None, None).unwrap();
    let mut wrong_account = record(&route);
    wrong_account.account_email = Some("other@example.com".to_string());
    assert_eq!(
        plan_restart(route.clone(), RestartPresence::Dormant, &wrong_account),
        Err(RestartPlanError::LaunchIntentUnbound)
    );
    assert_eq!(
        plan_restart(
            route.clone(),
            RestartPresence::ProcessReused,
            &record(&route),
        ),
        Err(RestartPlanError::ProcessIdentityChanged)
    );
}

#[test]
fn remote_route_fails_closed_without_exact_account_or_with_local_path() {
    let mut session = snapshot(Provider::Claude);
    session.config_dir = None;
    assert_eq!(
        SessionRoute::from_snapshot(&session, false, Some("host"), Some("node")),
        Err(SessionRouteError::MissingRemoteAccountIdentity)
    );
    session.account_id = Some("claude:work".to_string());
    session.config_dir = Some("/local-only".to_string());
    assert_eq!(
        SessionRoute::from_snapshot(&session, false, Some("host"), Some("node")),
        Err(SessionRouteError::LeakedRemoteConfigDirectory)
    );
}

#[test]
fn stale_cleanup_rechecks_revision_visibility_and_process_identity() {
    assert_eq!(
        authorize_stale_cleanup(7, 7, false, CleanupProcessEvidence::Dead),
        Ok(())
    );
    assert_eq!(
        authorize_stale_cleanup(7, 8, false, CleanupProcessEvidence::Dead),
        Err(CleanupRejection::InventoryChanged)
    );
    assert_eq!(
        authorize_stale_cleanup(7, 7, true, CleanupProcessEvidence::Dead),
        Err(CleanupRejection::SessionStillVisible)
    );
    assert_eq!(
        authorize_stale_cleanup(7, 7, false, CleanupProcessEvidence::ProcessReused),
        Err(CleanupRejection::ProcessIdentityChanged)
    );
    assert_eq!(
        authorize_stale_cleanup(7, 7, false, CleanupProcessEvidence::Unverifiable),
        Err(CleanupRejection::ProcessIdentityUnavailable)
    );
}

#[test]
fn rename_conflicts_are_scoped_to_exact_provider_host_and_account() {
    let route = SessionRoute::from_snapshot(&snapshot(Provider::Claude), true, None, None).unwrap();
    let mut same_scope = route.clone();
    same_scope.session_id = "session-2".to_string();
    let mut other_provider = same_scope.clone();
    other_provider.provider = Provider::Codex;

    assert_eq!(
        validate_rename_conflict(&route, " Deploy ", [(&same_scope, "Deploy")]),
        Err(SessionNameError::Conflict)
    );
    assert_eq!(
        validate_rename_conflict(&route, " Deploy ", [(&other_provider, "Deploy")]),
        Ok("Deploy".to_string())
    );
}

#[test]
fn retry_keeps_operation_identity_and_applied_state_is_idempotent() {
    let mut operation = LifecycleOperation::new("session-1".to_string());
    let id = operation.id;
    operation.mark_failed(true, "daemon disconnected");
    assert!(operation.retry());
    assert_eq!(operation.id, id);
    assert_eq!(operation.state, LifecycleOperationState::Pending);

    operation.mark_applied();
    operation.mark_failed(true, "late duplicate failure");
    assert_eq!(operation.state, LifecycleOperationState::Applied);
    assert!(!operation.retry());
}

#[test]
fn lifecycle_capabilities_require_exact_executable_routes() {
    let route = SessionRoute::from_snapshot(&snapshot(Provider::Claude), true, None, None).unwrap();
    assert_eq!(
        lifecycle_capabilities(&route, &RestartPresence::VerifiedProcess, true, false),
        SessionLifecycleCapabilities {
            can_restart: true,
            can_rename: true,
            can_cleanup_stale: false,
        }
    );
    assert!(
        !lifecycle_capabilities(&route, &RestartPresence::Unverifiable, true, true).can_restart
    );
    assert!(
        !lifecycle_capabilities(&route, &RestartPresence::VerifiedProcess, false, true).can_restart
    );
}
