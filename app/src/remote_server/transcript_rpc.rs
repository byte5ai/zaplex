//! Bounded, read-only agent transcript snapshots for remote Cockpit clients.
//!
//! Requests carry an opaque daemon account id and stable provider session id,
//! never a filesystem path. The daemon resolves the account against its fresh
//! local inventory, reads only provider transcript history below that account,
//! and returns the shared display projection. Raw JSONL, tool payloads,
//! encrypted content and credential stores never cross the protocol boundary.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};

use prost::Message as _;
use sha2::{Digest as _, Sha256};
use zaplex_cockpit::{ToolCall, TranscriptTurn, TurnRole};

use super::agent_account::{prepare_launch_environment_from_routes, AccountRoutes};
use super::proto::{
    AgentLaunchRoute, AgentTranscriptResponse, AgentTranscriptStatus, AgentTranscriptTool,
    AgentTranscriptTurn, ReadAgentTranscript,
};

pub(crate) const TRANSCRIPT_SCHEMA_VERSION: u32 = 1;
pub(crate) const MAX_TRANSCRIPT_BYTES: u64 = 64 * 1024 * 1024;
pub(crate) const MAX_TRANSCRIPT_LINES: usize = 20_000;
pub(crate) const MAX_TRANSCRIPT_TURNS: usize = 512;
pub(crate) const MAX_TRANSCRIPT_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_CLAUDE_PROJECT_DIRS: usize = 16_384;
const MAX_CLAUDE_TRANSCRIPT_FILES: usize = 65_536;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_THINKING_BYTES: usize = 32 * 1024;
const MAX_TOOL_NAME_BYTES: usize = 128;
const MAX_TOOLS_PER_TURN: usize = 32;
const MAX_MODEL_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TranscriptProvider {
    Claude,
    Codex,
}

impl TranscriptProvider {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    fn default_root(self, home: &Path) -> PathBuf {
        match self {
            Self::Claude => home.join(".claude"),
            Self::Codex => home.join(".codex"),
        }
    }

    fn environment_name(self) -> &'static str {
        match self {
            Self::Claude => "CLAUDE_CONFIG_DIR",
            Self::Codex => "CODEX_HOME",
        }
    }
}

pub(crate) struct ResolvedTranscriptRequest {
    provider: TranscriptProvider,
    session_id: String,
    config_root: PathBuf,
    known_revision: Option<String>,
}

fn response(
    provider: &str,
    session_id: &str,
    status: AgentTranscriptStatus,
    message: &str,
) -> AgentTranscriptResponse {
    AgentTranscriptResponse {
        schema_version: TRANSCRIPT_SCHEMA_VERSION,
        provider: provider.to_string(),
        session_id: session_id.to_string(),
        status: status.into(),
        turns: Vec::new(),
        truncated: false,
        source_revision: String::new(),
        message: message.to_string(),
    }
}

fn invalid_response(message: &str) -> AgentTranscriptResponse {
    response("", "", AgentTranscriptStatus::InvalidRequest, message)
}

fn valid_opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn validate_request(
    request: &ReadAgentTranscript,
) -> Result<TranscriptProvider, AgentTranscriptResponse> {
    if request.schema_version != TRANSCRIPT_SCHEMA_VERSION {
        return Err(invalid_response("unsupported transcript request version"));
    }
    let Some(provider) = TranscriptProvider::parse(&request.provider) else {
        return Err(invalid_response("unsupported transcript provider"));
    };
    if !valid_opaque_id(&request.account_id) || !valid_opaque_id(&request.session_id) {
        return Err(invalid_response("invalid transcript identity"));
    }
    if request.known_revision.len() > 128
        || !request
            .known_revision
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(invalid_response("invalid transcript revision"));
    }
    Ok(provider)
}

pub(crate) fn busy_response(request: &ReadAgentTranscript) -> AgentTranscriptResponse {
    match validate_request(request) {
        Ok(provider) => response(
            provider.as_str(),
            &request.session_id,
            AgentTranscriptStatus::Unavailable,
            "transcript reader is busy; retry",
        ),
        Err(response) => response,
    }
}

