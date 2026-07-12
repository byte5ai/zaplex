//! Live-session detection for Codex accounts (Agent-Cockpit Step 8 — parity).
//!
//! Codex has **no process registry** (there is no `sessions/*.json` busy/pid
//! store like Claude Code's), only append-only rollout transcripts under
//! `<codex_home>/sessions/YYYY/MM/DD/rollout-*.jsonl`. So — unlike
//! [`crate::sessions::live_sessions`], which joins a real pid-alive registry to
//! its transcripts — Codex "liveness" can only be *inferred* from the
//! transcript, and the honest mapping is deliberately conservative:
//!
//! - **Only recently-touched rollouts count.** A rollout whose last activity is
//!   older than [`CODEX_LIVE_WINDOW`] is not listed at all: with no pid we
//!   cannot prove its process is still alive, so we scope discovery to sessions
//!   active within the window rather than claim a stale session is live. (Fully
//!   transcript-only *dormant* [`SessionState::Idle`] discovery is a separate
//!   future feature, matching Claude, which also does not surface Idle yet.)
//! - **State** mirrors Claude's `stop_reason` logic as faithfully as Codex
//!   allows: the rollout's last turn-level event decides it — `task_complete`
//!   (the agent handed control back) → [`SessionState::Waiting`]; a started but
//!   not-yet-complete turn → [`SessionState::Monitor`] ("working, hands off").
//! - **pid is `0` (unknown).** Codex records no pid, so guardrail signalling
//!   (stop/kill by pid) cannot target a Codex session — an honest capability
//!   gap surfaced as an unsignalable session, never a faked pid.
//!
//! Model, effort, context tokens, cwd and session id all come straight from the
//! rollout (Codex, unlike Claude, records the reasoning **effort** in
//! `turn_context`, so effort here is real rather than launch-registry-derived).
//! Privacy invariant holds: only token counts + coordinates are read, never
//! message text.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use walkdir::WalkDir;

use crate::types::{Provider, SessionSnapshot, SessionState};

/// A rollout whose last activity is older than this is not treated as live
/// (Codex has no pid to confirm the process, so discovery is scoped to recent
/// activity). Matches the spirit of the Claude background-job active window.
const CODEX_LIVE_WINDOW: Duration = Duration::minutes(15);

/// Recursively find the first sub-value under `key` anywhere in `v` (rollout
/// lines wrap their payload, and the token-usage object nests under `info`).
fn find<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(map) => {
            if let Some(found) = map.get(key) {
                return Some(found);
            }
            map.values().find_map(|val| find(val, key))
        }
        Value::Array(arr) => arr.iter().find_map(|val| find(val, key)),
        _ => None,
    }
}

/// The signals distilled from one rollout transcript.
#[derive(Debug, Default, Clone)]
struct RolloutInfo {
    /// Session id from `session_meta` (falls back to the file stem).
    session_id: String,
    cwd: String,
    model: String,
    /// Reasoning effort from `turn_context` (Codex records it; may be absent).
    effort: Option<String>,
    /// Current context occupancy: the latest turn's prompt tokens
    /// (`last_token_usage.input_tokens`, which already includes the cached part).
    ctx_tokens: u64,
    last_ts: Option<DateTime<Utc>>,
    /// The last turn-level event was `task_complete` (agent handed back).
    ended: bool,
    /// A real turn was observed (a `task_started`/`task_complete`/usage line) —
    /// guards against listing an empty/aborted rollout as a session.
    has_turn: bool,
}

