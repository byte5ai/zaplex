use std::time::SystemTime;
use std::{fs::OpenOptions, io::Write};

use chrono::{DateTime, Duration, Utc};
use serde_json::json;

use super::*;
use crate::types::{Provider, SessionState, TaskItem, TaskState, TaskStatus};

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
    assert!(!crate::process_identity::probe_registered_process(u32::MAX, None, 0).alive);
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

fn task_tool_line(name: &str, id: &str, input: serde_json::Value) -> serde_json::Value {
    json!({
        "type": "assistant",
        "timestamp": "2026-07-03T00:00:00Z",
        "message": {
            "stop_reason": "tool_use",
            "model": "claude-opus-4-8",
            "usage": {"input_tokens": 100},
            "content": [{
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }]
        }
    })
}

fn task_create_result_line(tool_use_id: &str, task_id: &str, subject: &str) -> serde_json::Value {
    json!({
        "type": "user",
        "timestamp": "2026-07-03T00:00:01Z",
        "message": {
            "content": [{
                "type": "tool_result",
                "tool_use_id": tool_use_id,
                "content": format!("Task #{task_id} created successfully: {subject}")
            }]
        },
        "toolUseResult": {
            "task": {
                "id": task_id,
                "subject": subject
            }
        }
    })
}

#[test]
fn busy_session_is_active() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(tmp.path(), "s1", "busy", "", &[assistant_line("end_turn")]);
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
    assert_eq!(
        sessions[0].task_state, None,
        "a transcript without task tools preserves the existing coarse state path"
    );
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    assert!(
        sessions[0].process_fingerprint.is_some(),
        "a matching Claude procStart must bind the live process"
    );
}

#[test]
fn modern_interactive_registry_without_status_is_visible() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "modern",
        "",
        "interactive",
        &[assistant_line("end_turn")],
    );
    let registry = tmp.path().join("sessions").join("modern.json");
    let mut entry: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&registry).unwrap()).unwrap();
    entry.as_object_mut().unwrap().remove("status");
    std::fs::write(registry, serde_json::to_vec(&entry).unwrap()).unwrap();

    let sessions = live_sessions(tmp.path(), Utc::now());
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "modern");
    assert_eq!(sessions[0].state, SessionState::Waiting);
}

#[test]
fn modern_background_registry_without_status_is_visible_as_monitor() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "background",
        "",
        "bg",
        &[assistant_line("end_turn")],
    );

    let sessions = live_sessions(tmp.path(), Utc::now());
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "background");
    assert_eq!(sessions[0].state, SessionState::Monitor);
}

#[test]
fn modern_statusless_shell_helper_is_still_hidden() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "helper",
        "",
        "shell",
        &[assistant_line("end_turn")],
    );

    assert!(live_sessions(tmp.path(), Utc::now()).is_empty());
}

#[test]
fn claude_shell_helper_is_not_a_session() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "shell-helper",
        "",
        "shell",
        &[assistant_line("end_turn")],
    );

    let registry = tmp.path().join("sessions").join("shell-helper.json");
    let mut entry: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&registry).unwrap()).unwrap();
    entry.as_object_mut().unwrap().remove("status");
    std::fs::write(registry, serde_json::to_vec(&entry).unwrap()).unwrap();

    let scan = scan_sessions(tmp.path(), Utc::now(), Duration::hours(6), 24);
    assert!(scan.live.is_empty());
    assert!(scan.idle.is_empty());
}

#[test]
fn modern_statusless_unknown_helper_is_still_hidden() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "helper",
        "",
        "worker",
        &[assistant_line("end_turn")],
    );

    assert!(live_sessions(tmp.path(), Utc::now()).is_empty());
}

