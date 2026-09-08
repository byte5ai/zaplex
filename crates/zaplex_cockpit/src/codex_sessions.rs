//! Session discovery and local transcript history for Codex accounts.
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
//! The background discovery path never surfaces conversational text, token
//! strings, or credentials. [`load_transcript`] is the explicit viewer path;
//! it projects conversation text while excluding tool payloads, encrypted
//! content, session metadata, and credential stores. The structured titles
//! deliberately emitted by `update_plan` remain the sole background
//! task-progress projection.

use std::collections::HashMap;
use std::fmt;
use std::fs::{File, Metadata, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use walkdir::WalkDir;

use crate::transcript::{LoadedTranscript, ToolCall, TranscriptTurn, TurnRole};
use crate::types::{Provider, SessionSnapshot, SessionState, TaskState};

/// A rollout whose last activity is older than this is not treated as live
/// (Codex has no pid to confirm the process, so discovery is scoped to recent
/// activity). Matches the spirit of the Claude background-job active window.
const CODEX_LIVE_WINDOW: Duration = Duration::minutes(15);
const ROLLOUT_CACHE_LIMIT: usize = 512;
const TRANSCRIPT_HEADER_MAX_BYTES: u64 = 64 * 1024;
const TRANSCRIPT_HEADER_SCAN_MAX_BYTES: u64 = 8 * 1024 * 1024;
const TRANSCRIPT_HISTORY_MAX_FILES: usize = 32_768;
pub const TRANSCRIPT_MAX_LINES: usize = 20_000;

/// Maximum Codex rollout size accepted by [`load_transcript`].
///
/// Rollouts are append-only and can become large. The viewer must fail
/// explicitly instead of allocating in proportion to an untrusted file.
pub const TRANSCRIPT_MAX_BYTES: u64 = 64 * 1024 * 1024;

/// A failure while resolving or parsing a local Codex rollout transcript.
#[derive(Debug)]
pub enum TranscriptError {
    InvalidSessionId,
    HistoryLimitExceeded { max_files: usize },
    TranscriptLookupLimitExceeded { max_bytes: u64 },
    AmbiguousSessionId { session_id: String },
    TranscriptTooLarge { max_bytes: u64 },
    MalformedTranscript,
    UnsupportedTranscript,
    Io(std::io::Error),
    Walk(walkdir::Error),
}

impl fmt::Display for TranscriptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSessionId => formatter.write_str("invalid Codex session id"),
            Self::HistoryLimitExceeded { max_files } => {
                write!(
                    formatter,
                    "Codex history exceeds the {max_files}-file limit"
                )
            }
            Self::TranscriptLookupLimitExceeded { max_bytes } => {
                write!(
                    formatter,
                    "Codex transcript lookup exceeds the {max_bytes}-byte scan limit"
                )
            }
            Self::AmbiguousSessionId { session_id } => {
                write!(
                    formatter,
                    "multiple Codex rollouts match session {session_id}"
                )
            }
            Self::TranscriptTooLarge { max_bytes } => {
                write!(
                    formatter,
                    "Codex transcript exceeds the {max_bytes}-byte limit"
                )
            }
            Self::MalformedTranscript => formatter.write_str("malformed Codex transcript"),
            Self::UnsupportedTranscript => {
                formatter.write_str("unsupported Codex transcript format")
            }
            Self::Io(error) => write!(formatter, "Codex transcript I/O failed: {error}"),
            Self::Walk(error) => write!(formatter, "Codex history traversal failed: {error}"),
        }
    }
}

impl std::error::Error for TranscriptError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Walk(error) => Some(error),
            Self::InvalidSessionId
            | Self::HistoryLimitExceeded { .. }
            | Self::TranscriptLookupLimitExceeded { .. }
            | Self::AmbiguousSessionId { .. }
            | Self::TranscriptTooLarge { .. }
            | Self::MalformedTranscript
            | Self::UnsupportedTranscript => None,
        }
    }
}

impl From<std::io::Error> for TranscriptError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<walkdir::Error> for TranscriptError {
    fn from(error: walkdir::Error) -> Self {
        Self::Walk(error)
    }
}

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
    /// The last turn-level event handed control back to the user.
    ended: bool,
    /// A real turn was observed (a lifecycle or usage line) —
    /// guards against listing an empty/aborted rollout as a session.
    has_turn: bool,
    task_state: Option<TaskState>,
}

