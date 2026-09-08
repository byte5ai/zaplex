//! Exact, fail-closed cleanup for orphaned Claude process-registry entries.
//!
//! A missing process is not enough on its own: the transcript must remain
//! independently addressable, the registry entry must be unique, and its
//! content revision must be unchanged when cleanup executes. Missing or
//! unparseable identity stays untouched, while a previous-boot registration or
//! verified pid reuse can be removed without acting on the occupying process.

use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest as _, Sha256};

const MAX_REGISTRY_ENTRIES: usize = 4_096;
const MAX_REGISTRY_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeStaleRegistryCandidate {
    pub config_dir: PathBuf,
    pub session_id: String,
    pub revision: String,
    pub pid: u32,
    pub registry_proc_start: Option<String>,
    pub registry_started_at_millis: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaudeRegistryCleanupOutcome {
    Applied,
    AlreadyApplied,
}

#[derive(Debug)]
pub enum ClaudeRegistryLifecycleError {
    InvalidSessionId,
    AmbiguousRegistryEntry,
    RegistryLimitExceeded,
    UnsafeRegistryEntry,
    MissingTranscript,
    ProcessIdentityUnverifiable,
    RegistryChanged,
    Io(std::io::Error),
    Json(serde_json::Error),
    Transcript(crate::sessions::TranscriptError),
}

impl std::fmt::Display for ClaudeRegistryLifecycleError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSessionId => formatter.write_str("invalid Claude session id"),
            Self::AmbiguousRegistryEntry => {
                formatter.write_str("multiple Claude registry entries match the session")
            }
            Self::RegistryLimitExceeded => formatter.write_str("Claude registry exceeds its limit"),
            Self::UnsafeRegistryEntry => formatter.write_str("unsafe Claude registry entry"),
            Self::MissingTranscript => {
                formatter.write_str("the Claude transcript is not independently addressable")
            }
            Self::ProcessIdentityUnverifiable => {
                formatter.write_str("the Claude registry process may still be live")
            }
            Self::RegistryChanged => formatter.write_str("the Claude registry entry changed"),
            Self::Io(error) => write!(formatter, "Claude registry I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "Claude registry JSON failed: {error}"),
            Self::Transcript(error) => write!(formatter, "Claude transcript failed: {error}"),
        }
    }
}

impl std::error::Error for ClaudeRegistryLifecycleError {}

impl From<std::io::Error> for ClaudeRegistryLifecycleError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ClaudeRegistryLifecycleError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<crate::sessions::TranscriptError> for ClaudeRegistryLifecycleError {
    fn from(error: crate::sessions::TranscriptError) -> Self {
        Self::Transcript(error)
    }
}

#[derive(Debug)]
struct RegistryEntry {
    path: PathBuf,
    raw: Vec<u8>,
    pid: u32,
    proc_start: Option<String>,
    started_at: i64,
}

fn valid_session_id(session_id: &str) -> bool {
    !session_id.is_empty()
        && session_id.len() <= 256
        && !session_id.chars().any(char::is_control)
        && !session_id.contains('/')
        && !session_id.contains('\\')
}

fn registry_revision(raw: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"zaplex-claude-registry-entry-v1\0");
    digest.update(raw);
    let digest = digest.finalize();
    format!("{digest:x}")
}

fn read_entry(path: &Path) -> Result<Option<(Value, Vec<u8>)>, ClaudeRegistryLifecycleError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_REGISTRY_BYTES
    {
        return Err(ClaudeRegistryLifecycleError::UnsafeRegistryEntry);
    }
    let raw = std::fs::read(path)?;
    if raw.len() as u64 > MAX_REGISTRY_BYTES {
        return Err(ClaudeRegistryLifecycleError::UnsafeRegistryEntry);
    }
    let value = match serde_json::from_slice(&raw) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    Ok(Some((value, raw)))
}