#[test]
fn recent_substantial_transcript_survives_registry_cleanup_only_as_idle() {
    let tmp = tempfile::tempdir().unwrap();
    let mut first = assistant_line("end_turn");
    first["cwd"] = json!("/tmp/proj");
    fake_account(
        tmp.path(),
        "history",
        "idle",
        "interactive",
        &[first, assistant_line("end_turn")],
    );
    std::fs::remove_file(tmp.path().join("sessions").join("history.json")).unwrap();

    let scan = scan_sessions(tmp.path(), Utc::now(), Duration::hours(6), 24);
    assert!(
        scan.live.is_empty(),
        "transcript-only history must not enter the live Cockpit tree"
    );
    assert_eq!(scan.idle.len(), 1);
    assert_eq!(scan.idle[0].session_id, "history");
    assert_eq!(scan.idle[0].state, SessionState::Idle);
    assert_eq!(scan.idle[0].cwd, "/tmp/proj");
    assert_eq!(scan.idle[0].pid, 0, "history is not a running process");
}

#[test]
fn legacy_claude_transcript_without_registry_is_resumable() {
    let tmp = tempfile::tempdir().unwrap();
    let mut first = assistant_line("end_turn");
    first["cwd"] = json!("/tmp/legacy-project");
    fake_account(
        tmp.path(),
        "legacy-history",
        "idle",
        "interactive",
        &[first, assistant_line("end_turn")],
    );
    std::fs::remove_file(tmp.path().join("sessions").join("legacy-history.json")).unwrap();

    let scan = scan_sessions(tmp.path(), Utc::now(), Duration::hours(6), 24);
    assert!(scan.live.is_empty());
    assert_eq!(scan.idle.len(), 1);
    assert_eq!(scan.idle[0].session_id, "legacy-history");
    assert_eq!(scan.idle[0].cwd, "/tmp/legacy-project");
    assert_eq!(scan.idle[0].state, SessionState::Idle);
}

#[test]
fn transcript_only_history_must_be_substantial() {
    let tmp = tempfile::tempdir().unwrap();
    let mut line = assistant_line("end_turn");
    line["cwd"] = json!("/tmp/proj");
    fake_account(tmp.path(), "fragment", "idle", "interactive", &[line]);
    std::fs::remove_file(tmp.path().join("sessions").join("fragment.json")).unwrap();

    let scan = scan_sessions(tmp.path(), Utc::now(), Duration::hours(6), 24);
    assert!(scan.live.is_empty());
    assert!(
        scan.idle.is_empty(),
        "one text turn without a tool is an automation fragment, not resumable history"
    );
}

#[test]
fn transcript_only_history_requires_a_launch_directory() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "unlocated",
        "idle",
        "interactive",
        &[assistant_line("end_turn"), assistant_line("end_turn")],
    );
    std::fs::remove_file(tmp.path().join("sessions").join("unlocated.json")).unwrap();

    let scan = scan_sessions(tmp.path(), Utc::now(), Duration::hours(6), 24);
    assert!(scan.live.is_empty());
    assert!(
        scan.idle.is_empty(),
        "history without its recorded cwd cannot be resumed safely"
    );
}

#[test]
fn transcript_only_observer_history_is_hidden() {
    let tmp = tempfile::tempdir().unwrap();
    let mut first = assistant_line("end_turn");
    first["cwd"] = json!("/tmp/observer-sessions/proj");
    fake_account(
        tmp.path(),
        "observer",
        "idle",
        "interactive",
        &[first, assistant_line("end_turn")],
    );
    std::fs::remove_file(tmp.path().join("sessions").join("observer.json")).unwrap();

    let scan = scan_sessions(tmp.path(), Utc::now(), Duration::hours(6), 24);
    assert!(scan.live.is_empty());
    assert!(scan.idle.is_empty());
}

