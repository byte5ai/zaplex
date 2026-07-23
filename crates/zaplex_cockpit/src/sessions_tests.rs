use std::time::SystemTime;

use chrono::{DateTime, Duration, Utc};
use serde_json::json;

use super::*;
use crate::types::{Provider, SessionState};

/// Builds a fake account dir with one registry entry + transcript.
fn fake_account(
    dir: &Path,
    session_id: &str,
    status: &str,
    kind: &str,
    transcript_lines: &[serde_json::Value],
) {
    // Own pid: guaranteed alive.
    fake_account_at(
        dir,
        session_id,
        status,
        kind,
        transcript_lines,
        std::process::id(),
        Utc::now(),
    );
}

/// As [`fake_account`], but with an explicit pid and registry timestamp — the
/// two inputs that decide live-vs-dormant.
fn fake_account_at(
    dir: &Path,
    session_id: &str,
    status: &str,
    kind: &str,
    transcript_lines: &[serde_json::Value],
    pid: u32,
    updated: DateTime<Utc>,
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
        "startedAt": updated.timestamp_millis(),
        "updatedAt": updated.timestamp_millis(),
        "pid": pid,
        "procStart": crate::process_identity::registry_start_for_process(pid),
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
    let path = projects.join(format!("{session_id}.jsonl"));
    std::fs::write(&path, transcript).unwrap();
    // Age the transcript with the registry. Both move together in reality — the
    // CLI writes them on the same activity — and dormant discovery ranks on the
    // later of the two, so a fixture that aged only the registry would describe
    // a session that never exists: idle for days, yet written to seconds ago.
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(SystemTime::from(updated))
        .unwrap();
}

/// A pid that is certainly gone: spawn a throwaway child and reap it. Guessing
/// some high number would be worse than merely unreliable — `pid_alive` casts
/// `u32` to a signed `pid_t`, so anything above `i32::MAX` wraps negative, and
/// pid -1 addresses *every* process.
fn dead_pid() -> u32 {
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "exit 0"])
        .spawn()
        .expect("spawn a throwaway child");
    let pid = child.id();
    child.wait().expect("reap it");
    pid
}

#[cfg(unix)]
#[test]
fn pid_outside_the_signed_process_id_range_is_not_alive() {
    assert!(
        !crate::process_identity::probe_registered_process(u32::MAX, None, 0).alive
    );
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
    // New inventory fields: Claude provider, unknown effort, project resolved
    // from the (non-repo) cwd — root == cwd, name == basename.
    assert_eq!(sessions[0].provider, Provider::Claude);
    assert_eq!(sessions[0].effort, None);
    assert_eq!(sessions[0].project_root, "/tmp/proj");
    assert_eq!(sessions[0].project_name, "proj");
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    assert!(
        sessions[0].process_fingerprint.is_some(),
        "a matching Claude procStart must bind the live process"
    );
}

fn replace_registry_proc_start(dir: &Path, session_id: &str, proc_start: Option<&str>) {
    let path = dir.join("sessions").join(format!("{session_id}.json"));
    let mut entry: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    match proc_start {
        Some(proc_start) => {
            entry["procStart"] = serde_json::Value::String(proc_start.to_string());
        }
        None => {
            entry.as_object_mut().unwrap().remove("procStart");
        }
    }
    std::fs::write(path, serde_json::to_vec(&entry).unwrap()).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn mismatching_registry_process_start_keeps_the_session_visible_but_unsignalable() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "recycled",
        "busy",
        "",
        &[assistant_line("tool_use")],
    );
    replace_registry_proc_start(tmp.path(), "recycled", Some("not-this-process"));

    let sessions = live_sessions(tmp.path(), Utc::now());
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "recycled");
    assert_eq!(sessions[0].process_fingerprint, None);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn missing_registry_process_start_keeps_the_session_visible_but_unsignalable() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "unproven",
        "busy",
        "",
        &[assistant_line("tool_use")],
    );
    replace_registry_proc_start(tmp.path(), "unproven", None);

    let sessions = live_sessions(tmp.path(), Utc::now());
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "unproven");
    assert_eq!(sessions[0].process_fingerprint, None);
}

#[test]
fn legacy_session_snapshot_without_a_fingerprint_deserializes_as_unverified() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "legacy-wire",
        "busy",
        "",
        &[assistant_line("tool_use")],
    );
    let snapshot = live_sessions(tmp.path(), Utc::now()).remove(0);
    let mut encoded = serde_json::to_value(snapshot).unwrap();
    encoded
        .as_object_mut()
        .unwrap()
        .remove("process_fingerprint");

    let decoded: crate::types::SessionSnapshot = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.process_fingerprint, None);
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

// ── Dormant (idle) discovery ────────────────────────────────────────────────

const MAX_AGE: Duration = Duration::days(7);

#[test]
fn a_finished_session_is_discovered_as_dormant_and_resumable() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account_at(
        tmp.path(),
        "gone",
        "idle",
        "",
        &[assistant_line("end_turn")],
        dead_pid(),
        Utc::now(),
    );

    let idle = idle_sessions(tmp.path(), Utc::now(), MAX_AGE, 50);
    assert_eq!(idle.len(), 1, "the finished session must be discoverable");
    assert_eq!(idle[0].state, SessionState::Idle);
    // The session id is what `claude --resume <id>` needs; without it the row
    // could be shown but never adopted.
    assert_eq!(idle[0].session_id, "gone");
    assert_eq!(idle[0].provider, Provider::Claude);
    // Still transcript-backed, so the row is not an empty shell.
    assert_eq!(idle[0].model, "claude-opus-4-8");
}

