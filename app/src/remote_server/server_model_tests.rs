use std::collections::{HashMap, HashSet};

use std::fs;

use super::super::proto::{
    list_directory_response, read_file_chunk_response, resolve_path_response, server_message,
    write_file_chunk_response, AgentProcessSignal, AgentProcessSignalRequest,
    AgentProcessSignalStatus, AgentPtyBindingStatus, AgentSessionIdentity, AgentSessionInfo,
    Authenticate, BindAgentPty, CreateDirectory, Initialize, ListDirectory, ReadFileChunk,
    ResolvePath, UnbindAgentPty, WriteFileChunk,
};
use super::super::protocol::RequestId;
#[cfg(feature = "local_fs")]
use super::super::server_buffer_tracker::ServerBufferTracker;
#[cfg(feature = "local_fs")]
use super::collect_directory_entries;
use super::{
    execute_agent_process_signal_with, server_features_with_runtime_support,
    AgentTranscriptReadPermit, PendingFileOps, ServerModel, MAX_CONCURRENT_AGENT_TRANSCRIPT_READS,
};
#[cfg(unix)]
use super::{
    push_recent_managed_exit, ManagedExitRecord, ManagedMemoryReadPermit,
    MAX_CONCURRENT_MANAGED_MEMORY_READS, MAX_RECENT_MANAGED_EXITS, RECENT_MANAGED_EXIT_TTL_MILLIS,
};
use zaplex_cockpit::{GuardrailSignal, ProcessSignalError};
#[cfg(unix)]
use zaplex_remote_session::types::FEATURE_MULTIPLEXER_INVENTORY_V1;
use zaplex_remote_session::types::{
    FEATURE_AGENT_ACCOUNT_ROUTING_V1, FEATURE_AGENT_PROCESS_SIGNAL_V1,
    FEATURE_AGENT_TRANSCRIPT_READ_V1, FEATURE_MANAGED_AGENT_FLEET_V1,
};

fn test_model() -> ServerModel {
    ServerModel {
        connection_senders: HashMap::new(),
        connection_features: HashMap::new(),
        snapshot_sent_roots_by_connection: HashMap::new(),
        grace_timer_cancel: None,
        in_progress: HashMap::new(),
        host_id: "test-host-id".to_string(),
        executors: HashMap::new(),
        pending_file_ops: PendingFileOps::new(),
        #[cfg(feature = "local_fs")]
        buffers: ServerBufferTracker::new(),
        auth_token: None,
        #[cfg(unix)]
        sessions: HashMap::new(),
        #[cfg(unix)]
        agent_pty_bindings: Default::default(),
        #[cfg(unix)]
        next_pty_generation: 1,
        #[cfg(unix)]
        safe_files: super::super::safe_file::SafeFileServer::unavailable_for_test(),
        agent_transcript_cache: std::sync::Arc::new(std::sync::Mutex::new(
            zaplex_cockpit::TranscriptScanCache::default(),
        )),
        agent_account_routes: Default::default(),
        fresh_agent_account_routes_for_test: None,
        agent_transcript_reads_in_flight: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(
            0,
        )),
        #[cfg(unix)]
        managed_memory_reads_in_flight: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        #[cfg(unix)]
        recent_managed_exits: std::collections::VecDeque::new(),
        #[cfg(unix)]
        managed_min_available_bytes: Ok(super::super::managed_fleet::DEFAULT_MIN_AVAILABLE_BYTES),
    }
}

fn request_id() -> RequestId {
    RequestId::from("test-request".to_string())
}

#[test]
fn fresh_model_starts_without_auth_token() {
    let model = test_model();

    assert_eq!(model.auth_token(), None);
}

#[test]
fn transcript_read_permits_cap_parallel_work_and_release_on_drop() {
    let in_flight = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let permits: Vec<_> = (0..MAX_CONCURRENT_AGENT_TRANSCRIPT_READS)
        .map(|_| AgentTranscriptReadPermit::try_acquire(std::sync::Arc::clone(&in_flight)).unwrap())
        .collect();

    assert!(AgentTranscriptReadPermit::try_acquire(std::sync::Arc::clone(&in_flight)).is_none());
    drop(permits);
    assert!(AgentTranscriptReadPermit::try_acquire(in_flight).is_some());
}

#[cfg(unix)]
#[test]
fn managed_memory_permit_bounds_global_procfs_work_and_releases() {
    let in_flight = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let permits: Vec<_> = (0..MAX_CONCURRENT_MANAGED_MEMORY_READS)
        .map(|_| ManagedMemoryReadPermit::try_acquire(std::sync::Arc::clone(&in_flight)).unwrap())
        .collect();

    assert!(ManagedMemoryReadPermit::try_acquire(std::sync::Arc::clone(&in_flight)).is_none());
    drop(permits);
    assert!(ManagedMemoryReadPermit::try_acquire(in_flight).is_some());
}

#[cfg(target_os = "linux")]
#[test]
fn list_sessions_returns_typed_busy_measurement_without_starting_more_procfs_work() {
    warpui::App::test((), |mut app| async move {
        let model = app.add_singleton_model(|_ctx| test_model());
        let message = model.update(&mut app, |model, ctx| {
            model.managed_memory_reads_in_flight.store(
                MAX_CONCURRENT_MANAGED_MEMORY_READS,
                std::sync::atomic::Ordering::Release,
            );
            model
                .handle_list_sessions(&request_id(), uuid::Uuid::new_v4(), ctx)
                .into_message()
        });
        let server_message::Message::SessionList(list) = message else {
            panic!("expected busy SessionList");
        };
        let host = list.host_available_memory.expect("typed busy measurement");
        assert_eq!(host.diagnostic_code, "busy");
        assert!(host.bytes.is_none());
    });
}

#[cfg(unix)]
#[test]
fn recent_managed_exit_records_are_deduplicated_ttl_pruned_and_bounded() {
    let key =
        super::super::managed_fleet::ManagedLaunchKey::new("host", "opaque", "/project", "claude")
            .unwrap();
    let plan =
        super::super::managed_fleet::ManagedLaunchPlan::interactive_agent("launch", key).unwrap();
    let make_record = |index: usize, exited_at_epoch_millis: u64| ManagedExitRecord {
        plan: plan.clone(),
        account_route_identity: super::super::agent_account::AccountRouteIdentity::DefaultAccount,
        session_id: format!("session-{index}"),
        generation: index as u64 + 1,
        exit_code: Some(1),
        exited_at_epoch_millis,
        shell: "/bin/sh".to_string(),
        rows: 24,
        cols: 80,
        ring_ceiling_bytes: 1024,
        diagnostic: super::ManagedExitDiagnostic::ProcessEnded,
    };
    let mut records = std::collections::VecDeque::new();
    for index in 0..=MAX_RECENT_MANAGED_EXITS {
        push_recent_managed_exit(&mut records, make_record(index, 10_000));
    }
    assert_eq!(records.len(), MAX_RECENT_MANAGED_EXITS);
    assert_eq!(records.front().unwrap().session_id, "session-1");

    let duplicate = make_record(MAX_RECENT_MANAGED_EXITS, 11_000);
    push_recent_managed_exit(&mut records, duplicate);
    assert_eq!(records.len(), MAX_RECENT_MANAGED_EXITS);

    push_recent_managed_exit(
        &mut records,
        make_record(100, 11_000 + RECENT_MANAGED_EXIT_TTL_MILLIS + 1),
    );
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].session_id, "session-100");
    let proto = records[0].to_proto();
    assert_eq!(proto.diagnostic_code, "process-ended");
    assert_eq!(proto.exit_code, Some(1));
    let mut stopped = make_record(101, proto.exited_at_epoch_millis + 1);
    stopped.diagnostic = super::ManagedExitDiagnostic::Stopped;
    assert_eq!(stopped.to_proto().diagnostic_code, "stopped");

    let exact_restart = super::super::proto::ManagedSessionLifecycleRequest {
        schema_version: 1,
        action: super::super::proto::ManagedSessionLifecycleAction::Restart.into(),
        session_id: "session-100".to_string(),
        expected_generation: 101,
        launch_id: "launch".to_string(),
        provider: "claude".to_string(),
        account_id: "opaque".to_string(),
        project_root: "/project".to_string(),
    };
    assert!(records[0].matches(&exact_restart));
    assert!(
        !records[0].matches(&super::super::proto::ManagedSessionLifecycleRequest {
            expected_generation: 102,
            ..exact_restart
        })
    );
}

#[cfg(unix)]
#[test]
fn initial_managed_start_never_uses_a_stale_same_inode_account_route() {
    let project = tempfile::tempdir().unwrap();
    let account = tempfile::tempdir().unwrap();
    let project_root = std::fs::canonicalize(project.path()).unwrap();
    let account_root = std::fs::canonicalize(account.path()).unwrap();
    let project_identity =
        super::super::managed_fleet::ManagedProjectIdentity::capture(&project_root).unwrap();
    let key = super::super::managed_fleet::ManagedLaunchKey::new(
        "host",
        "stale-account-id",
        project_root.to_str().unwrap(),
        "claude",
    )
    .unwrap();
    let plan = super::super::managed_fleet::ManagedLaunchPlan::interactive_agent("launch", key)
        .unwrap()
        .with_project_identity(project_identity);

    let mut stale_cache = super::super::agent_account::AccountRouteCache::default();
    stale_cache.replace_for_test("claude", "stale-account-id", Some(account_root.clone()));
    assert!(super::super::agent_account::current_account_route_identity(
        &stale_cache,
        "claude",
        "stale-account-id",
    )
    .is_ok());

    let mut fresh_scan = super::super::agent_account::AccountRouteCache::default();
    fresh_scan.replace_for_test("claude", "current-account-id", Some(account_root));
    assert_eq!(
        super::fresh_managed_launch_identity(fresh_scan.routes_for_test(), &plan),
        Err("account-route-changed")
    );
}

#[test]
fn initialize_with_auth_token_stores_token() {
    let mut model = test_model();

    model.handle_initialize(
        uuid::Uuid::nil(),
        Initialize {
            auth_token: "initial-token".to_string(),
            features: vec![],
        },
        &request_id(),
    );

    assert_eq!(model.auth_token(), Some("initial-token"));
}

#[test]
fn empty_initialize_preserves_existing_auth_token() {
    let mut model = test_model();
    model.handle_initialize(
        uuid::Uuid::nil(),
        Initialize {
            auth_token: "initial-token".to_string(),
            features: vec![],
        },
        &request_id(),
    );

    model.handle_initialize(
        uuid::Uuid::nil(),
        Initialize {
            auth_token: String::new(),
            features: vec![],
        },
        &request_id(),
    );

    assert_eq!(model.auth_token(), Some("initial-token"));
}

#[cfg(unix)]
#[test]
fn multiplexer_inventory_requires_client_capability_negotiation() {
    let mut model = test_model();
    let conn = uuid::Uuid::new_v4();

    assert!(!model.client_supports_multiplexer_inventory(conn));
    model.connection_features.insert(
        conn,
        HashSet::from([FEATURE_MULTIPLEXER_INVENTORY_V1.to_string()]),
    );
    assert!(model.client_supports_multiplexer_inventory(conn));
}

#[test]
fn agent_process_signal_requires_client_capability_negotiation() {
    let mut model = test_model();
    let conn = uuid::Uuid::new_v4();

    assert!(!model.client_supports_agent_process_signal(conn));
    model.connection_features.insert(
        conn,
        HashSet::from([FEATURE_AGENT_PROCESS_SIGNAL_V1.to_string()]),
    );
    assert_eq!(
        model.client_supports_agent_process_signal(conn),
        zaplex_cockpit::local_process_signalling_supported()
    );
}

#[test]
fn agent_account_routing_requires_client_capability_negotiation() {
    let mut model = test_model();
    let conn = uuid::Uuid::new_v4();

    assert!(!model.client_supports_agent_account_routing(conn));
    model.connection_features.insert(
        conn,
        HashSet::from([FEATURE_AGENT_ACCOUNT_ROUTING_V1.to_string()]),
    );
    assert!(model.client_supports_agent_account_routing(conn));
}