/// Resolve the request against a freshly scanned daemon account inventory. The
/// resulting path never leaves this process and is canonicalized before use.
pub(crate) fn resolve_request(
    routes: &AccountRoutes,
    request: ReadAgentTranscript,
) -> Result<ResolvedTranscriptRequest, AgentTranscriptResponse> {
    let provider = validate_request(&request)?;

    let mut environment = HashMap::new();
    let route = AgentLaunchRoute {
        schema_version: 1,
        provider: provider.as_str().to_string(),
        account_id: request.account_id,
    };
    if prepare_launch_environment_from_routes(routes, Some(&route), &mut environment).is_err() {
        return Err(invalid_response(
            "unknown, stale or ambiguous daemon account id",
        ));
    }
    let root = match environment.remove(provider.environment_name()) {
        Some(root) => PathBuf::from(root),
        None => {
            let Some(home) = dirs::home_dir() else {
                return Err(response(
                    provider.as_str(),
                    &request.session_id,
                    AgentTranscriptStatus::Unavailable,
                    "daemon home directory is unavailable",
                ));
            };
            provider.default_root(&home)
        }
    };
    let config_root = std::fs::canonicalize(root).map_err(|_| {
        response(
            provider.as_str(),
            &request.session_id,
            AgentTranscriptStatus::Unavailable,
            "provider transcript history is unavailable",
        )
    })?;
    if !config_root.is_dir() {
        return Err(response(
            provider.as_str(),
            &request.session_id,
            AgentTranscriptStatus::Unavailable,
            "provider transcript history is unavailable",
        ));
    }

    Ok(ResolvedTranscriptRequest {
        provider,
        session_id: request.session_id,
        config_root,
        known_revision: (!request.known_revision.is_empty()).then_some(request.known_revision),
    })
}

#[derive(Debug)]
enum TranscriptPathError {
    Missing,
    Malformed,
    TooLarge,
    Unavailable,
}

fn claude_transcript_path(
    config_root: &Path,
    session_id: &str,
) -> Result<PathBuf, TranscriptPathError> {
    let projects = config_root.join("projects");
    let entries = std::fs::read_dir(projects).map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => TranscriptPathError::Missing,
        _ => TranscriptPathError::Unavailable,
    })?;
    let mut project_count = 0usize;
    let mut file_count = 0usize;
    let mut matched = None;
    for project in entries {
        project_count += 1;
        if project_count > MAX_CLAUDE_PROJECT_DIRS {
            return Err(TranscriptPathError::TooLarge);
        }
        let project = project.map_err(|_| TranscriptPathError::Unavailable)?;
        let file_type = project
            .file_type()
            .map_err(|_| TranscriptPathError::Unavailable)?;
        if !file_type.is_dir() || file_type.is_symlink() {
            continue;
        }
        let files =
            std::fs::read_dir(project.path()).map_err(|_| TranscriptPathError::Unavailable)?;
        for file in files {
            file_count += 1;
            if file_count > MAX_CLAUDE_TRANSCRIPT_FILES {
                return Err(TranscriptPathError::TooLarge);
            }
            let file = file.map_err(|_| TranscriptPathError::Unavailable)?;
            let file_type = file
                .file_type()
                .map_err(|_| TranscriptPathError::Unavailable)?;
            if !file_type.is_file() || file_type.is_symlink() {
                continue;
            }
            let path = file.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("jsonl")
                || path.file_stem().and_then(|stem| stem.to_str()) != Some(session_id)
            {
                continue;
            }
            if matched.replace(path).is_some() {
                return Err(TranscriptPathError::Malformed);
            }
        }
    }
    matched.ok_or(TranscriptPathError::Missing)
}

fn codex_transcript_path(
    config_root: &Path,
    session_id: &str,
) -> Result<PathBuf, TranscriptPathError> {
    match zaplex_cockpit::codex_sessions::transcript_path(config_root, session_id) {
        Ok(Some(path)) => Ok(path),
        Ok(None) => Err(TranscriptPathError::Missing),
        Err(zaplex_cockpit::codex_sessions::TranscriptError::HistoryLimitExceeded { .. })
        | Err(zaplex_cockpit::codex_sessions::TranscriptError::TranscriptLookupLimitExceeded {
            ..
        })
        | Err(zaplex_cockpit::codex_sessions::TranscriptError::TranscriptTooLarge { .. }) => {
            Err(TranscriptPathError::TooLarge)
        }
        Err(zaplex_cockpit::codex_sessions::TranscriptError::AmbiguousSessionId { .. })
        | Err(zaplex_cockpit::codex_sessions::TranscriptError::MalformedTranscript) => {
            Err(TranscriptPathError::Malformed)
        }
        Err(zaplex_cockpit::codex_sessions::TranscriptError::InvalidSessionId)
        | Err(zaplex_cockpit::codex_sessions::TranscriptError::UnsupportedTranscript)
        | Err(zaplex_cockpit::codex_sessions::TranscriptError::Io(_))
        | Err(zaplex_cockpit::codex_sessions::TranscriptError::Walk(_)) => {
            Err(TranscriptPathError::Unavailable)
        }
    }
}

