//! Live-session detection for Claude Code accounts (cockpit C3a).
//!
//! Evolved from claudeplex's status algorithm (`collect.ts`
//! readRegistry/stateOf), including compatibility with current Claude Code
//! registries that identify real sessions by `kind` and omit `status`:
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
//!
//! [`idle_sessions`] covers the other half: conversations whose process is
//! **gone** but whose transcript survives, including recent substantial history
//! after Claude has removed the registry row. A registry-backed session is
//! probed once, so the live and idle sets can never overlap.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::Value;

use crate::types::{Provider, SessionSnapshot, SessionState};

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
    /// OS process-start marker written by Claude Code alongside the pid.
    proc_start: Option<String>,
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
                if u > 0 {
                    u
                } else {
                    int_of("statusUpdatedAt")
                }
            },
            pid: v.get("pid").and_then(Value::as_u64).unwrap_or(0) as u32,
            proc_start: v
                .get("procStart")
                .and_then(Value::as_str)
                .map(str::to_string),
        };
        match by_id.get(&reg.session_id) {
            Some(prev)
                if reg.updated_at.max(reg.started_at) < prev.updated_at.max(prev.started_at) => {}
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
    if r.status == "shell" || r.kind == "shell" {
        return false;
    }
    // Older registries used `status` (busy/idle) to distinguish sessions from
    // shell helpers. Current Claude Code omits that field and uses a typed
    // `kind` instead. Accept only known conversation kinds on that modern path
    // so an arbitrary status-less helper does not become an agent row.
    !r.status.is_empty() || matches!(r.kind.as_str(), "interactive" | "bg")
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
    turns: usize,
    tools: usize,
}