/// Read a rollout transcript and distil its live-session signals. Best-effort
/// and defensive: each line is an independent JSON object, malformed lines are
/// skipped, and both the wrapped (`{type,payload}`) and flat shapes are handled.
fn parse_rollout(path: &Path) -> RolloutInfo {
    let mut info = RolloutInfo::default();
    let Ok(content) = std::fs::read_to_string(path) else {
        return info;
    };
    // Session id fallback: the rollout filename ends with the session uuid.
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        info.session_id = stem.rsplit_once('-').map_or(stem, |(_, id)| id).to_string();
    }

    for line in content.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let typ = v.get("type").and_then(Value::as_str).unwrap_or("");
        // Top-level timestamp advances last-activity on every line.
        if let Some(ts) = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        {
            info.last_ts = Some(ts.with_timezone(&Utc));
        }
        match typ {
            "session_meta" => {
                if let Some(id) = find(&v, "id").and_then(Value::as_str) {
                    info.session_id = id.to_string();
                }
                if let Some(cwd) = find(&v, "cwd").and_then(Value::as_str) {
                    info.cwd = cwd.to_string();
                }
            }
            "turn_context" => {
                // Model / cwd / effort of the most recent turn win.
                if let Some(m) = find(&v, "model").and_then(Value::as_str) {
                    info.model = m.to_string();
                }
                if let Some(cwd) = find(&v, "cwd").and_then(Value::as_str) {
                    info.cwd = cwd.to_string();
                }
                info.effort = find(&v, "effort")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_string)
                    .or_else(|| info.effort.clone());
            }
            "event_msg" => {
                match find(&v, "type")
                    .and_then(Value::as_str)
                    // `find` returns the outer "event_msg" first; re-read the
                    // inner payload type explicitly.
                    .filter(|t| *t != "event_msg")
                    .or_else(|| {
                        v.get("payload")
                            .and_then(|p| p.get("type"))
                            .and_then(Value::as_str)
                    }) {
                    Some("task_started") => {
                        info.ended = false;
                        info.has_turn = true;
                    }
                    Some("task_complete") => {
                        info.ended = true;
                        info.has_turn = true;
                    }
                    _ => {}
                }
                // Current context size: the latest per-turn prompt tokens.
                if let Some(last) = find(&v, "last_token_usage") {
                    if let Some(input) = last.get("input_tokens").and_then(Value::as_u64) {
                        info.ctx_tokens = input;
                        info.has_turn = true;
                    }
                }
            }
            _ => {}
        }
    }
    info
}

/// State from the distilled signals, mirroring Claude's ended→Waiting /
/// mid-run→Monitor split. (Recency gating happens in [`live_sessions`]; by the
/// time we classify, the session is already known to be recently active.)
fn state_of(ended: bool) -> SessionState {
    if ended {
        SessionState::Waiting
    } else {
        SessionState::Monitor
    }
}

/// Collect the live Codex agent-sessions of a `<codex_home>`: rollout
/// transcripts touched within [`CODEX_LIVE_WINDOW`], each distilled to
/// (model, effort, context, state, project) with `provider = Codex` and a `0`
/// pid (Codex records none). Sorted waiting-first, like the Claude path.
pub fn live_sessions(codex_home: &Path, now: DateTime<Utc>) -> Vec<SessionSnapshot> {
    let sessions_dir = codex_home.join("sessions");
    let cutoff = now - CODEX_LIVE_WINDOW;
    let mut out: Vec<SessionSnapshot> = Vec::new();

    for file in WalkDir::new(&sessions_dir)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
    {
        let name = file.file_name().to_str().unwrap_or("");
        if !(name.starts_with("rollout-") && name.ends_with(".jsonl")) {
            continue;
        }
        // Recency gate on the file's mtime — cheap, and with no pid it is the
        // only honest liveness proxy. Older rollouts are dormant, not listed.
        let mtime: Option<DateTime<Utc>> = file
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(DateTime::<Utc>::from);
        if let Some(mtime) = mtime {
            if mtime < cutoff {
                continue;
            }
        } else {
            continue;
        }

        let info = parse_rollout(file.path());
        // Cross-check the transcript's own last activity against the window and
        // require a real turn — an empty/aborted rollout is not a session.
        let last_activity = info.last_ts.or(mtime).unwrap_or(now);
        if !info.has_turn || last_activity < cutoff {
            continue;
        }
        let project = crate::project::resolve_project(Path::new(&info.cwd));
        out.push(SessionSnapshot {
            session_id: info.session_id,
            cwd: info.cwd,
            // Codex rollouts carry no session name.
            name: String::new(),
            state: state_of(info.ended),
            provider: Provider::Codex,
            model: info.model,
            effort: info.effort,
            ctx_tokens: info.ctx_tokens,
            project_root: project.root,
            project_name: project.name,
            branch: project.branch,
            worktree: project.worktree,
            last_activity,
            // Codex records no pid — guardrail signalling can't target it.
            pid: 0,
        });
    }

    // Waiting first (they need the user), then by recency — same order as the
    // Claude path so the two providers interleave consistently in the tree.
    out.sort_by(|a, b| {
        let rank = |s: &SessionSnapshot| match s.state {
            SessionState::Waiting => 0u8,
            SessionState::Active => 1,
            SessionState::Monitor => 2,
            SessionState::Idle => 3,
        };
        rank(a)
            .cmp(&rank(b))
            .then(b.last_activity.cmp(&a.last_activity))
    });
    out
}

#[cfg(test)]
#[path = "codex_sessions_tests.rs"]
mod tests;