fn source_path(request: &ResolvedTranscriptRequest) -> Result<PathBuf, TranscriptPathError> {
    match request.provider {
        TranscriptProvider::Claude => {
            claude_transcript_path(&request.config_root, &request.session_id)
        }
        TranscriptProvider::Codex => {
            codex_transcript_path(&request.config_root, &request.session_id)
        }
    }
}

fn source_revision(content: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"zaplex-agent-transcript-revision-v1\0");
    digest.update(content);
    hex::encode(digest.finalize())
}

struct CheckedSource {
    content: String,
    revision: String,
}

#[cfg(unix)]
fn same_file(opened: &std::fs::Metadata, resolved: &std::fs::Metadata) -> bool {
    opened.dev() == resolved.dev() && opened.ino() == resolved.ino()
}

#[cfg(not(unix))]
fn same_file(opened: &std::fs::Metadata, resolved: &std::fs::Metadata) -> bool {
    opened.len() == resolved.len()
        && opened.modified().ok().is_some()
        && opened.modified().ok() == resolved.modified().ok()
}

fn open_source(
    config_root: &Path,
    path: PathBuf,
) -> Result<(File, std::fs::Metadata), TranscriptPathError> {
    let symlink_metadata =
        std::fs::symlink_metadata(&path).map_err(|_| TranscriptPathError::Unavailable)?;
    if symlink_metadata.file_type().is_symlink() || !symlink_metadata.is_file() {
        return Err(TranscriptPathError::Malformed);
    }

    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let file = options
        .open(&path)
        .map_err(|_| TranscriptPathError::Unavailable)?;
    let opened_metadata = file
        .metadata()
        .map_err(|_| TranscriptPathError::Unavailable)?;
    if !opened_metadata.is_file() {
        return Err(TranscriptPathError::Malformed);
    }
    if opened_metadata.len() > MAX_TRANSCRIPT_BYTES {
        return Err(TranscriptPathError::TooLarge);
    }

    let canonical = std::fs::canonicalize(&path).map_err(|_| TranscriptPathError::Unavailable)?;
    if !canonical.starts_with(config_root) {
        return Err(TranscriptPathError::Malformed);
    }
    let resolved_metadata = canonical
        .metadata()
        .map_err(|_| TranscriptPathError::Unavailable)?;
    if !same_file(&opened_metadata, &resolved_metadata) {
        return Err(TranscriptPathError::Unavailable);
    }

    Ok((file, opened_metadata))
}

fn read_source(
    file: File,
    metadata: &std::fs::Metadata,
) -> Result<CheckedSource, TranscriptPathError> {
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_TRANSCRIPT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| TranscriptPathError::Unavailable)?;
    if bytes.len() as u64 > MAX_TRANSCRIPT_BYTES {
        return Err(TranscriptPathError::TooLarge);
    }
    let newlines = bytes.iter().filter(|byte| **byte == b'\n').count();
    let lines = newlines + usize::from(!bytes.is_empty() && bytes.last() != Some(&b'\n'));
    if lines > MAX_TRANSCRIPT_LINES {
        return Err(TranscriptPathError::TooLarge);
    }
    let revision = source_revision(&bytes);
    let content = String::from_utf8(bytes).map_err(|_| TranscriptPathError::Malformed)?;
    Ok(CheckedSource { content, revision })
}

fn check_source(config_root: &Path, path: PathBuf) -> Result<CheckedSource, TranscriptPathError> {
    let (file, metadata) = open_source(config_root, path)?;
    read_source(file, &metadata)
}

