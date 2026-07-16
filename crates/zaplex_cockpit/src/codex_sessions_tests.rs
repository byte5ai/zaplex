use std::fs;
use std::time::SystemTime;

use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Value};

use super::*;
use crate::types::{Provider, SessionState};

/// Write a rollout file under `<home>/sessions/2026/07/07/` with the given
/// wrapped (`{type,timestamp,payload}`) lines.
fn write_rollout(home: &Path, file: &str, lines: &[Value]) -> PathBuf {
    let dir = home.join("sessions").join("2026").join("07").join("07");
    fs::create_dir_all(&dir).unwrap();
    let body: String = lines
        .iter()
        .map(|l| serde_json::to_string(l).unwrap() + "\n")
        .collect();
    let path = dir.join(file);
    fs::write(&path, body).unwrap();
    path
}

/// Re-stamp every line's own clock. Rollout lines carry their timestamp inline,
/// so a realistic dormant rollout has to be old *inside* as well as on disk — a
/// file cannot gain fresh content without its mtime moving too.
fn stamped(lines: &[Value], ts: DateTime<Utc>) -> Vec<Value> {
    lines
        .iter()
        .map(|l| {
            let mut l = l.clone();
            l["timestamp"] = json!(ts.to_rfc3339());
            l
        })
        .collect()
}

/// A rollout genuinely last touched at `ts`, inside and out.
fn write_rollout_at(home: &Path, file: &str, lines: &[Value], ts: DateTime<Utc>) {
    let path = write_rollout(home, file, &stamped(lines, ts));
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::from(ts))
        .unwrap();
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

// ── Dormant (idle) discovery ────────────────────────────────────────────────

const MAX_AGE: Duration = Duration::days(7);

fn dormant_lines(id: &str) -> Vec<Value> {
    vec![
        session_meta("/tmp/proj", id),
        turn_context("gpt-5.5", "/tmp/proj", Some("high")),
        token_count(1000),
        event("task_complete"),
    ]
}

#[test]
fn a_dormant_rollout_is_discovered_as_idle_and_resumable() {
    let tmp = tempfile::tempdir().unwrap();
    let now = Utc::now();
    write_rollout_at(
        tmp.path(),
        "rollout-2026-07-07T09-00-00-gone.jsonl",
        &dormant_lines("sess-gone"),
        now - Duration::hours(2),
    );

    let idle = idle_sessions(tmp.path(), now, MAX_AGE, 50);
    assert_eq!(idle.len(), 1);
    assert_eq!(idle[0].state, SessionState::Idle);
    // What `codex resume <id>` needs.
    assert_eq!(idle[0].session_id, "sess-gone");
    assert_eq!(idle[0].provider, Provider::Codex);
    // The rollout's own facts survive the dormant classification.
    assert_eq!(idle[0].model, "gpt-5.5");
    assert_eq!(idle[0].effort.as_deref(), Some("high"));
}

/// A rollout that ended with `task_complete` would classify as Waiting while
/// live. Once dormant, the process is gone and nothing is waiting on the user —
/// the state must reflect that, not the last turn.
#[test]
fn a_dormant_rollout_is_idle_regardless_of_how_its_last_turn_ended() {
    let tmp = tempfile::tempdir().unwrap();
    let now = Utc::now();
    write_rollout_at(
        tmp.path(),
        "rollout-2026-07-07T09-00-00-w.jsonl",
        &dormant_lines("sess-w"),
        now - Duration::hours(2),
    );
    // Same content, fresh → the live path calls it Waiting.
    write_rollout_at(
        tmp.path(),
        "rollout-2026-07-07T12-00-00-live.jsonl",
        &dormant_lines("sess-live"),
        now,
    );

    let live = live_sessions(tmp.path(), now);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].session_id, "sess-live");
    assert_eq!(live[0].state, SessionState::Waiting);

    let idle = idle_sessions(tmp.path(), now, MAX_AGE, 50);
    assert_eq!(idle.len(), 1);
    assert_eq!(idle[0].session_id, "sess-w");
    assert_eq!(idle[0].state, SessionState::Idle);
}