#[test]
fn claude_session_reconcile_refreshes_structured_task_updates() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "task-session",
        "busy",
        "",
        &[
            task_tool_line(
                "TaskCreate",
                "toolu_create",
                json!({
                    "subject":"Wire task state",
                    "description":"Carry structured progress"
                }),
            ),
            task_create_result_line("toolu_create", "2", "Wire task state"),
        ],
    );

    let first = live_sessions(tmp.path(), Utc::now()).remove(0);
    assert_eq!(
        first.task_state,
        Some(TaskState {
            tasks: vec![TaskItem {
                id: "2".into(),
                title: "Wire task state".into(),
                status: TaskStatus::Pending,
            }],
        })
    );

    let transcript = tmp
        .path()
        .join("projects")
        .join("-tmp-proj")
        .join("task-session.jsonl");
    let mut file = OpenOptions::new().append(true).open(transcript).unwrap();
    writeln!(
        file,
        "{}",
        serde_json::to_string(&task_tool_line(
            "TaskUpdate",
            "toolu_update",
            json!({"taskId":"2","status":"completed"}),
        ))
        .unwrap()
    )
    .unwrap();

    let refreshed = live_sessions(tmp.path(), Utc::now()).remove(0);
    assert_eq!(
        refreshed.task_state.unwrap().tasks[0].status,
        TaskStatus::Completed,
        "the existing disk-scan reconcile path must observe appended task updates"
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
fn legacy_session_snapshot_without_task_state_deserializes_as_no_structured_state() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "legacy-task-wire",
        "busy",
        "",
        &[assistant_line("tool_use")],
    );
    let snapshot = live_sessions(tmp.path(), Utc::now()).remove(0);
    let mut encoded = serde_json::to_value(snapshot).unwrap();
    encoded.as_object_mut().unwrap().remove("task_state");

    let decoded: crate::types::SessionSnapshot = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.task_state, None);
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
    fake_account(
        tmp.path(),
        "running",
        "busy",
        "",
        &[assistant_line("end_turn")],
    );
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
        live.iter()
            .all(|l| idle.iter().all(|i| i.session_id != l.session_id)),
        "a session must never appear as both running and dormant"
    );
}

#[test]
fn dormant_history_never_enters_live_tree() {
    let tmp = tempfile::tempdir().unwrap();
    fake_account(
        tmp.path(),
        "live-session",
        "busy",
        "interactive",
        &[assistant_line("tool_use")],
    );
    fake_account_at(
        tmp.path(),
        "dormant-session",
        "idle",
        "interactive",
        &[assistant_line("end_turn")],
        dead_pid(),
        Utc::now(),
    );

    let scan = scan_sessions(tmp.path(), Utc::now(), MAX_AGE, 50);
    assert_eq!(scan.live.len(), 1);
    assert_eq!(scan.live[0].session_id, "live-session");
    assert_eq!(scan.idle.len(), 1);
    assert_eq!(scan.idle[0].session_id, "dormant-session");
    assert!(scan
        .live
        .iter()
        .all(|session| session.session_id != "dormant-session"));
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
    assert_eq!(
        idle_sessions(tmp.path(), now, Duration::days(30), 50).len(),
        1
    );
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
        all.iter()
            .map(|s| s.session_id.as_str())
            .collect::<Vec<_>>(),
        ["newest", "middle", "oldest"],
        "most recent first"
    );

    let capped = idle_sessions(tmp.path(), now, MAX_AGE, 2);
    assert_eq!(
        capped
            .iter()
            .map(|s| s.session_id.as_str())
            .collect::<Vec<_>>(),
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
        capped
            .iter()
            .map(|s| s.session_id.as_str())
            .collect::<Vec<_>>(),
        ["lagging-registry"],
        "the cap must keep the session that was actually touched last"
    );
}

/// Move a transcript's mtime without touching its registry entry.
fn touch_transcript(dir: &Path, session_id: &str, at: DateTime<Utc>) {
    std::fs::File::options()
        .write(true)
        .open(
            dir.join("projects")
                .join("-tmp-proj")
                .join(format!("{session_id}.jsonl")),
        )
        .unwrap()
        .set_modified(SystemTime::from(at))
        .unwrap();
}

