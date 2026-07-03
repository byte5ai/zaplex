//! Live-session detection for Claude Code accounts (cockpit C3a).
//!
//! Port of claudeplex's status algorithm (`collect.ts` readRegistry/stateOf):
//! Claude Code maintains its own process registry under
//! `<config_dir>/sessions/*.json` — the authoritative set of running sessions.
//! Each registry entry is joined to its transcript (`projects/**/<id>.jsonl`)
//! to derive whether the assistant's last turn *ended* (waiting for the user)
//! or is mid-tool-run (working). States:
//!
//! - **Active** — the registry reports the session as `busy`.
//! - **Waiting** — the last assistant turn ended (`stop_reason != tool_use`):
//!   the session needs YOU. The cockpit's most important signal.
//! - **Monitor** — mid tool-run / live background job: working, hands off.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use crate::types::{SessionSnapshot, SessionState};

/// How much transcript tail to inspect for the ended/model/context signals.
/// Registry sessions' last turns are comfortably inside this window; if a
/// single tool result exceeds it, the visible tail IS that tool result — which
/// classifies as Monitor ("Claude will continue"), the correct reading.
const TAIL_BYTES: u64 = 256 * 1024;

/// A raw entry from `<config_dir>/sessions/*.json`.
#[derive(Debug, Clone)]
struct RegEntry {
    session_id: String,
    cwd: String,
    status: String,
    kind: String,
    name: String,
    started_at: i64,
    updated_at: i64,
    pid: u32,
}

fn read_registry(config_dir: &Path) -> Vec<RegEntry> {
    let dir = config_dir.join("sessions");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut by_id: HashMap<String, RegEntry> = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let Some(session_id) = v.get("sessionId").and_then(Value::as_str) else {
            continue;
        };
        let str_of = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let int_of = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
        let reg = RegEntry {
            session_id: session_id.to_string(),
            cwd: str_of("cwd"),
            status: str_of("status"),
            kind: str_of("kind"),
            name: str_of("name"),
            started_at: int_of("startedAt"),
            updated_at: {
                let u = int_of("updatedAt");
                if u > 0 { u } else { int_of("statusUpdatedAt") }
            },
            pid: v.get("pid").and_then(Value::as_u64).unwrap_or(0) as u32,
        };
        match by_id.get(&reg.session_id) {
            Some(prev) if reg.updated_at < prev.updated_at => {}
            _ => {
                by_id.insert(reg.session_id.clone(), reg);
            }
        }
    }
    by_id.into_values().collect()
}

/// Drop internal infra (memory observers) and non-session shell helpers.
fn is_real_reg(r: &RegEntry) -> bool {
    if r.cwd.contains("observer-sessions") || r.cwd.contains(".claude-mem") {
        return false;
    }
    !(r.status == "shell" || r.status.is_empty())
}

/// Is a pid still running? (EPERM means it exists but isn't ours — alive.)
/// Unknown pid (0) → don't hide the session.
#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return true;
    }
    // SAFETY: kill with signal 0 only performs the permission/existence check.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[cfg(not(unix))]
fn pid_alive(_pid: u32) -> bool {
    true
}

/// Signals derived from a transcript's tail.
#[derive(Debug, Default, Clone)]
struct TranscriptTail {
    /// The assistant's last turn ended (`stop_reason != tool_use`) — waiting.
    ended: bool,
    model: String,
    /// Context-window fill of the latest assistant turn (input + cache tokens).
    ctx_tokens: u64,
    last_ts: Option<DateTime<Utc>>,
}