#[derive(Clone, Debug)]
struct CachedRollout {
    fingerprint: crate::transcript::FileFingerprint,
    info: RolloutInfo,
    last_used: u64,
}

/// Bounded cache for complete Codex rollout parsing.
///
/// Codex stores all session signals and structured task state in the same
/// append-only rollout. An unchanged `(mtime, size)` pair can therefore reuse
/// the complete distilled result instead of reopening the transcript on every
/// reconcile tick.
#[derive(Clone, Debug, Default)]
pub(crate) struct RolloutCache {
    entries: HashMap<PathBuf, CachedRollout>,
    clock: u64,
    #[cfg(test)]
    fail_next_parse: bool,
}

impl RolloutCache {
    fn parse_file(&mut self, path: &Path) -> Result<RolloutInfo, ()> {
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_parse) {
            self.entries.remove(path);
            return Err(());
        }
        let fingerprint = match crate::transcript::file_fingerprint(path) {
            Ok(fingerprint) => fingerprint,
            Err(_) => {
                self.entries.remove(path);
                return Err(());
            }
        };
        self.clock = self.clock.wrapping_add(1);
        if let Some(cached) = self.entries.get_mut(path) {
            if cached.fingerprint == fingerprint {
                cached.last_used = self.clock;
                return Ok(cached.info.clone());
            }
        }

        let info = match std::fs::read_to_string(path) {
            Ok(content) => parse_rollout_content(path, &content),
            Err(_) => {
                self.entries.remove(path);
                return Err(());
            }
        };
        self.entries.insert(
            path.to_path_buf(),
            CachedRollout {
                fingerprint,
                info: info.clone(),
                last_used: self.clock,
            },
        );
        self.evict_lru();
        Ok(info)
    }

    fn evict_lru(&mut self) {
        while self.entries.len() > ROLLOUT_CACHE_LIMIT {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(path, _)| path.clone())
            else {
                return;
            };
            self.entries.remove(&oldest);
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_next_parse(&mut self) {
        self.fail_next_parse = true;
    }
}

/// Session id derived from a rollout's file name — the fallback for a rollout
/// with no `session_meta` line to name itself.
///
/// Shared so that discovery and usage attribution can never disagree about what
/// a rollout's id is: they must produce the same string, or spend would be
/// stamped with an id no session row carries.
///
/// The name is `rollout-<timestamp>-<uuid>`, and **both halves contain dashes**
/// (`rollout-2026-06-29T14-34-13-019f135f-7fcc-7d93-8a28-4835d98f8f0a`), so the
/// id is the last five dash-separated groups, not the last one. Taking only the
/// last group yields a fragment of the uuid — enough to look plausible in a row,
/// but `codex resume <fragment>` would not find the session.
pub(crate) fn session_id_from_path(path: &Path) -> String {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return String::new();
    };
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() < UUID_GROUPS {
        // Not the expected shape — hand back the stem rather than a guess.
        return stem.to_string();
    }
    parts[parts.len() - UUID_GROUPS..].join("-")
}

/// A uuid is five dash-separated groups (`8-4-4-4-12`).
const UUID_GROUPS: usize = 5;

fn valid_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id.len() <= 128
        && session_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn uuid_session_id(session_id: &str) -> bool {
    let expected_lengths = [8usize, 4, 4, 4, 12];
    let mut groups = session_id.split('-');
    expected_lengths.iter().all(|expected_length| {
        groups.next().is_some_and(|group| {
            group.len() == *expected_length && group.bytes().all(|b| b.is_ascii_hexdigit())
        })
    }) && groups.next().is_none()
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TranscriptFileIdentity {
    device: u64,
    inode: u64,
}

#[cfg(not(unix))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TranscriptFileIdentity {
    length: u64,
    modified: std::time::SystemTime,
}

