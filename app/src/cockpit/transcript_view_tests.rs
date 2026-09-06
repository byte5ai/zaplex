use crate::remote_server::proto::{
    AgentTranscriptResponse, AgentTranscriptStatus, AgentTranscriptTool, AgentTranscriptTurn,
};
use std::collections::HashSet;
use std::fs;
use zaplex_cockpit::SessionState;

use super::*;

const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn response(status: AgentTranscriptStatus) -> AgentTranscriptResponse {
    let (source_revision, message) = match status {
        AgentTranscriptStatus::Loaded | AgentTranscriptStatus::NotModified => {
            ("a".repeat(64), String::new())
        }
        AgentTranscriptStatus::Empty => (
            "a".repeat(64),
            "transcript contains no visible conversation turns".into(),
        ),
        AgentTranscriptStatus::Unsupported => {
            ("a".repeat(64), "transcript format is unsupported".into())
        }
        AgentTranscriptStatus::Malformed => {
            ("a".repeat(64), "transcript history is malformed".into())
        }
        AgentTranscriptStatus::Missing => {
            (String::new(), "transcript history was not found".into())
        }
        AgentTranscriptStatus::TooLarge => (
            String::new(),
            "transcript history exceeds the daemon read limits".into(),
        ),
        AgentTranscriptStatus::Unavailable => {
            (String::new(), "transcript history is unavailable".into())
        }
        AgentTranscriptStatus::InvalidRequest | AgentTranscriptStatus::Unspecified => {
            (String::new(), String::new())
        }
    };
    AgentTranscriptResponse {
        schema_version: 1,
        provider: "claude".into(),
        session_id: "session-1".into(),
        status: status.into(),
        turns: Vec::new(),
        truncated: false,
        source_revision,
        message,
    }
}

fn turn(role: &str, text: &str) -> AgentTranscriptTurn {
    AgentTranscriptTurn {
        role: role.into(),
        text: text.into(),
        thinking: String::new(),
        tools: vec![AgentTranscriptTool {
            name: "read_file".into(),
        }],
        model: "claude-opus-4-1".into(),
        timestamp: "2026-08-20T12:00:00Z".into(),
    }
}

fn local_turn(text: String) -> TranscriptTurn {
    TranscriptTurn {
        role: zaplex_cockpit::TurnRole::Assistant,
        text,
        thinking: String::new(),
        tools: Vec::new(),
        model: Some("claude-opus-4-1".into()),
        usage: None,
        timestamp: None,
    }
}

fn loaded_local(turns: Vec<TranscriptTurn>) -> LoadedTranscript {
    LoadedTranscript {
        turns,
        source_revision: "b".repeat(64),
    }
}

const CODEX_SESSION_ID: &str = "01a00093-815b-7b11-9c70-9f8275d0d9be";

fn write_codex_transcript(root: &Path, content: &str) -> PathBuf {
    let directory = root.join("sessions/2026/08/20");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!(
        "rollout-2026-08-20T12-00-00-{CODEX_SESSION_ID}.jsonl"
    ));
    fs::write(&path, content).unwrap();
    path
}

#[test]
fn codex_transcript_states_distinguish_missing_unsupported_and_malformed() {
    let missing_root = tempfile::tempdir().unwrap();
    assert_eq!(
        load_local_transcript(Provider::Codex, missing_root.path(), CODEX_SESSION_ID).state,
        TranscriptDocumentState::Missing
    );

    let unsupported_root = tempfile::tempdir().unwrap();
    write_codex_transcript(
        unsupported_root.path(),
        &serde_json::json!({
            "type": "session_meta",
            "payload": { "id": CODEX_SESSION_ID }
        })
        .to_string(),
    );
    assert_eq!(
        load_local_transcript(Provider::Codex, unsupported_root.path(), CODEX_SESSION_ID,).state,
        TranscriptDocumentState::Unsupported
    );

    let malformed_root = tempfile::tempdir().unwrap();
    write_codex_transcript(malformed_root.path(), "not json\n{partial");
    assert_eq!(
        load_local_transcript(Provider::Codex, malformed_root.path(), CODEX_SESSION_ID,).state,
        TranscriptDocumentState::Malformed
    );
}