#[test]
fn agent_transcript_read_requires_client_capability_negotiation() {
    let mut model = test_model();
    let conn = uuid::Uuid::new_v4();

    assert!(!model.client_supports_agent_transcript_read(conn));
    model.connection_features.insert(
        conn,
        HashSet::from([FEATURE_AGENT_TRANSCRIPT_READ_V1.to_string()]),
    );
    assert!(model.client_supports_agent_transcript_read(conn));
}

#[test]
fn daemon_signal_advertisement_requires_runtime_backend_support() {
    let unsupported = server_features_with_runtime_support(false, true);
    assert!(!unsupported
        .iter()
        .any(|feature| feature == FEATURE_AGENT_PROCESS_SIGNAL_V1));
    assert!(!unsupported
        .iter()
        .any(|feature| feature == FEATURE_MANAGED_AGENT_FLEET_V1));

    let supported = server_features_with_runtime_support(true, true);
    assert_eq!(
        supported
            .iter()
            .any(|feature| feature == FEATURE_AGENT_PROCESS_SIGNAL_V1),
        cfg!(target_os = "linux")
    );
    assert!(supported
        .iter()
        .any(|feature| feature == FEATURE_AGENT_TRANSCRIPT_READ_V1));
    assert_eq!(
        supported
            .iter()
            .any(|feature| feature == FEATURE_MANAGED_AGENT_FLEET_V1),
        cfg!(target_os = "linux")
    );
}

#[cfg(unix)]
#[test]
fn managed_fleet_negotiation_is_usable_only_on_linux_daemons() {
    let mut model = test_model();
    let conn = uuid::Uuid::new_v4();
    model.connection_features.insert(
        conn,
        HashSet::from([FEATURE_MANAGED_AGENT_FLEET_V1.to_string()]),
    );

    assert!(!model.client_supports_managed_fleet_with_runtime(conn, false));
    assert_eq!(
        model.client_supports_managed_fleet_with_runtime(conn, true),
        cfg!(target_os = "linux")
    );
}

#[test]
fn authenticate_with_auth_token_replaces_auth_token() {
    let mut model = test_model();
    model.handle_initialize(
        uuid::Uuid::nil(),
        Initialize {
            auth_token: "initial-token".to_string(),
            features: vec![],
        },
        &request_id(),
    );

    model.handle_authenticate(Authenticate {
        auth_token: "rotated-token".to_string(),
    });

    assert_eq!(model.auth_token(), Some("rotated-token"));
}

#[test]
fn empty_authenticate_preserves_existing_auth_token() {
    let mut model = test_model();
    model.handle_initialize(
        uuid::Uuid::nil(),
        Initialize {
            auth_token: "initial-token".to_string(),
            features: vec![],
        },
        &request_id(),
    );

    model.handle_authenticate(Authenticate {
        auth_token: String::new(),
    });

    assert_eq!(model.auth_token(), Some("initial-token"));
}

fn process_signal_request(signal: AgentProcessSignal) -> AgentProcessSignalRequest {
    AgentProcessSignalRequest {
        session_id: "agent-session-1".to_string(),
        pid: 4242,
        expected_process_fingerprint: "linux-v1:boot-id:12345".to_string(),
        signal: signal.into(),
    }
}

fn current_process_session() -> AgentSessionInfo {
    AgentSessionInfo {
        session_id: "agent-session-1".to_string(),
        pid: 4242,
        process_fingerprint: "linux-v1:boot-id:12345".to_string(),
        ..Default::default()
    }
}

#[cfg(unix)]
fn binding_identity(session_id: &str) -> AgentSessionIdentity {
    AgentSessionIdentity {
        session_id: session_id.to_string(),
        provider: "codex".to_string(),
        account_email: "agent@example.com".to_string(),
        config_dir: "/home/agent/.codex".to_string(),
        account_id: String::new(),
    }
}

#[cfg(unix)]
fn binding_status(outcome: super::HandlerOutcome) -> AgentPtyBindingStatus {
    let server_message::Message::AgentPtyBindingResponse(response) = outcome.into_message() else {
        panic!("expected AgentPtyBindingResponse");
    };
    AgentPtyBindingStatus::try_from(response.status).unwrap()
}

#[cfg(unix)]
fn bind_status(
    model: &mut ServerModel,
    conn: uuid::Uuid,
    msg: BindAgentPty,
) -> AgentPtyBindingStatus {
    let identity = msg.agent.as_ref().unwrap();
    let live_agents = HashSet::from([zaplex_remote_session::agent_binding::AgentIdentity {
        provider: identity.provider.clone(),
        session_id: identity.session_id.clone(),
        account_email: (!identity.account_email.is_empty()).then(|| identity.account_email.clone()),
        config_dir: (!identity.config_dir.is_empty()).then(|| identity.config_dir.clone()),
        account_id: None,
    }]);
    AgentPtyBindingStatus::try_from(model.execute_bind_agent_pty(conn, msg, &live_agents).status)
        .unwrap()
}

#[cfg(unix)]
#[test]
fn v1_client_cannot_mutate_agent_pty_binding() {
    let mut model = test_model();
    let conn = uuid::Uuid::new_v4();
    model
        .connection_features
        .insert(conn, HashSet::from(["agent-pty-binding".to_string()]));
    model
        .agent_pty_bindings
        .register_pty("pty-1", 7, "test-host-id", conn.as_u128());

    let status = bind_status(
        &mut model,
        conn,
        BindAgentPty {
            agent: Some(binding_identity("agent-1")),
            pty_session_id: "pty-1".to_string(),
            pty_session_generation: 7,
            handoff_from: None,
            host_id: "test-host-id".to_string(),
        },
    );

    assert_eq!(status, AgentPtyBindingStatus::CapabilityRequired);
}

#[cfg(unix)]
#[test]
fn daemon_bind_and_unbind_preserve_historical_agent() {
    let mut model = test_model();
    let conn = uuid::Uuid::new_v4();
    model
        .connection_features
        .insert(conn, HashSet::from(["agent-pty-binding-v2".to_string()]));
    model
        .agent_pty_bindings
        .register_pty("pty-1", 7, "test-host-id", conn.as_u128());
    let identity = binding_identity("agent-1");

    assert_eq!(
        bind_status(
            &mut model,
            conn,
            BindAgentPty {
                agent: Some(identity.clone()),
                pty_session_id: "pty-1".to_string(),
                pty_session_generation: 7,
                handoff_from: None,
                host_id: "test-host-id".to_string(),
            },
        ),
        AgentPtyBindingStatus::Bound
    );
    assert_eq!(
        binding_status(model.handle_unbind_agent_pty(
            conn,
            UnbindAgentPty {
                agent: Some(identity.clone()),
                pty_session_id: "pty-1".to_string(),
                pty_session_generation: 7,
                host_id: "test-host-id".to_string(),
            },
        )),
        AgentPtyBindingStatus::Unbound
    );
    assert!(
        !model
            .agent_pty_bindings
            .binding_for(&zaplex_remote_session::agent_binding::AgentIdentity {
                provider: identity.provider,
                session_id: identity.session_id,
                account_email: Some(identity.account_email),
                config_dir: Some(identity.config_dir),
                account_id: None,
            })
            .unwrap()
            .foreground
    );
}

#[cfg(unix)]
#[test]
fn daemon_rejects_stale_and_foreign_agent_pty_bindings() {
    let mut model = test_model();
    let owner = uuid::Uuid::new_v4();
    let foreign = uuid::Uuid::new_v4();
    for conn in [owner, foreign] {
        model
            .connection_features
            .insert(conn, HashSet::from(["agent-pty-binding-v2".to_string()]));
    }
    model
        .agent_pty_bindings
        .register_pty("pty-1", 7, "test-host-id", owner.as_u128());

    let request = |generation| BindAgentPty {
        agent: Some(binding_identity("agent-1")),
        pty_session_id: "pty-1".to_string(),
        pty_session_generation: generation,
        handoff_from: None,
        host_id: "test-host-id".to_string(),
    };
    assert_eq!(
        bind_status(&mut model, owner, request(6)),
        AgentPtyBindingStatus::StaleGeneration
    );
    assert_eq!(
        bind_status(&mut model, foreign, request(7)),
        AgentPtyBindingStatus::ForeignConnection
    );
}

#[cfg(unix)]
#[test]
fn daemon_rejects_foreign_host_identity_and_undiscovered_agent_tuple() {
    let mut model = test_model();
    let conn = uuid::Uuid::new_v4();
    model
        .connection_features
        .insert(conn, HashSet::from(["agent-pty-binding-v2".to_string()]));
    model
        .agent_pty_bindings
        .register_pty("pty-1", 7, "test-host-id", conn.as_u128());
    let request = |host_id: &str| BindAgentPty {
        agent: Some(binding_identity("agent-1")),
        pty_session_id: "pty-1".to_string(),
        pty_session_generation: 7,
        handoff_from: None,
        host_id: host_id.to_string(),
    };

    assert_eq!(
        bind_status(&mut model, conn, request("another-daemon")),
        AgentPtyBindingStatus::ForeignDaemon
    );
    let response = model.execute_bind_agent_pty(conn, request("test-host-id"), &HashSet::new());
    assert_eq!(
        AgentPtyBindingStatus::try_from(response.status),
        Ok(AgentPtyBindingStatus::IdentityNotDiscovered)
    );
    assert!(model
        .agent_pty_bindings
        .foreground_for_pty("pty-1", 7)
        .is_none());
}

#[cfg(unix)]
#[test]
fn dormant_inventory_reconciles_foreground_binding_to_history() {
    let mut model = test_model();
    let conn = uuid::Uuid::new_v4();
    model
        .connection_features
        .insert(conn, HashSet::from(["agent-pty-binding-v2".to_string()]));
    model
        .agent_pty_bindings
        .register_pty("pty-1", 7, "test-host-id", conn.as_u128());
    assert_eq!(
        bind_status(
            &mut model,
            conn,
            BindAgentPty {
                agent: Some(binding_identity("agent-ended")),
                pty_session_id: "pty-1".to_string(),
                pty_session_generation: 7,
                handoff_from: None,
                host_id: "test-host-id".to_string(),
            },
        ),
        AgentPtyBindingStatus::Bound
    );
    let mut dormant = vec![AgentSessionInfo {
        session_id: "agent-ended".to_string(),
        provider: "codex".to_string(),
        account_email: "agent@example.com".to_string(),
        config_dir: "/home/agent/.codex".to_string(),
        state: "idle".to_string(),
        ..Default::default()
    }];

    model.reconcile_and_overlay_agent_bindings(conn, &mut dormant);

    assert_eq!(dormant[0].pty_session_id, "pty-1");
    assert_eq!(dormant[0].pty_session_generation, 7);
    assert!(!dormant[0].pty_foreground);
    assert!(model
        .agent_pty_bindings
        .foreground_for_pty("pty-1", 7)
        .is_none());
}

#[cfg(unix)]
#[test]
fn agent_qualified_attach_requires_fresh_exact_live_identity() {
    let mut model = test_model();
    let conn = uuid::Uuid::new_v4();
    model
        .connection_features
        .insert(conn, HashSet::from(["agent-pty-binding-v2".to_string()]));
    model
        .agent_pty_bindings
        .register_pty("pty-1", 7, "test-host-id", conn.as_u128());
    let expected = zaplex_remote_session::agent_binding::AgentIdentity {
        provider: "codex".to_string(),
        session_id: "agent-1".to_string(),
        account_email: Some("agent@example.com".to_string()),
        config_dir: Some("/home/agent/.codex".to_string()),
        account_id: None,
    };
    assert_eq!(
        bind_status(
            &mut model,
            conn,
            BindAgentPty {
                agent: Some(binding_identity("agent-1")),
                pty_session_id: "pty-1".to_string(),
                pty_session_generation: 7,
                handoff_from: None,
                host_id: "test-host-id".to_string(),
            },
        ),
        AgentPtyBindingStatus::Bound
    );

    assert!(model
        .validate_fresh_agent_attach(conn, &expected, &HashSet::from([expected.clone()]))
        .is_ok());
    let error = model
        .validate_fresh_agent_attach(conn, &expected, &HashSet::new())
        .unwrap_err();

    assert!(error.message.contains("no longer present"));
    assert!(model
        .agent_pty_bindings
        .foreground_for_pty("pty-1", 7)
        .is_none());
}

