//! Live-session detection for Codex accounts (Agent-Cockpit Step 8 — parity).
//!
//! Codex has **no process registry** (there is no `sessions/*.json` busy/pid
//! store like Claude Code's), only append-only rollout transcripts under
//! `<codex_home>/sessions/YYYY/MM/DD/rollout-*.jsonl`. So — unlike
//! [`crate::sessions::live_sessions`], which joins a real pid-alive registry to
//! its transcripts — Codex "liveness" can only be *inferred* from the
//! transcript, and the honest mapping is deliberately conservative:
//!
//! - **Only recently-touched rollouts count as live.** A rollout whose last
//!   activity is older than [`CODEX_LIVE_WINDOW`] is never called live: with no
//!   pid we cannot prove its process is still alive, so liveness is scoped to
//!   sessions active within the window rather than claimed for a stale one.
//!   Those older rollouts are not lost — [`scan_sessions`] classifies them as
//!   dormant, resumable conversations. The window is the single line between the
//!   two halves, drawn on the transcript's own last timestamp, so every rollout
//!   within `max_age` lands on exactly one side.
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

use std::path::{Path, PathBuf};

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
/// Every rollout transcript under `<codex_home>/sessions` with its mtime.
/// Rollouts whose mtime is unreadable are skipped: recency is the only liveness
/// proxy Codex offers, and a session we cannot date cannot be classified.
fn rollout_files(codex_home: &Path) -> impl Iterator<Item = (PathBuf, DateTime<Utc>)> {
    WalkDir::new(codex_home.join("sessions"))
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let name = e.file_name().to_str().unwrap_or("");
            name.starts_with("rollout-") && name.ends_with(".jsonl")
        })
        .filter_map(|e| {
            let mtime = e
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map(DateTime::<Utc>::from)?;
            Some((e.into_path(), mtime))
        })
}

/// Parse one rollout into a snapshot. `None` when it holds no real turn — an
/// empty or aborted rollout is not a session. `force_state` overrides the
/// turn-derived state (a dormant rollout is Idle regardless of how it ended).
fn snapshot_of(
    path: &Path,
    mtime: DateTime<Utc>,
    now: DateTime<Utc>,
    force_state: Option<SessionState>,
) -> Option<SessionSnapshot> {
    let info = parse_rollout(path);
    if !info.has_turn {
        return None;
    }
    let project = crate::project::resolve_project(Path::new(&info.cwd));
    Some(SessionSnapshot {
        session_id: info.session_id,
        cwd: info.cwd,
        // Codex rollouts carry no session name.
        name: String::new(),
        state: force_state.unwrap_or_else(|| state_of(info.ended)),
        provider: Provider::Codex,
        model: info.model,
        effort: info.effort,
        ctx_tokens: info.ctx_tokens,
        project_root: project.root,
        project_name: project.name,
        branch: project.branch,
        worktree: project.worktree,
        // Set by the snapshot builder from the owning account (per pin).
        config_dir: None,
        last_activity: info.last_ts.or(Some(mtime)).unwrap_or(now),
        // Codex records no pid — guardrail signalling can't target it.
        pid: 0,
    })
}

/// Both halves of a `<codex_home>`'s sessions, classified in one walk.
pub struct SessionScan {
    /// Touched inside [`CODEX_LIVE_WINDOW`] — as close to "running" as a
    /// pid-less provider gets.
    pub live: Vec<SessionSnapshot>,
    /// Dormant but resumable, most-recent first and capped.
    pub idle: Vec<SessionSnapshot>,
}

/// Walk the rollouts once and put each on exactly one side of
/// [`CODEX_LIVE_WINDOW`].
///
/// The transcript's **own** last timestamp decides for every rollout that gets
/// parsed; mtime only picks who gets parsed. Keeping those two jobs apart is the
/// point — deciding with mtime as well would let the file's disk time and its
/// content disagree, and a rollout touched without gaining content (fresh mtime,
/// old turns) would then fall out of both lists: not live, because
/// [`live_sessions`] has always judged it on its timestamps, and not dormant,
/// because its mtime looks current.
///
/// Recently-touched rollouts are few, so all of them are parsed; the dormant
/// tail is open-ended, so it is ranked and capped on mtime first and only
/// `limit` of those are read. The cap is therefore only as good as mtime is a
/// proxy for the last turn — true for an appending CLI, not for a restored or
/// back-dated file.
///
/// One walk, one classification: two separate passes could disagree about the
/// same rollout and list it twice.
pub fn scan_sessions(
    codex_home: &Path,
    now: DateTime<Utc>,
    max_age: Duration,
    limit: usize,
) -> SessionScan {
    let live_cutoff = now - CODEX_LIVE_WINDOW;
    let age_cutoff = now - max_age;

    // Cheap split. `fresh` is bounded by how much was touched in the last few
    // minutes; `dormant` is the open-ended history, so it gets capped here.
    let mut fresh: Vec<(PathBuf, DateTime<Utc>)> = Vec::new();
    let mut dormant: Vec<(PathBuf, DateTime<Utc>)> = Vec::new();
    for (path, mtime) in rollout_files(codex_home) {
        if mtime >= live_cutoff {
            fresh.push((path, mtime));
        } else if limit > 0 && mtime >= age_cutoff {
            dormant.push((path, mtime));
        }
    }
    dormant.sort_by(|a, b| b.1.cmp(&a.1));
    dormant.truncate(limit);

    let mut live: Vec<SessionSnapshot> = Vec::new();
    let mut idle: Vec<SessionSnapshot> = Vec::new();

    // Everything parsed is classified by the same rule, whichever gate it came
    // through: the transcript's own last timestamp decides. mtime only chose who
    // got parsed — deciding *with* it as well would let the two disagree.
    for (path, mtime) in fresh.into_iter().chain(dormant) {
        let Some(mut s) = snapshot_of(&path, mtime, now, None) else {
            continue;
        };
        if s.last_activity >= live_cutoff {
            live.push(s);
        } else if limit > 0 && s.last_activity >= age_cutoff {
            s.state = SessionState::Idle;
            idle.push(s);
        }
        // Older than the retention bound: not usefully resumable, dropped.
    }

    // Waiting first (they need the user), then by recency — same order as the
    // Claude path so the two providers interleave consistently in the tree.
    live.sort_by(|a, b| {
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
    idle.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
    // The mtime cap bounded the dormant tail; re-apply it now that the
    // touched-but-stale ones have joined.
    idle.truncate(limit);

    SessionScan { live, idle }
}

pub fn live_sessions(codex_home: &Path, now: DateTime<Utc>) -> Vec<SessionSnapshot> {
    scan_sessions(codex_home, now, Duration::zero(), 0).live
}

/// Dormant Codex sessions. See [`scan_sessions`], which this delegates to;
/// prefer it when both halves are wanted.
pub fn idle_sessions(
    codex_home: &Path,
    now: DateTime<Utc>,
    max_age: Duration,
    limit: usize,
) -> Vec<SessionSnapshot> {
    scan_sessions(codex_home, now, max_age, limit).idle
}

#[cfg(test)]
#[path = "codex_sessions_tests.rs"]
mod tests;