fn path_error_response(
    provider: TranscriptProvider,
    session_id: &str,
    error: TranscriptPathError,
) -> AgentTranscriptResponse {
    let (status, message) = match error {
        TranscriptPathError::Missing => (
            AgentTranscriptStatus::Missing,
            "transcript history was not found",
        ),
        TranscriptPathError::Malformed => (
            AgentTranscriptStatus::Malformed,
            "transcript history is malformed or ambiguous",
        ),
        TranscriptPathError::TooLarge => (
            AgentTranscriptStatus::TooLarge,
            "transcript history exceeds the daemon read limits",
        ),
        TranscriptPathError::Unavailable => (
            AgentTranscriptStatus::Unavailable,
            "transcript history is unavailable",
        ),
    };
    response(provider.as_str(), session_id, status, message)
}

fn parse_claude(content: &str) -> Result<Vec<TranscriptTurn>, AgentTranscriptStatus> {
    let mut valid = 0usize;
    let mut supported = 0usize;
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        valid += 1;
        if matches!(
            value.get("type").and_then(serde_json::Value::as_str),
            Some("user" | "assistant")
        ) {
            supported += 1;
        }
    }
    if valid == 0 && !content.trim().is_empty() {
        return Err(AgentTranscriptStatus::Malformed);
    }
    if supported == 0 && valid > 0 {
        return Err(AgentTranscriptStatus::Unsupported);
    }
    Ok(zaplex_cockpit::parse_transcript(content))
}