#[test]
fn transcript_viewer_returns_stable_content_revision_and_updates_on_append() {
    let tmp = tempfile::tempdir().unwrap();
    let session_id = "viewer-session";
    fake_account(
        tmp.path(),
        session_id,
        "idle",
        "",
        &[assistant_line("end_turn")],
    );

    let first = load_transcript_with_revision(tmp.path(), session_id)
        .unwrap()
        .unwrap();
    let second = load_transcript_with_revision(tmp.path(), session_id)
        .unwrap()
        .unwrap();
    assert_eq!(first.source_revision, second.source_revision);
    assert_eq!(first.turns, second.turns);

    let path = tmp
        .path()
        .join("projects")
        .join("-tmp-proj")
        .join(format!("{session_id}.jsonl"));
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    writeln!(
        file,
        "{}",
        serde_json::to_string(&json!({
            "type": "user",
            "message": {"content": "continue"}
        }))
        .unwrap()
    )
    .unwrap();
    let changed = load_transcript_with_revision(tmp.path(), session_id)
        .unwrap()
        .unwrap();
    assert_ne!(first.source_revision, changed.source_revision);
    assert_eq!(changed.turns.last().unwrap().text, "continue");
}

#[test]
fn append_during_read_does_not_make_claude_transcript_unavailable() {
    let tmp = tempfile::tempdir().unwrap();
    let session_id = "viewer-growing-session";
    fake_account(
        tmp.path(),
        session_id,
        "idle",
        "",
        &[assistant_line("end_turn")],
    );

    let loaded = load_transcript_with_revision_after_read(tmp.path(), session_id, |path| {
        let mut file = OpenOptions::new().append(true).open(path).unwrap();
        writeln!(
            file,
            "{}",
            serde_json::to_string(&json!({
                "type": "assistant",
                "message": {
                    "content": [{"type": "text", "text": "appended reply"}],
                    "stop_reason": "end_turn"
                }
            }))
            .unwrap()
        )
        .unwrap();
    });

    let loaded = loaded.expect("append-only growth is accepted").unwrap();
    assert!(
        loaded
            .turns
            .iter()
            .all(|turn| turn.text != "appended reply"),
        "the in-flight append must not extend the bounded snapshot"
    );
    let refreshed = load_transcript_with_revision(tmp.path(), session_id)
        .unwrap()
        .unwrap();
    assert!(
        refreshed
            .turns
            .iter()
            .any(|turn| turn.text == "appended reply"),
        "a later snapshot must include the completed append"
    );
    assert_ne!(
        loaded.source_revision, refreshed.source_revision,
        "the revision identifies the accepted byte extent"
    );
}

#[test]
fn transcript_viewer_ignores_an_incomplete_final_jsonl_record() {
    let tmp = tempfile::tempdir().unwrap();
    let session_id = "viewer-partial-session";
    fake_account(
        tmp.path(),
        session_id,
        "idle",
        "",
        &[assistant_line("end_turn")],
    );
    let complete = load_transcript_with_revision(tmp.path(), session_id)
        .unwrap()
        .unwrap();
    let path = tmp
        .path()
        .join("projects/-tmp-proj")
        .join(format!("{session_id}.jsonl"));
    let mut file = OpenOptions::new().append(true).open(path).unwrap();
    write!(
        file,
        r#"{{"type":"assistant","message":{{"content":"unfinished"#
    )
    .unwrap();
    file.write_all(&[0xf0, 0x9f]).unwrap();

    let partial = load_transcript_with_revision(tmp.path(), session_id)
        .unwrap()
        .unwrap();
    assert_eq!(partial.turns, complete.turns);
    assert_eq!(partial.source_revision, complete.source_revision);
}

#[test]
fn transcript_viewer_retries_a_truncation_detected_after_read() {
    let tmp = tempfile::tempdir().unwrap();
    let session_id = "viewer-truncated-session";
    fake_account(
        tmp.path(),
        session_id,
        "idle",
        "",
        &[assistant_line("end_turn")],
    );

    let loaded = load_transcript_with_revision_after_read(tmp.path(), session_id, |path| {
        OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(0)
            .unwrap();
    });

    assert!(
        matches!(loaded, Err(TranscriptError::ChangedDuringRead)),
        "truncation invalidates the accepted extent"
    );
}

