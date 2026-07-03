use chrono::Utc;
use serde_json::json;

use super::*;
use crate::types::SessionState;

/// Builds a fake account dir with one registry entry + transcript.
fn fake_account(
    dir: &Path,
    session_id: &str,
    status: &str,
    kind: &str,
    transcript_lines: &[serde_json::Value],
) {
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    let projects = dir.join("projects").join("-tmp-proj");
    std::fs::create_dir_all(&projects).unwrap();

    let reg = json!({
        "sessionId": session_id,
        "cwd": "/tmp/proj",
        "status": status,
        "kind": kind,
        "name": "test-session",
        "startedAt": Utc::now().timestamp_millis(),
        "updatedAt": Utc::now().timestamp_millis(),
        // Own pid: guaranteed alive.
        "pid": std::process::id(),
    });
    std::fs::write(
        sessions.join(format!("{session_id}.json")),
        serde_json::to_string(&reg).unwrap(),
    )
    .unwrap();

    let transcript: String = transcript_lines
        .iter()
        .map(|l| serde_json::to_string(l).unwrap() + "\n")
        .collect();
    std::fs::write(projects.join(format!("{session_id}.jsonl")), transcript).unwrap();
}

fn assistant_line(stop_reason: &str) -> serde_json::Value {
    json!({
        "type": "assistant",
        "timestamp": "2026-07-03T00:00:00Z",
        "message": {
            "stop_reason": stop_reason,
            "model": "claude-opus-4-8",
            "usage": {"input_tokens": 100, "cache_read_input_tokens": 900},
            "content": [{"type": "text", "text": "done"}]
        }
    })
}

#[test]
fn busy_session_is_active() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "s1",
        "busy",
        "",
        &[assistant_line("end_turn")],
    );
    let sessions = live_sessions(tmp.path(), Utc::now());
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].state, SessionState::Active);
    assert_eq!(sessions[0].model, "claude-opus-4-8");
    assert_eq!(sessions[0].ctx_tokens, 1000);
}

#[test]
fn ended_turn_is_waiting() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(tmp.path(), "s1", "idle", "", &[assistant_line("end_turn")]);
    let sessions = live_sessions(tmp.path(), Utc::now());
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].state, SessionState::Waiting);
}

#[test]
fn tool_use_turn_is_monitor() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(tmp.path(), "s1", "idle", "", &[assistant_line("tool_use")]);
    let sessions = live_sessions(tmp.path(), Utc::now());
    assert_eq!(sessions[0].state, SessionState::Monitor);
}

#[test]
fn tool_result_after_end_is_monitor() {
    // assistant end followed by a tool result → Claude continues → Monitor.
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "s1",
        "idle",
        "",
        &[
            assistant_line("end_turn"),
            json!({"type": "user", "message": {"content": [{"type": "tool_result"}]}}),
        ],
    );
    let sessions = live_sessions(tmp.path(), Utc::now());
    assert_eq!(sessions[0].state, SessionState::Monitor);
}

#[test]
fn shell_and_infra_entries_are_filtered() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(tmp.path(), "s1", "shell", "", &[assistant_line("end_turn")]);
    assert!(live_sessions(tmp.path(), Utc::now()).is_empty());
}

#[test]
fn registry_without_transcript_is_filtered() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(tmp.path(), "s1", "idle", "", &[assistant_line("end_turn")]);
    // Remove the transcript → helper process, not a real session.
    std::fs::remove_file(
        tmp.path()
            .join("projects")
            .join("-tmp-proj")
            .join("s1.jsonl"),
    )
    .unwrap();
    assert!(live_sessions(tmp.path(), Utc::now()).is_empty());
}

#[test]
fn waiting_sorts_before_active() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(tmp.path(), "s1", "busy", "", &[assistant_line("tool_use")]);
    fake_account(tmp.path(), "s2", "idle", "", &[assistant_line("end_turn")]);
    let sessions = live_sessions(tmp.path(), Utc::now());
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].state, SessionState::Waiting);
    assert_eq!(sessions[1].state, SessionState::Active);
}
