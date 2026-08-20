use chrono::Utc;
use std::collections::BTreeMap;
use std::fs;
#[cfg(not(target_family = "wasm"))]
use zaplex_cockpit::{AgentInventoryStatus, FleetTree, HostAvailability, HostNode, ProjectNode};
use zaplex_cockpit::{
    Provider, SessionSnapshot, SessionState, ToolCall, TranscriptTurn, TurnRole, WindowTotals,
};
#[cfg(not(target_family = "wasm"))]
use zaplex_remote_session::types::FEATURE_AGENT_TRANSCRIPT_READ_V1;

#[cfg(not(target_family = "wasm"))]
use crate::remote_server::proto::{
    AgentTranscriptResponse, AgentTranscriptStatus, AgentTranscriptTool, AgentTranscriptTurn,
};

use super::{
    format_provider_transcript_markdown, load_local_codex_transcript,
    local_codex_transcript_error_state, matching_session_row, parse_hex_color, session_key,
    session_table_viewport_height, session_today_cost, table_row_needs_attention,
    transcript_action_target, transcript_state_document, TableRow, TranscriptActionTarget,
    TranscriptDocumentState, SESSION_TABLE_HEADER_HEIGHT, SESSION_TABLE_MAX_VISIBLE_ROWS,
    SESSION_TABLE_ROW_HEIGHT,
};
#[cfg(not(target_family = "wasm"))]
use super::{
    remote_transcript_document, remote_transcript_route, remote_transcript_route_is_current,
    RemoteTranscriptProjectionError, RemoteTranscriptRoute,
};

#[test]
fn remote_host_without_pricing_data_has_no_fabricated_cost() {
    let mut local_totals = BTreeMap::new();
    local_totals.insert(
        "same-session-id".to_string(),
        WindowTotals {
            cost_usd: 4.2,
            ..WindowTotals::default()
        },
    );

    assert_eq!(
        session_today_cost(false, "same-session-id", &local_totals),
        None,
        "a remote row must not inherit a same-id local transcript cost"
    );
    assert_eq!(
        session_today_cost(true, "same-session-id", &local_totals),
        Some(4.2)
    );
}

#[test]
fn session_table_body_clips_to_zone_card() {
    let capped = SESSION_TABLE_HEADER_HEIGHT
        + SESSION_TABLE_MAX_VISIBLE_ROWS as f32 * SESSION_TABLE_ROW_HEIGHT;
    assert_eq!(session_table_viewport_height(1), 62.0);
    assert_eq!(session_table_viewport_height(100), capped);
}

#[test]
fn cockpit_table_clips_to_card() {
    assert_eq!(
        session_table_viewport_height(SESSION_TABLE_MAX_VISIBLE_ROWS + 20),
        session_table_viewport_height(SESSION_TABLE_MAX_VISIBLE_ROWS)
    );
}

fn session(config_dir: Option<&str>, account_email: Option<&str>) -> SessionSnapshot {
    SessionSnapshot {
        session_id: "copied-session".to_string(),
        cwd: "/work/project".to_string(),
        name: "job".to_string(),
        state: SessionState::Idle,
        provider: Provider::Claude,
        model: String::new(),
        effort: None,
        ctx_tokens: 0,
        project_root: "/work/project".to_string(),
        repo_root: "/work/project".to_string(),
        project_name: "project".to_string(),
        branch: None,
        worktree: None,
        config_dir: config_dir.map(str::to_string),
        account_email: account_email.map(str::to_string),
        account_id: None,
        process_fingerprint: None,
        pty_session_id: None,
        pty_session_generation: None,
        pty_foreground: false,
        task_state: None,
        last_activity: Utc::now(),
        pid: 0,
    }
}

fn row(session: SessionSnapshot) -> TableRow {
    TableRow::Session {
        session,
        host: None,
        host_id: None,
        is_local: true,
        today_cost: None,
    }
}

#[test]
fn only_waiting_session_rows_receive_attention_background() {
    let mut waiting = session(None, None);
    waiting.state = SessionState::Waiting;
    assert!(table_row_needs_attention(&row(waiting)));

    let idle = session(None, None);
    assert!(!table_row_needs_attention(&row(idle)));
    assert!(!table_row_needs_attention(&TableRow::Group {
        key: "group".into(),
        name: "project".into(),
        host: None,
        host_id: None,
        count: 1,
        collapsed: false,
    }));
}