fn matching_entry(
    config_dir: &Path,
    session_id: &str,
) -> Result<Option<RegistryEntry>, ClaudeRegistryLifecycleError> {
    if !valid_session_id(session_id) {
        return Err(ClaudeRegistryLifecycleError::InvalidSessionId);
    }
    let directory = config_dir.join("sessions");
    let entries = match std::fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let canonical_directory = std::fs::canonicalize(&directory)?;
    let mut matched = None;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_REGISTRY_ENTRIES {
            return Err(ClaudeRegistryLifecycleError::RegistryLimitExceeded);
        }
        let path = entry?.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let Some((value, raw)) = read_entry(&path)? else {
            continue;
        };
        if value.get("sessionId").and_then(Value::as_str) != Some(session_id) {
            continue;
        }
        let canonical_path = std::fs::canonicalize(&path)?;
        if !canonical_path.starts_with(&canonical_directory) {
            return Err(ClaudeRegistryLifecycleError::UnsafeRegistryEntry);
        }
        if matched.is_some() {
            return Err(ClaudeRegistryLifecycleError::AmbiguousRegistryEntry);
        }
        let pid = value
            .get("pid")
            .and_then(Value::as_u64)
            .and_then(|pid| u32::try_from(pid).ok())
            .filter(|pid| *pid > 0)
            .ok_or(ClaudeRegistryLifecycleError::UnsafeRegistryEntry)?;
        let started_at = value
            .get("startedAt")
            .and_then(Value::as_i64)
            .filter(|started_at| *started_at > 0)
            .ok_or(ClaudeRegistryLifecycleError::UnsafeRegistryEntry)?;
        matched = Some(RegistryEntry {
            path,
            raw,
            pid,
            proc_start: value
                .get("procStart")
                .and_then(Value::as_str)
                .map(str::to_string),
            started_at,
        });
    }
    Ok(matched)
}

/// Return a cleanup candidate only when the provider process is proven absent
/// and the conversation transcript remains independently loadable.
pub fn claude_stale_registry_candidate(
    config_dir: &Path,
    session_id: &str,
) -> Result<Option<ClaudeStaleRegistryCandidate>, ClaudeRegistryLifecycleError> {
    let Some(entry) = matching_entry(config_dir, session_id)? else {
        return Ok(None);
    };
    let process =
        crate::probe_registered_process(entry.pid, entry.proc_start.as_deref(), entry.started_at);
    if !process.presence.allows_registry_cleanup() {
        return Ok(None);
    }
    if crate::sessions::load_transcript_with_revision(config_dir, session_id)?.is_none() {
        return Err(ClaudeRegistryLifecycleError::MissingTranscript);
    }
    Ok(Some(ClaudeStaleRegistryCandidate {
        config_dir: config_dir.to_path_buf(),
        session_id: session_id.to_string(),
        revision: registry_revision(&entry.raw),
        pid: entry.pid,
        registry_proc_start: entry.proc_start,
        registry_started_at_millis: entry.started_at,
    }))
}

/// Revalidate and remove only the exact unchanged orphaned registry entry.
/// The transcript is never deleted.
pub fn cleanup_claude_stale_registry_entry(
    candidate: &ClaudeStaleRegistryCandidate,
) -> Result<ClaudeRegistryCleanupOutcome, ClaudeRegistryLifecycleError> {
    let Some(entry) = matching_entry(&candidate.config_dir, &candidate.session_id)? else {
        return Ok(ClaudeRegistryCleanupOutcome::AlreadyApplied);
    };
    if registry_revision(&entry.raw) != candidate.revision
        || entry.pid != candidate.pid
        || entry.proc_start != candidate.registry_proc_start
        || entry.started_at != candidate.registry_started_at_millis
    {
        return Err(ClaudeRegistryLifecycleError::RegistryChanged);
    }
    let process =
        crate::probe_registered_process(entry.pid, entry.proc_start.as_deref(), entry.started_at);
    if !process.presence.allows_registry_cleanup() {
        return Err(ClaudeRegistryLifecycleError::ProcessIdentityUnverifiable);
    }
    if crate::sessions::load_transcript_with_revision(&candidate.config_dir, &candidate.session_id)?
        .is_none()
    {
        return Err(ClaudeRegistryLifecycleError::MissingTranscript);
    }
    std::fs::remove_file(entry.path)?;
    Ok(ClaudeRegistryCleanupOutcome::Applied)
}

#[cfg(test)]
#[path = "claude_registry_lifecycle_tests.rs"]
mod tests;
