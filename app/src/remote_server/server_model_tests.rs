use std::collections::HashMap;

use std::fs;

use super::super::proto::{
    list_directory_response, read_file_chunk_response, resolve_path_response, server_message,
    write_file_chunk_response, Authenticate, CreateDirectory, Initialize, ListDirectory,
    ReadFileChunk, ResolvePath, WriteFileChunk,
};
use super::super::protocol::RequestId;
#[cfg(feature = "local_fs")]
use super::super::server_buffer_tracker::ServerBufferTracker;
use super::{PendingFileOps, ServerModel};

fn test_model() -> ServerModel {
    ServerModel {
        connection_senders: HashMap::new(),
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
        Initialize {
            auth_token: "initial-token".to_string(),
        },
        &request_id(),
    );

    assert_eq!(model.auth_token(), Some("initial-token"));
}

#[test]
fn empty_initialize_preserves_existing_auth_token() {
    let mut model = test_model();
    model.handle_initialize(
        Initialize {
            auth_token: "initial-token".to_string(),
        },
        &request_id(),
    );

    model.handle_initialize(
        Initialize {
            auth_token: String::new(),
        },
        &request_id(),
    );

    assert_eq!(model.auth_token(), Some("initial-token"));
}

#[test]
fn authenticate_with_auth_token_replaces_auth_token() {
    let mut model = test_model();
    model.handle_initialize(
        Initialize {
            auth_token: "initial-token".to_string(),
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
        Initialize {
            auth_token: "initial-token".to_string(),
        },
        &request_id(),
    );

    model.handle_authenticate(Authenticate {
        auth_token: String::new(),
    });

    assert_eq!(model.auth_token(), Some("initial-token"));
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
    assert_eq!(success.kind, super::super::proto::FileSystemEntryKind::File as i32);
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
    use super::test_model;
    use crate::remote_server::proto::{
        client_message, server_message, AttachSession, ClientMessage, CloseSession, OpenSession,
        ServerMessage, SessionInput, SessionSize,
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
            model.update(&mut app, |m, ctx| m.register_connection(conn_id, conn_tx, ctx));

            // OpenSession -> spawns PTY+shell, replies SessionOpened.
            model.update(&mut app, |m, ctx| m.handle_message(conn_id, open_session_msg(), ctx));

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

    fn input_msg(session_id: &str, bytes: &[u8]) -> ClientMessage {
        ClientMessage {
            request_id: String::new(),
            message: Some(client_message::Message::SessionInput(SessionInput {
                session_id: session_id.to_string(),
                bytes: bytes.to_vec(),
            })),
        }
    }

    fn attach_msg(session_id: &str, last_seq: u64) -> ClientMessage {
        ClientMessage {
            request_id: "attach-1".to_string(),
            message: Some(client_message::Message::AttachSession(AttachSession {
                session_id: session_id.to_string(),
                last_seq,
            })),
        }
    }

    fn close_msg(session_id: &str) -> ClientMessage {
        ClientMessage {
            request_id: String::new(),
            message: Some(client_message::Message::CloseSession(CloseSession {
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
            model.update(&mut app, |m, ctx| m.register_connection(conn_id, conn_tx, ctx));
            model.update(&mut app, |m, ctx| m.handle_message(conn_id, open_session_msg(), ctx));

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
            model.update(&mut app, |m, ctx| m.register_connection(conn_id2, conn_tx2, ctx));

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

            model.update(&mut app, |m, ctx| m.handle_message(conn_id2, close_msg(&session_id), ctx));
        });
    }
}