fn parse_codex(content: &str) -> Result<Vec<TranscriptTurn>, AgentTranscriptStatus> {
    match zaplex_cockpit::codex_sessions::parse_transcript_content(content) {
        Ok(turns) => Ok(turns),
        Err(zaplex_cockpit::codex_sessions::TranscriptError::MalformedTranscript) => {
            Err(AgentTranscriptStatus::Malformed)
        }
        Err(zaplex_cockpit::codex_sessions::TranscriptError::UnsupportedTranscript) => {
            Err(AgentTranscriptStatus::Unsupported)
        }
        Err(zaplex_cockpit::codex_sessions::TranscriptError::TranscriptTooLarge { .. })
        | Err(zaplex_cockpit::codex_sessions::TranscriptError::HistoryLimitExceeded { .. })
        | Err(zaplex_cockpit::codex_sessions::TranscriptError::TranscriptLookupLimitExceeded {
            ..
        }) => Err(AgentTranscriptStatus::TooLarge),
        Err(zaplex_cockpit::codex_sessions::TranscriptError::InvalidSessionId)
        | Err(zaplex_cockpit::codex_sessions::TranscriptError::AmbiguousSessionId { .. })
        | Err(zaplex_cockpit::codex_sessions::TranscriptError::Io(_))
        | Err(zaplex_cockpit::codex_sessions::TranscriptError::Walk(_)) => {
            Err(AgentTranscriptStatus::Unavailable)
        }
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> (String, bool) {
    if value.len() <= max_bytes {
        return (value.to_string(), false);
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    (value[..end].to_string(), true)
}

fn display_text(value: &str, max_bytes: usize) -> (String, bool) {
    let filtered: String = value
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect();
    truncate_utf8(&filtered, max_bytes)
}

fn display_tool(tool: ToolCall) -> (Option<AgentTranscriptTool>, bool) {
    let filtered: String = tool
        .name
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    let (name, truncated) = truncate_utf8(filtered.trim(), MAX_TOOL_NAME_BYTES);
    (
        (!name.is_empty()).then_some(AgentTranscriptTool { name }),
        truncated,
    )
}

fn wire_turn(turn: TranscriptTurn) -> (AgentTranscriptTurn, bool) {
    let (text, text_truncated) = display_text(&turn.text, MAX_TEXT_BYTES);
    let (thinking, thinking_truncated) = display_text(&turn.thinking, MAX_THINKING_BYTES);
    let mut truncated = text_truncated || thinking_truncated;
    let tool_count = turn.tools.len();
    let mut tools = Vec::new();
    for tool in turn.tools.into_iter().take(MAX_TOOLS_PER_TURN) {
        let (tool, tool_truncated) = display_tool(tool);
        truncated |= tool_truncated;
        if let Some(tool) = tool {
            tools.push(tool);
        }
    }
    truncated |= tool_count > MAX_TOOLS_PER_TURN;
    let (model, model_truncated) = turn
        .model
        .as_deref()
        .map(|model| display_text(model, MAX_MODEL_BYTES))
        .unwrap_or_default();
    truncated |= model_truncated;
    (
        AgentTranscriptTurn {
            role: match turn.role {
                TurnRole::User => "user",
                TurnRole::Assistant => "assistant",
            }
            .to_string(),
            text,
            thinking,
            tools,
            model,
            timestamp: turn
                .timestamp
                .map(|timestamp| timestamp.to_rfc3339())
                .unwrap_or_default(),
        },
        truncated,
    )
}

fn loaded_response(
    request: &ResolvedTranscriptRequest,
    revision: String,
    turns: Vec<TranscriptTurn>,
) -> AgentTranscriptResponse {
    if turns.is_empty() {
        let mut empty = response(
            request.provider.as_str(),
            &request.session_id,
            AgentTranscriptStatus::Empty,
            "transcript contains no visible conversation turns",
        );
        empty.source_revision = revision;
        return empty;
    }

    let original_turn_count = turns.len();
    let mut truncated = original_turn_count > MAX_TRANSCRIPT_TURNS;
    let start = original_turn_count.saturating_sub(MAX_TRANSCRIPT_TURNS);
    let mut wire_turns = Vec::with_capacity(original_turn_count - start);
    for turn in turns.into_iter().skip(start) {
        let (turn, turn_truncated) = wire_turn(turn);
        truncated |= turn_truncated;
        wire_turns.push(turn);
    }
    let mut loaded = response(
        request.provider.as_str(),
        &request.session_id,
        AgentTranscriptStatus::Loaded,
        "",
    );
    loaded.turns = wire_turns;
    loaded.truncated = truncated;
    loaded.source_revision = revision;
    while loaded.encoded_len() > MAX_TRANSCRIPT_RESPONSE_BYTES && !loaded.turns.is_empty() {
        loaded.turns.remove(0);
        loaded.truncated = true;
    }
    if loaded.turns.is_empty() {
        return response(
            request.provider.as_str(),
            &request.session_id,
            AgentTranscriptStatus::TooLarge,
            "transcript display exceeds the daemon response limit",
        );
    }
    loaded
}

fn status_response(
    request: &ResolvedTranscriptRequest,
    status: AgentTranscriptStatus,
    revision: String,
) -> AgentTranscriptResponse {
    let message = match status {
        AgentTranscriptStatus::Missing => "transcript history was not found",
        AgentTranscriptStatus::Empty => "transcript contains no visible conversation turns",
        AgentTranscriptStatus::Unsupported => "transcript format is unsupported",
        AgentTranscriptStatus::Malformed => "transcript history is malformed",
        AgentTranscriptStatus::TooLarge => "transcript history exceeds the daemon read limits",
        AgentTranscriptStatus::InvalidRequest => "invalid transcript request",
        AgentTranscriptStatus::Unavailable => "transcript history is unavailable",
        AgentTranscriptStatus::Loaded
        | AgentTranscriptStatus::NotModified
        | AgentTranscriptStatus::Unspecified => "",
    };
    let mut response = response(
        request.provider.as_str(),
        &request.session_id,
        status,
        message,
    );
    response.source_revision = revision;
    response
}

/// Read and project one transcript snapshot. All filesystem work happens on
/// the daemon background executor; this function is synchronous by design.
pub(crate) fn read_transcript(request: ResolvedTranscriptRequest) -> AgentTranscriptResponse {
    let path = match source_path(&request) {
        Ok(path) => path,
        Err(error) => return path_error_response(request.provider, &request.session_id, error),
    };
    let checked = match check_source(&request.config_root, path) {
        Ok(checked) => checked,
        Err(error) => return path_error_response(request.provider, &request.session_id, error),
    };
    if request.known_revision.as_deref() == Some(checked.revision.as_str()) {
        return status_response(
            &request,
            AgentTranscriptStatus::NotModified,
            checked.revision,
        );
    }
    let turns = match request.provider {
        TranscriptProvider::Claude => parse_claude(&checked.content),
        TranscriptProvider::Codex => parse_codex(&checked.content),
    };
    let turns = match turns {
        Ok(turns) => turns,
        Err(status) => return status_response(&request, status, checked.revision),
    };
    loaded_response(&request, checked.revision, turns)
}

#[cfg(test)]
#[path = "transcript_rpc_tests.rs"]
mod tests;
