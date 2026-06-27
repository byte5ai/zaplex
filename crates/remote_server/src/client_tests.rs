use futures::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::proto::{
    client_message, read_file_chunk_response, resolve_path_response, run_command_response,
    server_message, write_file_chunk_response, ClientMessage, ErrorCode, FileSystemEntryKind,
    InitializeResponse, ReadFileChunkResponse, ReadFileChunkSuccess, ResolvePathResponse,
    ResolvePathSuccess, RunCommandResponse, RunCommandSuccess, ServerMessage, SessionAttached,
    SessionExited, SessionOpened, SessionOutput, SessionSize,
    WriteFileChunkResponse, WriteFileChunkSuccess,
};
use crate::protocol;
use warp_core::SessionId;
use warpui::r#async::executor;

use super::*;

/// Generic mock server: loops reading ClientMessages and responds using the
/// provided closure. Exits cleanly on EOF.
async fn mock_server_with<F>(
    mut reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
    responder: F,
) where
    F: Fn(&ClientMessage) -> server_message::Message,
{
    loop {
        match protocol::read_client_message(&mut reader).await {
            Ok(msg) => {
                let response = ServerMessage {
                    request_id: msg.request_id.clone(),
                    message: Some(responder(&msg)),
                };
                protocol::write_server_message(&mut writer, &response)
                    .await
                    .unwrap();
            }
            Err(protocol::ProtocolError::UnexpectedEof) => break,
            Err(e) => panic!("mock server error: {e}"),
        }
    }
}

/// Sets up a duplex stream, spawns `mock_server_with` with the given responder,
/// and returns a connected `RemoteServerClient`, its event receiver, and the
/// background executor (which must be kept alive for the test duration).
fn setup_mock_client<F>(
    responder: F,
) -> (
    RemoteServerClient,
    async_channel::Receiver<ClientEvent>,
    executor::Background,
)
where
    F: Fn(&ClientMessage) -> server_message::Message + Send + 'static,
{
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);

    tokio::spawn(mock_server_with(
        server_read.compat(),
        server_write.compat_write(),
        responder,
    ));

    let executor = executor::Background::default();
    let (client, event_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);
    (client, event_rx, executor)
}

#[tokio::test]
async fn initialize_round_trip() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|_| {
        server_message::Message::InitializeResponse(InitializeResponse {
            server_version: "test-0.1.0".to_string(),
            host_id: "test-host-id".to_string(),
            features: vec![],
        })
    });

    let resp = client.initialize(None).await.unwrap();
    assert_eq!(resp.server_version, "test-0.1.0");
    assert_eq!(resp.host_id, "test-host-id");
}

#[tokio::test]
async fn initialize_sends_empty_auth_token_when_none() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match &msg.message {
            Some(client_message::Message::Initialize(init)) => {
                assert!(init.auth_token.is_empty());
            }
            other => panic!("Expected Initialize, got {other:?}"),
        }
        server_message::Message::InitializeResponse(InitializeResponse {
            server_version: "test-0.1.0".to_string(),
            host_id: "test-host-id".to_string(),
            features: vec![],
        })
    });

    client.initialize(None).await.unwrap();
}

#[tokio::test]
async fn initialize_sends_auth_token_when_provided() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match &msg.message {
            Some(client_message::Message::Initialize(init)) => {
                assert_eq!(init.auth_token, "secret-token");
            }
            other => panic!("Expected Initialize, got {other:?}"),
        }
        server_message::Message::InitializeResponse(InitializeResponse {
            server_version: "test-0.1.0".to_string(),
            host_id: "test-host-id".to_string(),
            features: vec![],
        })
    });

    client.initialize(Some("secret-token")).await.unwrap();
}