#[test]
fn verified_agent_process_signal_calls_only_the_typed_backend() {
    let current_sessions = [current_process_session()];
    let response = execute_agent_process_signal_with(
        process_signal_request(AgentProcessSignal::Interrupt),
        &current_sessions,
        true,
        |pid, fingerprint, signal| {
            assert_eq!(pid, 4242);
            assert_eq!(fingerprint, "linux-v1:boot-id:12345");
            assert_eq!(signal, GuardrailSignal::Interrupt);
            Ok(())
        },
    );

    assert_eq!(
        AgentProcessSignalStatus::try_from(response.status),
        Ok(AgentProcessSignalStatus::Sent)
    );
    assert_eq!(response.session_id, "agent-session-1");
    assert_eq!(response.pid, 4242);
    assert!(response.error_message.is_empty());
}

#[test]
fn verified_agent_process_signal_rejects_unknown_signal_before_backend() {
    let mut request = process_signal_request(AgentProcessSignal::Kill);
    request.signal = 999;
    let current_sessions = [current_process_session()];
    let response =
        execute_agent_process_signal_with(request, &current_sessions, true, |_, _, _| {
            panic!("invalid signal must never reach the process backend")
        });

    assert_eq!(
        AgentProcessSignalStatus::try_from(response.status),
        Ok(AgentProcessSignalStatus::InvalidRequest)
    );
}

#[test]
fn signal_fails_closed_without_provable_process_identity() {
    let mut request = process_signal_request(AgentProcessSignal::Interrupt);
    request.expected_process_fingerprint = String::new();
    let current_sessions = [current_process_session()];
    let response =
        execute_agent_process_signal_with(request, &current_sessions, true, |_, _, _| {
            panic!("missing identity must never reach the process backend")
        });

    assert_eq!(
        AgentProcessSignalStatus::try_from(response.status),
        Ok(AgentProcessSignalStatus::IdentityUnverifiable)
    );
}

#[test]
fn agent_process_signal_rejects_unnegotiated_capability_before_backend() {
    let current_sessions = [current_process_session()];
    let response = execute_agent_process_signal_with(
        process_signal_request(AgentProcessSignal::Interrupt),
        &current_sessions,
        false,
        |_, _, _| panic!("an unnegotiated request must never reach the process backend"),
    );

    assert_eq!(
        AgentProcessSignalStatus::try_from(response.status),
        Ok(AgentProcessSignalStatus::InvalidRequest)
    );
    assert!(response.error_message.contains("not negotiated"));
}

#[test]
fn agent_process_signal_rejects_foreign_session_id_before_backend() {
    let mut request = process_signal_request(AgentProcessSignal::Interrupt);
    request.session_id = "foreign-session".to_string();
    let current_sessions = [current_process_session()];
    let response =
        execute_agent_process_signal_with(request, &current_sessions, true, |_, _, _| {
            panic!("a foreign session must never reach the process backend")
        });

    assert_eq!(
        AgentProcessSignalStatus::try_from(response.status),
        Ok(AgentProcessSignalStatus::InvalidRequest)
    );
}

#[test]
fn agent_process_signal_rejects_inventory_pid_mismatch_before_backend() {
    let mut request = process_signal_request(AgentProcessSignal::Interrupt);
    request.pid += 1;
    let current_sessions = [current_process_session()];
    let response =
        execute_agent_process_signal_with(request, &current_sessions, true, |_, _, _| {
            panic!("a mismatched pid must never reach the process backend")
        });

    assert_eq!(
        AgentProcessSignalStatus::try_from(response.status),
        Ok(AgentProcessSignalStatus::StaleIdentity)
    );
}

#[test]
fn agent_process_signal_rejects_inventory_fingerprint_mismatch_before_backend() {
    let mut request = process_signal_request(AgentProcessSignal::Interrupt);
    request.expected_process_fingerprint = "linux-v1:boot-id:foreign".to_string();
    let current_sessions = [current_process_session()];
    let response =
        execute_agent_process_signal_with(request, &current_sessions, true, |_, _, _| {
            panic!("a mismatched fingerprint must never reach the process backend")
        });

    assert_eq!(
        AgentProcessSignalStatus::try_from(response.status),
        Ok(AgentProcessSignalStatus::StaleIdentity)
    );
}

#[test]
fn verified_agent_process_signal_returns_typed_failure_reasons() {
    let cases = [
        (
            ProcessSignalError::IdentityChanged,
            AgentProcessSignalStatus::StaleIdentity,
        ),
        (
            ProcessSignalError::IdentityUnavailable("unreadable".to_string()),
            AgentProcessSignalStatus::IdentityUnverifiable,
        ),
        (
            ProcessSignalError::InvalidPid,
            AgentProcessSignalStatus::InvalidRequest,
        ),
        (
            ProcessSignalError::SignalFailed("permission denied".to_string()),
            AgentProcessSignalStatus::SignalFailed,
        ),
        (
            ProcessSignalError::UnsupportedPlatform,
            AgentProcessSignalStatus::IdentityUnverifiable,
        ),
    ];

    for (error, expected_status) in cases {
        let response = execute_agent_process_signal_with(
            process_signal_request(AgentProcessSignal::Kill),
            &[current_process_session()],
            true,
            |_, _, _| Err(error.clone()),
        );
        assert_eq!(
            AgentProcessSignalStatus::try_from(response.status),
            Ok(expected_status)
        );
        assert!(!response.error_message.is_empty());
    }
}

#[cfg(feature = "local_fs")]
#[test]
fn resolve_path_reports_file_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("note.txt");
    fs::write(&file_path, "hello").unwrap();
    let model = test_model();

    let response = model.handle_resolve_path(ResolvePath {
        path: file_path.to_string_lossy().to_string(),
    });

    let server_message::Message::ResolvePathResponse(response) = response.into_message() else {
        panic!("expected ResolvePathResponse");
    };
    let Some(resolve_path_response::Result::Success(success)) = response.result else {
        panic!("expected resolve path success");
    };
    assert_eq!(
        success.canonical_path,
        fs::canonicalize(&file_path).unwrap().to_string_lossy()
    );
    assert_eq!(
        success.kind,
        super::super::proto::FileSystemEntryKind::File as i32
    );
    assert_eq!(success.size_bytes, Some(5));
}

#[cfg(feature = "local_fs")]
#[test]
fn resolve_path_distinguishes_missing_paths_from_io_errors() {
    let directory = tempfile::tempdir().unwrap();
    let missing_path = directory.path().join("missing.txt");
    let model = test_model();

    let response = model.handle_resolve_path(ResolvePath {
        path: missing_path.to_string_lossy().to_string(),
    });

    let server_message::Message::ResolvePathResponse(response) = response.into_message() else {
        panic!("expected ResolvePathResponse");
    };
    assert!(matches!(
        response.result,
        Some(resolve_path_response::Result::NotFound(_))
    ));
}

#[cfg(feature = "local_fs")]
#[test]
fn list_directory_returns_sorted_metadata() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("b.txt"), "b").unwrap();
    fs::create_dir(dir.path().join("a-dir")).unwrap();
    let model = test_model();

    let response = model.handle_list_directory(ListDirectory {
        path: dir.path().to_string_lossy().to_string(),
    });

    let server_message::Message::ListDirectoryResponse(response) = response.into_message() else {
        panic!("expected ListDirectoryResponse");
    };
    let Some(list_directory_response::Result::Success(success)) = response.result else {
        panic!("expected list directory success");
    };
    assert_eq!(
        success.canonical_path,
        fs::canonicalize(dir.path()).unwrap().to_string_lossy()
    );
    assert_eq!(success.entries.len(), 2);
    assert_eq!(success.entries[0].name, "a-dir");
    assert_eq!(
        success.entries[0].kind,
        super::super::proto::FileSystemEntryKind::Directory as i32
    );
    assert_eq!(success.entries[1].name, "b.txt");
    assert_eq!(
        success.entries[1].kind,
        super::super::proto::FileSystemEntryKind::File as i32
    );
    assert_eq!(success.entries[1].size_bytes, Some(1));
}

#[cfg(feature = "local_fs")]
#[test]
fn list_directory_skips_entry_removed_after_readdir() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("keep"), "present").unwrap();
    let vanished_path = directory.path().join("vanish");
    fs::write(&vanished_path, "temporary").unwrap();
    let entries = fs::read_dir(directory.path()).unwrap().collect::<Vec<_>>();
    fs::remove_file(vanished_path).unwrap();

    let entries = collect_directory_entries(entries);

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "keep");
}

#[cfg(feature = "local_fs")]
#[test]
fn read_and_write_file_chunks_round_trip_binary_data() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("blob.bin");
    let model = test_model();

    let write_response = model.handle_write_file_chunk(WriteFileChunk {
        path: file_path.to_string_lossy().to_string(),
        offset: 0,
        bytes: vec![0, 1, 2, 3],
        truncate: true,
        executable: None,
    });
    let server_message::Message::WriteFileChunkResponse(write_response) =
        write_response.into_message()
    else {
        panic!("expected WriteFileChunkResponse");
    };
    let Some(write_file_chunk_response::Result::Success(write_success)) = write_response.result
    else {
        panic!("expected write chunk success");
    };
    assert_eq!(write_success.next_offset, 4);

    let read_response = model.handle_read_file_chunk(ReadFileChunk {
        path: file_path.to_string_lossy().to_string(),
        offset: 1,
        max_bytes: 2,
    });
    let server_message::Message::ReadFileChunkResponse(read_response) =
        read_response.into_message()
    else {
        panic!("expected ReadFileChunkResponse");
    };
    let Some(read_file_chunk_response::Result::Success(read_success)) = read_response.result else {
        panic!("expected read chunk success");
    };
    assert_eq!(read_success.bytes, vec![1, 2]);
    assert_eq!(read_success.next_offset, 3);
    assert_eq!(read_success.total_size, Some(4));
    assert!(!read_success.eof);
}

#[cfg(all(feature = "local_fs", unix))]
#[test]
fn read_file_chunk_rejects_symlinks_and_special_files() {
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;

    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("target.txt");
    let file_link = directory.path().join("file-link");
    let directory_link = directory.path().join("directory-link");
    let broken_link = directory.path().join("broken-link");
    let socket = directory.path().join("socket");
    fs::write(&target, "secret").unwrap();
    symlink(&target, &file_link).unwrap();
    symlink(directory.path(), &directory_link).unwrap();
    symlink(directory.path().join("missing"), &broken_link).unwrap();
    let _listener = UnixListener::bind(&socket).unwrap();
    let model = test_model();

    for path in [file_link, directory_link, broken_link, socket] {
        let response = model.handle_read_file_chunk(ReadFileChunk {
            path: path.to_string_lossy().to_string(),
            offset: 0,
            max_bytes: 1024,
        });
        let server_message::Message::ReadFileChunkResponse(response) = response.into_message()
        else {
            panic!("expected ReadFileChunkResponse");
        };
        assert!(matches!(
            response.result,
            Some(read_file_chunk_response::Result::Error(_))
        ));
    }
}

#[cfg(all(feature = "local_fs", unix))]
#[test]
fn list_directory_rejects_a_symlink_root() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let directory_link = directory.path().join("directory-link");
    symlink(directory.path(), &directory_link).unwrap();
    let model = test_model();

    let response = model.handle_list_directory(ListDirectory {
        path: directory_link.to_string_lossy().to_string(),
    });
    let server_message::Message::ListDirectoryResponse(response) = response.into_message() else {
        panic!("expected ListDirectoryResponse");
    };
    assert!(matches!(
        response.result,
        Some(super::super::proto::list_directory_response::Result::Error(
            _
        ))
    ));
}