#[test]
fn row_menu_resolves_copied_id_to_exact_account() {
    let default = session(None, Some("shared@example.com"));
    let work = session(Some("/accounts/claude-work"), Some("shared@example.com"));
    let work_key = session_key(true, None, &work);
    let rows = vec![row(default), row(work)];

    let matched = matching_session_row(&rows, &work_key).expect("work-account row");
    assert_eq!(
        matched.session.config_dir.as_deref(),
        Some("/accounts/claude-work"),
        "the menu must not take the first same-id row from another account"
    );
}

#[test]
fn row_menu_refuses_duplicate_unknown_account_identity() {
    let first_unknown = session(None, None);
    let second_unknown = session(None, None);
    let ambiguous_key = session_key(true, None, &first_unknown);
    let rows = vec![row(first_unknown), row(second_unknown)];

    assert!(
        matching_session_row(&rows, &ambiguous_key).is_none(),
        "ambiguous legacy rows must fail closed instead of selecting the first account"
    );
}

#[test]
fn parses_six_digit_hex() {
    let c = parse_hex_color("#22C55E").expect("valid 6-digit hex");
    assert_eq!((c.r, c.g, c.b, c.a), (0x22, 0xC5, 0x5E, 255));
}

#[test]
fn parses_three_digit_shorthand() {
    // #f0a → ff 00 aa (each nibble doubled).
    let c = parse_hex_color("#f0a").expect("valid 3-digit hex");
    assert_eq!((c.r, c.g, c.b, c.a), (0xff, 0x00, 0xaa, 255));
}

#[test]
fn rejects_malformed_returns_none() {
    for bad in [
        "", "22C55E", "#", "#12", "#1234", "#12345", "#GGGGGG", "#12345Z",
    ] {
        assert!(parse_hex_color(bad).is_none(), "{bad:?} must not parse");
    }
}

/// The doc promised "never a panic" and did not deliver. `len()` counts
/// bytes; the slices index char boundaries. `#éa` measures 3 bytes, takes
/// the shorthand branch, and `&hex[0..1]` cuts the `é` in half — aborting
/// the app while it renders an account card, over a value someone typed into
/// instances.json by hand.
#[test]
fn a_non_ascii_colour_yields_no_tint_rather_than_taking_the_app_down() {
    // 3 bytes, 2 chars: exactly the shorthand branch's length check.
    assert_eq!(parse_hex_color("#éa"), None);
    // 6 bytes, 3 chars: the same trap on the long branch.
    assert_eq!(parse_hex_color("#ééé"), None);
    assert_eq!(parse_hex_color("#22C55é"), None);
    assert_eq!(parse_hex_color("#🎨🎨"), None);
}

/// Malformed-but-ASCII stays malformed — the guard must not start accepting
/// things it used to reject.
#[test]
fn ascii_rubbish_is_still_rejected() {
    for bad in [
        "#", "#12", "#1234", "#12345", "#1234567", "#GGGGGG", "22C55E", "",
    ] {
        assert_eq!(parse_hex_color(bad), None, "{bad:?} must not parse");
    }
}

/// …and the valid cases still work.
#[test]
fn the_guard_does_not_reject_real_colours() {
    assert!(parse_hex_color("#22C55E").is_some());
    assert!(parse_hex_color("#f0a").is_some());
    assert!(parse_hex_color("#FFFFFF").is_some());
}

#[test]
fn transcript_actions_are_capability_gated_by_provider_and_host() {
    assert_eq!(
        transcript_action_target(Provider::Claude, true, None),
        Some(TranscriptActionTarget::ClaudeWorkspace)
    );
    assert_eq!(
        transcript_action_target(Provider::Codex, true, None),
        Some(TranscriptActionTarget::CodexLocal)
    );
    assert_eq!(
        transcript_action_target(Provider::Antigravity, true, None),
        None
    );
    assert_eq!(
        transcript_action_target(Provider::Claude, false, None),
        None
    );
    assert_eq!(transcript_action_target(Provider::Codex, false, None), None);
}