/// The live window is the single line between the two sets: every rollout falls
/// on exactly one side, so none is counted twice or lost.
#[test]
fn live_and_dormant_rollouts_never_overlap() {
    let tmp = tempfile::tempdir().unwrap();
    let now = Utc::now();
    write_rollout_at(
        tmp.path(),
        "rollout-2026-07-07T12-00-00-a.jsonl",
        &dormant_lines("fresh"),
        now,
    );
    write_rollout_at(
        tmp.path(),
        "rollout-2026-07-07T09-00-00-b.jsonl",
        &dormant_lines("old"),
        now - Duration::hours(2),
    );

    let live = live_sessions(tmp.path(), now);
    let idle = idle_sessions(tmp.path(), now, MAX_AGE, 50);
    assert_eq!(
        live.iter().map(|s| s.session_id.as_str()).collect::<Vec<_>>(),
        ["fresh"]
    );
    assert_eq!(
        idle.iter().map(|s| s.session_id.as_str()).collect::<Vec<_>>(),
        ["old"]
    );
}

#[test]
fn dormant_rollouts_stop_at_the_age_bound_and_are_capped_by_recency() {
    let tmp = tempfile::tempdir().unwrap();
    let now = Utc::now();
    for (i, id) in ["oldest", "middle", "newest"].iter().enumerate() {
        write_rollout_at(
            tmp.path(),
            &format!("rollout-2026-07-07T0{i}-00-00-{id}.jsonl"),
            &dormant_lines(id),
            now - Duration::hours(4 - i as i64),
        );
    }
    // Beyond the bound: not usefully resumable.
    write_rollout_at(
        tmp.path(),
        "rollout-2026-07-07T00-00-00-ancient.jsonl",
        &dormant_lines("ancient"),
        now - Duration::days(8),
    );

    let all = idle_sessions(tmp.path(), now, MAX_AGE, 50);
    assert_eq!(
        all.iter().map(|s| s.session_id.as_str()).collect::<Vec<_>>(),
        ["newest", "middle", "oldest"],
        "most recent first, and the ancient one is gone"
    );

    let capped = idle_sessions(tmp.path(), now, MAX_AGE, 2);
    assert_eq!(
        capped.iter().map(|s| s.session_id.as_str()).collect::<Vec<_>>(),
        ["newest", "middle"],
        "the cap keeps the most recent, not the first walked"
    );
    assert!(idle_sessions(tmp.path(), now, MAX_AGE, 0).is_empty());
}

/// A rollout touched without gaining content (fresh mtime, old turns) is not
/// live — `live_sessions` has always rejected it on its own timestamps. It must
/// therefore be *dormant*, not vanish: classifying on mtime alone would drop it
/// from both lists, losing a resumable conversation without a word.
#[test]
fn a_touched_but_stale_rollout_is_dormant_rather_than_lost() {
    let tmp = tempfile::tempdir().unwrap();
    let now = Utc::now();
    // Content an hour old (outside the live window), file written just now.
    let path = write_rollout(
        tmp.path(),
        "rollout-2026-07-07T11-00-00-touched.jsonl",
        &stamped(&dormant_lines("sess-touched"), now - Duration::hours(1)),
    );
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::from(now))
        .unwrap();

    let scan = scan_sessions(tmp.path(), now, MAX_AGE, 50);
    assert!(scan.live.is_empty(), "stale content is not live");
    assert_eq!(
        scan.idle.iter().map(|s| s.session_id.as_str()).collect::<Vec<_>>(),
        ["sess-touched"],
        "it is dormant and resumable — it must not fall through both lists"
    );
    assert_eq!(scan.idle[0].state, SessionState::Idle);
}