#[tokio::test]
async fn authenticate_sends_fire_and_forget_message() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, _server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let executor = executor::Background::default();
    let (client, _event_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);

    client.authenticate("rotated-secret");

    let msg = protocol::read_client_message(&mut server_read.compat())
        .await
        .unwrap();
    match msg.message {
        Some(client_message::Message::Authenticate(auth)) => {
            assert_eq!(auth.auth_token, "rotated-secret");
        }
        other => panic!("Expected Authenticate, got {other:?}"),
    }
}

#[tokio::test]
async fn disconnected_on_closed_stream() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    // Drop the server side immediately.
    drop(server_stream);

    let (client_read, client_write) = tokio::io::split(client_stream);
    let executor = executor::Background::default();
    let (client, disconnect_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);

    // An initialize call on a dead stream must complete with an error rather than hang.
    let result = client.initialize(None).await;
    assert!(result.is_err());

    // The reader task should detect EOF and emit a Disconnected event.
    let event = disconnect_rx.recv().await.unwrap();
    assert!(matches!(event, ClientEvent::Disconnected));
}

#[tokio::test]
async fn run_command_round_trip() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        let command = match &msg.message {
            Some(client_message::Message::RunCommand(req)) => req.command.clone(),
            other => panic!("Expected RunCommand, got {other:?}"),
        };
        server_message::Message::RunCommandResponse(RunCommandResponse {
            result: Some(run_command_response::Result::Success(RunCommandSuccess {
                stdout: format!("output of: {command}").into_bytes(),
                stderr: Vec::new(),
                exit_code: Some(0),
            })),
        })
    });

    let resp = client
        .run_command(
            SessionId::from(42u64),
            "echo hello".to_string(),
            None,
            Default::default(),
        )
        .await
        .unwrap();
    let success = match resp.result {
        Some(run_command_response::Result::Success(s)) => s,
        other => panic!("Expected RunCommandSuccess, got {other:?}"),
    };
    assert_eq!(success.stdout, b"output of: echo hello");
    assert!(success.stderr.is_empty());
    assert_eq!(success.exit_code, Some(0));
}

#[tokio::test]
async fn resolve_path_round_trip() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match &msg.message {
            Some(client_message::Message::ResolvePath(req)) => {
                assert_eq!(req.path, "~/project");
            }
            other => panic!("Expected ResolvePath, got {other:?}"),
        }
        server_message::Message::ResolvePathResponse(ResolvePathResponse {
            result: Some(resolve_path_response::Result::Success(ResolvePathSuccess {
                canonical_path: "/home/me/project".to_string(),
                kind: FileSystemEntryKind::Directory as i32,
                size_bytes: None,
            })),
        })
    });

    let resp = client.resolve_path("~/project".to_string()).await.unwrap();
    let Some(resolve_path_response::Result::Success(success)) = resp.result else {
        panic!("expected resolve path success");
    };
    assert_eq!(success.canonical_path, "/home/me/project");
    assert_eq!(success.kind, FileSystemEntryKind::Directory as i32);
}

#[tokio::test]
async fn read_file_chunk_round_trip() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match &msg.message {
            Some(client_message::Message::ReadFileChunk(req)) => {
                assert_eq!(req.path, "/tmp/blob.bin");
                assert_eq!(req.offset, 4);
                assert_eq!(req.max_bytes, 2);
            }
            other => panic!("Expected ReadFileChunk, got {other:?}"),
        }
        server_message::Message::ReadFileChunkResponse(ReadFileChunkResponse {
            result: Some(read_file_chunk_response::Result::Success(
                ReadFileChunkSuccess {
                    bytes: vec![5, 6],
                    next_offset: 6,
                    total_size: Some(8),
                    eof: false,
                },
            )),
        })
    });

    let resp = client
        .read_file_chunk("/tmp/blob.bin".to_string(), 4, 2)
        .await
        .unwrap();
    let Some(read_file_chunk_response::Result::Success(success)) = resp.result else {
        panic!("expected read chunk success");
    };
    assert_eq!(success.bytes, vec![5, 6]);
    assert_eq!(success.next_offset, 6);
}

