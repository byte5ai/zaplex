use prost::Message;

use crate::proto::{
    client_message, server_message, AgentSessionInfo, AgentTaskItem, ClientMessage, Initialize,
    InitializeResponse, MultiplexerKind, MultiplexerSessionInfo, MultiplexerSessionList,
    ServerMessage,
};

use super::*;

const LEGACY_DAEMON_INVENTORY: &[u8] = &[
    0x0a, 0x06, b'l', b'e', b'g', b'a', b'c', b'y', 0xea, 0x01, 0x11, 0x0a, 0x0f, 0x0a, 0x07, b'a',
    b'g', b'e', b'n', b't', b'-', b'1', 0x12, 0x04, b'/', b't', b'm', b'p',
];

fn decode_legacy_daemon_inventory() -> crate::proto::AgentSessionList {
    let decoded = ServerMessage::decode(LEGACY_DAEMON_INVENTORY).unwrap();
    assert_eq!(decoded.request_id, "legacy");
    let Some(server_message::Message::AgentSessionList(list)) = decoded.message else {
        panic!("legacy fixture must decode as AgentSessionList");
    };
    list
}

#[tokio::test]
async fn round_trip_client_message() {
    let msg = ClientMessage {
        request_id: "test-123".to_string(),
        message: Some(client_message::Message::Initialize(Initialize {
            auth_token: String::new(),
            features: vec![],
        })),
    };

    let mut buf = Vec::new();
    write_client_message(&mut buf, &msg).await.unwrap();

    let mut cursor = &buf[..];
    let decoded: ClientMessage = read_client_message(&mut cursor).await.unwrap();

    assert_eq!(decoded.request_id, "test-123");
    match decoded.message {
        Some(client_message::Message::Initialize(_)) => {}
        other => panic!("unexpected message variant: {other:?}"),
    }
}

#[tokio::test]
async fn round_trip_server_message() {
    let msg = ServerMessage {
        request_id: "resp-456".to_string(),
        message: Some(server_message::Message::InitializeResponse(
            InitializeResponse {
                server_version: "0.1.0".to_string(),
                host_id: "test-host".to_string(),
                features: vec![],
            },
        )),
    };

    let mut buf = Vec::new();
    write_server_message(&mut buf, &msg).await.unwrap();

    let mut cursor = &buf[..];
    let decoded: ServerMessage = read_server_message(&mut cursor).await.unwrap();

    assert_eq!(decoded.request_id, "resp-456");
    match decoded.message {
        Some(server_message::Message::InitializeResponse(resp)) => {
            assert_eq!(resp.server_version, "0.1.0");
        }
        other => panic!("unexpected message variant: {other:?}"),
    }
}

#[test]
fn real_legacy_client_fixture_interoperates() {
    // Captured old ClientMessage { request_id: "legacy", initialize:
    // Initialize { auth_token: "token" } }. This byte fixture predates the
    // additive Initialize.features field and is deliberately not produced by
    // the current encoder.
    const LEGACY_CLIENT_INITIALIZE: &[u8] = &[
        0x0a, 0x06, b'l', b'e', b'g', b'a', b'c', b'y', 0x12, 0x07, 0x0a, 0x05, b't', b'o', b'k',
        b'e', b'n',
    ];
    let decoded = ClientMessage::decode(LEGACY_CLIENT_INITIALIZE).unwrap();
    let client_message::Message::Initialize(initialize) = decoded.message.unwrap() else {
        panic!("legacy fixture must decode as Initialize");
    };
    assert_eq!(initialize.auth_token, "token");
    assert!(initialize.features.is_empty());
}

#[test]
fn real_legacy_schema_fixture_decodes_without_pty_binding() {
    // Captured old ServerMessage { request_id: "legacy", agent_session_list:
    // [AgentSessionInfo { session_id: "agent-1", cwd: "/tmp" }] }. This full
    // daemon envelope predates every PTY-binding field and is deliberately not
    // produced by the current encoder.
    let list = decode_legacy_daemon_inventory();
    assert_eq!(list.sessions.len(), 1);
    assert_eq!(list.sessions[0].session_id, "agent-1");
    assert!(list.sessions[0].pty_session_id.is_empty());
    assert_eq!(list.sessions[0].pty_session_generation, 0);
    assert!(!list.sessions[0].pty_foreground);
    assert!(!list.sessions[0].has_task_state);
    assert!(list.sessions[0].task_items.is_empty());
}

#[test]
fn older_daemon_without_pty_binding_decodes_safely() {
    let list = decode_legacy_daemon_inventory();
    let session = &list.sessions[0];

    assert!(session.pty_session_id.is_empty());
    assert_eq!(session.pty_session_generation, 0);
    assert!(!session.pty_foreground);
}