#[cfg(feature = "local_fs")]
#[test]
fn create_directory_creates_nested_directories() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a/b/c");
    let model = test_model();

    let response = model.handle_create_directory(CreateDirectory {
        path: nested.to_string_lossy().to_string(),
    });

    let server_message::Message::CreateDirectoryResponse(response) = response.into_message() else {
        panic!("expected CreateDirectoryResponse");
    };
    assert!(matches!(
        response.result,
        Some(super::super::proto::create_directory_response::Result::Success(_))
    ));
    assert!(nested.is_dir());
}

// ---- Daemon session host: end-to-end glue (Stage 2) -----------------------
//
// Drives the full server-side glue headlessly on a real warpui test App: an
// OpenSession message spawns a real PTY+shell, SessionInput reaches that PTY,
// the background reader task streams PTY bytes back as SessionOutput pushes via
// the model, and CloseSession reaps the shell and emits SessionExited. This is
// the path that was previously only compile-verified (no async-model harness).
// Unix-only: the daemon owns the PTY (PTY ownership is unix-only).

#[cfg(unix)]
mod daemon_session {
    use super::super::HOST_RING_CAP_BYTES;
    use super::{bind_status, binding_identity, test_model};
    use crate::remote_server::proto::{
        client_message, server_message, AttachSession, BindAgentPty, ClientMessage, CloseSession,
        DetachSession, ListSessions, ManagedLaunch, ManagedSessionLifecycleAction,
        ManagedSessionLifecycleRequest, ManagedSessionLifecycleStatus, OpenSession,
        ReadAgentTranscript, ResizeSession, ServerMessage, SessionInput, SessionList, SessionSize,
    };
    use futures::future::Either;
    use std::time::Duration;
    use warpui::App;
    use zaplex_remote_session::types::{
        FEATURE_AGENT_ACCOUNT_ROUTING_V1, FEATURE_AGENT_TRANSCRIPT_READ_V1,
        FEATURE_MANAGED_AGENT_FLEET_V1,
    };

    /// Awaits `rx.recv()` but gives up after `dur` so a stuck test fails instead
    /// of hanging the CI job.
    async fn recv_deadline(
        rx: &async_channel::Receiver<ServerMessage>,
        dur: Duration,
    ) -> Option<ServerMessage> {
        let timer = async_io::Timer::after(dur);
        match futures::future::select(std::pin::pin!(rx.recv()), std::pin::pin!(timer)).await {
            Either::Left((Ok(msg), _)) => Some(msg),
            _ => None,
        }
    }