impl TranscriptFileIdentity {
    #[cfg(unix)]
    fn from_metadata(metadata: &Metadata) -> Result<Self, TranscriptError> {
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    #[cfg(not(unix))]
    fn from_metadata(metadata: &Metadata) -> Result<Self, TranscriptError> {
        Ok(Self {
            length: metadata.len(),
            modified: metadata.modified()?,
        })
    }
}

#[derive(Clone, Debug)]
struct TranscriptCandidate {
    path: PathBuf,
    identity: TranscriptFileIdentity,
}

struct OpenedTranscript {
    file: File,
    metadata: Metadata,
    bytes: Vec<u8>,
}

struct ResolvedTranscript {
    candidate: TranscriptCandidate,
    opened: Option<OpenedTranscript>,
    provider_root: PathBuf,
}

fn transcript_history_candidates(
    codex_home: &Path,
) -> Result<Vec<TranscriptCandidate>, TranscriptError> {
    let roots = [
        (codex_home.join("sessions"), 4usize),
        (codex_home.join("archived_sessions"), 1usize),
    ];
    let mut candidates = Vec::new();
    for (root, max_depth) in roots {
        if !root.try_exists()? {
            continue;
        }
        for entry in WalkDir::new(root).max_depth(max_depth).follow_links(false) {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let name = entry.file_name().to_str().unwrap_or("");
            if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
                continue;
            }
            if candidates.len() == TRANSCRIPT_HISTORY_MAX_FILES {
                return Err(TranscriptError::HistoryLimitExceeded {
                    max_files: TRANSCRIPT_HISTORY_MAX_FILES,
                });
            }
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                continue;
            }
            candidates.push(TranscriptCandidate {
                path: entry.into_path(),
                identity: TranscriptFileIdentity::from_metadata(&metadata)?,
            });
        }
    }
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(candidates)
}

fn open_transcript(
    canonical_provider_root: &Path,
    candidate: &TranscriptCandidate,
) -> Result<OpenedTranscript, TranscriptError> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options.open(&candidate.path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file()
        || TranscriptFileIdentity::from_metadata(&metadata)? != candidate.identity
    {
        return Err(TranscriptError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Codex transcript changed after resolution",
        )));
    }
    let canonical_path = std::fs::canonicalize(&candidate.path)?;
    if !canonical_path.starts_with(canonical_provider_root) {
        return Err(TranscriptError::MalformedTranscript);
    }
    if TranscriptFileIdentity::from_metadata(&canonical_path.metadata()?)? != candidate.identity {
        return Err(TranscriptError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Codex transcript root changed after open",
        )));
    }
    Ok(OpenedTranscript {
        file,
        metadata,
        bytes: Vec::new(),
    })
}

fn session_id_from_header(
    mut opened: OpenedTranscript,
    max_bytes: u64,
) -> Result<(Option<String>, u64, OpenedTranscript), TranscriptError> {
    let read_limit = opened.metadata.len().min(TRANSCRIPT_HEADER_MAX_BYTES);
    if read_limit > max_bytes {
        return Err(TranscriptError::TranscriptLookupLimitExceeded {
            max_bytes: TRANSCRIPT_HEADER_SCAN_MAX_BYTES,
        });
    }
    (&mut opened.file)
        .take(read_limit)
        .read_to_end(&mut opened.bytes)?;
    let bytes_read = opened.bytes.len() as u64;
    let Ok(header) = std::str::from_utf8(&opened.bytes) else {
        return Ok((None, bytes_read, opened));
    };
    let mut session_id = None;
    for line in header.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(object) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if object.get("type").and_then(Value::as_str) != Some("session_meta") {
            continue;
        }
        session_id = find(&object, "id")
            .or_else(|| find(&object, "session_id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        break;
    }
    Ok((session_id, bytes_read, opened))
}

fn unique_transcript_match(
    session_id: &str,
    mut matches: Vec<TranscriptCandidate>,
) -> Result<Option<TranscriptCandidate>, TranscriptError> {
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err(TranscriptError::AmbiguousSessionId {
            session_id: session_id.to_string(),
        }),
    }
}