#[cfg(not(target_family = "wasm"))]
fn remote_route(provider: Provider) -> RemoteTranscriptRoute {
    RemoteTranscriptRoute {
        provider,
        host_id: "host-1".into(),
        account_id: "account-1".into(),
        session_id: "session-1".into(),
    }
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn remote_transcript_action_requires_exact_connected_capable_route() {
    let features = vec![FEATURE_AGENT_TRANSCRIPT_READ_V1.to_string()];
    let route = remote_transcript_route(
        Provider::Claude,
        false,
        Some("host-1"),
        Some("account-1"),
        "session-1",
        Some(("host-1", &features)),
    )
    .expect("connected capable Claude route");
    assert_eq!(
        transcript_action_target(Provider::Claude, false, Some(route.clone())),
        Some(TranscriptActionTarget::Remote(route))
    );

    assert!(remote_transcript_route(
        Provider::Codex,
        false,
        Some("host-1"),
        Some("account-1"),
        "session-1",
        Some(("host-1", &features)),
    )
    .is_some());
    assert!(remote_transcript_route(
        Provider::Antigravity,
        false,
        Some("host-1"),
        Some("account-1"),
        "session-1",
        Some(("host-1", &features)),
    )
    .is_none());
    assert!(remote_transcript_route(
        Provider::Claude,
        true,
        Some("host-1"),
        Some("account-1"),
        "session-1",
        Some(("host-1", &features)),
    )
    .is_none());
    assert!(remote_transcript_route(
        Provider::Claude,
        false,
        Some("host-1"),
        None,
        "session-1",
        Some(("host-1", &features)),
    )
    .is_none());
    assert!(remote_transcript_route(
        Provider::Claude,
        false,
        Some("host-1"),
        Some("account-1"),
        "session-1",
        Some(("host-2", &features)),
    )
    .is_none());
    assert!(remote_transcript_route(
        Provider::Claude,
        false,
        Some("host-1"),
        Some("account-1"),
        "session-1",
        Some(("host-1", &[])),
    )
    .is_none());
    assert!(remote_transcript_route(
        Provider::Claude,
        false,
        Some("host-1/path"),
        Some("account-1"),
        "session-1",
        Some(("host-1/path", &features)),
    )
    .is_none());
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn remote_transcript_route_must_still_match_one_available_inventory_session() {
    let route = remote_route(Provider::Claude);
    let mut remote_session = session(None, None);
    remote_session.provider = Provider::Claude;
    remote_session.session_id = route.session_id.clone();
    remote_session.account_id = Some(route.account_id.clone());
    let tree = FleetTree {
        hosts: vec![HostNode {
            host: "devhost".into(),
            is_local: false,
            host_id: Some(route.host_id.clone()),
            availability: HostAvailability::Available,
            inventory_status: AgentInventoryStatus::Ready,
            registry_node_id: Some("registry-1".into()),
            needs_me: 0,
            projects: vec![ProjectNode {
                root: "/work/project".into(),
                name: "project".into(),
                needs_me: 0,
                sessions: vec![remote_session],
            }],
        }],
        needs_me: 0,
    };
    assert!(remote_transcript_route_is_current(&tree, &route));

    let mut stale_route = route;
    stale_route.account_id = "account-2".into();
    assert!(!remote_transcript_route_is_current(&tree, &stale_route));
}

#[test]
fn provider_neutral_transcript_renderer_labels_codex_without_tool_payloads() {
    let turns = vec![
        TranscriptTurn {
            role: TurnRole::User,
            text: "Inspect the project".into(),
            thinking: String::new(),
            tools: Vec::new(),
            model: None,
            usage: None,
            timestamp: None,
        },
        TranscriptTurn {
            role: TurnRole::Assistant,
            text: "Ready".into(),
            thinking: "Checked the files".into(),
            tools: vec![ToolCall {
                name: "read_file".into(),
            }],
            model: Some("gpt-5.6".into()),
            usage: None,
            timestamp: None,
        },
    ];

    let markdown = format_provider_transcript_markdown(Provider::Codex, &turns);
    assert!(markdown.contains("## Codex · gpt-5.6"));
    assert!(!markdown.contains("## Claude"));
    assert!(markdown.contains("`⚙ read_file`"));
}

#[cfg(not(target_family = "wasm"))]
fn remote_response(status: AgentTranscriptStatus) -> AgentTranscriptResponse {
    AgentTranscriptResponse {
        schema_version: 1,
        provider: "claude".into(),
        session_id: "session-1".into(),
        status: status as i32,
        turns: Vec::new(),
        truncated: false,
        source_revision: "opaque-revision".into(),
        message: "/private/provider/path must-not-appear".into(),
    }
}

#[cfg(not(target_family = "wasm"))]
fn remote_turn(role: &str, text: &str) -> AgentTranscriptTurn {
    AgentTranscriptTurn {
        role: role.into(),
        text: text.into(),
        thinking: String::new(),
        tools: Vec::new(),
        model: String::new(),
        timestamp: String::new(),
    }
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn remote_loaded_transcript_projects_only_safe_provider_neutral_fields() {
    let route = remote_route(Provider::Claude);
    let mut response = remote_response(AgentTranscriptStatus::Loaded);
    response.truncated = true;
    response.turns = vec![
        remote_turn("future-role", "ignored wire content"),
        remote_turn("user", "Question"),
        AgentTranscriptTurn {
            role: "assistant".into(),
            text: "Answer".into(),
            thinking: "Reasoning summary".into(),
            tools: vec![AgentTranscriptTool {
                name: "read_file".into(),
            }],
            model: "claude-opus-4-1".into(),
            timestamp: "2026-08-20T12:00:00Z".into(),
        },
    ];

    let document = remote_transcript_document(&route, response).expect("safe projection");
    assert_eq!(document.state, TranscriptDocumentState::Ready);
    assert!(document.markdown.starts_with("> "));
    assert!(document.markdown.contains("## Claude · claude-opus-4-1"));
    assert!(document.markdown.contains("Question"));
    assert!(document.markdown.contains("Answer"));
    assert!(document.markdown.contains("`⚙ read_file`"));
    assert!(!document.markdown.contains("ignored wire content"));
    assert!(!document.markdown.contains("must-not-appear"));
    assert!(!document.markdown.contains("/private/provider/path"));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn remote_transcript_statuses_map_to_distinct_neutral_documents() {
    for (status, expected) in [
        (
            AgentTranscriptStatus::Missing,
            TranscriptDocumentState::Missing,
        ),
        (AgentTranscriptStatus::Empty, TranscriptDocumentState::Empty),
        (
            AgentTranscriptStatus::Unsupported,
            TranscriptDocumentState::Unsupported,
        ),
        (
            AgentTranscriptStatus::Malformed,
            TranscriptDocumentState::Malformed,
        ),
        (
            AgentTranscriptStatus::TooLarge,
            TranscriptDocumentState::TooLarge,
        ),
        (
            AgentTranscriptStatus::Unavailable,
            TranscriptDocumentState::Unavailable,
        ),
    ] {
        let document =
            remote_transcript_document(&remote_route(Provider::Claude), remote_response(status))
                .expect("typed status document");
        assert_eq!(document.state, expected);
        assert!(!document.markdown.contains("must-not-appear"));
        assert!(!document.markdown.contains("/private/provider/path"));
    }
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn remote_transcript_fails_closed_for_invalid_status_envelope_or_payload() {
    let route = remote_route(Provider::Claude);
    for status in [
        AgentTranscriptStatus::Unspecified,
        AgentTranscriptStatus::NotModified,
        AgentTranscriptStatus::InvalidRequest,
    ] {
        assert_eq!(
            remote_transcript_document(&route, remote_response(status)),
            Err(RemoteTranscriptProjectionError::InvalidStatus)
        );
    }

    let mut unknown_status = remote_response(AgentTranscriptStatus::Missing);
    unknown_status.status = i32::MAX;
    assert_eq!(
        remote_transcript_document(&route, unknown_status),
        Err(RemoteTranscriptProjectionError::InvalidStatus)
    );

    let mut wrong_provider = remote_response(AgentTranscriptStatus::Missing);
    wrong_provider.provider = "codex".into();
    assert_eq!(
        remote_transcript_document(&route, wrong_provider),
        Err(RemoteTranscriptProjectionError::InvalidEnvelope)
    );

    let empty_loaded = remote_response(AgentTranscriptStatus::Loaded);
    assert_eq!(
        remote_transcript_document(&route, empty_loaded),
        Err(RemoteTranscriptProjectionError::InvalidPayload)
    );

    let mut oversized_timestamp = remote_response(AgentTranscriptStatus::Loaded);
    oversized_timestamp.turns = vec![AgentTranscriptTurn {
        timestamp: "x".repeat(65),
        ..remote_turn("assistant", "Answer")
    }];
    assert_eq!(
        remote_transcript_document(&route, oversized_timestamp),
        Err(RemoteTranscriptProjectionError::InvalidPayload)
    );
}

fn write_codex_rollout(home: &std::path::Path, id: &str, lines: &[serde_json::Value]) {
    let dir = home.join("sessions/2026/08/20");
    fs::create_dir_all(&dir).unwrap();
    let content = lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        dir.join(format!("rollout-2026-08-20T12-00-00-{id}.jsonl")),
        content,
    )
    .unwrap();
}

#[test]
fn local_codex_rollout_opens_as_a_provider_neutral_document() {
    let home = tempfile::tempdir().unwrap();
    let id = "019f135f-7fcc-7d93-8a28-4835d98f8f0a";
    write_codex_rollout(
        home.path(),
        id,
        &[
            serde_json::json!({"type":"session_meta","payload":{"id":id}}),
            serde_json::json!({"type":"turn_context","payload":{"model":"gpt-5.6"}}),
            serde_json::json!({
                "type":"response_item",
                "payload":{"type":"message","role":"user","content":[
                    {"type":"input_text","text":"Question"}
                ]}
            }),
            serde_json::json!({
                "type":"response_item",
                "payload":{
                    "type":"function_call",
                    "name":"read_file",
                    "arguments":"{\"token\":\"must-not-appear\"}"
                }
            }),
            serde_json::json!({
                "type":"response_item",
                "payload":{
                    "type":"function_call_output",
                    "output":"must-not-appear"
                }
            }),
            serde_json::json!({
                "type":"response_item",
                "payload":{"type":"message","role":"assistant","content":[
                    {"type":"output_text","text":"Answer"}
                ]}
            }),
        ],
    );

    let document = load_local_codex_transcript(home.path(), id);
    assert_eq!(document.state, TranscriptDocumentState::Ready);
    assert!(document.markdown.contains("## Codex · gpt-5.6"));
    assert!(document.markdown.contains("Question"));
    assert!(document.markdown.contains("Answer"));
    assert!(!document.markdown.contains("must-not-appear"));
}

#[test]
fn codex_transcript_states_distinguish_missing_unsupported_and_malformed() {
    let missing_home = tempfile::tempdir().unwrap();
    let missing = load_local_codex_transcript(missing_home.path(), "missing");
    assert_eq!(missing.state, TranscriptDocumentState::Missing);

    let unsupported_home = tempfile::tempdir().unwrap();
    let unsupported_id = "019f135f-7fcc-7d93-8a28-4835d98f8f0a";
    write_codex_rollout(
        unsupported_home.path(),
        unsupported_id,
        &[serde_json::json!({
            "type":"session_meta",
            "payload":{"id":unsupported_id}
        })],
    );
    let unsupported = load_local_codex_transcript(unsupported_home.path(), unsupported_id);
    assert_eq!(unsupported.state, TranscriptDocumentState::Unsupported);

    let malformed_home = tempfile::tempdir().unwrap();
    let malformed_id = "01a00093-815b-7b11-9c70-9f8275d0d9be";
    let dir = malformed_home.path().join("archived_sessions");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join(format!("rollout-2026-08-20T12-00-00-{malformed_id}.jsonl")),
        "not json\n{partial",
    )
    .unwrap();
    let malformed = load_local_codex_transcript(malformed_home.path(), malformed_id);
    assert_eq!(malformed.state, TranscriptDocumentState::Malformed);

    assert_ne!(missing.markdown, unsupported.markdown);
    assert_ne!(unsupported.markdown, malformed.markdown);
    assert_eq!(
        transcript_state_document(Provider::Codex, TranscriptDocumentState::Empty).state,
        TranscriptDocumentState::Empty
    );
}

#[test]
fn local_codex_errors_map_to_safe_provider_neutral_states() {
    use zaplex_cockpit::codex_sessions::TranscriptError;

    for error in [
        TranscriptError::InvalidSessionId,
        TranscriptError::AmbiguousSessionId {
            session_id: "session-1".into(),
        },
        TranscriptError::MalformedTranscript,
    ] {
        assert_eq!(
            local_codex_transcript_error_state(&error),
            TranscriptDocumentState::Malformed
        );
    }

    for error in [
        TranscriptError::HistoryLimitExceeded { max_files: 1 },
        TranscriptError::TranscriptLookupLimitExceeded { max_bytes: 1 },
        TranscriptError::TranscriptTooLarge { max_bytes: 1 },
    ] {
        assert_eq!(
            local_codex_transcript_error_state(&error),
            TranscriptDocumentState::TooLarge
        );
    }

    let io_error = TranscriptError::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "/private/provider/path must-not-appear",
    ));
    assert_eq!(
        local_codex_transcript_error_state(&io_error),
        TranscriptDocumentState::Unavailable
    );

    let missing_root = tempfile::tempdir().unwrap().path().join("does-not-exist");
    let walk_error = walkdir::WalkDir::new(missing_root)
        .into_iter()
        .next()
        .expect("walk result")
        .expect_err("missing root must fail");
    assert_eq!(
        local_codex_transcript_error_state(&TranscriptError::Walk(walk_error)),
        TranscriptDocumentState::Unavailable
    );
}