#[tokio::test]
async fn write_file_chunk_round_trip() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match &msg.message {
            Some(client_message::Message::WriteFileChunk(req)) => {
                assert_eq!(req.path, "/tmp/blob.bin");
                assert_eq!(req.offset, 0);
                assert_eq!(req.bytes, vec![1, 2, 3]);
                assert!(req.truncate);
            }
            other => panic!("Expected WriteFileChunk, got {other:?}"),
        }
        server_message::Message::WriteFileChunkResponse(WriteFileChunkResponse {
            result: Some(write_file_chunk_response::Result::Success(
                WriteFileChunkSuccess { next_offset: 3 },
            )),
        })
    });

    let resp = client
        .write_file_chunk("/tmp/blob.bin".to_string(), 0, vec![1, 2, 3], true, None)
        .await
        .unwrap();
    let Some(write_file_chunk_response::Result::Success(success)) = resp.result else {
        panic!("expected write chunk success");
    };
    assert_eq!(success.next_offset, 3);
}

#[tokio::test]
async fn concurrent_in_flight_requests() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|_| {
        server_message::Message::InitializeResponse(InitializeResponse {
            server_version: "test-0.1.0".to_string(),
            host_id: "test-host-id".to_string(),
            features: vec![],
        })
    });
    let client = std::sync::Arc::new(client);

    let mut handles = Vec::new();
    for _ in 0..10 {
        let c = std::sync::Arc::clone(&client);
        handles.push(tokio::spawn(async move {
            c.initialize(None)
                .await
                .expect("concurrent initialize failed")
        }));
    }

    for h in handles {
        let resp = h.await.unwrap();
        assert_eq!(resp.server_version, "test-0.1.0");
        assert_eq!(resp.host_id, "test-host-id");
    }
}

/// Simulates a server that reads raw bytes, sends an error response for
/// malformed messages where the request_id is parseable, then continues
/// processing valid messages.
async fn mock_server_with_error_handling(
    mut reader: impl AsyncRead + Unpin,
    mut writer: impl AsyncWrite + Unpin,
) {
    loop {
        match protocol::read_client_message(&mut reader).await {
            Ok(msg) => {
                let response = ServerMessage {
                    request_id: msg.request_id,
                    message: Some(server_message::Message::InitializeResponse(
                        InitializeResponse {
                            server_version: "test-0.1.0".to_string(),
                            host_id: "test-host-id".to_string(),
                            features: vec![],
                        },
                    )),
                };
                protocol::write_server_message(&mut writer, &response)
                    .await
                    .unwrap();
            }
            Err(protocol::ProtocolError::Decode(_, Some(ref id))) => {
                let error_response = ServerMessage {
                    request_id: id.to_string(),
                    message: Some(server_message::Message::Error(
                        crate::proto::ErrorResponse {
                            code: ErrorCode::InvalidRequest.into(),
                            message: "malformed message".to_string(),
                        },
                    )),
                };
                protocol::write_server_message(&mut writer, &error_response)
                    .await
                    .unwrap();
            }
            Err(protocol::ProtocolError::Decode(_, None)) => {}
            Err(protocol::ProtocolError::UnexpectedEof) => break,
            Err(e) => panic!("mock server error: {e}"),
        }
    }
}