    /// Drains messages until a `SessionOutput` whose accumulated bytes contain
    /// `needle`, or the overall deadline elapses.
    async fn wait_for_output(
        rx: &async_channel::Receiver<ServerMessage>,
        needle: &[u8],
        total: Duration,
    ) -> bool {
        let collect = async {
            let mut buf: Vec<u8> = Vec::new();
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        if let Some(server_message::Message::SessionOutput(out)) = msg.message {
                            buf.extend_from_slice(&out.bytes);
                            if buf.windows(needle.len()).any(|w| w == needle) {
                                return true;
                            }
                        }
                    }
                    Err(_) => return false,
                }
            }
        };
        let timer = async_io::Timer::after(total);
        match futures::future::select(std::pin::pin!(collect), std::pin::pin!(timer)).await {
            Either::Left((found, _)) => found,
            Either::Right(_) => false,
        }
    }

    async fn collect_output_until(
        rx: &async_channel::Receiver<ServerMessage>,
        needle: &[u8],
        total: Duration,
    ) -> Option<Vec<u8>> {
        let collect = async {
            let mut output = Vec::new();
            loop {
                let message = rx.recv().await.ok()?;
                if let Some(server_message::Message::SessionOutput(chunk)) = message.message {
                    output.extend_from_slice(&chunk.bytes);
                    if output.windows(needle.len()).any(|window| window == needle) {
                        return Some(output);
                    }
                }
            }
        };
        let timer = async_io::Timer::after(total);
        match futures::future::select(std::pin::pin!(collect), std::pin::pin!(timer)).await {
            Either::Left((output, _)) => output,
            Either::Right(_) => None,
        }
    }

    async fn wait_for_exit(
        rx: &async_channel::Receiver<ServerMessage>,
        session_id: &str,
        total: Duration,
    ) -> bool {
        let collect = async {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        if let Some(server_message::Message::SessionExited(e)) = msg.message {
                            if e.session_id == session_id {
                                return true;
                            }
                        }
                    }
                    Err(_) => return false,
                }
            }
        };
        let timer = async_io::Timer::after(total);
        match futures::future::select(std::pin::pin!(collect), std::pin::pin!(timer)).await {
            Either::Left((found, _)) => found,
            Either::Right(_) => false,
        }
    }

    fn open_session_msg() -> ClientMessage {
        ClientMessage {
            request_id: "open-1".to_string(),
            message: Some(client_message::Message::OpenSession(OpenSession {
                cwd: None,
                ring_ceiling_bytes: None,
                shell: Some("/bin/bash".to_string()),
                env: std::collections::HashMap::new(),
                size: Some(SessionSize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                }),
                agent_launch_route: None,
                managed_launch: None,
                requested_min_available_bytes: None,
            })),
        }
    }

    #[test]
    fn open_streams_output_then_close_exits() {
        App::test((), |mut app| async move {
            // Build the model via the struct-literal helper (no `new()`), so the
            // test doesn't need FileModel/RepoMetadata singletons — but still
            // gets a real ModelContext (executor + spawner) from the App.
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx)
            });

            // OpenSession -> spawns PTY+shell, replies SessionOpened.
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_session_msg(), ctx)
            });

            let session_id = {
                let msg = recv_deadline(&conn_rx, Duration::from_secs(10))
                    .await
                    .expect("expected a server message after OpenSession");
                match msg.message {
                    Some(server_message::Message::SessionOpened(o)) => o.session_id,
                    other => panic!("expected SessionOpened, got {other:?}"),
                }
            };
            assert!(!session_id.is_empty(), "daemon assigned a session id");

            // SessionInput: the executed output (not the echoed input) carries
            // the marker, proving the byte round-trip reached the real shell.
            // `D4''EM0N` echoes verbatim but executes to `D4EM0N`.
            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id,
                    ClientMessage {
                        request_id: String::new(),
                        message: Some(client_message::Message::SessionInput(SessionInput {
                            session_id: session_id.clone(),
                            bytes: b"echo D4''EM0N\n".to_vec(),
                            startup_command_id: String::new(),
                        })),
                    },
                    ctx,
                )
            });
            assert!(
                wait_for_output(&conn_rx, b"D4EM0N", Duration::from_secs(15)).await,
                "expected SessionOutput containing the executed marker"
            );

            // CloseSession -> reaps the shell, emits SessionExited.
            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id,
                    ClientMessage {
                        request_id: String::new(),
                        message: Some(client_message::Message::CloseSession(CloseSession {
                            session_id: session_id.clone(),
                        })),
                    },
                    ctx,
                )
            });
            assert!(
                wait_for_exit(&conn_rx, &session_id, Duration::from_secs(10)).await,
                "expected SessionExited after CloseSession"
            );
        });
    }

    /// Regression for #132: the daemon writes both the bootstrap and the
    /// post-bootstrap startup command into a remote PTY. Those control inputs
    /// may execute, but their source bytes must never become observable
    /// SessionOutput, even when the shell has restored ECHO before startup.
    #[test]
    fn bootstrap_and_startup_input_bytes_are_not_observable_output() {
        App::test((), |mut app| async move {
            const BOOTSTRAPPED_HEX: &[u8] = b"426f6f747374726170706564";
            const BOOTSTRAP_SOURCE: &[u8] = b"read -r -d '' ZAPLEX_BOOTSTRAP_VAR";
            const STARTUP_SOURCE: &[u8] = b"ZAPLEX_STARTUP_ECHO_MUST_NOT_RENDER";
            const STARTUP_RESULT: &[u8] = b"ZAPLEX_STARTUP_EXECUTED";

            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx)
            });
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_session_msg(), ctx)
            });

            let mut observed = Vec::new();
            let session_id = loop {
                let message = recv_deadline(&conn_rx, Duration::from_secs(10))
                    .await
                    .expect("session opened before the deadline");
                match message.message {
                    Some(server_message::Message::SessionOpened(opened)) => {
                        break opened.session_id;
                    }
                    Some(server_message::Message::SessionOutput(output)) => {
                        observed.extend_from_slice(&output.bytes);
                    }
                    _ => {}
                }
            };
            if !observed
                .windows(BOOTSTRAPPED_HEX.len())
                .any(|window| window == BOOTSTRAPPED_HEX)
            {
                observed.extend(
                    collect_output_until(&conn_rx, BOOTSTRAPPED_HEX, Duration::from_secs(20))
                        .await
                        .expect("daemon shell reached the Bootstrapped hook"),
                );
            }
            assert!(
                !observed
                    .windows(BOOTSTRAP_SOURCE.len())
                    .any(|window| window == BOOTSTRAP_SOURCE),
                "bootstrap source bytes must not be visible terminal output"
            );

            let startup = format!(
                "printf 'ZAPLEX_STARTUP_%s\\n' EXECUTED # {}",
                String::from_utf8_lossy(STARTUP_SOURCE)
            );
            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id,
                    startup_input_msg(
                        "hidden-startup-request",
                        &session_id,
                        "hidden-startup-command",
                        format!("{startup}\n").as_bytes(),
                    ),
                    ctx,
                )
            });

            let startup_output =
                collect_output_until(&conn_rx, STARTUP_RESULT, Duration::from_secs(15))
                    .await
                    .expect("startup command executed and produced its result");
            assert!(
                !startup_output
                    .windows(STARTUP_SOURCE.len())
                    .any(|window| window == STARTUP_SOURCE),
                "startup command source bytes must not be visible terminal output"
            );

            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, close_msg(&session_id), ctx)
            });
        });
    }

    fn input_msg(session_id: &str, bytes: &[u8]) -> ClientMessage {
        ClientMessage {
            request_id: String::new(),
            message: Some(client_message::Message::SessionInput(SessionInput {
                session_id: session_id.to_string(),
                bytes: bytes.to_vec(),
                startup_command_id: String::new(),
            })),
        }
    }

    fn startup_input_msg(
        request_id: &str,
        session_id: &str,
        startup_command_id: &str,
        bytes: &[u8],
    ) -> ClientMessage {
        assert!(
            !startup_command_id.is_empty(),
            "startup delivery requires a stable non-empty command id"
        );
        ClientMessage {
            request_id: request_id.to_string(),
            message: Some(client_message::Message::SessionInput(SessionInput {
                session_id: session_id.to_string(),
                bytes: bytes.to_vec(),
                startup_command_id: startup_command_id.to_string(),
            })),
        }
    }

    /// Returns the next correlated startup-command acknowledgement, skipping
    /// unrelated session output that may still be arriving from shell bootstrap.
    async fn recv_startup_command_ack(
        rx: &async_channel::Receiver<ServerMessage>,
    ) -> Option<(String, String, String, bool)> {
        for _ in 0..100 {
            match recv_deadline(rx, Duration::from_secs(2)).await {
                Some(message) => {
                    let request_id = message.request_id;
                    if let Some(server_message::Message::StartupCommandAck(ack)) = message.message {
                        return Some((
                            request_id,
                            ack.session_id,
                            ack.startup_command_id,
                            ack.accepted,
                        ));
                    }
                }
                None => return None,
            }
        }
        None
    }

    /// Startup input is a retryable request, not an ordinary fire-and-forget
    /// keystroke. The daemon acknowledges every accepted request while using
    /// the stable command id to enqueue identical retries exactly once.
    #[test]
    fn lost_ack_after_execution_is_deduplicated_on_retry() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx)
            });
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_session_msg(), ctx)
            });
            let session_id = recv_session_opened(&conn_rx).await.expect("session opened");

            // Replace only the ordered writer queue with a probe. This keeps the
            // real server routing and per-session state while making the number
            // and exact contents of accepted writes deterministic.
            let (writer_tx, writer_rx) =
                async_channel::unbounded::<crate::remote_server::session_host::PtyInput>();
            model.update(&mut app, |m, _ctx| {
                m.sessions.get_mut(&session_id).unwrap().input_tx = writer_tx;
            });

            let bytes = b"codex resume stable-session\n";
            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id,
                    startup_input_msg("startup-1", &session_id, "command-1", bytes),
                    ctx,
                )
            });
            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id,
                    startup_input_msg("startup-1-retry", &session_id, "command-1", bytes),
                    ctx,
                )
            });
            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id,
                    startup_input_msg(
                        "startup-2",
                        &session_id,
                        "command-2",
                        b"claude --resume another-session\n",
                    ),
                    ctx,
                )
            });

            assert_eq!(
                recv_startup_command_ack(&conn_rx).await,
                Some((
                    "startup-1".to_string(),
                    session_id.clone(),
                    "command-1".to_string(),
                    true,
                )),
                "the first accepted delivery receives a positive acknowledgement"
            );
            assert_eq!(
                recv_startup_command_ack(&conn_rx).await,
                Some((
                    "startup-1-retry".to_string(),
                    session_id.clone(),
                    "command-1".to_string(),
                    true,
                )),
                "an already accepted command id is acknowledged again"
            );
            assert_eq!(
                recv_startup_command_ack(&conn_rx).await,
                Some((
                    "startup-2".to_string(),
                    session_id.clone(),
                    "command-2".to_string(),
                    true,
                )),
                "a different command id is accepted independently"
            );

            assert_eq!(writer_rx.recv().await.unwrap().into_bytes(), bytes);
            assert_eq!(
                writer_rx.recv().await.unwrap().into_bytes(),
                b"claude --resume another-session\n"
            );
            assert!(
                writer_rx.try_recv().is_err(),
                "the duplicate command id must not enqueue its bytes twice"
            );

            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, close_msg(&session_id), ctx)
            });
        });
    }

    #[test]
    fn malformed_startup_input_is_rejected_before_enqueue() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx)
            });
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_session_msg(), ctx)
            });
            let session_id = recv_session_opened(&conn_rx).await.expect("session opened");

            let (writer_tx, writer_rx) =
                async_channel::unbounded::<crate::remote_server::session_host::PtyInput>();
            model.update(&mut app, |m, _ctx| {
                m.sessions.get_mut(&session_id).unwrap().input_tx = writer_tx;
            });

            for (index, bytes) in [
                &b""[..],
                &b"missing-final-newline"[..],
                &b"first\nsecond\n"[..],
                &b"first\r\nsecond\n"[..],
            ]
            .into_iter()
            .enumerate()
            {
                let request_id = format!("malformed-{index}");
                let command_id = format!("malformed-command-{index}");
                model.update(&mut app, |m, ctx| {
                    m.handle_message(
                        conn_id,
                        startup_input_msg(&request_id, &session_id, &command_id, bytes),
                        ctx,
                    )
                });
                assert_eq!(
                    recv_startup_command_ack(&conn_rx).await,
                    Some((request_id, session_id.clone(), command_id, false,))
                );
            }

            assert!(
                writer_rx.try_recv().is_err(),
                "malformed startup input must never reach the PTY writer"
            );
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, close_msg(&session_id), ctx)
            });
        });
    }

    /// A command id becomes complete only after the ordered writer accepted its
    /// bytes. Recording it before `try_send` succeeds would turn a transient
    /// queue failure into permanent command loss on retry.
    #[test]
    fn startup_command_is_retained_when_writer_is_closed() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx)
            });
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_session_msg(), ctx)
            });
            let session_id = recv_session_opened(&conn_rx).await.expect("session opened");

            let (failed_tx, failed_rx) =
                async_channel::unbounded::<crate::remote_server::session_host::PtyInput>();
            drop(failed_rx);
            model.update(&mut app, |m, _ctx| {
                m.sessions.get_mut(&session_id).unwrap().input_tx = failed_tx;
            });
            let bytes = b"codex resume retry-after-writer-failure\n";
            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id,
                    startup_input_msg("failed-attempt", &session_id, "command-retry", bytes),
                    ctx,
                )
            });

            assert_eq!(
                recv_startup_command_ack(&conn_rx).await,
                Some((
                    "failed-attempt".to_string(),
                    session_id.clone(),
                    "command-retry".to_string(),
                    false,
                )),
                "a failed writer enqueue must never receive a positive acknowledgement"
            );

            // Repair the writer and retry the same stable command id. Acceptance
            // proves the failed attempt did not poison the deduplication ledger.
            let (healthy_tx, healthy_rx) =
                async_channel::unbounded::<crate::remote_server::session_host::PtyInput>();
            model.update(&mut app, |m, _ctx| {
                m.sessions.get_mut(&session_id).unwrap().input_tx = healthy_tx;
            });
            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id,
                    startup_input_msg("successful-retry", &session_id, "command-retry", bytes),
                    ctx,
                )
            });

            assert_eq!(
                recv_startup_command_ack(&conn_rx).await,
                Some((
                    "successful-retry".to_string(),
                    session_id.clone(),
                    "command-retry".to_string(),
                    true,
                ))
            );
            assert_eq!(healthy_rx.recv().await.unwrap().into_bytes(), bytes);
            assert!(
                healthy_rx.try_recv().is_err(),
                "the successful retry is enqueued exactly once"
            );

            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, close_msg(&session_id), ctx)
            });
        });
    }

    #[test]
    fn startup_command_ledger_is_bounded_without_breaking_known_id_deduplication() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx)
            });
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_session_msg(), ctx)
            });
            let session_id = recv_session_opened(&conn_rx).await.expect("session opened");

            let (writer_tx, writer_rx) =
                async_channel::unbounded::<crate::remote_server::session_host::PtyInput>();
            model.update(&mut app, |m, _ctx| {
                m.sessions.get_mut(&session_id).unwrap().input_tx = writer_tx;
            });

            for index in 0..crate::remote_server::session_host::MAX_ACCEPTED_STARTUP_COMMANDS {
                let request_id = format!("bounded-request-{index}");
                let command_id = format!("bounded-command-{index}");
                let bytes = format!("command-{index}\n");
                model.update(&mut app, |m, ctx| {
                    m.handle_message(
                        conn_id,
                        startup_input_msg(&request_id, &session_id, &command_id, bytes.as_bytes()),
                        ctx,
                    )
                });
            }

            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id,
                    startup_input_msg(
                        "overflow-request",
                        &session_id,
                        "overflow-command",
                        b"must-not-run\n",
                    ),
                    ctx,
                )
            });
            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id,
                    startup_input_msg(
                        "known-retry",
                        &session_id,
                        "bounded-command-0",
                        b"command-0\n",
                    ),
                    ctx,
                )
            });

            for index in 0..crate::remote_server::session_host::MAX_ACCEPTED_STARTUP_COMMANDS {
                assert_eq!(
                    recv_startup_command_ack(&conn_rx).await,
                    Some((
                        format!("bounded-request-{index}"),
                        session_id.clone(),
                        format!("bounded-command-{index}"),
                        true,
                    ))
                );
                assert_eq!(
                    writer_rx.try_recv().unwrap().into_bytes(),
                    format!("command-{index}\n").into_bytes()
                );
            }
            assert_eq!(
                recv_startup_command_ack(&conn_rx).await,
                Some((
                    "overflow-request".to_string(),
                    session_id.clone(),
                    "overflow-command".to_string(),
                    false,
                )),
                "a new id beyond the fixed ledger ceiling is rejected"
            );
            assert_eq!(
                recv_startup_command_ack(&conn_rx).await,
                Some((
                    "known-retry".to_string(),
                    session_id.clone(),
                    "bounded-command-0".to_string(),
                    true,
                )),
                "a known id remains positively deduplicated after the ledger is full"
            );
            assert!(
                writer_rx.try_recv().is_err(),
                "neither the overflow id nor the known retry may enqueue more bytes"
            );

            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, close_msg(&session_id), ctx)
            });
        });
    }

    fn attach_msg(session_id: &str, last_seq: u64) -> ClientMessage {
        ClientMessage {
            request_id: "attach-1".to_string(),
            message: Some(client_message::Message::AttachSession(AttachSession {
                session_id: session_id.to_string(),
                last_seq,
                supports_bootstrap_preamble: true,
                expected_generation: None,
                expected_agent_binding: None,
            })),
        }
    }

    #[test]
    fn generation_checked_attach_validates_agent_and_transfers_binding_authority() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (first_tx, first_rx) = async_channel::unbounded::<ServerMessage>();
            let first = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(first, first_tx, ctx)
            });
            model.update(&mut app, |m, ctx| {
                m.handle_message(first, open_session_msg(), ctx)
            });
            let session_id = recv_session_opened(&first_rx)
                .await
                .expect("session opened");
            let generation = model.read(&app, |m, _| m.sessions[&session_id].generation);

            let (second_tx, _second_rx) = async_channel::unbounded::<ServerMessage>();
            let second = uuid::Uuid::new_v4();
            let (probe_tx, probe_rx) =
                async_channel::unbounded::<crate::remote_server::session_host::PtyInput>();
            model.update(&mut app, |m, ctx| {
                m.register_connection(second, second_tx, ctx)
            });
            model.update(&mut app, |m, ctx| {
                m.sessions.get_mut(&session_id).unwrap().input_tx = probe_tx;
                m.connection_features.insert(
                    first,
                    std::collections::HashSet::from(["agent-pty-binding-v2".to_string()]),
                );
                m.connection_features.insert(
                    second,
                    std::collections::HashSet::from(["agent-pty-binding-v2".to_string()]),
                );
                assert_eq!(
                    bind_status(
                        m,
                        first,
                        BindAgentPty {
                            agent: Some(binding_identity("agent-1")),
                            pty_session_id: session_id.clone(),
                            pty_session_generation: generation,
                            handoff_from: None,
                            host_id: "test-host-id".to_string(),
                        },
                    ),
                    super::AgentPtyBindingStatus::Bound
                );
                assert_eq!(
                    m.handle_session_input(
                        second,
                        SessionInput {
                            session_id: session_id.clone(),
                            bytes: b"foreign".to_vec(),
                            startup_command_id: String::new(),
                        },
                    ),
                    None
                );
                m.handle_resize_session(
                    second,
                    ResizeSession {
                        session_id: session_id.clone(),
                        size: Some(SessionSize {
                            rows: 90,
                            cols: 120,
                            pixel_width: 0,
                            pixel_height: 0,
                        }),
                    },
                );
                assert!(
                    probe_rx.try_recv().is_err(),
                    "a non-owning connection must not write to the foreground PTY"
                );
                assert_eq!(
                    (m.sessions[&session_id].rows, m.sessions[&session_id].cols),
                    (24, 80),
                    "a non-owning connection must not resize the foreground PTY"
                );
                let live_conflict = m.handle_attach_session(
                    second,
                    AttachSession {
                        session_id: session_id.clone(),
                        last_seq: 0,
                        supports_bootstrap_preamble: true,
                        expected_generation: Some(generation),
                        expected_agent_binding: Some(binding_identity("agent-1")),
                    },
                );
                let server_message::Message::Error(error) = live_conflict.into_message() else {
                    panic!("a second live connection must not steal PTY ownership");
                };
                assert!(error.message.contains("already attached"));
                m.deregister_connection(first, ctx);
                let downgraded = m.handle_attach_session(
                    second,
                    AttachSession {
                        session_id: session_id.clone(),
                        last_seq: 0,
                        supports_bootstrap_preamble: true,
                        expected_generation: None,
                        expected_agent_binding: None,
                    },
                );
                let server_message::Message::Error(error) = downgraded.into_message() else {
                    panic!("a capable client must not downgrade to id-only attach");
                };
                assert!(error.message.contains("requires a PTY generation"));
                let stale = m.handle_attach_session(
                    second,
                    AttachSession {
                        session_id: session_id.clone(),
                        last_seq: 0,
                        supports_bootstrap_preamble: true,
                        expected_generation: Some(generation),
                        expected_agent_binding: Some(binding_identity("agent-stale")),
                    },
                );
                let server_message::Message::Error(error) = stale.into_message() else {
                    panic!("a stale agent row must fail before PTY ownership transfers");
                };
                assert!(error.message.contains("foreground agent changed"));
                assert_eq!(
                    m.sessions[&session_id].attached, first,
                    "a rejected stale row must not transfer session ownership"
                );
                assert_eq!(
                    bind_status(
                        m,
                        second,
                        BindAgentPty {
                            agent: Some(binding_identity("agent-1")),
                            pty_session_id: session_id.clone(),
                            pty_session_generation: generation,
                            handoff_from: None,
                            host_id: "test-host-id".to_string(),
                        },
                    ),
                    super::AgentPtyBindingStatus::ForeignConnection,
                    "a rejected stale row must not transfer PTY mutation authority"
                );
                let attached = m
                    .handle_attach_session(
                        second,
                        AttachSession {
                            session_id: session_id.clone(),
                            last_seq: 0,
                            supports_bootstrap_preamble: true,
                            expected_generation: Some(generation),
                            expected_agent_binding: Some(binding_identity("agent-1")),
                        },
                    )
                    .into_message();
                let server_message::Message::SessionAttached(attached) = attached else {
                    panic!("expected SessionAttached");
                };
                assert_eq!(
                    attached.agent_binding,
                    Some(binding_identity("agent-1")),
                    "a capability-aware generic adopt must hydrate the foreground identity"
                );
                assert_eq!(
                    m.handle_session_input(
                        second,
                        SessionInput {
                            session_id: session_id.clone(),
                            bytes: b"owned".to_vec(),
                            startup_command_id: String::new(),
                        },
                    ),
                    None
                );
                assert_eq!(
                    probe_rx.try_recv().map(|input| input.into_bytes()),
                    Ok(b"owned".to_vec()),
                    "only the connection that completed attach may write to the PTY"
                );
            });

            model.update(&mut app, |m, _ctx| {
                m.connection_features.insert(
                    first,
                    std::collections::HashSet::from(["agent-pty-binding-v2".to_string()]),
                );
            });
            let bind_request = || BindAgentPty {
                agent: Some(binding_identity("agent-1")),
                pty_session_id: session_id.clone(),
                pty_session_generation: generation,
                handoff_from: None,
                host_id: "test-host-id".to_string(),
            };
            assert_eq!(
                model.update(&mut app, |m, _ctx| {
                    bind_status(m, first, bind_request())
                }),
                super::AgentPtyBindingStatus::ForeignConnection,
                "the old connection must lose mutation authority even after an id-only attach"
            );
            assert_eq!(
                model.update(&mut app, |m, _ctx| {
                    bind_status(m, second, bind_request())
                }),
                super::AgentPtyBindingStatus::Bound
            );

            model.update(&mut app, |m, ctx| {
                m.handle_message(second, close_msg(&session_id), ctx)
            });
            model.read(&app, |m, _ctx| {
                let identity = zaplex_remote_session::agent_binding::AgentIdentity {
                    provider: "codex".to_string(),
                    session_id: "agent-1".to_string(),
                    account_email: Some("agent@example.com".to_string()),
                    config_dir: Some("/home/agent/.codex".to_string()),
                    account_id: None,
                };
                let historical = m.agent_pty_bindings.binding_for(&identity).unwrap();
                assert_eq!(historical.pty_session_id, session_id);
                assert_eq!(historical.pty_generation, generation);
                assert!(!historical.foreground);
            });
        });
    }

    fn close_msg(session_id: &str) -> ClientMessage {
        ClientMessage {
            request_id: String::new(),
            message: Some(client_message::Message::CloseSession(CloseSession {
                session_id: session_id.to_string(),
            })),
        }
    }

    fn detach_msg(session_id: &str) -> ClientMessage {
        ClientMessage {
            request_id: String::new(),
            message: Some(client_message::Message::DetachSession(DetachSession {
                session_id: session_id.to_string(),
            })),
        }
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|w| w == needle)
    }

    /// Stage 3 core: a session keeps running while the client is gone, buffers
    /// its output in the ring, and replays it on re-attach — then live output
    /// re-routes to the reconnected connection.
    #[test]
    fn detached_session_buffers_output_and_replays_on_reattach() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx)
            });
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_session_msg(), ctx)
            });

            let session_id = match recv_deadline(&conn_rx, Duration::from_secs(10)).await {
                Some(m) => match m.message {
                    Some(server_message::Message::SessionOpened(o)) => o.session_id,
                    other => panic!("expected SessionOpened, got {other:?}"),
                },
                None => panic!("no SessionOpened before deadline"),
            };

            // Output produced while attached streams normally.
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, input_msg(&session_id, b"echo BEFOR3\n"), ctx)
            });
            assert!(
                wait_for_output(&conn_rx, b"BEFOR3", Duration::from_secs(15)).await,
                "pre-drop output should stream to the attached connection"
            );

            // Simulate a client drop. The session must keep running (no grace
            // shutdown while a session is alive).
            model.update(&mut app, |m, ctx| m.deregister_connection(conn_id, ctx));

            // Output produced WHILE detached can only land in the ring.
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, input_msg(&session_id, b"echo WH1LE_GONE\n"), ctx)
            });

            // Reconnect on a fresh connection and re-attach from seq 0; replay
            // must contain both pre-drop and while-detached output.
            let (conn_tx2, conn_rx2) = async_channel::unbounded::<ServerMessage>();
            let conn_id2 = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id2, conn_tx2, ctx)
            });

            let mut replay_ok = false;
            for _ in 0..50 {
                model.update(&mut app, |m, ctx| {
                    m.handle_message(conn_id2, attach_msg(&session_id, 0), ctx)
                });
                if let Some(msg) = recv_deadline(&conn_rx2, Duration::from_secs(2)).await {
                    if let Some(server_message::Message::SessionAttached(a)) = msg.message {
                        if contains(&a.replay, b"BEFOR3") && contains(&a.replay, b"WH1LE_GONE") {
                            replay_ok = true;
                            break;
                        }
                    }
                }
                async_io::Timer::after(Duration::from_millis(100)).await;
            }
            assert!(
                replay_ok,
                "re-attach replay must include both pre-drop and while-detached output"
            );

            // Live output now re-routes to the re-attached connection.
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id2, input_msg(&session_id, b"echo L1V3_NOW\n"), ctx)
            });
            assert!(
                wait_for_output(&conn_rx2, b"L1V3_NOW", Duration::from_secs(15)).await,
                "live output should re-route to the re-attached connection"
            );

            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id2, close_msg(&session_id), ctx)
            });
        });
    }

    fn open_in(cwd: &str) -> ClientMessage {
        ClientMessage {
            request_id: "open".to_string(),
            message: Some(client_message::Message::OpenSession(OpenSession {
                cwd: Some(cwd.to_string()),
                ring_ceiling_bytes: None,
                shell: Some("/bin/bash".to_string()),
                env: std::collections::HashMap::new(),
                size: Some(SessionSize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                }),
                agent_launch_route: None,
                managed_launch: None,
                requested_min_available_bytes: None,
            })),
        }
    }

    fn list_msg() -> ClientMessage {
        ClientMessage {
            request_id: "list".to_string(),
            message: Some(client_message::Message::ListSessions(ListSessions {})),
        }
    }

    /// First `SessionOpened` on the channel (skips any interleaved output).
    async fn recv_session_opened(rx: &async_channel::Receiver<ServerMessage>) -> Option<String> {
        for _ in 0..20 {
            match recv_deadline(rx, Duration::from_secs(10)).await {
                Some(m) => {
                    if let Some(server_message::Message::SessionOpened(o)) = m.message {
                        return Some(o.session_id);
                    }
                }
                None => return None,
            }
        }
        None
    }

    /// Next `SessionList` on the channel (skips interleaved output / exits).
    async fn recv_session_list(rx: &async_channel::Receiver<ServerMessage>) -> Option<SessionList> {
        for _ in 0..100 {
            match recv_deadline(rx, Duration::from_secs(5)).await {
                Some(m) => {
                    if let Some(server_message::Message::SessionList(list)) = m.message {
                        return Some(list);
                    }
                }
                None => return None,
            }
        }
        None
    }

    fn managed_open(cwd: &str, launch_id: &str) -> ClientMessage {
        ClientMessage {
            request_id: format!("open-{launch_id}"),
            message: Some(client_message::Message::OpenSession(OpenSession {
                cwd: Some(cwd.to_string()),
                shell: Some("/bin/bash".to_string()),
                env: std::collections::HashMap::new(),
                size: Some(SessionSize {
                    rows: 24,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                }),
                ring_ceiling_bytes: None,
                agent_launch_route: Some(crate::remote_server::proto::AgentLaunchRoute {
                    schema_version: 1,
                    provider: "claude".to_string(),
                    account_id: "opaque-account".to_string(),
                }),
                managed_launch: Some(ManagedLaunch {
                    schema_version: 1,
                    launch_id: launch_id.to_string(),
                    provider: "claude".to_string(),
                    project_root: cwd.to_string(),
                    kind: "interactive-agent".to_string(),
                    spawn_mode: String::new(),
                    capacity: 0,
                    permission_mode: String::new(),
                    display_name: String::new(),
                }),
                requested_min_available_bytes: None,
            })),
        }
    }

    fn enable_managed_fleet(model: &mut super::super::ServerModel, conn_id: uuid::Uuid) {
        model.connection_features.insert(
            conn_id,
            std::collections::HashSet::from([
                FEATURE_AGENT_ACCOUNT_ROUTING_V1.to_string(),
                FEATURE_MANAGED_AGENT_FLEET_V1.to_string(),
            ]),
        );
        model
            .agent_account_routes
            .replace_for_test("claude", "opaque-account", None);
        model.fresh_agent_account_routes_for_test =
            Some(model.agent_account_routes.routes_for_test().clone());
    }

    #[test]
    fn transcript_read_rejects_stale_cached_account_after_fresh_same_inode_scan() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            let account = tempfile::tempdir().unwrap();
            let account_root = std::fs::canonicalize(account.path()).unwrap();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx);
                m.connection_features.insert(
                    conn_id,
                    std::collections::HashSet::from([
                        FEATURE_AGENT_ACCOUNT_ROUTING_V1.to_string(),
                        FEATURE_AGENT_TRANSCRIPT_READ_V1.to_string(),
                    ]),
                );
                m.agent_account_routes.replace_for_test(
                    "claude",
                    "stale-account",
                    Some(account_root.clone()),
                );
                let mut fresh_scan =
                    super::super::super::agent_account::AccountRouteCache::default();
                fresh_scan.replace_for_test(
                    "claude",
                    "current-account",
                    Some(account_root.clone()),
                );
                m.fresh_agent_account_routes_for_test = Some(fresh_scan.routes_for_test().clone());
                m.handle_message(
                    conn_id,
                    ClientMessage {
                        request_id: "stale-transcript".to_string(),
                        message: Some(client_message::Message::ReadAgentTranscript(
                            ReadAgentTranscript {
                                schema_version: 1,
                                provider: "claude".to_string(),
                                account_id: "stale-account".to_string(),
                                session_id: "019f135f-7fcc-7d93-8a28-4835d98f8f0a".to_string(),
                                known_revision: String::new(),
                            },
                        )),
                    },
                    ctx,
                );
            });

            let response = loop {
                let message = recv_deadline(&conn_rx, Duration::from_secs(10))
                    .await
                    .expect("fresh transcript route response");
                if let Some(server_message::Message::AgentTranscriptResponse(response)) =
                    message.message
                {
                    break response;
                }
            };
            assert_eq!(
                super::super::super::proto::AgentTranscriptStatus::try_from(response.status)
                    .unwrap(),
                super::super::super::proto::AgentTranscriptStatus::InvalidRequest
            );
            assert!(response.turns.is_empty());
        });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn unexpected_managed_eof_records_a_bounded_restart_identity() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx)
            });
            let project = tempfile::tempdir().unwrap();
            let cwd = project.path().to_string_lossy().to_string();
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_in(&cwd), ctx)
            });
            let session_id = recv_session_opened(&conn_rx)
                .await
                .expect("ordinary session opened");

            model.update(&mut app, |m, ctx| {
                let project_identity =
                    super::super::super::managed_fleet::ManagedProjectIdentity::capture(
                        project.path(),
                    )
                    .unwrap();
                let key = super::super::super::managed_fleet::ManagedLaunchKey::new(
                    &m.host_id,
                    "opaque-account",
                    &cwd,
                    "claude",
                )
                .unwrap();
                let plan =
                    super::super::super::managed_fleet::ManagedLaunchPlan::interactive_agent(
                        "ended-1", key,
                    )
                    .unwrap()
                    .with_project_identity(project_identity);
                let session = m.sessions.get_mut(&session_id).unwrap();
                let process_root =
                    super::super::super::fleet_memory::managed_linux_process_identity(
                        &super::super::super::fleet_memory::RealProcfs,
                        session.child.id(),
                        true,
                    )
                    .unwrap()
                    .unwrap();
                session.managed = Some(
                    super::super::super::managed_fleet::ManagedSessionMetadata::new_verified(
                        plan,
                        process_root,
                        super::super::super::agent_account::AccountRouteIdentity::DefaultAccount,
                    ),
                );
                session.child.kill().unwrap();
                m.on_session_reader_eof(&session_id, ctx);

                assert!(!m.sessions.contains_key(&session_id));
                assert_eq!(m.recent_managed_exits.len(), 1);
                let exit = m.recent_managed_exits[0].to_proto();
                assert_eq!(exit.session_id, session_id);
                assert_eq!(exit.diagnostic_code, "process-ended");
                assert_eq!(exit.managed.unwrap().generation, 1);
            });
        });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_stop_is_visible_and_exactly_restartable_without_eof_double_recording() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx);
                enable_managed_fleet(m, conn_id);
                m.managed_min_available_bytes = Ok(1);
            });
            let project = tempfile::tempdir().unwrap();
            let cwd = project.path().to_string_lossy().to_string();

            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, managed_open(&cwd, "keepalive-1"), ctx)
            });
            let opened = loop {
                let message = recv_deadline(&conn_rx, Duration::from_secs(10))
                    .await
                    .expect("managed open response");
                if let Some(server_message::Message::SessionOpened(opened)) = message.message {
                    break opened;
                }
            };

            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, close_msg(&opened.session_id), ctx);
                assert!(
                    m.sessions.contains_key(&opened.session_id),
                    "generic CloseSession must not remove a managed session"
                );
            });

            model.update(&mut app, |m, ctx| m.deregister_connection(conn_id, ctx));
            model.update(&mut app, |m, _ctx| {
                assert_eq!(m.gc_sessions(u64::MAX, 1, 0), 0);
                assert!(m.sessions.contains_key(&opened.session_id));
            });

            let (conn_tx2, conn_rx2) = async_channel::unbounded::<ServerMessage>();
            let conn_id2 = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id2, conn_tx2, ctx);
                enable_managed_fleet(m, conn_id2);
                m.handle_message(
                    conn_id2,
                    ClientMessage {
                        request_id: "attach-managed".to_string(),
                        message: Some(client_message::Message::AttachSession(AttachSession {
                            session_id: opened.session_id.clone(),
                            last_seq: 0,
                            supports_bootstrap_preamble: true,
                            expected_generation: None,
                            expected_agent_binding: None,
                        })),
                    },
                    ctx,
                );
            });
            let attach_error = loop {
                let message = recv_deadline(&conn_rx2, Duration::from_secs(10))
                    .await
                    .expect("managed attach rejection");
                if let Some(server_message::Message::Error(error)) = message.message {
                    break error;
                }
            };
            assert!(attach_error.message.contains(
                "managed attach requires an exact nonzero generation and foreground agent binding"
            ));
            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id2,
                    ClientMessage {
                        request_id: "attach-managed-without-binding".to_string(),
                        message: Some(client_message::Message::AttachSession(AttachSession {
                            session_id: opened.session_id.clone(),
                            last_seq: 0,
                            supports_bootstrap_preamble: true,
                            expected_generation: Some(opened.generation),
                            expected_agent_binding: None,
                        })),
                    },
                    ctx,
                );
            });
            let missing_binding_error = loop {
                let message = recv_deadline(&conn_rx2, Duration::from_secs(10))
                    .await
                    .expect("managed binding rejection");
                if let Some(server_message::Message::Error(error)) = message.message {
                    break error;
                }
            };
            assert!(missing_binding_error.message.contains(
                "managed attach requires an exact nonzero generation and foreground agent binding"
            ));
            model.read(&app, |m, _ctx| {
                assert!(m.sessions.contains_key(&opened.session_id));
            });

            model.update(&mut app, |m, ctx| {
                let mut changed_routes =
                    super::super::super::agent_account::AccountRouteCache::default();
                changed_routes.replace_for_test("claude", "replacement-account", None);
                m.fresh_agent_account_routes_for_test =
                    Some(changed_routes.routes_for_test().clone());
                m.handle_message(
                    conn_id2,
                    ClientMessage {
                        request_id: "stop-managed-changed-route".to_string(),
                        message: Some(client_message::Message::ManagedSessionLifecycle(
                            ManagedSessionLifecycleRequest {
                                schema_version: 1,
                                action: ManagedSessionLifecycleAction::Stop.into(),
                                session_id: opened.session_id.clone(),
                                expected_generation: opened.generation,
                                launch_id: "keepalive-1".to_string(),
                                provider: "claude".to_string(),
                                account_id: "opaque-account".to_string(),
                                project_root: cwd.clone(),
                            },
                        )),
                    },
                    ctx,
                );
            });
            let changed_route = loop {
                let message = recv_deadline(&conn_rx2, Duration::from_secs(10))
                    .await
                    .expect("changed-route stop response");
                if let Some(server_message::Message::ManagedSessionLifecycleResponse(response)) =
                    message.message
                {
                    break response;
                }
            };
            assert_eq!(
                ManagedSessionLifecycleStatus::try_from(changed_route.status).unwrap(),
                ManagedSessionLifecycleStatus::StaleIdentity
            );
            assert_eq!(changed_route.diagnostic_code, "account-route-changed");
            model.update(&mut app, |m, _ctx| {
                assert!(m.sessions.contains_key(&opened.session_id));
                enable_managed_fleet(m, conn_id2);
            });

            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id2,
                    ClientMessage {
                        request_id: "stop-managed".to_string(),
                        message: Some(client_message::Message::ManagedSessionLifecycle(
                            ManagedSessionLifecycleRequest {
                                schema_version: 1,
                                action: ManagedSessionLifecycleAction::Stop.into(),
                                session_id: opened.session_id.clone(),
                                expected_generation: opened.generation,
                                launch_id: "keepalive-1".to_string(),
                                provider: "claude".to_string(),
                                account_id: "opaque-account".to_string(),
                                project_root: cwd.clone(),
                            },
                        )),
                    },
                    ctx,
                );
            });
            let stopped = loop {
                let message = recv_deadline(&conn_rx2, Duration::from_secs(10))
                    .await
                    .expect("managed stop response");
                if let Some(server_message::Message::ManagedSessionLifecycleResponse(response)) =
                    message.message
                {
                    break response;
                }
            };
            assert_eq!(
                ManagedSessionLifecycleStatus::try_from(stopped.status).unwrap(),
                ManagedSessionLifecycleStatus::Stopped,
                "managed stop failed with diagnostic code: {}",
                stopped.diagnostic_code
            );
            async_io::Timer::after(Duration::from_millis(100)).await;
            model.read(&app, |m, _ctx| {
                assert!(!m.sessions.contains_key(&opened.session_id));
                assert_eq!(m.recent_managed_exits.len(), 1);
                assert_eq!(
                    m.recent_managed_exits[0].diagnostic,
                    super::super::ManagedExitDiagnostic::Stopped
                );
            });

            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id2, list_msg(), ctx)
            });
            let listed = recv_session_list(&conn_rx2)
                .await
                .expect("stopped managed inventory");
            assert!(listed.sessions.is_empty());
            assert_eq!(listed.recent_managed_exits.len(), 1);
            assert_eq!(listed.recent_managed_exits[0].diagnostic_code, "stopped");

            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id2,
                    ClientMessage {
                        request_id: "restart-stale-managed".to_string(),
                        message: Some(client_message::Message::ManagedSessionLifecycle(
                            ManagedSessionLifecycleRequest {
                                schema_version: 1,
                                action: ManagedSessionLifecycleAction::Restart.into(),
                                session_id: opened.session_id.clone(),
                                expected_generation: opened.generation + 1,
                                launch_id: "keepalive-1".to_string(),
                                provider: "claude".to_string(),
                                account_id: "opaque-account".to_string(),
                                project_root: cwd.clone(),
                            },
                        )),
                    },
                    ctx,
                );
            });
            let stale_restart = loop {
                let message = recv_deadline(&conn_rx2, Duration::from_secs(10))
                    .await
                    .expect("stale restart response");
                if let Some(server_message::Message::ManagedSessionLifecycleResponse(response)) =
                    message.message
                {
                    break response;
                }
            };
            assert_eq!(
                ManagedSessionLifecycleStatus::try_from(stale_restart.status).unwrap(),
                ManagedSessionLifecycleStatus::NotRunning
            );

            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id2,
                    ClientMessage {
                        request_id: "restart-stopped-managed".to_string(),
                        message: Some(client_message::Message::ManagedSessionLifecycle(
                            ManagedSessionLifecycleRequest {
                                schema_version: 1,
                                action: ManagedSessionLifecycleAction::Restart.into(),
                                session_id: opened.session_id.clone(),
                                expected_generation: opened.generation,
                                launch_id: "keepalive-1".to_string(),
                                provider: "claude".to_string(),
                                account_id: "opaque-account".to_string(),
                                project_root: cwd.clone(),
                            },
                        )),
                    },
                    ctx,
                );
            });
            let restarted = loop {
                let message = recv_deadline(&conn_rx2, Duration::from_secs(10))
                    .await
                    .expect("exact restart response");
                if let Some(server_message::Message::ManagedSessionLifecycleResponse(response)) =
                    message.message
                {
                    break response;
                }
            };
            assert_eq!(
                ManagedSessionLifecycleStatus::try_from(restarted.status).unwrap(),
                ManagedSessionLifecycleStatus::Restarted
            );
            assert!(!restarted.replacement_session_id.is_empty());
            assert_ne!(restarted.replacement_generation, 0);
            model.update(&mut app, |m, ctx| {
                assert!(m.recent_managed_exits.is_empty());
                m.handle_close_managed_session_verified(&restarted.replacement_session_id, ctx)
                    .unwrap();
                assert_eq!(m.recent_managed_exits.len(), 1);
                assert_eq!(
                    m.recent_managed_exits[0].diagnostic,
                    super::super::ManagedExitDiagnostic::Stopped
                );
            });
        });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_start_is_blocked_before_process_creation_below_daemon_floor() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx);
                enable_managed_fleet(m, conn_id);
                m.managed_min_available_bytes = Ok(u64::MAX);
            });
            let project = tempfile::tempdir().unwrap();
            let cwd = project.path().to_string_lossy().to_string();

            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, managed_open(&cwd, "blocked-1"), ctx)
            });
            let response = loop {
                let message = recv_deadline(&conn_rx, Duration::from_secs(10))
                    .await
                    .expect("managed blocked response");
                if let Some(server_message::Message::Error(error)) = message.message {
                    break error;
                }
            };
            assert!(response.message.contains("below-floor"));
            model.update(&mut app, |m, _ctx| assert!(m.sessions.is_empty()));
        });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn managed_start_rejects_stale_cached_account_after_fresh_same_inode_scan() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            let account = tempfile::tempdir().unwrap();
            let account_root = std::fs::canonicalize(account.path()).unwrap();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx);
                enable_managed_fleet(m, conn_id);
                m.managed_min_available_bytes = Ok(1);
                m.agent_account_routes.replace_for_test(
                    "claude",
                    "opaque-account",
                    Some(account_root.clone()),
                );
                let mut fresh_scan =
                    super::super::super::agent_account::AccountRouteCache::default();
                fresh_scan.replace_for_test(
                    "claude",
                    "replacement-account",
                    Some(account_root.clone()),
                );
                m.fresh_agent_account_routes_for_test = Some(fresh_scan.routes_for_test().clone());
            });
            let project = tempfile::tempdir().unwrap();
            let cwd = project.path().to_string_lossy().to_string();

            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, managed_open(&cwd, "stale-route-1"), ctx)
            });
            let response = loop {
                let message = recv_deadline(&conn_rx, Duration::from_secs(10))
                    .await
                    .expect("managed stale-route response");
                if let Some(server_message::Message::Error(error)) = message.message {
                    break error;
                }
            };
            assert!(response.message.contains("account-route-changed"));
            model.update(&mut app, |m, _ctx| assert!(m.sessions.is_empty()));
        });
    }

    /// Stage 4: multiple sessions per daemon are listable, carry their cwd, and
    /// the list shrinks when a session is closed.
    #[test]
    fn list_sessions_reports_open_sessions() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx)
            });

            // Real, existing working directories — the daemon chdirs the PTY in.
            let dir_a = tempfile::tempdir().unwrap();
            let dir_b = tempfile::tempdir().unwrap();
            let path_a = dir_a.path().to_string_lossy().to_string();
            let path_b = dir_b.path().to_string_lossy().to_string();

            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_in(&path_a), ctx)
            });
            let id_a = recv_session_opened(&conn_rx)
                .await
                .expect("session A opened");
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_in(&path_b), ctx)
            });
            let id_b = recv_session_opened(&conn_rx)
                .await
                .expect("session B opened");
            assert_ne!(id_a, id_b);

            // ListSessions reports both, each with its cwd, all alive.
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, list_msg(), ctx)
            });
            let list = recv_session_list(&conn_rx).await.expect("SessionList");
            assert_eq!(list.sessions.len(), 2, "two sessions listed");
            assert_eq!(
                list.host_ring_cap_bytes, HOST_RING_CAP_BYTES as u64,
                "the UI must receive the daemon's actual host-wide ring cap"
            );
            let by_id: std::collections::HashMap<&str, &str> = list
                .sessions
                .iter()
                .map(|s| (s.session_id.as_str(), s.cwd.as_str()))
                .collect();
            assert_eq!(by_id.get(id_a.as_str()), Some(&path_a.as_str()));
            assert_eq!(by_id.get(id_b.as_str()), Some(&path_b.as_str()));
            assert!(list.sessions.iter().all(|s| s.alive));

            // Closing one shrinks the list to the survivor.
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, close_msg(&id_a), ctx)
            });
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, list_msg(), ctx)
            });
            let list2 = recv_session_list(&conn_rx)
                .await
                .expect("SessionList after close");
            assert_eq!(list2.sessions.len(), 1);
            assert_eq!(list2.sessions[0].session_id, id_b);

            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, close_msg(&id_b), ctx)
            });
        });
    }

    /// Stage 4 memory governor: the GC reaps idle detached sessions (age) and,
    /// when over the host ring cap, the oldest detached ones — never live ones.
    #[test]
    fn gc_reaps_idle_then_over_cap_detached_sessions() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx)
            });

            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().to_string_lossy().to_string();
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_in(&path), ctx)
            });
            let id1 = recv_session_opened(&conn_rx)
                .await
                .expect("session 1 opened");
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_in(&path), ctx)
            });
            let id2 = recv_session_opened(&conn_rx)
                .await
                .expect("session 2 opened");

            // Drop the connection: both sessions detach but keep running (the
            // grace guard keeps the daemon up).
            model.update(&mut app, |m, ctx| m.deregister_connection(conn_id, ctx));
            model.update(&mut app, |m, _ctx| {
                m.sessions.get_mut(&id1).unwrap().last_attached_ms = 0;
                m.sessions.get_mut(&id2).unwrap().last_attached_ms = 1_000_000_000_000;
            });

            // Age GC (60s max, unlimited cap): reap ancient id1, keep recent id2.
            let reaped = model.update(&mut app, |m, _ctx| {
                m.gc_sessions(1_000_000_000_000, 60_000, usize::MAX)
            });
            assert_eq!(reaped, 1, "ancient detached session reaped");
            model.update(&mut app, |m, _ctx| {
                assert!(!m.sessions.contains_key(&id1), "id1 reaped");
                assert!(m.sessions.contains_key(&id2), "id2 kept");
            });

            // Give id2 ring bytes, then a zero host cap reaps it (poll until its
            // shell output has landed in the ring).
            model.update(&mut app, |m, ctx| {
                m.handle_message(uuid::Uuid::new_v4(), input_msg(&id2, b"echo GC\n"), ctx)
            });
            let mut reaped2 = 0;
            for _ in 0..50 {
                reaped2 = model.update(&mut app, |m, _ctx| {
                    m.gc_sessions(1_000_000_000_000, u64::MAX, 0)
                });
                if reaped2 == 1 {
                    break;
                }
                async_io::Timer::after(Duration::from_millis(100)).await;
            }
            assert_eq!(
                reaped2, 1,
                "over-cap detached session reaped once it has ring bytes"
            );
            model.update(&mut app, |m, _ctx| {
                assert!(m.sessions.is_empty(), "all sessions reaped")
            });
        });
    }

    /// Regression: a `DetachSession` from a connection that no longer owns the
    /// attachment must be ignored. Otherwise a late detach from a closed/old tab
    /// would steal the attachment from a newer tab that adopted the same session,
    /// silently cutting off the new tab's live output.
    #[test]
    fn stale_detach_does_not_steal_a_newer_attachment() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_a = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_a, conn_tx, ctx)
            });

            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().to_string_lossy().to_string();
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_a, open_in(&path), ctx)
            });
            let id = recv_session_opened(&conn_rx).await.expect("session opened");

            // A newer tab (conn_b) adopts the session — it now owns the attachment.
            let conn_b = uuid::Uuid::new_v4();
            model.update(&mut app, |m, _| {
                m.sessions.get_mut(&id).unwrap().attached = conn_b;
            });

            // A stale detach from the OLD connection must NOT clear it.
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_a, detach_msg(&id), ctx)
            });
            model.update(&mut app, |m, _| {
                assert_eq!(
                    m.sessions.get(&id).unwrap().attached,
                    conn_b,
                    "stale detach from a non-owner must be ignored"
                );
            });

            // The current owner's detach does clear it.
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_b, detach_msg(&id), ctx)
            });
            model.update(&mut app, |m, _| {
                assert_eq!(
                    m.sessions.get(&id).unwrap().attached,
                    uuid::Uuid::nil(),
                    "the owning connection's detach clears the attachment"
                );
            });
        });
    }

    /// Regression: when the GC reaps the *last* session while no proxies are
    /// connected, the daemon must arm its shutdown grace timer. Otherwise it
    /// lingers forever — `deregister_connection` deliberately skips the timer
    /// while a session still exists, and nothing re-evaluated idleness after the
    /// session was later reaped. Covers `maybe_arm_grace_after_gc`.
    #[test]
    fn gc_reaping_last_session_arms_grace_timer() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx)
            });

            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().to_string_lossy().to_string();
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_in(&path), ctx)
            });
            let _id = recv_session_opened(&conn_rx).await.expect("session opened");

            // Client drops but the session keeps running: no grace timer yet.
            model.update(&mut app, |m, ctx| m.deregister_connection(conn_id, ctx));
            model.update(&mut app, |m, _ctx| {
                assert!(
                    m.grace_timer_cancel.is_none(),
                    "session still alive — daemon stays up without a grace timer"
                );
                // Age the session so the next GC sweep reaps it.
                for s in m.sessions.values_mut() {
                    s.last_attached_ms = 0;
                }
            });

            // GC reaps the now-ancient last session, then re-evaluates idleness.
            model.update(&mut app, |m, ctx| {
                let reaped = m.gc_sessions(1_000_000_000_000, 60_000, usize::MAX);
                assert_eq!(reaped, 1, "the last detached session is reaped");
                m.maybe_arm_grace_when_idle(ctx);
            });

            model.update(&mut app, |m, _ctx| {
                assert!(m.sessions.is_empty(), "no sessions remain");
                assert!(
                    m.grace_timer_cancel.is_some(),
                    "GC emptied the daemon with no connections — grace timer must be armed"
                );
            });
        });
    }

    /// A daemon session must be a real Zaplex terminal (blocks / prompt marks /
    /// completions), not a bare VT. That takes two *independent* pieces, and this
    /// test pins both so a regression in either fails loudly:
    ///
    ///   1. **Shell integration ran** — the daemon injects the Zaplexify init
    ///      script as the session's first input. On startup that script emits the
    ///      InitShell DCS hook (`ESC P $ d …`); it appears in the session output
    ///      only if the bootstrap injection actually happened. (The script does
    ///      *not* set TERM_PROGRAM — that is piece 2.)
    ///   2. **Terminal identity** — the shell is spawned with
    ///      `TERM_PROGRAM=ZaplexTerminal` (a spawn env var in `spawn_session_pty`,
    ///      not from the script). Proven by `echo TP=$TERM_PROGRAM` printing the
    ///      executed value: the echoed input carries the literal `$TERM_PROGRAM`,
    ///      so `TP=ZaplexTerminal` appears only if the env var is really set.
    #[test]
    fn daemon_session_runs_zaplexify_bootstrap() {
        App::test((), |mut app| async move {
            let model = app.add_singleton_model(|_ctx| test_model());
            let (conn_tx, conn_rx) = async_channel::unbounded::<ServerMessage>();
            let conn_id = uuid::Uuid::new_v4();
            model.update(&mut app, |m, ctx| {
                m.register_connection(conn_id, conn_tx, ctx)
            });
            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, open_session_msg(), ctx)
            });
            let session_id = recv_session_opened(&conn_rx).await.expect("session opened");

            // (1) The integration script runs on open and emits the InitShell DCS
            // hook (ESC P $ d …) before any input of ours — this is what no bare
            // VT would produce.
            assert!(
                wait_for_output(&conn_rx, b"\x1bP$d", Duration::from_secs(20)).await,
                "daemon shell should run the Zaplexify integration (InitShell DCS hook in output)"
            );

            // (2) The shell carries the Zaplex terminal identity env.
            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id,
                    input_msg(&session_id, b"echo TP=$TERM_PROGRAM\n"),
                    ctx,
                )
            });
            assert!(
                wait_for_output(&conn_rx, b"TP=ZaplexTerminal", Duration::from_secs(20)).await,
                "daemon shell should be spawned with TERM_PROGRAM=ZaplexTerminal"
            );

            // (3) The daemon owns persistence itself, so its login shell must NOT
            // auto-launch the user's terminal multiplexer (byobu/tmux) — otherwise
            // it joins the user's existing session group and cross-contaminates
            // I/O. `BYOBU_DISABLE=1` must be set in the spawn env.
            model.update(&mut app, |m, ctx| {
                m.handle_message(
                    conn_id,
                    input_msg(&session_id, b"echo BD=$BYOBU_DISABLE\n"),
                    ctx,
                )
            });
            assert!(
                wait_for_output(&conn_rx, b"BD=1", Duration::from_secs(20)).await,
                "daemon shell must set BYOBU_DISABLE=1 (no multiplexer auto-attach)"
            );

            model.update(&mut app, |m, ctx| {
                m.handle_message(conn_id, close_msg(&session_id), ctx)
            });
        });
    }
}