/// Reads the last [`TAIL_BYTES`] of a transcript and derives the tail signals.
/// Lines are independent JSON objects, so a partial first line is skipped.
fn read_transcript_tail(path: &Path) -> TranscriptTail {
    let mut tail = TranscriptTail::default();
    let Ok(mut file) = std::fs::File::open(path) else {
        return tail;
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return tail;
    }
    let mut buf = String::new();
    if file.read_to_string(&mut buf).is_err() {
        return tail;
    }
    let mut lines = buf.lines();
    if start > 0 {
        lines.next(); // skip the partial first line
    }

    // "ended" is decided by the LAST relevant line kind, mirroring
    // claudeplex's parseSessionFile: assistant_end vs assistant_tool /
    // tool_result (Claude continues) vs plain user input.
    let mut last_kind_ended = false;
    for line in lines {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let typ = v.get("type").and_then(Value::as_str).unwrap_or("");
        match typ {
            "assistant" => {
                let Some(message) = v.get("message") else {
                    continue;
                };
                let stop_reason = message.get("stop_reason").and_then(Value::as_str);
                last_kind_ended = stop_reason != Some("tool_use");
                if let Some(model) = message.get("model").and_then(Value::as_str) {
                    tail.model = model.to_string();
                }
                if let Some(usage) = message.get("usage") {
                    let t = |k: &str| usage.get(k).and_then(Value::as_u64).unwrap_or(0);
                    tail.ctx_tokens = t("input_tokens")
                        + t("cache_read_input_tokens")
                        + t("cache_creation_input_tokens");
                }
                if let Some(ts) = v.get("timestamp").and_then(Value::as_str) {
                    if let Ok(parsed) = DateTime::parse_from_rfc3339(ts) {
                        tail.last_ts = Some(parsed.with_timezone(&Utc));
                    }
                }
            }
            "user" => {
                let is_meta = v.get("isMeta").and_then(Value::as_bool).unwrap_or(false);
                if is_meta {
                    continue;
                }
                match v.get("message").and_then(|m| m.get("content")) {
                    // A tool result → Claude will continue.
                    Some(Value::Array(_)) => last_kind_ended = false,
                    Some(Value::String(text))
                        if !text.contains("<system-reminder>")
                            && !text.contains("<local-command")
                            && !text.contains("<command-") =>
                    {
                        last_kind_ended = false;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    tail.ended = last_kind_ended;
    tail
}

/// claudeplex `stateOf`: busy → Active; live background job → Monitor;
/// otherwise ended → Waiting, mid-run → Monitor.
fn state_of(status: &str, ended: bool, background: bool) -> SessionState {
    if status == "busy" {
        return SessionState::Active;
    }
    if background {
        return SessionState::Monitor;
    }
    if ended {
        SessionState::Waiting
    } else {
        SessionState::Monitor
    }
}

/// Window in which a background job still counts as live without a busy status.
const ACTIVE_WINDOW_MS: i64 = 15 * 60 * 1000;

/// Maps every transcript under `projects/` by session id (file stem).
fn transcripts_by_id(config_dir: &Path) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();
    let projects = config_dir.join("projects");
    let Ok(project_dirs) = std::fs::read_dir(&projects) else {
        return map;
    };
    for project in project_dirs.flatten() {
        let Ok(files) = std::fs::read_dir(project.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    map.insert(stem.to_string(), path);
                }
            }
        }
    }
    map
}

/// Collects the live sessions of a Claude Code account: registry entries that
/// are real, PID-alive and transcript-backed, joined with their transcript
/// tail for the waiting/working classification.
pub fn live_sessions(config_dir: &Path, now: DateTime<Utc>) -> Vec<SessionSnapshot> {
    let transcripts = transcripts_by_id(config_dir);
    let mut sessions: Vec<SessionSnapshot> = read_registry(config_dir)
        .into_iter()
        .filter(|r| is_real_reg(r) && transcripts.contains_key(&r.session_id) && pid_alive(r.pid))
        .map(|r| {
            let tail = transcripts
                .get(&r.session_id)
                .map(|p| read_transcript_tail(p))
                .unwrap_or_default();
            let updated = Utc
                .timestamp_millis_opt(r.updated_at.max(r.started_at))
                .single()
                .unwrap_or(now);
            let last_activity = tail.last_ts.map_or(updated, |t| t.max(updated));
            let background = r.kind == "bg"
                && (r.status == "busy"
                    || (now - last_activity).num_milliseconds() < ACTIVE_WINDOW_MS);
            SessionSnapshot {
                session_id: r.session_id,
                cwd: r.cwd,
                name: r.name,
                state: state_of(&r.status, tail.ended, background),
                model: tail.model,
                ctx_tokens: tail.ctx_tokens,
                last_activity,
                pid: r.pid,
            }
        })
        .collect();
    // Waiting first (they need the user), then by recency.
    sessions.sort_by(|a, b| {
        let rank = |s: &SessionSnapshot| match s.state {
            SessionState::Waiting => 0u8,
            SessionState::Active => 1,
            SessionState::Monitor => 2,
        };
        rank(a)
            .cmp(&rank(b))
            .then(b.last_activity.cmp(&a.last_activity))
    });
    sessions
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
