use std::fs;

use chrono::{Duration, Utc};
use serde_json::{json, Value};

use super::*;
use crate::types::{Provider, SessionState};

/// Write a rollout file under `<home>/sessions/2026/07/07/` with the given
/// wrapped (`{type,timestamp,payload}`) lines.
fn write_rollout(home: &Path, file: &str, lines: &[Value]) {
    let dir = home.join("sessions").join("2026").join("07").join("07");
    fs::create_dir_all(&dir).unwrap();
    let body: String = lines
        .iter()
        .map(|l| serde_json::to_string(l).unwrap() + "\n")
        .collect();
    fs::write(dir.join(file), body).unwrap();
}

fn ts_now() -> String {
    Utc::now().to_rfc3339()
}

fn session_meta(cwd: &str, id: &str) -> Value {
    json!({"type":"session_meta","timestamp":ts_now(),
        "payload":{"id":id,"session_id":id,"cwd":cwd}})
}

fn turn_context(model: &str, cwd: &str, effort: Option<&str>) -> Value {
    json!({"type":"turn_context","timestamp":ts_now(),
        "payload":{"model":model,"cwd":cwd,"effort":effort,"summary":"auto"}})
}

fn token_count(input: u64) -> Value {
    json!({"type":"event_msg","timestamp":ts_now(),
        "payload":{"type":"token_count","info":{
            "last_token_usage":{"input_tokens":input,"cached_input_tokens":0,
                "output_tokens":10,"reasoning_output_tokens":5,"total_tokens":input+15},
            "total_token_usage":{"input_tokens":999999,"total_tokens":999999},
            "model_context_window":258400}}})
}

fn event(kind: &str) -> Value {
    json!({"type":"event_msg","timestamp":ts_now(),"payload":{"type":kind}})
}

#[test]
fn completed_turn_is_waiting_with_model_effort_ctx() {
    let tmp = tempfile::tempdir().unwrap();
    write_rollout(
        tmp.path(),
        "rollout-2026-07-07T12-00-00-abc.jsonl",
        &[
            session_meta("/tmp/proj", "sess-1"),
            turn_context("gpt-5.5", "/tmp/proj", Some("high")),
            token_count(136_000),
            event("task_started"),
            event("agent_message"),
            event("task_complete"),
        ],
    );
    let sessions = live_sessions(tmp.path(), Utc::now());
    assert_eq!(sessions.len(), 1);
    let s = &sessions[0];
    assert_eq!(s.provider, Provider::Codex);
    assert_eq!(s.session_id, "sess-1");
    assert_eq!(s.state, SessionState::Waiting);
    assert_eq!(s.model, "gpt-5.5");
    assert_eq!(s.effort.as_deref(), Some("high"));
    assert_eq!(s.ctx_tokens, 136_000);
    // 136k of the Codex 272k window → 50% fill.
    assert!((crate::context_fill(&s.model, s.ctx_tokens) - 0.5).abs() < 1e-6);
    assert_eq!(s.project_root, "/tmp/proj");
    assert_eq!(s.project_name, "proj");
    // Codex records no pid.
    assert_eq!(s.pid, 0);
}

#[test]
fn started_but_not_complete_turn_is_monitor() {
    let tmp = tempfile::tempdir().unwrap();
    write_rollout(
        tmp.path(),
        "rollout-2026-07-07T12-00-00-def.jsonl",
        &[
            session_meta("/tmp/proj", "sess-2"),
            turn_context("gpt-5.5", "/tmp/proj", None),
            token_count(1000),
            event("task_started"),
        ],
    );
    let sessions = live_sessions(tmp.path(), Utc::now());
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].state, SessionState::Monitor);
    // Absent effort stays honestly unknown.
    assert_eq!(sessions[0].effort, None);
}

#[test]
fn stale_rollout_is_not_listed() {
    let tmp = tempfile::tempdir().unwrap();
    // Same fresh mtime (just written), but the transcript's own last activity
    // is an hour old → outside the live window → skipped.
    let old = (Utc::now() - Duration::hours(1)).to_rfc3339();
    write_rollout(
        tmp.path(),
        "rollout-2026-07-07T11-00-00-old.jsonl",
        &[
            json!({"type":"session_meta","timestamp":old,
                "payload":{"id":"sess-old","cwd":"/tmp/proj"}}),
            json!({"type":"turn_context","timestamp":old,
                "payload":{"model":"gpt-5.5","cwd":"/tmp/proj"}}),
            json!({"type":"event_msg","timestamp":old,
                "payload":{"type":"task_complete"}}),
        ],
    );
    assert!(live_sessions(tmp.path(), Utc::now()).is_empty());
}

#[test]
fn rollout_without_a_turn_is_not_a_session() {
    let tmp = tempfile::tempdir().unwrap();
    write_rollout(
        tmp.path(),
        "rollout-2026-07-07T12-00-00-emp.jsonl",
        &[session_meta("/tmp/proj", "sess-empty")],
    );
    assert!(live_sessions(tmp.path(), Utc::now()).is_empty());
}

#[test]
fn waiting_sorts_before_working() {
    let tmp = tempfile::tempdir().unwrap();
    write_rollout(
        tmp.path(),
        "rollout-2026-07-07T12-00-00-w.jsonl",
        &[
            session_meta("/tmp/a", "wait"),
            turn_context("gpt-5.5", "/tmp/a", None),
            token_count(100),
            event("task_complete"),
        ],
    );
    write_rollout(
        tmp.path(),
        "rollout-2026-07-07T12-00-01-m.jsonl",
        &[
            session_meta("/tmp/b", "work"),
            turn_context("gpt-5.5", "/tmp/b", None),
            token_count(100),
            event("task_started"),
        ],
    );
    let sessions = live_sessions(tmp.path(), Utc::now());
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].state, SessionState::Waiting);
    assert_eq!(sessions[1].state, SessionState::Monitor);
}