#[test]
fn local_codex_errors_map_to_safe_provider_neutral_states() {
    let invalid_root = tempfile::tempdir().unwrap();
    let invalid = load_local_transcript(Provider::Codex, invalid_root.path(), "../auth.json");
    assert_eq!(invalid.state, TranscriptDocumentState::Malformed);
    assert!(!invalid.markdown.contains("auth.json"));
    assert!(!invalid
        .markdown
        .contains(&invalid_root.path().to_string_lossy().to_string()));

    let oversized_root = tempfile::tempdir().unwrap();
    let oversized_path = write_codex_transcript(oversized_root.path(), "");
    fs::File::options()
        .write(true)
        .open(oversized_path)
        .unwrap()
        .set_len(zaplex_cockpit::codex_sessions::TRANSCRIPT_MAX_BYTES + 1)
        .unwrap();
    let oversized = load_local_transcript(Provider::Codex, oversized_root.path(), CODEX_SESSION_ID);
    assert_eq!(oversized.state, TranscriptDocumentState::TooLarge);
    assert!(!oversized
        .markdown
        .contains(&oversized_root.path().to_string_lossy().to_string()));
}

#[test]
fn claude_changed_during_read_is_retryable_for_live_refresh() {
    let result = project_claude_transcript(Err(
        zaplex_cockpit::sessions::TranscriptError::ChangedDuringRead,
    ));

    assert_eq!(result, Err(()));
}

#[test]
fn local_projection_keeps_only_the_last_bounded_turn_window() {
    let mut turns = vec![local_turn("oldest-private-turn".into())];
    turns.extend((0..MAX_TRANSCRIPT_TURNS).map(|index| local_turn(format!("recent-{index}"))));

    let document = ready_document(Provider::Claude, loaded_local(turns), false);

    assert_eq!(document.state, TranscriptDocumentState::Ready);
    assert!(document.markdown.starts_with("> "));
    assert!(!document.markdown.contains("oldest-private-turn"));
    assert!(document
        .markdown
        .contains(&format!("recent-{}", MAX_TRANSCRIPT_TURNS - 1)));
}

#[test]
fn local_empty_projection_preserves_its_source_revision() {
    let document = ready_document(Provider::Claude, loaded_local(Vec::new()), false);
    let revision = "b".repeat(64);

    assert_eq!(document.state, TranscriptDocumentState::Empty);
    assert_eq!(document.source_revision.as_deref(), Some(revision.as_str()));
}

#[test]
fn local_projection_bounds_and_redacts_every_display_field_utf8_safely() {
    let mut turn = local_turn(format!("safe\0{}tail", "é".repeat(MAX_TEXT_BYTES)));
    turn.thinking = format!("{}tail", "💡".repeat(MAX_THINKING_BYTES / 4));
    turn.model = Some(format!("`model\n{}tail`", "é".repeat(MAX_MODEL_BYTES)));
    turn.tools = (0..MAX_TOOLS_PER_TURN + 1)
        .map(|index| zaplex_cockpit::ToolCall {
            name: if index == 0 {
                format!("`tool\0{}tail`", "é".repeat(MAX_TOOL_NAME_BYTES))
            } else {
                format!("tool-{index}")
            },
        })
        .collect();

    let document = ready_document(Provider::Claude, loaded_local(vec![turn.clone()]), false);
    assert!(document.markdown.starts_with("> "));

    let (bounded, truncated) = bounded_transcript_turns(vec![turn]);

    assert!(truncated);
    assert_eq!(bounded.len(), 1);
    let turn = &bounded[0];
    assert!(turn.text.len() <= MAX_TEXT_BYTES);
    assert!(turn.thinking.len() <= MAX_THINKING_BYTES);
    assert!(turn.model.as_ref().unwrap().len() <= MAX_MODEL_BYTES);
    assert_eq!(turn.tools.len(), MAX_TOOLS_PER_TURN);
    assert!(turn.tools[0].name.len() <= MAX_TOOL_NAME_BYTES);
    assert!(!turn.text.contains('\0'));
    assert!(!turn
        .model
        .as_ref()
        .unwrap()
        .chars()
        .any(|character| matches!(character, '`' | '\n')));
    assert!(!turn.tools[0]
        .name
        .chars()
        .any(|character| matches!(character, '`' | '\0')));
    assert!(!turn.text.contains("tail"));
    assert!(!turn.thinking.contains("tail"));
}

#[test]
fn local_projection_never_builds_an_unbounded_markdown_buffer() {
    let turns = (0..80)
        .map(|_| local_turn("x".repeat(MAX_TEXT_BYTES)))
        .collect();

    let document = ready_document(Provider::Claude, loaded_local(turns), false);

    assert_eq!(document.state, TranscriptDocumentState::Ready);
    assert!(document.markdown.starts_with("> "));
    assert!(document.markdown.len() <= MAX_TRANSCRIPT_MARKDOWN_BYTES);
    assert!(document.markdown.is_char_boundary(document.markdown.len()));
}