fn resolve_transcript(
    codex_home: &Path,
    session_id: &str,
) -> Result<Option<ResolvedTranscript>, TranscriptError> {
    if !valid_session_id(session_id) {
        return Err(TranscriptError::InvalidSessionId);
    }

    let candidates = transcript_history_candidates(codex_home)?;
    if candidates.is_empty() {
        return Ok(None);
    }
    let provider_root = std::fs::canonicalize(codex_home)?;
    let file_name_matches: Vec<TranscriptCandidate> = candidates
        .iter()
        .filter(|candidate| {
            let candidate_session_id = session_id_from_path(&candidate.path);
            uuid_session_id(&candidate_session_id) && candidate_session_id == session_id
        })
        .cloned()
        .collect();
    if !file_name_matches.is_empty() {
        return unique_transcript_match(session_id, file_name_matches).map(|candidate| {
            candidate.map(|candidate| ResolvedTranscript {
                candidate,
                opened: None,
                provider_root: provider_root.clone(),
            })
        });
    }

    let mut header_match = None;
    let mut header_bytes_read = 0u64;
    for candidate in candidates {
        let remaining_bytes = TRANSCRIPT_HEADER_SCAN_MAX_BYTES.saturating_sub(header_bytes_read);
        if remaining_bytes == 0 {
            return Err(TranscriptError::TranscriptLookupLimitExceeded {
                max_bytes: TRANSCRIPT_HEADER_SCAN_MAX_BYTES,
            });
        }
        let opened = open_transcript(&provider_root, &candidate)?;
        let (header_session_id, bytes_read, opened) =
            session_id_from_header(opened, remaining_bytes)?;
        header_bytes_read = header_bytes_read.saturating_add(bytes_read);
        if header_session_id.as_deref() != Some(session_id) {
            continue;
        }
        if header_match.is_some() {
            return Err(TranscriptError::AmbiguousSessionId {
                session_id: session_id.to_string(),
            });
        }
        header_match = Some(ResolvedTranscript {
            candidate,
            opened: Some(opened),
            provider_root: provider_root.clone(),
        });
    }
    Ok(header_match)
}

/// Resolve a stable Codex session id in the two local rollout layouts.
///
/// Current sessions live below `sessions/YYYY/MM/DD`, while archived sessions
/// are moved into `archived_sessions`. Symlinks are not followed, the number of
/// rollout candidates is capped, and the caller-supplied id is never used as a
/// path component. The file name carries the stable id in current Codex
/// versions; a bounded `session_meta` probe supports older/nonstandard names.
pub fn transcript_path(
    codex_home: &Path,
    session_id: &str,
) -> Result<Option<PathBuf>, TranscriptError> {
    resolve_transcript(codex_home, session_id)
        .map(|resolved| resolved.map(|resolved| resolved.candidate.path))
}

fn read_transcript(mut opened: OpenedTranscript) -> Result<String, TranscriptError> {
    if opened.metadata.len() > TRANSCRIPT_MAX_BYTES {
        return Err(TranscriptError::TranscriptTooLarge {
            max_bytes: TRANSCRIPT_MAX_BYTES,
        });
    }

    let remaining_bytes = (TRANSCRIPT_MAX_BYTES + 1).saturating_sub(opened.bytes.len() as u64);
    (&mut opened.file)
        .take(remaining_bytes)
        .read_to_end(&mut opened.bytes)?;
    if opened.bytes.len() as u64 > TRANSCRIPT_MAX_BYTES {
        return Err(TranscriptError::TranscriptTooLarge {
            max_bytes: TRANSCRIPT_MAX_BYTES,
        });
    }
    let newline_count = opened.bytes.iter().filter(|byte| **byte == b'\n').count();
    let line_count = newline_count
        + usize::from(!opened.bytes.is_empty() && opened.bytes.last() != Some(&b'\n'));
    if line_count > TRANSCRIPT_MAX_LINES {
        return Err(TranscriptError::TranscriptTooLarge {
            max_bytes: TRANSCRIPT_MAX_BYTES,
        });
    }
    String::from_utf8(opened.bytes).map_err(|_| TranscriptError::MalformedTranscript)
}

fn read_resolved_transcript(resolved: ResolvedTranscript) -> Result<String, TranscriptError> {
    let opened = match resolved.opened {
        Some(opened) => opened,
        None => open_transcript(&resolved.provider_root, &resolved.candidate)?,
    };
    read_transcript(opened)
}

fn append_text(target: &mut String, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    if !target.is_empty() {
        target.push_str("\n\n");
    }
    target.push_str(text);
}

fn content_text(content: Option<&Value>, item_type: &str) -> String {
    let mut text = String::new();
    let Some(content) = content.and_then(Value::as_array) else {
        return text;
    };
    for item in content {
        if item.get("type").and_then(Value::as_str) != Some(item_type) {
            continue;
        }
        if let Some(part) = item.get("text").and_then(Value::as_str) {
            append_text(&mut text, part);
        }
    }
    text
}