impl TranscriptTail {
    fn is_substantial(&self) -> bool {
        self.turns >= 2 || self.tools >= 1
    }
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
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return tail;
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.lines();
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
                match message.get("content") {
                    Some(Value::Array(parts)) => {
                        for part in parts {
                            match part.get("type").and_then(Value::as_str) {
                                Some("text")
                                    if part
                                        .get("text")
                                        .and_then(Value::as_str)
                                        .is_some_and(|text| !text.trim().is_empty()) =>
                                {
                                    tail.turns += 1;
                                }
                                Some("tool_use") => tail.tools += 1,
                                Some(_) | None => {}
                            }
                        }
                    }
                    Some(Value::String(text)) if !text.trim().is_empty() => tail.turns += 1,
                    Some(_) | None => {}
                }
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

/// Launch directory recorded near the beginning of a Claude transcript.
fn transcript_cwd(path: &Path) -> Option<String> {
    const HEAD_BYTES: u64 = 64 * 1024;
    let mut buf = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(HEAD_BYTES)
        .read_to_end(&mut buf)
        .ok()?;
    String::from_utf8_lossy(&buf).lines().find_map(|line| {
        serde_json::from_str::<Value>(line)
            .ok()?
            .get("cwd")?
            .as_str()
            .filter(|cwd| !cwd.is_empty())
            .map(str::to_string)
    })
}

fn transcript_modified(path: &Path, now: DateTime<Utc>) -> DateTime<Utc> {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .map(DateTime::<Utc>::from)
        .unwrap_or(now)
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

/// The on-disk transcript path for a given session id under `config_dir`, if
/// one exists. Used by the transcript viewer to locate a session's `.jsonl`.
pub fn transcript_path(config_dir: &Path, session_id: &str) -> Option<PathBuf> {
    transcripts_by_id(config_dir).remove(session_id)
}

/// The registry's own idea of when a session was last touched — available
/// without opening the (potentially large) transcript.
fn reg_updated(r: &RegEntry, now: DateTime<Utc>) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(r.updated_at.max(r.started_at))
        .single()
        .unwrap_or(now)
}

/// Join one registry entry to its transcript and build the snapshot.
///
/// Coarse state/model/context come from the bounded tail; structured tasks need
/// a complete replay because their creates may predate the tail. `state` is
/// derived from the tail unless the caller overrides it (a dormant session's
/// state is decided by its dead process, not by its last turn).
fn snapshot_of(
    r: RegEntry,
    transcript: &Path,
    now: DateTime<Utc>,
    force_state: Option<SessionState>,
    process_fingerprint: Option<String>,
    task_cache: &mut crate::transcript::TaskStateCache,
) -> SessionSnapshot {
    let tail = read_transcript_tail(transcript);
    let updated = reg_updated(&r, now);
    let last_activity = tail.last_ts.map_or(updated, |t| t.max(updated));
    let background = r.kind == "bg"
        && (r.status == "busy" || (now - last_activity).num_milliseconds() < ACTIVE_WINDOW_MS);
    let project = crate::project::resolve_project(Path::new(&r.cwd));
    SessionSnapshot {
        session_id: r.session_id,
        cwd: r.cwd,
        name: r.name,
        state: force_state.unwrap_or_else(|| state_of(&r.status, tail.ended, background)),
        provider: Provider::Claude,
        model: tail.model,
        // Not in the transcript; populated at launch time later.
        effort: None,
        ctx_tokens: tail.ctx_tokens,
        project_root: project.root,
        repo_root: project.repo_root,
        project_name: project.name,
        branch: project.branch,
        worktree: project.worktree,
        // Both set by the owning account via `Account::stamp` — discovery
        // reads a transcript, which knows nothing about the account above it.
        config_dir: None,
        account_email: None,
        account_id: None,
        process_fingerprint,
        pty_session_id: None,
        pty_session_generation: None,
        pty_foreground: false,
        task_state: task_cache.parse_file(Provider::Claude, transcript),
        last_activity,
        pid: r.pid,
    }
}

/// A cheap upper estimate of a session's last activity, used to rank and cap
/// dormant candidates *before* any transcript is opened.
///
/// The registry's own `updatedAt` alone is not enough: it can lag the
/// conversation, and `last_activity` is `max(tail, updatedAt)`, so ranking on
/// `updatedAt` could cut a session that is in truth more recent than one it
/// kept. The transcript's mtime moves with every turn, so the later of the two
/// tracks the real figure closely — and costs one `stat`, not a parse.
///
/// A close estimate, not a bound: the tail's own timestamp can still exceed the
/// file's mtime if the two clocks disagree, or if the transcript was restored or
/// back-dated. Ranking is then off by that skew. Reading every tail to rule it
/// out is precisely the cost this exists to avoid.
fn recency_estimate(r: &RegEntry, transcript: &Path, now: DateTime<Utc>) -> DateTime<Utc> {
    let updated = reg_updated(r, now);
    std::fs::metadata(transcript)
        .and_then(|m| m.modified())
        .map(DateTime::<Utc>::from)
        .map_or(updated, |mtime| mtime.max(updated))
}

/// Both halves of an account's sessions, classified in one pass.
pub struct SessionScan {
    /// Proven running: the registry's pid answered.
    pub live: Vec<SessionSnapshot>,
    /// Dormant but resumable, most-recent first and capped.
    pub idle: Vec<SessionSnapshot>,
}

/// Classify every registry entry of a Claude Code account **once**: real,
/// transcript-backed entries are probed for liveness a single time and land in
/// exactly one half of the [`SessionScan`]. Recent substantial transcripts with
/// no remaining registry row supplement the dormant half.
///
/// Two separate scans would not do. `live` and `idle` ask complementary
/// questions, but asked at two different moments they can both answer "yes" for
/// one session — the process only has to exit in between — and it would show up
/// as running *and* dormant at once. One probe per entry makes the split a fact
/// rather than a hope, and reads the registry and transcript index once instead
/// of twice.
///
/// The dormant half is deliberately conservative and bounded:
/// - A pid of `0` means *unknown*, not dead (the process probe reports it alive), so
///   such an entry stays live and is never claimed dormant — we don't assert
///   "resumable" where we cannot show the process is gone.
/// - A live pid is signalable only when its exact process start matches Claude's
///   registry `procStart`. Missing/mismatching identity keeps the row visible
///   with no fingerprint, so Stop/Kill fails closed without hiding the session.
/// - Only the last `max_age` counts; older conversations are not usefully
///   resumable and would only be noise.
/// - At most `limit`, most-recent first.
///
/// Cost: a heavy user has hundreds of dead entries, and reading every transcript
/// on each refresh would be real I/O. So dormant candidates are ranked and
/// capped on [`recency_estimate`] — registry time plus one `stat`. Registry-backed
/// candidates are parsed only after the final cap; transcript-only history opens
/// at most `limit * 4` recent tails to reject tiny automation fragments before
/// the same final cap. Live entries are few (a running process each), so all of
/// them are read. What this does **not** avoid:
/// [`transcripts_by_id`] still walks the whole `projects/` tree to build the id
/// index, as it must for any lookup, and `read_registry` still parses every
/// entry. The saving is on transcript *contents*, not on the directory scan.
pub(crate) fn scan_sessions_with_cache(
    config_dir: &Path,
    now: DateTime<Utc>,
    max_age: Duration,
    limit: usize,
    task_cache: &mut crate::transcript::TaskStateCache,
) -> SessionScan {
    let transcripts = transcripts_by_id(config_dir);
    let cutoff = now - max_age;

    let mut live_entries: Vec<(RegEntry, PathBuf, Option<String>)> = Vec::new();
    let mut idle_candidates: Vec<(DateTime<Utc>, RegEntry, PathBuf)> = Vec::new();
    let registry = read_registry(config_dir);
    let registry_ids: HashSet<String> = registry
        .iter()
        .map(|entry| entry.session_id.clone())
        .collect();

    for r in registry {
        if !is_real_reg(&r) {
            continue;
        }
        let Some(path) = transcripts.get(&r.session_id).cloned() else {
            // No transcript → a helper process, not a session.
            continue;
        };
        // The single probe decides both liveness and whether this exact process
        // is safely bound to Claude's registry entry.
        let process = crate::process_identity::probe_registered_process(
            r.pid,
            r.proc_start.as_deref(),
            r.started_at,
        );
        if process.alive {
            live_entries.push((r, path, process.fingerprint));
        } else if limit > 0 {
            let est = recency_estimate(&r, &path, now);
            if est >= cutoff {
                idle_candidates.push((est, r, path));
            }
        }
    }

    // Claudeplex also shows recent substantial transcript history after the
    // corresponding process-registry row has disappeared. Preserve that
    // behavior for account detail panes; these snapshots are Idle and therefore
    // never enter the live Cockpit tree. Rank cheaply by mtime first, then open
    // only a bounded recent set to reject tiny automation fragments.
    if limit > 0 {
        let mut transcript_only: Vec<(DateTime<Utc>, String, PathBuf)> = transcripts
            .iter()
            .filter(|(session_id, _)| !registry_ids.contains(*session_id))
            .filter_map(|(session_id, path)| {
                let modified = transcript_modified(path, now);
                (modified >= cutoff).then(|| (modified, session_id.clone(), path.clone()))
            })
            .collect();
        transcript_only.sort_by(|a, b| b.0.cmp(&a.0));
        transcript_only.truncate(limit.saturating_mul(4));

        for (modified, session_id, path) in transcript_only {
            let tail = read_transcript_tail(&path);
            let Some(cwd) = transcript_cwd(&path) else {
                continue;
            };
            if !tail.is_substantial()
                || cwd.contains("observer-sessions")
                || cwd.contains(".claude-mem")
            {
                continue;
            }
            idle_candidates.push((
                modified,
                RegEntry {
                    session_id,
                    cwd,
                    status: String::new(),
                    kind: "interactive".to_string(),
                    name: String::new(),
                    started_at: modified.timestamp_millis(),
                    updated_at: modified.timestamp_millis(),
                    pid: 0,
                    proc_start: None,
                },
                path,
            ));
        }
    }

    let mut live: Vec<SessionSnapshot> = live_entries
        .into_iter()
        .map(|(r, path, fingerprint)| snapshot_of(r, &path, now, None, fingerprint, task_cache))
        .collect();
    // Waiting first (they need the user), then by recency.
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

    idle_candidates.sort_by(|a, b| b.0.cmp(&a.0));
    idle_candidates.truncate(limit);
    let mut idle: Vec<SessionSnapshot> = idle_candidates
        .into_iter()
        .map(|(_, r, path)| snapshot_of(r, &path, now, Some(SessionState::Idle), None, task_cache))
        .collect();
    idle.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));

    SessionScan { live, idle }
}

pub fn scan_sessions(
    config_dir: &Path,
    now: DateTime<Utc>,
    max_age: Duration,
    limit: usize,
) -> SessionScan {
    scan_sessions_with_cache(
        config_dir,
        now,
        max_age,
        limit,
        &mut crate::transcript::TaskStateCache::default(),
    )
}

/// The live sessions of a Claude Code account: registry entries that are real,
/// PID-alive and transcript-backed, joined with their transcript tail for the
/// waiting/working classification. Callers that also want the dormant ones must
/// use [`scan_sessions`] rather than pair this with a second scan — see there.
pub fn live_sessions(config_dir: &Path, now: DateTime<Utc>) -> Vec<SessionSnapshot> {
    scan_sessions(config_dir, now, Duration::zero(), 0).live
}

/// The dormant sessions of a Claude Code account. See [`scan_sessions`], which
/// this delegates to; prefer it when both halves are wanted.
pub fn idle_sessions(
    config_dir: &Path,
    now: DateTime<Utc>,
    max_age: Duration,
    limit: usize,
) -> Vec<SessionSnapshot> {
    scan_sessions(config_dir, now, max_age, limit).idle
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