#[test]
fn remote_projection_accepts_loaded_and_matching_not_modified_revisions() {
    let mut loaded = response(AgentTranscriptStatus::Loaded);
    loaded.turns = vec![turn("assistant", "done")];
    let projection = project_remote_transcript(Provider::Claude, "session-1", None, loaded)
        .expect("valid loaded response");
    let RemoteTranscriptProjection::Modified(document) = projection else {
        panic!("loaded response must modify the document");
    };
    assert_eq!(document.state, TranscriptDocumentState::Ready);
    assert_eq!(
        document.source_revision.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert!(document.markdown.contains("done"));

    let unchanged = project_remote_transcript(
        Provider::Claude,
        "session-1",
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
        response(AgentTranscriptStatus::NotModified),
    )
    .expect("matching revision");
    assert_eq!(unchanged, RemoteTranscriptProjection::NotModified);
}

#[test]
fn remote_loaded_transcript_rejects_nonempty_status_messages() {
    let mut loaded = response(AgentTranscriptStatus::Loaded);
    loaded.turns = vec![turn("assistant", "safe answer")];
    loaded.message = "credential-must-not-cross-projection".into();
    assert_eq!(
        project_remote_transcript(Provider::Claude, "session-1", None, loaded),
        Err(RemoteTranscriptProjectionError::InvalidPayload)
    );
}

#[test]
fn transcript_projection_renders_supported_timestamps_compactly() {
    let mut user = local_turn("Question".into());
    user.role = zaplex_cockpit::TurnRole::User;
    user.timestamp = Some("2026-08-20T12:34:56Z".parse().unwrap());
    let mut assistant = local_turn("Answer".into());
    assistant.timestamp = Some("2026-08-20T12:35:10Z".parse().unwrap());

    let document = ready_document(Provider::Claude, loaded_local(vec![user, assistant]), false);

    assert!(document.markdown.contains("## You · 2026-08-20 12:34 UTC"));
    assert!(document
        .markdown
        .contains("## Claude · claude-opus-4-1 · 2026-08-20 12:35 UTC"));
}

#[test]
fn remote_projection_rejects_retargeted_or_forged_not_modified_responses() {
    let mut wrong_session = response(AgentTranscriptStatus::Missing);
    wrong_session.session_id = "other".into();
    assert_eq!(
        project_remote_transcript(Provider::Claude, "session-1", None, wrong_session),
        Err(RemoteTranscriptProjectionError::InvalidEnvelope)
    );

    assert_eq!(
        project_remote_transcript(
            Provider::Claude,
            "session-1",
            Some("b"),
            response(AgentTranscriptStatus::NotModified),
        ),
        Err(RemoteTranscriptProjectionError::InvalidPayload)
    );
}

#[test]
fn remote_projection_keeps_explicit_source_states_distinct() {
    for (status, state) in [
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
        let projection =
            project_remote_transcript(Provider::Claude, "session-1", None, response(status))
                .unwrap();
        let RemoteTranscriptProjection::Modified(document) = projection else {
            panic!("an explicit state must modify the document");
        };
        assert_eq!(document.state, state);
        if matches!(
            status,
            AgentTranscriptStatus::Empty
                | AgentTranscriptStatus::Unsupported
                | AgentTranscriptStatus::Malformed
        ) {
            assert_eq!(document.source_revision.as_deref(), Some(REVISION));
        } else {
            assert_eq!(document.source_revision, None);
        }
    }
}

#[test]
fn remote_transcript_statuses_map_to_distinct_neutral_documents() {
    let expected = [
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
    ];
    let states = expected
        .into_iter()
        .map(|(status, expected_state)| {
            let RemoteTranscriptProjection::Modified(document) =
                project_remote_transcript(Provider::Claude, "session-1", None, response(status))
                    .unwrap()
            else {
                panic!("explicit source status must produce a document");
            };
            assert_eq!(document.state, expected_state);
            document.state
        })
        .collect::<Vec<_>>();
    assert_eq!(states, expected.map(|(_, state)| state));
}

#[test]
fn remote_transcript_fails_closed_for_invalid_status_envelope_or_payload() {
    let mut invalid_envelope = response(AgentTranscriptStatus::Missing);
    invalid_envelope.schema_version = 2;
    assert_eq!(
        project_remote_transcript(Provider::Claude, "session-1", None, invalid_envelope),
        Err(RemoteTranscriptProjectionError::InvalidEnvelope)
    );

    let mut invalid_status = response(AgentTranscriptStatus::Missing);
    invalid_status.status = i32::MAX;
    assert_eq!(
        project_remote_transcript(Provider::Claude, "session-1", None, invalid_status),
        Err(RemoteTranscriptProjectionError::InvalidStatus)
    );

    let mut invalid_payload = response(AgentTranscriptStatus::Loaded);
    invalid_payload.turns = vec![turn("assistant", &"x".repeat(64 * 1024 + 1))];
    assert_eq!(
        project_remote_transcript(Provider::Claude, "session-1", None, invalid_payload),
        Err(RemoteTranscriptProjectionError::InvalidPayload)
    );

    let mut oversized_aggregate = response(AgentTranscriptStatus::Loaded);
    oversized_aggregate.turns = (0..65)
        .map(|_| turn("assistant", &"x".repeat(MAX_TEXT_BYTES)))
        .collect();
    assert_eq!(
        project_remote_transcript(Provider::Claude, "session-1", None, oversized_aggregate),
        Err(RemoteTranscriptProjectionError::InvalidEnvelope)
    );

    let mut state_with_turns = response(AgentTranscriptStatus::Missing);
    state_with_turns.turns = vec![turn("assistant", "must not be accepted")];
    assert_eq!(
        project_remote_transcript(Provider::Claude, "session-1", None, state_with_turns),
        Err(RemoteTranscriptProjectionError::InvalidPayload)
    );

    let mut empty_without_revision = response(AgentTranscriptStatus::Empty);
    empty_without_revision.source_revision.clear();
    assert_eq!(
        project_remote_transcript(Provider::Claude, "session-1", None, empty_without_revision),
        Err(RemoteTranscriptProjectionError::InvalidPayload)
    );

    let mut missing_with_revision = response(AgentTranscriptStatus::Missing);
    missing_with_revision.source_revision = REVISION.into();
    assert_eq!(
        project_remote_transcript(Provider::Claude, "session-1", None, missing_with_revision),
        Err(RemoteTranscriptProjectionError::InvalidPayload)
    );

    let mut loaded_matching_known = response(AgentTranscriptStatus::Loaded);
    loaded_matching_known.turns = vec![turn("assistant", "duplicate")];
    assert_eq!(
        project_remote_transcript(
            Provider::Claude,
            "session-1",
            Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            loaded_matching_known,
        ),
        Err(RemoteTranscriptProjectionError::InvalidPayload)
    );

    let mut unknown_role = response(AgentTranscriptStatus::Loaded);
    unknown_role.turns = vec![turn("system", "must not be silently skipped")];
    assert_eq!(
        project_remote_transcript(Provider::Claude, "session-1", None, unknown_role),
        Err(RemoteTranscriptProjectionError::InvalidPayload)
    );
}

#[test]
fn watch_state_allows_one_refresh_and_rejects_late_generations() {
    let mut state = TranscriptWatchState::with_revision(Some("first".into()));
    let first = state.begin_refresh().unwrap();
    assert_eq!(state.begin_refresh(), None);
    assert!(!state.finish_refresh(first + 1, Some("stale".into())));
    assert_eq!(state.revision(), Some("first"));
    assert!(state.finish_refresh(first, Some("second".into())));
    assert_eq!(state.revision(), Some("second"));
    let unchanged = state.begin_refresh().unwrap();
    assert!(state.finish_not_modified(unchanged));
    assert_eq!(state.revision(), Some("second"));
    let explicit_source_state = state.begin_refresh().unwrap();
    assert!(state.finish_refresh(explicit_source_state, None));
    assert_eq!(state.revision(), None);
}

#[test]
fn watch_lifetime_stops_for_closed_missing_or_dormant_documents() {
    assert!(should_follow_transcript(true, Some(SessionState::Active)));
    assert!(should_follow_transcript(true, Some(SessionState::Waiting)));
    assert!(!should_follow_transcript(false, Some(SessionState::Active)));
    assert!(!should_follow_transcript(true, Some(SessionState::Idle)));
    assert!(!should_follow_transcript(true, None));
}

#[test]
fn stable_targets_do_not_collapse_duplicate_display_or_session_labels() {
    let targets = HashSet::from([
        TranscriptTarget::Remote {
            provider: Provider::Claude,
            host_id: "host-a".into(),
            account_id: "account-a".into(),
            session_id: "same-session".into(),
        },
        TranscriptTarget::Remote {
            provider: Provider::Claude,
            host_id: "host-b".into(),
            account_id: "account-a".into(),
            session_id: "same-session".into(),
        },
        TranscriptTarget::Remote {
            provider: Provider::Claude,
            host_id: "host-a".into(),
            account_id: "account-b".into(),
            session_id: "same-session".into(),
        },
    ]);
    assert_eq!(targets.len(), 3);
}