fn rollout_timestamp(object: &Value) -> Option<DateTime<Utc>> {
    object
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

#[derive(Default)]
struct TranscriptAccumulator {
    turns: Vec<TranscriptTurn>,
    model: Option<String>,
    thinking: String,
    tools: Vec<ToolCall>,
    pending_timestamp: Option<DateTime<Utc>>,
    saw_message: bool,
}

impl TranscriptAccumulator {
    fn set_model(&mut self, model: Option<&str>) {
        if let Some(model) = model.filter(|model| !model.trim().is_empty()) {
            self.model = Some(model.to_string());
        }
    }

    fn add_thinking(&mut self, thinking: &str, timestamp: Option<DateTime<Utc>>) {
        append_text(&mut self.thinking, thinking);
        if self.pending_timestamp.is_none() {
            self.pending_timestamp = timestamp;
        }
    }

    fn add_tool(&mut self, name: &str, timestamp: Option<DateTime<Utc>>) {
        if name.trim().is_empty() {
            return;
        }
        self.tools.push(ToolCall {
            name: name.to_string(),
        });
        if self.pending_timestamp.is_none() {
            self.pending_timestamp = timestamp;
        }
    }

    fn push_user(&mut self, text: String, timestamp: Option<DateTime<Utc>>) {
        self.saw_message = true;
        self.flush_pending();
        if text.trim().is_empty() {
            return;
        }
        self.turns.push(TranscriptTurn {
            role: TurnRole::User,
            text,
            thinking: String::new(),
            tools: Vec::new(),
            model: None,
            usage: None,
            timestamp,
        });
    }

    fn push_assistant(&mut self, text: String, timestamp: Option<DateTime<Utc>>) {
        self.saw_message = true;
        if text.trim().is_empty() && self.thinking.is_empty() && self.tools.is_empty() {
            return;
        }
        self.turns.push(TranscriptTurn {
            role: TurnRole::Assistant,
            text,
            thinking: std::mem::take(&mut self.thinking),
            tools: std::mem::take(&mut self.tools),
            model: self.model.clone(),
            usage: None,
            timestamp: timestamp.or(self.pending_timestamp.take()),
        });
        self.pending_timestamp = None;
    }

    fn flush_pending(&mut self) {
        if self.thinking.is_empty() && self.tools.is_empty() {
            self.pending_timestamp = None;
            return;
        }
        self.turns.push(TranscriptTurn {
            role: TurnRole::Assistant,
            text: String::new(),
            thinking: std::mem::take(&mut self.thinking),
            tools: std::mem::take(&mut self.tools),
            model: self.model.clone(),
            usage: None,
            timestamp: self.pending_timestamp.take(),
        });
    }

    fn finish(mut self) -> Self {
        self.flush_pending();
        self
    }
}

/// Project an already bounded Codex rollout into provider-neutral turns.
///
/// This parser performs no filesystem access. Callers that accept untrusted
/// input must enforce their byte and line limits before invoking it.
pub fn parse_transcript_content(content: &str) -> Result<Vec<TranscriptTurn>, TranscriptError> {
    let mut canonical = TranscriptAccumulator::default();
    let mut fallback = TranscriptAccumulator::default();
    let mut valid_objects = 0usize;
    let mut malformed_lines = 0usize;

    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(object) = serde_json::from_str::<Value>(line) else {
            malformed_lines += 1;
            continue;
        };
        valid_objects += 1;
        let timestamp = rollout_timestamp(&object);
        match object.get("type").and_then(Value::as_str) {
            Some("turn_context") => {
                let model = object
                    .get("payload")
                    .and_then(|payload| payload.get("model"))
                    .and_then(Value::as_str);
                canonical.set_model(model);
                fallback.set_model(model);
            }
            Some("response_item") => {
                let Some(payload) = object.get("payload") else {
                    continue;
                };
                match payload.get("type").and_then(Value::as_str) {
                    Some("message") => match payload.get("role").and_then(Value::as_str) {
                        Some("user") => canonical.push_user(
                            content_text(payload.get("content"), "input_text"),
                            timestamp,
                        ),
                        Some("assistant") => canonical.push_assistant(
                            content_text(payload.get("content"), "output_text"),
                            timestamp,
                        ),
                        Some("developer") | Some(_) | None => {}
                    },
                    Some("reasoning") => {
                        let thinking = content_text(payload.get("summary"), "summary_text");
                        canonical.add_thinking(&thinking, timestamp);
                    }
                    Some("function_call") | Some("custom_tool_call") => {
                        if let Some(name) = payload.get("name").and_then(Value::as_str) {
                            canonical.add_tool(name, timestamp);
                        }
                    }
                    Some("function_call_output")
                    | Some("custom_tool_call_output")
                    | Some("agent_message")
                    | Some(_)
                    | None => {}
                }
            }
            Some("event_msg") => {
                let Some(payload) = object.get("payload") else {
                    continue;
                };
                match payload.get("type").and_then(Value::as_str) {
                    Some("user_message") => fallback.push_user(
                        payload
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        timestamp,
                    ),
                    Some("agent_message") => fallback.push_assistant(
                        payload
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        timestamp,
                    ),
                    Some("agent_reasoning") => fallback.add_thinking(
                        payload.get("text").and_then(Value::as_str).unwrap_or(""),
                        timestamp,
                    ),
                    Some(_) | None => {}
                }
            }
            Some(_) | None => {}
        }
    }

    if valid_objects == 0 {
        return Err(TranscriptError::MalformedTranscript);
    }
    let canonical = canonical.finish();
    let fallback = fallback.finish();
    if canonical.saw_message {
        return Ok(canonical.turns);
    }
    if fallback.saw_message {
        return Ok(fallback.turns);
    }
    if malformed_lines > 0 {
        return Err(TranscriptError::MalformedTranscript);
    }
    Err(TranscriptError::UnsupportedTranscript)
}

