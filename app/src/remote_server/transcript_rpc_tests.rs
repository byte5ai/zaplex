use std::fs;
use std::path::Path;

use prost::Message as _;
use serde_json::json;

use super::*;

const CLAUDE_ID: &str = "019f135f-7fcc-7d93-8a28-4835d98f8f0a";
const CODEX_ID: &str = "01a00093-815b-7b11-9c70-9f8275d0d9be";

fn resolved(
    provider: TranscriptProvider,
    root: &Path,
    session_id: &str,
) -> ResolvedTranscriptRequest {
    ResolvedTranscriptRequest {
        provider,
        session_id: session_id.to_string(),
        config_root: fs::canonicalize(root).unwrap(),
        known_revision: None,
    }
}

fn status(response: &AgentTranscriptResponse) -> AgentTranscriptStatus {
    AgentTranscriptStatus::try_from(response.status).unwrap()
}

fn write_claude(root: &Path, session_id: &str, lines: &[serde_json::Value]) -> PathBuf {
    let project = root.join("projects/project");
    fs::create_dir_all(&project).unwrap();
    let content = lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let path = project.join(format!("{session_id}.jsonl"));
    fs::write(&path, content).unwrap();
    path
}

fn write_codex(root: &Path, session_id: &str, lines: &[serde_json::Value]) -> PathBuf {
    let sessions = root.join("sessions/2026/08/20");
    fs::create_dir_all(&sessions).unwrap();
    let content = lines
        .iter()
        .map(|line| line.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let path = sessions.join(format!("rollout-2026-08-20T12-00-00-{session_id}.jsonl"));
    fs::write(&path, content).unwrap();
    path
}

#[test]
fn fresh_same_inode_account_swap_rejects_a_cached_transcript_route() {
    let account = tempfile::tempdir().unwrap();
    let account_root = fs::canonicalize(account.path()).unwrap();
    let request = ReadAgentTranscript {
        schema_version: TRANSCRIPT_SCHEMA_VERSION,
        provider: "claude".to_string(),
        account_id: "stale-account-id".to_string(),
        session_id: CLAUDE_ID.to_string(),
        known_revision: String::new(),
    };
    let mut stale_cache = super::super::agent_account::AccountRouteCache::default();
    stale_cache.replace_for_test("claude", "stale-account-id", Some(account_root.clone()));
    assert!(resolve_request(stale_cache.routes_for_test(), request.clone()).is_ok());

    let mut fresh_scan = super::super::agent_account::AccountRouteCache::default();
    fresh_scan.replace_for_test("claude", "current-account-id", Some(account_root));
    let response = match resolve_request(fresh_scan.routes_for_test(), request) {
        Ok(_) => panic!("stale opaque account id must not resolve against a fresh scan"),
        Err(response) => response,
    };

    assert_eq!(status(&response), AgentTranscriptStatus::InvalidRequest);
    assert!(response.message.contains("unknown"));
}

#[test]
fn busy_response_validates_identity_before_echoing_it() {
    let valid = ReadAgentTranscript {
        schema_version: TRANSCRIPT_SCHEMA_VERSION,
        provider: "codex".to_string(),
        account_id: "opaque-account".to_string(),
        session_id: CODEX_ID.to_string(),
        known_revision: String::new(),
    };
    let busy = busy_response(&valid);
    assert_eq!(status(&busy), AgentTranscriptStatus::Unavailable);
    assert_eq!(busy.provider, "codex");
    assert_eq!(busy.session_id, CODEX_ID);

    let invalid = busy_response(&ReadAgentTranscript {
        session_id: "unbounded identity!".repeat(128),
        ..valid
    });
    assert_eq!(status(&invalid), AgentTranscriptStatus::InvalidRequest);
    assert!(invalid.provider.is_empty());
    assert!(invalid.session_id.is_empty());
}

#[test]
fn claude_snapshot_contains_only_the_display_projection() {
    let root = tempfile::tempdir().unwrap();
    write_claude(
        root.path(),
        CLAUDE_ID,
        &[
            json!({"type":"user","message":{"content":"Question"}}),
            json!({
                "type":"assistant",
                "message":{
                    "model":"claude-opus-4-8",
                    "content":[
                        {"type":"tool_use","name":"read_file","input":{
                            "token":"credential-must-not-cross-wire"
                        }},
                        {"type":"text","text":"Answer"}
                    ]
                }
            }),
            json!({
                "type":"user",
                "message":{"content":[{"type":"tool_result","content":
                    "credential-must-not-cross-wire"}]}
            }),
        ],
    );

    let response = read_transcript(resolved(TranscriptProvider::Claude, root.path(), CLAUDE_ID));
    assert_eq!(status(&response), AgentTranscriptStatus::Loaded);
    assert_eq!(response.turns.len(), 2);
    assert_eq!(response.turns[0].text, "Question");
    assert_eq!(response.turns[1].text, "Answer");
    assert_eq!(response.turns[1].tools[0].name, "read_file");
    assert_eq!(response.source_revision.len(), 64);
    assert!(!format!("{response:?}").contains("credential-must-not-cross-wire"));
    assert!(response.encoded_len() <= MAX_TRANSCRIPT_RESPONSE_BYTES);
}

#[test]
fn daemon_snapshot_revision_is_accepted_by_the_shared_remote_projection() {
    let root = tempfile::tempdir().unwrap();
    write_claude(
        root.path(),
        CLAUDE_ID,
        &[json!({"type":"user","message":{"content":"Question"}})],
    );
    let response = read_transcript(resolved(TranscriptProvider::Claude, root.path(), CLAUDE_ID));

    let projection = crate::cockpit::transcript_view::project_remote_transcript(
        zaplex_cockpit::Provider::Claude,
        CLAUDE_ID,
        None,
        response,
    )
    .expect("daemon response must satisfy the client projection contract");
    assert!(matches!(
        projection,
        crate::cockpit::transcript_view::RemoteTranscriptProjection::Modified(document)
            if document.state
                == crate::cockpit::transcript_view::TranscriptDocumentState::Ready
    ));
}

#[test]
fn codex_snapshot_contains_only_the_display_projection() {
    let root = tempfile::tempdir().unwrap();
    write_codex(
        root.path(),
        CODEX_ID,
        &[
            json!({"type":"session_meta","payload":{"id":CODEX_ID}}),
            json!({"type":"turn_context","payload":{"model":"gpt-5.6"}}),
            json!({
                "type":"response_item",
                "payload":{"type":"message","role":"user","content":[
                    {"type":"input_text","text":"Question"}
                ]}
            }),
            json!({
                "type":"response_item",
                "payload":{"type":"function_call","name":"read_file",
                    "arguments":"{\"token\":\"credential-must-not-cross-wire\"}"}
            }),
            json!({
                "type":"response_item",
                "payload":{"type":"function_call_output",
                    "output":"credential-must-not-cross-wire"}
            }),
            json!({
                "type":"response_item",
                "payload":{"type":"message","role":"assistant","content":[
                    {"type":"output_text","text":"Answer"}
                ]}
            }),
        ],
    );

    let response = read_transcript(resolved(TranscriptProvider::Codex, root.path(), CODEX_ID));
    assert_eq!(status(&response), AgentTranscriptStatus::Loaded);
    assert_eq!(response.turns.len(), 2);
    assert_eq!(response.turns[1].tools[0].name, "read_file");
    assert!(!format!("{response:?}").contains("credential-must-not-cross-wire"));
}

#[test]
fn missing_empty_unsupported_and_malformed_are_distinct() {
    let missing_root = tempfile::tempdir().unwrap();
    fs::create_dir_all(missing_root.path().join("projects")).unwrap();
    let missing = read_transcript(resolved(
        TranscriptProvider::Claude,
        missing_root.path(),
        CLAUDE_ID,
    ));
    assert_eq!(status(&missing), AgentTranscriptStatus::Missing);

    let empty_root = tempfile::tempdir().unwrap();
    write_claude(
        empty_root.path(),
        CLAUDE_ID,
        &[json!({"type":"user","isMeta":true,"message":{"content":"internal"}})],
    );
    let empty = read_transcript(resolved(
        TranscriptProvider::Claude,
        empty_root.path(),
        CLAUDE_ID,
    ));
    assert_eq!(status(&empty), AgentTranscriptStatus::Empty);

    let unsupported_root = tempfile::tempdir().unwrap();
    write_claude(
        unsupported_root.path(),
        CLAUDE_ID,
        &[json!({"type":"future_transcript_record","payload":{}})],
    );
    let unsupported = read_transcript(resolved(
        TranscriptProvider::Claude,
        unsupported_root.path(),
        CLAUDE_ID,
    ));
    assert_eq!(status(&unsupported), AgentTranscriptStatus::Unsupported);

    let malformed_root = tempfile::tempdir().unwrap();
    let malformed_path = malformed_root
        .path()
        .join("projects/project")
        .join(format!("{CLAUDE_ID}.jsonl"));
    fs::create_dir_all(malformed_path.parent().unwrap()).unwrap();
    fs::write(malformed_path, "not json\n{partial").unwrap();
    let malformed = read_transcript(resolved(
        TranscriptProvider::Claude,
        malformed_root.path(),
        CLAUDE_ID,
    ));
    assert_eq!(status(&malformed), AgentTranscriptStatus::Malformed);
}

#[test]
fn codex_unsupported_and_malformed_states_survive_the_rpc_projection() {
    let unsupported_root = tempfile::tempdir().unwrap();
    write_codex(
        unsupported_root.path(),
        CODEX_ID,
        &[json!({"type":"session_meta","payload":{"id":CODEX_ID}})],
    );
    let unsupported = read_transcript(resolved(
        TranscriptProvider::Codex,
        unsupported_root.path(),
        CODEX_ID,
    ));
    assert_eq!(status(&unsupported), AgentTranscriptStatus::Unsupported);

    let malformed_root = tempfile::tempdir().unwrap();
    let malformed = write_codex(malformed_root.path(), CODEX_ID, &[]);
    fs::write(malformed, "not json\n{partial").unwrap();
    let malformed = read_transcript(resolved(
        TranscriptProvider::Codex,
        malformed_root.path(),
        CODEX_ID,
    ));
    assert_eq!(status(&malformed), AgentTranscriptStatus::Malformed);
}

#[test]
fn unchanged_revision_avoids_retransmitting_turns() {
    let root = tempfile::tempdir().unwrap();
    write_claude(
        root.path(),
        CLAUDE_ID,
        &[json!({"type":"user","message":{"content":"Question"}})],
    );
    let first = read_transcript(resolved(TranscriptProvider::Claude, root.path(), CLAUDE_ID));
    assert_eq!(status(&first), AgentTranscriptStatus::Loaded);
    assert!(!first.source_revision.is_empty());

    let mut next = resolved(TranscriptProvider::Claude, root.path(), CLAUDE_ID);
    next.known_revision = Some(first.source_revision);
    let unchanged = read_transcript(next);
    assert_eq!(status(&unchanged), AgentTranscriptStatus::NotModified);
    assert!(unchanged.turns.is_empty());
}

#[test]
fn revision_tracks_content_when_a_source_is_replaced_at_the_same_size() {
    let root = tempfile::tempdir().unwrap();
    let path = write_claude(
        root.path(),
        CLAUDE_ID,
        &[json!({"type":"user","message":{"content":"Question"}})],
    );
    let first = read_transcript(resolved(TranscriptProvider::Claude, root.path(), CLAUDE_ID));
    assert_eq!(status(&first), AgentTranscriptStatus::Loaded);

    let replacement = json!({"type":"user","message":{"content":"Changed!"}}).to_string();
    assert_eq!(
        replacement.len(),
        fs::metadata(&path).unwrap().len() as usize
    );
    fs::write(path, replacement).unwrap();

    let mut next = resolved(TranscriptProvider::Claude, root.path(), CLAUDE_ID);
    next.known_revision = Some(first.source_revision.clone());
    let changed = read_transcript(next);
    assert_eq!(status(&changed), AgentTranscriptStatus::Loaded);
    assert_ne!(changed.source_revision, first.source_revision);
    assert_eq!(changed.turns[0].text, "Changed!");
}

#[cfg(unix)]
#[test]
fn opened_source_content_is_stable_when_the_path_is_replaced() {
    let root = tempfile::tempdir().unwrap();
    let original = json!({"type":"user","message":{"content":"original"}}).to_string();
    let path = write_claude(
        root.path(),
        CLAUDE_ID,
        &[json!({"type":"user","message":{"content":"original"}})],
    );
    let config_root = fs::canonicalize(root.path()).unwrap();
    let (file, metadata) = open_source(&config_root, path.clone()).unwrap();

    fs::rename(&path, path.with_extension("old")).unwrap();
    fs::write(
        &path,
        json!({"type":"user","message":{"content":"replacement"}}).to_string(),
    )
    .unwrap();

    let checked = read_source(file, &metadata).unwrap();
    assert_eq!(checked.content, original);
    assert_eq!(checked.revision, source_revision(original.as_bytes()));
}

#[test]
fn source_and_response_limits_fail_closed_or_truncate() {
    let oversized_root = tempfile::tempdir().unwrap();
    let oversized = write_claude(oversized_root.path(), CLAUDE_ID, &[]);
    fs::File::options()
        .write(true)
        .open(oversized)
        .unwrap()
        .set_len(MAX_TRANSCRIPT_BYTES + 1)
        .unwrap();
    let oversized = read_transcript(resolved(
        TranscriptProvider::Claude,
        oversized_root.path(),
        CLAUDE_ID,
    ));
    assert_eq!(status(&oversized), AgentTranscriptStatus::TooLarge);

    let too_many_lines_root = tempfile::tempdir().unwrap();
    let too_many_lines = write_claude(too_many_lines_root.path(), CLAUDE_ID, &[]);
    fs::write(too_many_lines, "{}\n".repeat(MAX_TRANSCRIPT_LINES + 1)).unwrap();
    let too_many_lines = read_transcript(resolved(
        TranscriptProvider::Claude,
        too_many_lines_root.path(),
        CLAUDE_ID,
    ));
    assert_eq!(status(&too_many_lines), AgentTranscriptStatus::TooLarge);

    let many_root = tempfile::tempdir().unwrap();
    let turns: Vec<serde_json::Value> = (0..(MAX_TRANSCRIPT_TURNS + 8))
        .map(|index| json!({"type":"user","message":{"content":format!("turn-{index}")}}))
        .collect();
    write_claude(many_root.path(), CLAUDE_ID, &turns);
    let many = read_transcript(resolved(
        TranscriptProvider::Claude,
        many_root.path(),
        CLAUDE_ID,
    ));
    assert_eq!(status(&many), AgentTranscriptStatus::Loaded);
    assert_eq!(many.turns.len(), MAX_TRANSCRIPT_TURNS);
    assert!(many.truncated);
    assert_eq!(many.turns[0].text, "turn-8");
    assert!(many.encoded_len() <= MAX_TRANSCRIPT_RESPONSE_BYTES);
}

#[cfg(unix)]
#[test]
fn transcript_resolution_never_follows_a_symlink() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_file = outside.path().join(format!("{CLAUDE_ID}.jsonl"));
    fs::write(
        &outside_file,
        json!({"type":"user","message":{"content":"outside"}}).to_string(),
    )
    .unwrap();
    let project = root.path().join("projects/project");
    fs::create_dir_all(&project).unwrap();
    symlink(&outside_file, project.join(format!("{CLAUDE_ID}.jsonl"))).unwrap();

    let response = read_transcript(resolved(TranscriptProvider::Claude, root.path(), CLAUDE_ID));
    assert_eq!(status(&response), AgentTranscriptStatus::Missing);
    assert!(!format!("{response:?}").contains("outside"));
}