/// The central invariant: `pid_alive` decides, so no session can be in both
/// lists. Live surfaces (Conductor, account status) read `sessions`; the table's
/// Idle filter reads the other. An overlap would double-count a session.
#[test]
fn live_and_dormant_sessions_never_overlap() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(tmp.path(), "running", "busy", "", &[assistant_line("end_turn")]);
    fake_account_at(
        tmp.path(),
        "finished",
        "idle",
        "",
        &[assistant_line("end_turn")],
        dead_pid(),
        Utc::now(),
    );

    let live = live_sessions(tmp.path(), Utc::now());
    let idle = idle_sessions(tmp.path(), Utc::now(), MAX_AGE, 50);

    assert_eq!(live.len(), 1);
    assert_eq!(live[0].session_id, "running");
    assert_eq!(idle.len(), 1);
    assert_eq!(idle[0].session_id, "finished");
    assert!(
        live.iter().all(|l| idle.iter().all(|i| i.session_id != l.session_id)),
        "a session must never appear as both running and dormant"
    );
}

/// pid 0 means the registry never recorded one — unknown, not dead. Claiming
/// such a session dormant would assert a fact we cannot show.
#[test]
fn an_unknown_pid_is_never_claimed_dormant() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account_at(
        tmp.path(),
        "nopid",
        "idle",
        "",
        &[assistant_line("end_turn")],
        0,
        Utc::now(),
    );

    assert!(
        idle_sessions(tmp.path(), Utc::now(), MAX_AGE, 50).is_empty(),
        "an unknown pid is not proof the process is gone"
    );
    // And it keeps its existing home: `pid_alive` treats 0 as alive, so the
    // session stays visible rather than vanishing between the two lists.
    assert_eq!(live_sessions(tmp.path(), Utc::now()).len(), 1);
}

#[test]
fn dormant_discovery_stops_at_the_age_bound() {
    let tmp = tempfile::tempdir().unwrap();
    let now = Utc::now();
    fake_account_at(
        tmp.path(),
        "ancient",
        "idle",
        "",
        &[assistant_line("end_turn")],
        dead_pid(),
        now - Duration::days(8),
    );

    assert!(
        idle_sessions(tmp.path(), now, MAX_AGE, 50).is_empty(),
        "a conversation older than the bound is not usefully resumable"
    );
    // Same session, wider bound → found. Proves the age gate is what excluded
    // it, not some unrelated filter.
    assert_eq!(idle_sessions(tmp.path(), now, Duration::days(30), 50).len(), 1);
}

#[test]
fn dormant_discovery_is_capped_and_most_recent_first() {
    let tmp = tempfile::tempdir().unwrap();
    let now = Utc::now();
    // Oldest first, so a cap that ignored recency would keep the wrong ones.
    for (i, id) in ["oldest", "middle", "newest"].iter().enumerate() {
        fake_account_at(
            tmp.path(),
            id,
            "idle",
            "",
            &[assistant_line("end_turn")],
            dead_pid(),
            now - Duration::hours(3 - i as i64),
        );
    }

    let all = idle_sessions(tmp.path(), now, MAX_AGE, 50);
    assert_eq!(
        all.iter().map(|s| s.session_id.as_str()).collect::<Vec<_>>(),
        ["newest", "middle", "oldest"],
        "most recent first"
    );

    let capped = idle_sessions(tmp.path(), now, MAX_AGE, 2);
    assert_eq!(
        capped.iter().map(|s| s.session_id.as_str()).collect::<Vec<_>>(),
        ["newest", "middle"],
        "the cap keeps the most recent, not the first found"
    );
    assert!(idle_sessions(tmp.path(), now, MAX_AGE, 0).is_empty());
}

/// The cap has to be taken on something that tracks the truth. `last_activity`
/// is `max(transcript tail, registry updatedAt)`, so ranking on `updatedAt`
/// alone could cut a session that is in fact more recent than one it keeps.
/// Here the registry lags badly while the transcript is current — the fresh
/// session must still win the single slot.
#[test]
fn the_cap_ranks_on_real_recency_not_a_lagging_registry() {
    let tmp = tempfile::tempdir().unwrap();
    let now = Utc::now();

    // Registry says 6 days ago, but the conversation was written to a minute
    // ago: a long-running session whose registry entry went stale.
    fake_account_at(
        tmp.path(),
        "lagging-registry",
        "idle",
        "",
        &[assistant_line("end_turn")],
        dead_pid(),
        now - Duration::days(6),
    );
    touch_transcript(tmp.path(), "lagging-registry", now - Duration::minutes(1));

    // Honestly two hours old, registry and transcript agreeing.
    fake_account_at(
        tmp.path(),
        "genuinely-older",
        "idle",
        "",
        &[assistant_line("end_turn")],
        dead_pid(),
        now - Duration::hours(2),
    );

    let capped = idle_sessions(tmp.path(), now, MAX_AGE, 1);
    assert_eq!(
        capped.iter().map(|s| s.session_id.as_str()).collect::<Vec<_>>(),
        ["lagging-registry"],
        "the cap must keep the session that was actually touched last"
    );
}

/// Move a transcript's mtime without touching its registry entry.
fn touch_transcript(dir: &Path, session_id: &str, at: DateTime<Utc>) {
    std::fs::File::options()
        .write(true)
        .open(dir.join("projects").join("-tmp-proj").join(format!("{session_id}.jsonl")))
        .unwrap()
        .set_modified(SystemTime::from(at))
        .unwrap();
}