#[test]
fn transcript_viewer_rejects_ambiguous_and_invalid_session_identity() {
    let tmp = tempfile::tempdir().unwrap();
    let session_id = "duplicate-session";
    for project in ["one", "two"] {
        let project = tmp.path().join("projects").join(project);
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join(format!("{session_id}.jsonl")),
            serde_json::to_string(&assistant_line("end_turn")).unwrap(),
        )
        .unwrap();
    }
    assert!(matches!(
        load_transcript_with_revision(tmp.path(), session_id),
        Err(TranscriptError::AmbiguousSessionId)
    ));
    assert!(matches!(
        load_transcript_with_revision(tmp.path(), "../escape"),
        Err(TranscriptError::InvalidSessionId)
    ));
}

#[cfg(unix)]
#[test]
fn transcript_viewer_never_follows_a_transcript_symlink() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path().join("projects").join("project");
    std::fs::create_dir_all(&project).unwrap();
    let outside = tmp.path().join("outside.jsonl");
    std::fs::write(
        &outside,
        serde_json::to_string(&assistant_line("end_turn")).unwrap(),
    )
    .unwrap();
    symlink(&outside, project.join("linked-session.jsonl")).unwrap();

    assert!(
        load_transcript_with_revision(tmp.path(), "linked-session")
            .unwrap()
            .is_none(),
        "a symlink is not an eligible transcript candidate"
    );
}

#[cfg(unix)]
#[test]
fn transcript_viewer_rejects_a_path_replaced_after_resolution() {
    let account = tempfile::tempdir().unwrap();
    let session_id = "path-replacement";
    let project = account.path().join("projects/project");
    std::fs::create_dir_all(&project).unwrap();
    let path = project.join(format!("{session_id}.jsonl"));
    std::fs::write(
        &path,
        serde_json::to_string(&assistant_line("end_turn")).unwrap(),
    )
    .unwrap();
    let resolved = resolve_transcript_for_viewer(account.path(), session_id)
        .unwrap()
        .unwrap();

    std::fs::rename(&path, path.with_extension("resolved")).unwrap();
    std::fs::write(
        &path,
        serde_json::to_string(&assistant_line("tool_use")).unwrap(),
    )
    .unwrap();

    assert!(matches!(
        open_transcript_for_viewer(&resolved),
        Err(TranscriptError::ChangedDuringRead)
    ));
}

#[cfg(unix)]
#[test]
fn transcript_viewer_rejects_a_provider_root_replaced_after_resolution() {
    use std::os::unix::fs::symlink;

    let base = tempfile::tempdir().unwrap();
    let replacement = tempfile::tempdir().unwrap();
    let account = base.path().join("account");
    let session_id = "root-replacement";
    let original_project = account.join("projects/project");
    std::fs::create_dir_all(&original_project).unwrap();
    let original_path = original_project.join(format!("{session_id}.jsonl"));
    std::fs::write(
        &original_path,
        serde_json::to_string(&assistant_line("end_turn")).unwrap(),
    )
    .unwrap();
    let resolved = resolve_transcript_for_viewer(&account, session_id)
        .unwrap()
        .unwrap();

    std::fs::rename(&account, base.path().join("account-original")).unwrap();
    let replacement_project = replacement.path().join("projects/project");
    std::fs::create_dir_all(&replacement_project).unwrap();
    std::fs::hard_link(
        base.path()
            .join("account-original/projects/project")
            .join(format!("{session_id}.jsonl")),
        replacement_project.join(format!("{session_id}.jsonl")),
    )
    .unwrap();
    symlink(replacement.path(), &account).unwrap();

    assert!(matches!(
        open_transcript_for_viewer(&resolved),
        Err(TranscriptError::MalformedTranscript)
    ));
}