/// Sends a corrupted protobuf with a valid request_id to the server,
/// verifying the server responds with an ErrorResponse for that request_id.
#[tokio::test]
async fn server_returns_error_for_malformed_message_with_parseable_id() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);

    tokio::spawn(mock_server_with_error_handling(
        server_read.compat(),
        server_write.compat_write(),
    ));

    // Manually construct a corrupted message with a valid request_id field
    // followed by bytes that cause a prost decode failure.
    let mut payload = Vec::new();
    // Field 1 (string): tag=0x0a, length=15, "malformed-req-1"
    payload.push(0x0a);
    payload.push(15);
    payload.extend_from_slice(b"malformed-req-1");
    // Invalid trailing bytes: field tag with reserved wire type 7 causes
    // prost to fail, but our try_extract_request_id stops after field 1.
    payload.extend_from_slice(&[0x0F, 0x01]); // field 1, wire type 7 (invalid)

    // Write the corrupted message with length prefix.
    let mut client_write = client_write.compat_write();
    let len = payload.len() as u32;
    client_write.write_all(&len.to_le_bytes()).await.unwrap();
    client_write.write_all(&payload).await.unwrap();
    client_write.flush().await.unwrap();

    // Read the error response from the server.
    let mut client_reader = futures::io::BufReader::new(client_read.compat());
    let response: ServerMessage = protocol::read_server_message(&mut client_reader)
        .await
        .unwrap();

    assert_eq!(response.request_id, "malformed-req-1");
    match response.message {
        Some(server_message::Message::Error(e)) => {
            assert_eq!(e.code(), ErrorCode::InvalidRequest);
        }
        other => panic!("expected ErrorResponse, got: {other:?}"),
    }
}

// ---- Native daemon session protocol (Stage 2) -----------------------------
//
// Headless round-trip coverage for the client half of the daemon-hosted session
// protocol: OpenSession/AttachSession requests, SessionOutput/SessionExited
// server pushes surfacing as ClientEvents, and the fire-and-forget
// input/resize/detach frames. The server PTY-spawn half is covered separately
// by `session_pty_tests` in app/src/terminal/local_tty/unix.rs.

#[tokio::test]
async fn open_session_round_trip() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match &msg.message {
            Some(client_message::Message::OpenSession(open)) => {
                let size = open.size.as_ref().expect("OpenSession carries size");
                assert_eq!(size.rows, 30);
                assert_eq!(size.cols, 100);
                assert_eq!(open.cwd.as_deref(), Some("/home/me"));
                assert_eq!(open.shell.as_deref(), Some("/bin/zsh"));
                assert_eq!(open.env.get("FOO").map(String::as_str), Some("bar"));
            }
            other => panic!("expected OpenSession, got {other:?}"),
        }
        server_message::Message::SessionOpened(SessionOpened {
            session_id: "sess-1".to_string(),
        })
    });

    let mut env = std::collections::HashMap::new();
    env.insert("FOO".to_string(), "bar".to_string());
    let resp = client
        .open_session(
            Some("/home/me".to_string()),
            Some("/bin/zsh".to_string()),
            env,
            30,
            100,
        )
        .await
        .unwrap();
    assert_eq!(resp.session_id, "sess-1");
}

#[tokio::test]
async fn attach_session_round_trip() {
    let (client, _disconnect_rx, _executor) = setup_mock_client(|msg| {
        match &msg.message {
            Some(client_message::Message::AttachSession(att)) => {
                assert_eq!(att.session_id, "sess-1");
                assert_eq!(att.last_seq, 42);
            }
            other => panic!("expected AttachSession, got {other:?}"),
        }
        server_message::Message::SessionAttached(SessionAttached {
            session_id: "sess-1".to_string(),
            size: Some(SessionSize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            }),
            base_seq: 42,
            replay: b"replayed".to_vec(),
        })
    });

    let resp = client.attach_session("sess-1".to_string(), 42).await.unwrap();
    assert_eq!(resp.base_seq, 42);
    assert_eq!(resp.replay, b"replayed");
}