/// Load one local Codex rollout as provider-neutral transcript turns.
///
/// Conversation text, reasoning summaries, and tool names are projected.
/// Tool arguments/results, session metadata, encrypted payloads, and credential
/// stores are deliberately excluded. `Ok(None)` means the stable session id is
/// not present in local history; unsupported or malformed files return an
/// explicit error.
pub fn load_transcript(
    codex_home: &Path,
    session_id: &str,
) -> Result<Option<Vec<TranscriptTurn>>, TranscriptError> {
    load_transcript_with_revision(codex_home, session_id)
        .map(|transcript| transcript.map(|transcript| transcript.turns))
}

/// Load one local Codex rollout and return the revision of the exact bounded
/// source that was parsed. The revision is content-derived, so an mtime-only
/// change does not rewrite an open generated document.
pub fn load_transcript_with_revision(
    codex_home: &Path,
    session_id: &str,
) -> Result<Option<LoadedTranscript>, TranscriptError> {
    let Some(resolved) = resolve_transcript(codex_home, session_id)? else {
        return Ok(None);
    };
    let content = read_resolved_transcript(resolved)?;
    let source_revision = hex_revision(&content);
    parse_transcript_content(&content).map(|turns| {
        Some(LoadedTranscript {
            turns,
            source_revision,
        })
    })
}

fn hex_revision(content: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(b"zaplex-codex-transcript-revision-v1\0");
    digest.update(content.as_bytes());
    let digest = digest.finalize();
    format!("{digest:x}")
}

/// Distil one rollout transcript's live-session signals. Best-effort and
/// defensive: each line is an independent JSON object, malformed lines are
/// skipped, and both the wrapped (`{type,payload}`) and flat shapes are handled.
fn parse_rollout_content(path: &Path, content: &str) -> RolloutInfo {
    let mut info = RolloutInfo::default();
    info.session_id = session_id_from_path(path);
    info.task_state = crate::transcript::parse_task_state(Provider::Codex, content);
    let mut active_turn_id: Option<String> = None;

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
                        active_turn_id = v
                            .get("payload")
                            .and_then(|payload| payload.get("turn_id"))
                            .and_then(Value::as_str)
                            .filter(|turn_id| !turn_id.is_empty())
                            .map(str::to_string);
                    }
                    Some("task_complete" | "turn_aborted") => {
                        info.ended = true;
                        info.has_turn = true;
                        active_turn_id = None;
                    }
                    Some("error") => {
                        let error_turn_id = v
                            .get("payload")
                            .and_then(|payload| payload.get("turn_id"))
                            .and_then(Value::as_str);
                        if active_turn_id.as_deref() == error_turn_id && active_turn_id.is_some() {
                            info.ended = true;
                            info.has_turn = true;
                            active_turn_id = None;
                        }
                    }
                    Some(_) | None => {}
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

/// Rollout candidates plus the completeness of their discovery walk.
struct RolloutFileScan {
    files: Vec<(PathBuf, DateTime<Utc>)>,
    io_error: bool,
}