#[derive(Clone, PartialEq, Message)]
struct LegacyAgentSessionInfo {
    #[prost(string, tag = "1")]
    session_id: String,
}

#[derive(Clone, PartialEq, Message)]
struct LegacyServerEnvelope {
    #[prost(string, tag = "1")]
    request_id: String,
}

#[test]
fn older_client_ignores_new_pty_binding_field() {
    let current = AgentSessionInfo {
        session_id: "agent-1".to_string(),
        pty_session_id: "pty-7".to_string(),
        pty_session_generation: 42,
        pty_foreground: true,
        ..Default::default()
    };

    let legacy = LegacyAgentSessionInfo::decode(current.encode_to_vec().as_slice()).unwrap();

    assert_eq!(legacy.session_id, "agent-1");
}

#[test]
fn older_client_ignores_new_structured_task_fields() {
    let current = AgentSessionInfo {
        session_id: "agent-1".to_string(),
        has_task_state: true,
        task_items: vec![AgentTaskItem {
            id: "0".to_string(),
            title: "Inspect".to_string(),
            status: "in_progress".to_string(),
        }],
        ..Default::default()
    };

    let legacy = LegacyAgentSessionInfo::decode(current.encode_to_vec().as_slice()).unwrap();

    assert_eq!(legacy.session_id, "agent-1");
}

#[test]
fn older_client_ignores_multiplexer_inventory_response() {
    let current = ServerMessage {
        request_id: "multiplexers".to_string(),
        message: Some(server_message::Message::MultiplexerSessionList(
            MultiplexerSessionList {
                sessions: vec![MultiplexerSessionInfo {
                    kind: MultiplexerKind::Tmux.into(),
                    target: "release; touch /tmp/never".to_string(),
                    name: "release; touch /tmp/never".to_string(),
                    windows: 2,
                    attached_clients: 0,
                }],
                warnings: vec![],
            },
        )),
    };

    let legacy = LegacyServerEnvelope::decode(current.encode_to_vec().as_slice()).unwrap();

    assert_eq!(legacy.request_id, "multiplexers");
}

#[test]
fn real_legacy_session_attach_fixtures_use_id_only() {
    // Captured old ServerMessage { request_id: "legacy", session_list:
    // [SessionInfo { session_id: "pty-old", alive: true }] }. It predates
    // SessionInfo.generation, which must decode to the legacy zero sentinel.
    const LEGACY_SESSION_LIST: &[u8] = &[
        0x0a, 0x06, b'l', b'e', b'g', b'a', b'c', b'y', 0xca, 0x01, 0x0d, 0x0a, 0x0b, 0x0a, 0x07,
        b'p', b't', b'y', b'-', b'o', b'l', b'd', 0x20, 0x01,
    ];
    let decoded = ServerMessage::decode(LEGACY_SESSION_LIST).unwrap();
    let Some(server_message::Message::SessionList(list)) = decoded.message else {
        panic!("legacy fixture must decode as SessionList");
    };
    assert_eq!(list.sessions.len(), 1);
    assert_eq!(list.sessions[0].session_id, "pty-old");
    assert_eq!(list.sessions[0].generation, 0);

    // Captured old ClientMessage { request_id: "legacy", attach_session:
    // AttachSession { session_id: "pty-old" } }. The missing generation check
    // stays absent, so a new client can deliberately retain this wire path when
    // it consumes the zero-generation legacy listing above.
    const LEGACY_ATTACH: &[u8] = &[
        0x0a, 0x06, b'l', b'e', b'g', b'a', b'c', b'y', 0xc2, 0x01, 0x09, 0x0a, 0x07, b'p', b't',
        b'y', b'-', b'o', b'l', b'd',
    ];
    let decoded = ClientMessage::decode(LEGACY_ATTACH).unwrap();
    let Some(client_message::Message::AttachSession(attach)) = decoded.message else {
        panic!("legacy fixture must decode as AttachSession");
    };
    assert_eq!(attach.session_id, "pty-old");
    assert_eq!(attach.expected_generation, None);
    assert_eq!(attach.expected_agent_binding, None);
}

#[tokio::test]
async fn read_unexpected_eof_on_empty_input() {
    let mut cursor: &[u8] = &[];
    let result = read_client_message(&mut cursor).await;
    assert!(matches!(result, Err(ProtocolError::UnexpectedEof)));
}

#[tokio::test]
async fn read_truncated_payload() {
    // Write a length prefix claiming 100 bytes, but only provide 4.
    let mut buf = Vec::new();
    buf.extend_from_slice(&100u32.to_le_bytes());
    buf.extend_from_slice(&[0u8; 4]);

    let mut cursor = &buf[..];
    let result = read_client_message(&mut cursor).await;
    assert!(matches!(result, Err(ProtocolError::UnexpectedEof)));
}