#[tokio::test]
async fn session_output_push_surfaces_as_event() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let executor = executor::Background::default();
    let (_client, event_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);

    tokio::spawn(async move {
        let _server_read = server_read; // keep the duplex open for the reader task
        let mut writer = server_write.compat_write();
        protocol::write_server_message(
            &mut writer,
            &ServerMessage {
                request_id: String::new(), // empty => push
                message: Some(server_message::Message::SessionOutput(SessionOutput {
                    session_id: "sess-1".to_string(),
                    seq: 7,
                    bytes: b"hello pty".to_vec(),
                })),
            },
        )
        .await
        .unwrap();
        std::future::pending::<()>().await;
    });

    match event_rx.recv().await.unwrap() {
        ClientEvent::SessionOutput {
            session_id,
            seq,
            bytes,
        } => {
            assert_eq!(session_id, "sess-1");
            assert_eq!(seq, 7);
            assert_eq!(bytes, b"hello pty");
        }
        other => panic!("expected SessionOutput, got {other:?}"),
    }
}

#[tokio::test]
async fn session_exited_push_surfaces_as_event() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let executor = executor::Background::default();
    let (_client, event_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);

    tokio::spawn(async move {
        let _server_read = server_read;
        let mut writer = server_write.compat_write();
        protocol::write_server_message(
            &mut writer,
            &ServerMessage {
                request_id: String::new(),
                message: Some(server_message::Message::SessionExited(SessionExited {
                    session_id: "sess-1".to_string(),
                    exit_code: Some(0),
                })),
            },
        )
        .await
        .unwrap();
        std::future::pending::<()>().await;
    });

    match event_rx.recv().await.unwrap() {
        ClientEvent::SessionExited {
            session_id,
            exit_code,
        } => {
            assert_eq!(session_id, "sess-1");
            assert_eq!(exit_code, Some(0));
        }
        other => panic!("expected SessionExited, got {other:?}"),
    }
}

/// Reads a single fire-and-forget client frame from the server side of a duplex.
async fn read_one_client_frame(
    client: RemoteServerClient,
    send: impl FnOnce(&RemoteServerClient),
    server_read: tokio::io::ReadHalf<tokio::io::DuplexStream>,
) -> ClientMessage {
    send(&client);
    let mut reader = server_read.compat();
    protocol::read_client_message(&mut reader).await.unwrap()
}

#[tokio::test]
async fn send_session_input_sends_frame() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, _server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let executor = executor::Background::default();
    let (client, _event_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);

    let msg = read_one_client_frame(
        client,
        |c| c.send_session_input("sess-1".to_string(), b"abc".to_vec()).unwrap(),
        server_read,
    )
    .await;
    match msg.message {
        Some(client_message::Message::SessionInput(si)) => {
            assert_eq!(si.session_id, "sess-1");
            assert_eq!(si.bytes, b"abc");
        }
        other => panic!("expected SessionInput, got {other:?}"),
    }
}

#[tokio::test]
async fn send_resize_session_sends_frame() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, _server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let executor = executor::Background::default();
    let (client, _event_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);

    let msg = read_one_client_frame(
        client,
        |c| c.send_resize_session("sess-1".to_string(), 50, 120).unwrap(),
        server_read,
    )
    .await;
    match msg.message {
        Some(client_message::Message::ResizeSession(rs)) => {
            assert_eq!(rs.session_id, "sess-1");
            let size = rs.size.expect("resize carries size");
            assert_eq!(size.rows, 50);
            assert_eq!(size.cols, 120);
        }
        other => panic!("expected ResizeSession, got {other:?}"),
    }
}

#[tokio::test]
async fn send_detach_session_sends_frame() {
    let (client_stream, server_stream) = tokio::io::duplex(4096);
    let (server_read, _server_write) = tokio::io::split(server_stream);
    let (client_read, client_write) = tokio::io::split(client_stream);
    let executor = executor::Background::default();
    let (client, _event_rx) =
        RemoteServerClient::new(client_read.compat(), client_write.compat_write(), &executor);

    let msg = read_one_client_frame(
        client,
        |c| c.send_detach_session("sess-1".to_string()).unwrap(),
        server_read,
    )
    .await;
    match msg.message {
        Some(client_message::Message::DetachSession(d)) => {
            assert_eq!(d.session_id, "sess-1");
        }
        other => panic!("expected DetachSession, got {other:?}"),
    }
}
