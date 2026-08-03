use std::collections::{HashMap, HashSet};

use std::fs;

use super::super::proto::{
    list_directory_response, read_file_chunk_response, resolve_path_response, server_message,
    write_file_chunk_response, AgentProcessSignal, AgentProcessSignalRequest,
    AgentProcessSignalStatus, AgentPtyBindingStatus, AgentSessionIdentity, Authenticate,
    BindAgentPty, CreateDirectory, Initialize, ListDirectory, ReadFileChunk, ResolvePath,
    UnbindAgentPty, WriteFileChunk,
};
use super::super::protocol::RequestId;
#[cfg(feature = "local_fs")]
use super::super::server_buffer_tracker::ServerBufferTracker;
use super::{execute_agent_process_signal_with, PendingFileOps, ServerModel};
use zaplex_cockpit::{GuardrailSignal, ProcessSignalError};
#[cfg(unix)]
use zaplex_remote_session::types::FEATURE_MULTIPLEXER_INVENTORY_V1;

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

#[cfg(unix)]
fn binding_identity(session_id: &str) -> AgentSessionIdentity {
    AgentSessionIdentity {
        session_id: session_id.to_string(),
        provider: "codex".to_string(),
        account_email: "agent@example.com".to_string(),
        config_dir: "/home/agent/.codex".to_string(),
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
#[test]
fn legacy_client_cannot_bind_agent_pty() {
    let mut model = test_model();
    let conn = uuid::Uuid::new_v4();
    model
        .agent_pty_bindings
        .register_pty("pty-1", 7, conn.as_u128());

    let status = binding_status(model.handle_bind_agent_pty(
        conn,
        BindAgentPty {
            agent: Some(binding_identity("agent-1")),
            pty_session_id: "pty-1".to_string(),
            pty_session_generation: 7,
            handoff_from: None,
        },
    ));

    assert_eq!(status, AgentPtyBindingStatus::CapabilityRequired);
}

#[cfg(unix)]
#[test]
fn daemon_bind_and_unbind_preserve_historical_agent() {
    let mut model = test_model();
    let conn = uuid::Uuid::new_v4();
    model
        .connection_features
        .insert(conn, HashSet::from(["agent-pty-binding".to_string()]));
    model
        .agent_pty_bindings
        .register_pty("pty-1", 7, conn.as_u128());
    let identity = binding_identity("agent-1");

    assert_eq!(
        binding_status(model.handle_bind_agent_pty(
            conn,
            BindAgentPty {
                agent: Some(identity.clone()),
                pty_session_id: "pty-1".to_string(),
                pty_session_generation: 7,
                handoff_from: None,
            },
        )),
        AgentPtyBindingStatus::Bound
    );
    assert_eq!(
        binding_status(model.handle_unbind_agent_pty(
            conn,
            UnbindAgentPty {
                agent: Some(identity.clone()),
                pty_session_id: "pty-1".to_string(),
                pty_session_generation: 7,
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
            .insert(conn, HashSet::from(["agent-pty-binding".to_string()]));
    }
    model
        .agent_pty_bindings
        .register_pty("pty-1", 7, owner.as_u128());

    let request = |generation| BindAgentPty {
        agent: Some(binding_identity("agent-1")),
        pty_session_id: "pty-1".to_string(),
        pty_session_generation: generation,
        handoff_from: None,
    };
    assert_eq!(
        binding_status(model.handle_bind_agent_pty(owner, request(6))),
        AgentPtyBindingStatus::StaleGeneration
    );
    assert_eq!(
        binding_status(model.handle_bind_agent_pty(foreign, request(7))),
        AgentPtyBindingStatus::ForeignConnection
    );
}

#[test]
fn verified_agent_process_signal_calls_only_the_typed_backend() {
    let response = execute_agent_process_signal_with(
        process_signal_request(AgentProcessSignal::Interrupt),
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
    let response = execute_agent_process_signal_with(request, |_, _, _| {
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
    let response = execute_agent_process_signal_with(request, |_, _, _| {
        panic!("missing identity must never reach the process backend")
    });

    assert_eq!(
        AgentProcessSignalStatus::try_from(response.status),
        Ok(AgentProcessSignalStatus::IdentityUnverifiable)
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
    use super::{binding_identity, binding_status, test_model};
    use crate::remote_server::proto::{
        client_message, server_message, AttachSession, BindAgentPty, ClientMessage, CloseSession,
        DetachSession, ListSessions, OpenSession, ResizeSession, ServerMessage, SessionInput,
        SessionList, SessionSize,
    };
    use futures::future::Either;
    use std::time::Duration;
    use warpui::App;

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
                    std::collections::HashSet::from(["agent-pty-binding".to_string()]),
                );
                m.connection_features.insert(
                    second,
                    std::collections::HashSet::from(["agent-pty-binding".to_string()]),
                );
                assert_eq!(
                    binding_status(m.handle_bind_agent_pty(
                        first,
                        BindAgentPty {
                            agent: Some(binding_identity("agent-1")),
                            pty_session_id: session_id.clone(),
                            pty_session_generation: generation,
                            handoff_from: None,
                        },
                    )),
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
                    binding_status(m.handle_bind_agent_pty(
                        second,
                        BindAgentPty {
                            agent: Some(binding_identity("agent-1")),
                            pty_session_id: session_id.clone(),
                            pty_session_generation: generation,
                            handoff_from: None,
                        },
                    )),
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
                    std::collections::HashSet::from(["agent-pty-binding".to_string()]),
                );
            });
            let bind_request = || BindAgentPty {
                agent: Some(binding_identity("agent-1")),
                pty_session_id: session_id.clone(),
                pty_session_generation: generation,
                handoff_from: None,
            };
            assert_eq!(
                model.update(&mut app, |m, _ctx| {
                    binding_status(m.handle_bind_agent_pty(first, bind_request()))
                }),
                super::AgentPtyBindingStatus::ForeignConnection,
                "the old connection must lose mutation authority even after an id-only attach"
            );
            assert_eq!(
                model.update(&mut app, |m, _ctx| {
                    binding_status(m.handle_bind_agent_pty(second, bind_request()))
                }),
                super::AgentPtyBindingStatus::Bound
            );

            model.update(&mut app, |m, ctx| {
                m.handle_message(second, close_msg(&session_id), ctx)
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