#[tokio::test]
async fn round_trip_zero_length_message() {
    // A default ClientMessage with no fields set encodes to zero bytes.
    let msg = ClientMessage::default();

    let mut buf = Vec::new();
    write_client_message(&mut buf, &msg).await.unwrap();

    // The first 4 bytes should be the length (0).
    assert_eq!(&buf[..4], &0u32.to_le_bytes());

    let mut cursor = &buf[..];
    let decoded: ClientMessage = read_client_message(&mut cursor).await.unwrap();
    assert_eq!(decoded.request_id, "");
    assert!(decoded.message.is_none());
}

#[tokio::test]
async fn read_message_too_large() {
    // Write a length prefix exceeding MAX_MESSAGE_SIZE.
    let oversized_len = (MAX_MESSAGE_SIZE as u32) + 1;
    let buf = oversized_len.to_le_bytes();

    let mut cursor = &buf[..];
    let result = read_client_message(&mut cursor).await;
    assert!(matches!(result, Err(ProtocolError::MessageTooLarge { .. })));
}

#[tokio::test]
async fn write_message_too_large() {
    // Build a ClientMessage whose encoded size exceeds MAX_MESSAGE_SIZE.
    let msg = ClientMessage {
        request_id: "x".repeat(MAX_MESSAGE_SIZE + 1),
        message: None,
    };

    let mut buf = Vec::new();
    let result = write_client_message(&mut buf, &msg).await;
    assert!(matches!(result, Err(ProtocolError::MessageTooLarge { .. })));
    // Nothing should have been written to the stream.
    assert!(buf.is_empty());
}

#[test]
fn try_extract_request_id_from_valid_message() {
    let msg = ClientMessage {
        request_id: "abc-123".to_string(),
        message: Some(client_message::Message::Initialize(Initialize {
            auth_token: String::new(),
            features: vec![],
        })),
    };
    let buf = msg.encode_to_vec();
    assert_eq!(try_extract_request_id(&buf), Some("abc-123".to_string()));
}

#[test]
fn try_extract_request_id_from_corrupted_payload_with_valid_id() {
    // Manually construct bytes: valid request_id field followed by
    // corrupt trailing bytes (unterminated varint that would crash
    // a full prost decode but doesn't affect our field-1 extraction).
    let mut buf = Vec::new();
    // Field 1 (string): tag=0x0a, length=7, "req-456"
    buf.push(0x0a);
    buf.push(7);
    buf.extend_from_slice(b"req-456");
    // Corrupt trailing bytes: unterminated varint (all continuation bits set).
    buf.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);

    // request_id should still be extractable despite trailing corruption.
    assert_eq!(try_extract_request_id(&buf), Some("req-456".to_string()));
}
#[test]
fn try_extract_request_id_from_empty_bytes() {
    assert_eq!(try_extract_request_id(&[]), None);
}

#[test]
fn try_extract_request_id_from_garbage_bytes() {
    // Completely random bytes that don't form a valid protobuf.
    // This may or may not decode depending on what prost makes of it,
    // but should not panic. If it decodes to an empty request_id, we
    // return None.
    let result = try_extract_request_id(&[0xFF, 0xFF, 0xFF, 0xFF]);
    // We don't assert a specific value — just that it doesn't panic.
    // If prost happens to decode something, it'll be empty or garbage.
    let _ = result;
}

#[tokio::test]
async fn decode_error_extracts_request_id() {
    // Construct a corrupted message with a valid request_id field.
    let mut payload = Vec::new();
    // Field 1 (string): tag=0x0a, length=6, "req-42"
    payload.push(0x0a);
    payload.push(6);
    payload.extend_from_slice(b"req-42");
    // Invalid trailing bytes that cause prost decode failure.
    payload.extend_from_slice(&[0x0F, 0x01]);

    let mut buf = Vec::new();
    buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    buf.extend_from_slice(&payload);

    let mut cursor = &buf[..];
    let result = read_client_message(&mut cursor).await;
    match result {
        Err(ProtocolError::Decode(_, Some(id))) => {
            assert_eq!(id.to_string(), "req-42");
        }
        other => panic!("expected Decode error with request_id, got: {other:?}"),
    }
}

#[tokio::test]
async fn decode_error_none_when_no_request_id() {
    // Completely invalid protobuf bytes with no valid field 1.
    let garbage = vec![0xFF, 0xFE, 0xFD, 0xFC];
    let mut buf = Vec::new();
    buf.extend_from_slice(&(garbage.len() as u32).to_le_bytes());
    buf.extend_from_slice(&garbage);

    let mut cursor = &buf[..];
    let result = read_client_message(&mut cursor).await;
    match result {
        Err(ProtocolError::Decode(_, None)) => {}
        other => panic!("expected Decode error with None request_id, got: {other:?}"),
    }
}