/// Every rollout transcript under `<codex_home>/sessions` with its mtime.
/// A missing root is a normal empty history; any inaccessible descendant or
/// eligible file marks the result incomplete while preserving reachable rows.
fn rollout_files(codex_home: &Path) -> RolloutFileScan {
    let root = codex_home.join("sessions");
    match std::fs::symlink_metadata(&root) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => {
            return RolloutFileScan {
                files: Vec::new(),
                io_error: true,
            };
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return RolloutFileScan {
                files: Vec::new(),
                io_error: false,
            };
        }
        Err(_) => {
            return RolloutFileScan {
                files: Vec::new(),
                io_error: true,
            };
        }
    }

    let mut files = Vec::new();
    let mut io_error = false;
    for entry in WalkDir::new(root) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                io_error = true;
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_str().unwrap_or("");
        if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => {
                io_error = true;
                continue;
            }
        };
        let mtime = match metadata.modified() {
            Ok(mtime) => DateTime::<Utc>::from(mtime),
            Err(_) => {
                io_error = true;
                continue;
            }
        };
        files.push((entry.into_path(), mtime));
    }
    RolloutFileScan { files, io_error }
}

/// Parse one rollout into a snapshot. `None` when it holds no real turn — an
/// empty or aborted rollout is not a session. `force_state` overrides the
/// turn-derived state (a dormant rollout is Idle regardless of how it ended).
fn snapshot_of(
    path: &Path,
    mtime: DateTime<Utc>,
    now: DateTime<Utc>,
    force_state: Option<SessionState>,
    cache: &mut RolloutCache,
) -> Result<Option<SessionSnapshot>, ()> {
    let info = cache.parse_file(path)?;
    if !info.has_turn {
        return Ok(None);
    }
    let project = crate::project::resolve_project(Path::new(&info.cwd));
    Ok(Some(SessionSnapshot {
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
        repo_root: project.repo_root,
        project_name: project.name,
        branch: project.branch,
        worktree: project.worktree,
        // Both set by the owning account via `Account::stamp` — discovery
        // reads a transcript, which knows nothing about the account above it.
        config_dir: None,
        account_email: None,
        account_id: None,
        process_fingerprint: None,
        pty_session_id: None,
        pty_session_generation: None,
        pty_foreground: false,
        task_state: info.task_state,
        last_activity: info.last_ts.or(Some(mtime)).unwrap_or(now),
        // Codex records no pid — guardrail signalling can't target it.
        pid: 0,
    }))
}

/// Both halves of a `<codex_home>`'s sessions, classified in one walk.
pub struct SessionScan {
    /// Touched inside [`CODEX_LIVE_WINDOW`] — as close to "running" as a
    /// pid-less provider gets.
    pub live: Vec<SessionSnapshot>,
    /// Dormant but resumable, most-recent first and capped.
    pub idle: Vec<SessionSnapshot>,
    /// At least one eligible rollout or subtree could not be inspected.
    pub io_error: bool,
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
pub(crate) fn scan_sessions_with_cache(
    codex_home: &Path,
    now: DateTime<Utc>,
    max_age: Duration,
    limit: usize,
    cache: &mut RolloutCache,
) -> SessionScan {
    let live_cutoff = now - CODEX_LIVE_WINDOW;
    let age_cutoff = now - max_age;
    let rollout_scan = rollout_files(codex_home);
    let mut io_error = rollout_scan.io_error;

    // Cheap split. `fresh` is bounded by how much was touched in the last few
    // minutes; `dormant` is the open-ended history, so it gets capped here.
    let mut fresh: Vec<(PathBuf, DateTime<Utc>)> = Vec::new();
    let mut dormant: Vec<(PathBuf, DateTime<Utc>)> = Vec::new();
    for (path, mtime) in rollout_scan.files {
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
        let mut s = match snapshot_of(&path, mtime, now, None, cache) {
            Ok(Some(snapshot)) => snapshot,
            Ok(None) => continue,
            Err(()) => {
                io_error = true;
                continue;
            }
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

    SessionScan {
        live,
        idle,
        io_error,
    }
}

pub fn scan_sessions(
    codex_home: &Path,
    now: DateTime<Utc>,
    max_age: Duration,
    limit: usize,
) -> SessionScan {
    scan_sessions_with_cache(
        codex_home,
        now,
        max_age,
        limit,
        &mut RolloutCache::default(),
    )
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
