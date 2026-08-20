//! Provider-neutral transcript document projection and live-watch state.
//!
//! Filesystem paths stay inside local targets. Remote targets carry only
//! daemon-owned opaque identities and are revalidated by the caller before
//! every request.

use std::path::{Path, PathBuf};

use crate::remote_server::proto::{
    AgentTranscriptResponse, AgentTranscriptStatus, AgentTranscriptTurn,
};
use zaplex_cockpit::{LoadedTranscript, Provider, TranscriptTurn};

const MAX_TRANSCRIPT_TURNS: usize = 512;
const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_THINKING_BYTES: usize = 32 * 1024;
const MAX_TOOLS_PER_TURN: usize = 32;
const MAX_TOOL_NAME_BYTES: usize = 128;
const MAX_MODEL_BYTES: usize = 128;
const MAX_TRANSCRIPT_MARKDOWN_BYTES: usize = 4 * 1024 * 1024;
const MAX_REMOTE_STATUS_MESSAGE_BYTES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TranscriptTarget {
    Local {
        provider: Provider,
        config_root: PathBuf,
        session_id: String,
    },
    Remote {
        provider: Provider,
        host_id: String,
        account_id: String,
        session_id: String,
    },
}

impl TranscriptTarget {
    pub(crate) fn provider(&self) -> Provider {
        match self {
            Self::Local { provider, .. } | Self::Remote { provider, .. } => *provider,
        }
    }

    pub(crate) fn session_id(&self) -> &str {
        match self {
            Self::Local { session_id, .. } | Self::Remote { session_id, .. } => session_id,
        }
    }
}

pub(crate) fn should_follow_transcript(
    document_open: bool,
    state: Option<zaplex_cockpit::SessionState>,
) -> bool {
    document_open && state.is_some_and(|state| state != zaplex_cockpit::SessionState::Idle)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TranscriptDocumentState {
    Ready,
    Missing,
    Empty,
    Unsupported,
    Malformed,
    TooLarge,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TranscriptDocument {
    pub(crate) state: TranscriptDocumentState,
    pub(crate) markdown: String,
    pub(crate) source_revision: Option<String>,
}

pub(crate) fn transcript_title(provider: Provider) -> String {
    crate::t!(
        "cockpit-transcript-title",
        provider = provider_display_name(provider)
    )
    .to_string()
}

fn provider_display_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
        Provider::Antigravity => "Antigravity",
    }
}

pub(crate) fn state_document(
    provider: Provider,
    state: TranscriptDocumentState,
) -> TranscriptDocument {
    let detail = match state {
        TranscriptDocumentState::Missing => Some(crate::t!("cockpit-transcript-missing")),
        TranscriptDocumentState::Empty => Some(crate::t!("cockpit-transcript-empty")),
        TranscriptDocumentState::Unsupported => Some(crate::t!("cockpit-transcript-unsupported")),
        TranscriptDocumentState::Malformed => Some(crate::t!("cockpit-transcript-malformed")),
        TranscriptDocumentState::TooLarge => Some(crate::t!("cockpit-transcript-too-large")),
        TranscriptDocumentState::Unavailable => Some(crate::t!("cockpit-transcript-unavailable")),
        TranscriptDocumentState::Ready => None,
    };
    let mut markdown = format!("# {}", transcript_title(provider));
    if let Some(detail) = detail {
        markdown.push_str("\n\n");
        markdown.push_str(&detail);
    }
    TranscriptDocument {
        state,
        markdown,
        source_revision: None,
    }
}

fn bounded_display_value(value: &str, max_bytes: usize, preserve_layout: bool) -> (String, bool) {
    let mut output = String::with_capacity(value.len().min(max_bytes));
    let mut truncated = false;
    for character in value.chars() {
        let allowed = if preserve_layout {
            !character.is_control() || matches!(character, '\n' | '\t')
        } else {
            !character.is_control() && character != '`'
        };
        if !allowed {
            continue;
        }
        if output.len() + character.len_utf8() > max_bytes {
            truncated = true;
            break;
        }
        output.push(character);
    }
    (output, truncated)
}

fn bounded_transcript_turns(turns: Vec<TranscriptTurn>) -> (Vec<TranscriptTurn>, bool) {
    let original_turn_count = turns.len();
    let start = original_turn_count.saturating_sub(MAX_TRANSCRIPT_TURNS);
    let mut truncated = original_turn_count > MAX_TRANSCRIPT_TURNS;
    let mut bounded = Vec::with_capacity(original_turn_count - start);
    for turn in turns.into_iter().skip(start) {
        let (text, text_truncated) = bounded_display_value(&turn.text, MAX_TEXT_BYTES, true);
        let (thinking, thinking_truncated) =
            bounded_display_value(&turn.thinking, MAX_THINKING_BYTES, true);
        truncated |= text_truncated || thinking_truncated;

        let tool_count = turn.tools.len();
        let mut tools = Vec::with_capacity(tool_count.min(MAX_TOOLS_PER_TURN));
        for tool in turn.tools.into_iter().take(MAX_TOOLS_PER_TURN) {
            let (name, name_truncated) =
                bounded_display_value(tool.name.trim(), MAX_TOOL_NAME_BYTES, false);
            truncated |= name_truncated;
            if !name.is_empty() {
                tools.push(zaplex_cockpit::ToolCall { name });
            }
        }
        truncated |= tool_count > MAX_TOOLS_PER_TURN;

        let model = turn.model.and_then(|model| {
            let (model, model_truncated) =
                bounded_display_value(model.trim(), MAX_MODEL_BYTES, false);
            truncated |= model_truncated;
            (!model.is_empty()).then_some(model)
        });
        bounded.push(TranscriptTurn {
            role: turn.role,
            text,
            thinking,
            tools,
            model,
            usage: turn.usage,
            timestamp: turn.timestamp,
        });
    }
    (bounded, truncated)
}

fn push_bounded(output: &mut String, value: &str, max_bytes: usize) -> bool {
    let remaining = max_bytes.saturating_sub(output.len());
    if value.len() <= remaining {
        output.push_str(value);
        return true;
    }
    let mut end = remaining;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    output.push_str(&value[..end]);
    false
}

fn format_transcript_markdown(
    provider: Provider,
    turns: &[TranscriptTurn],
    max_bytes: usize,
) -> (String, bool) {
    let provider = provider_display_name(provider);
    let mut output = String::new();
    for turn in turns {
        let heading_complete = match turn.role {
            zaplex_cockpit::TurnRole::User => {
                if !push_bounded(&mut output, "## You", max_bytes) {
                    false
                } else if let Some(timestamp) = turn.timestamp.as_ref() {
                    push_bounded(
                        &mut output,
                        &format!(" · {}", timestamp.format("%Y-%m-%d %H:%M UTC")),
                        max_bytes,
                    ) && push_bounded(&mut output, "\n\n", max_bytes)
                } else {
                    push_bounded(&mut output, "\n\n", max_bytes)
                }
            }
            zaplex_cockpit::TurnRole::Assistant => {
                if !push_bounded(&mut output, "## ", max_bytes)
                    || !push_bounded(&mut output, provider, max_bytes)
                {
                    false
                } else {
                    let model_complete = turn.model.as_deref().is_none_or(|model| {
                        push_bounded(&mut output, " · ", max_bytes)
                            && push_bounded(&mut output, model, max_bytes)
                    });
                    let timestamp_complete = turn.timestamp.as_ref().is_none_or(|timestamp| {
                        push_bounded(
                            &mut output,
                            &format!(" · {}", timestamp.format("%Y-%m-%d %H:%M UTC")),
                            max_bytes,
                        )
                    });
                    model_complete
                        && timestamp_complete
                        && push_bounded(&mut output, "\n\n", max_bytes)
                }
            }
        };
        if !heading_complete {
            return (output, true);
        }
        if !turn.thinking.is_empty() {
            if !push_bounded(
                &mut output,
                "<details><summary>thinking</summary>\n\n",
                max_bytes,
            ) || !push_bounded(&mut output, turn.thinking.trim(), max_bytes)
                || !push_bounded(&mut output, "\n\n</details>\n\n", max_bytes)
            {
                return (output, true);
            }
        }
        if !turn.tools.is_empty() {
            if !push_bounded(&mut output, "`⚙ ", max_bytes) {
                return (output, true);
            }
            for (index, tool) in turn.tools.iter().enumerate() {
                if (index > 0 && !push_bounded(&mut output, ", ", max_bytes))
                    || !push_bounded(&mut output, &tool.name, max_bytes)
                {
                    return (output, true);
                }
            }
            if !push_bounded(&mut output, "`\n\n", max_bytes) {
                return (output, true);
            }
        }
        if !turn.text.is_empty() {
            if !push_bounded(&mut output, turn.text.trim(), max_bytes)
                || !push_bounded(&mut output, "\n\n", max_bytes)
            {
                return (output, true);
            }
        }
        if !push_bounded(&mut output, "---\n\n", max_bytes) {
            return (output, true);
        }
    }
    output.truncate(output.trim_end().len());
    (output, false)
}

fn ready_document(
    provider: Provider,
    loaded: LoadedTranscript,
    truncated: bool,
) -> TranscriptDocument {
    if loaded.turns.is_empty() {
        let mut document = state_document(provider, TranscriptDocumentState::Empty);
        document.source_revision = Some(loaded.source_revision);
        return document;
    }
    let (turns, display_truncated) = bounded_transcript_turns(loaded.turns);
    let truncation_banner = format!("> {}\n\n", crate::t!("cockpit-transcript-truncated"));
    let body_limit = MAX_TRANSCRIPT_MARKDOWN_BYTES.saturating_sub(truncation_banner.len());
    let (mut markdown, markdown_truncated) =
        format_transcript_markdown(provider, &turns, body_limit);
    let truncated = truncated || display_truncated || markdown_truncated;
    if truncated {
        markdown.insert_str(0, &truncation_banner);
    }
    TranscriptDocument {
        state: TranscriptDocumentState::Ready,
        markdown,
        source_revision: Some(loaded.source_revision),
    }
}

pub(crate) fn load_local_transcript(
    provider: Provider,
    config_root: &Path,
    session_id: &str,
) -> TranscriptDocument {
    match provider {
        Provider::Claude => {
            match zaplex_cockpit::sessions::load_transcript_with_revision(config_root, session_id) {
                Ok(Some(loaded)) => ready_document(provider, loaded, false),
                Ok(None) => state_document(provider, TranscriptDocumentState::Missing),
                Err(zaplex_cockpit::sessions::TranscriptError::InvalidSessionId)
                | Err(zaplex_cockpit::sessions::TranscriptError::AmbiguousSessionId)
                | Err(zaplex_cockpit::sessions::TranscriptError::MalformedTranscript) => {
                    state_document(provider, TranscriptDocumentState::Malformed)
                }
                Err(zaplex_cockpit::sessions::TranscriptError::HistoryLimitExceeded)
                | Err(zaplex_cockpit::sessions::TranscriptError::TranscriptTooLarge { .. }) => {
                    state_document(provider, TranscriptDocumentState::TooLarge)
                }
                Err(zaplex_cockpit::sessions::TranscriptError::UnsupportedTranscript) => {
                    state_document(provider, TranscriptDocumentState::Unsupported)
                }
                Err(zaplex_cockpit::sessions::TranscriptError::Io(_)) => {
                    state_document(provider, TranscriptDocumentState::Unavailable)
                }
            }
        }
        Provider::Codex => match zaplex_cockpit::codex_sessions::load_transcript_with_revision(
            config_root,
            session_id,
        ) {
            Ok(Some(loaded)) => ready_document(provider, loaded, false),
            Ok(None) => state_document(provider, TranscriptDocumentState::Missing),
            Err(zaplex_cockpit::codex_sessions::TranscriptError::InvalidSessionId)
            | Err(zaplex_cockpit::codex_sessions::TranscriptError::AmbiguousSessionId { .. })
            | Err(zaplex_cockpit::codex_sessions::TranscriptError::MalformedTranscript) => {
                state_document(provider, TranscriptDocumentState::Malformed)
            }
            Err(zaplex_cockpit::codex_sessions::TranscriptError::HistoryLimitExceeded {
                ..
            })
            | Err(
                zaplex_cockpit::codex_sessions::TranscriptError::TranscriptLookupLimitExceeded {
                    ..
                },
            )
            | Err(zaplex_cockpit::codex_sessions::TranscriptError::TranscriptTooLarge { .. }) => {
                state_document(provider, TranscriptDocumentState::TooLarge)
            }
            Err(zaplex_cockpit::codex_sessions::TranscriptError::UnsupportedTranscript) => {
                state_document(provider, TranscriptDocumentState::Unsupported)
            }
            Err(zaplex_cockpit::codex_sessions::TranscriptError::Io(_))
            | Err(zaplex_cockpit::codex_sessions::TranscriptError::Walk(_)) => {
                state_document(provider, TranscriptDocumentState::Unavailable)
            }
        },
        Provider::Antigravity => state_document(provider, TranscriptDocumentState::Unsupported),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RemoteTranscriptProjection {
    Modified(TranscriptDocument),
    NotModified,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RemoteTranscriptProjectionError {
    InvalidEnvelope,
    InvalidStatus,
    InvalidPayload,
}

fn valid_revision(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn remote_wire_turn(
    turn: AgentTranscriptTurn,
) -> Result<TranscriptTurn, RemoteTranscriptProjectionError> {
    const MAX_TIMESTAMP_BYTES: usize = 64;

    if turn.text.len() > MAX_TEXT_BYTES
        || turn.thinking.len() > MAX_THINKING_BYTES
        || turn.model.len() > MAX_MODEL_BYTES
        || turn.timestamp.len() > MAX_TIMESTAMP_BYTES
        || turn.tools.len() > MAX_TOOLS_PER_TURN
        || turn
            .tools
            .iter()
            .any(|tool| tool.name.trim().is_empty() || tool.name.len() > MAX_TOOL_NAME_BYTES)
    {
        return Err(RemoteTranscriptProjectionError::InvalidPayload);
    }
    let role = match turn.role.as_str() {
        "user" => zaplex_cockpit::TurnRole::User,
        "assistant" => zaplex_cockpit::TurnRole::Assistant,
        _ => return Err(RemoteTranscriptProjectionError::InvalidPayload),
    };
    let timestamp = if turn.timestamp.is_empty() {
        None
    } else {
        Some(
            chrono::DateTime::parse_from_rfc3339(&turn.timestamp)
                .map_err(|_| RemoteTranscriptProjectionError::InvalidPayload)?
                .with_timezone(&chrono::Utc),
        )
    };
    Ok(TranscriptTurn {
        role,
        text: turn.text,
        thinking: turn.thinking,
        tools: turn
            .tools
            .into_iter()
            .map(|tool| zaplex_cockpit::ToolCall { name: tool.name })
            .collect(),
        model: (!turn.model.is_empty()).then_some(turn.model),
        usage: None,
        timestamp,
    })
}

fn remote_payload_size(response: &AgentTranscriptResponse) -> Option<usize> {
    let mut size = response
        .provider
        .len()
        .checked_add(response.session_id.len())?
        .checked_add(response.source_revision.len())?
        .checked_add(response.message.len())?;
    for turn in &response.turns {
        size = size
            .checked_add(turn.role.len())?
            .checked_add(turn.text.len())?
            .checked_add(turn.thinking.len())?
            .checked_add(turn.model.len())?
            .checked_add(turn.timestamp.len())?;
        for tool in &turn.tools {
            size = size.checked_add(tool.name.len())?;
        }
    }
    Some(size)
}

fn valid_optional_revision(value: &str) -> bool {
    value.is_empty() || valid_revision(value)
}

fn valid_state_payload(response: &AgentTranscriptResponse) -> bool {
    response.turns.is_empty()
        && !response.truncated
        && !response.message.is_empty()
        && response.message.len() <= MAX_REMOTE_STATUS_MESSAGE_BYTES
}

fn state_projection(
    provider: Provider,
    state: TranscriptDocumentState,
    source_revision: String,
) -> RemoteTranscriptProjection {
    let mut document = state_document(provider, state);
    document.source_revision = (!source_revision.is_empty()).then_some(source_revision);
    RemoteTranscriptProjection::Modified(document)
}

pub(crate) fn project_remote_transcript(
    provider: Provider,
    session_id: &str,
    known_revision: Option<&str>,
    response: AgentTranscriptResponse,
) -> Result<RemoteTranscriptProjection, RemoteTranscriptProjectionError> {
    if response.schema_version != 1
        || response.provider != provider.as_str()
        || response.session_id != session_id
        || response.message.len() > MAX_REMOTE_STATUS_MESSAGE_BYTES
        || remote_payload_size(&response).is_none_or(|size| size > MAX_TRANSCRIPT_MARKDOWN_BYTES)
    {
        return Err(RemoteTranscriptProjectionError::InvalidEnvelope);
    }
    if known_revision.is_some_and(|revision| !valid_revision(revision)) {
        return Err(RemoteTranscriptProjectionError::InvalidPayload);
    }
    let status = AgentTranscriptStatus::try_from(response.status)
        .map_err(|_| RemoteTranscriptProjectionError::InvalidStatus)?;
    match status {
        AgentTranscriptStatus::Loaded => {
            if response.turns.is_empty()
                || response.turns.len() > MAX_TRANSCRIPT_TURNS
                || !valid_revision(&response.source_revision)
                || !response.message.is_empty()
                || known_revision == Some(response.source_revision.as_str())
            {
                return Err(RemoteTranscriptProjectionError::InvalidPayload);
            }
            let mut turns = Vec::with_capacity(response.turns.len());
            for turn in response.turns {
                turns.push(remote_wire_turn(turn)?);
            }
            Ok(RemoteTranscriptProjection::Modified(ready_document(
                provider,
                LoadedTranscript {
                    turns,
                    source_revision: response.source_revision,
                },
                response.truncated,
            )))
        }
        AgentTranscriptStatus::NotModified => {
            if known_revision != Some(response.source_revision.as_str())
                || !valid_revision(&response.source_revision)
                || !response.turns.is_empty()
                || response.truncated
                || !response.message.is_empty()
            {
                return Err(RemoteTranscriptProjectionError::InvalidPayload);
            }
            Ok(RemoteTranscriptProjection::NotModified)
        }
        AgentTranscriptStatus::Missing
            if valid_state_payload(&response) && response.source_revision.is_empty() =>
        {
            Ok(state_projection(
                provider,
                TranscriptDocumentState::Missing,
                response.source_revision,
            ))
        }
        AgentTranscriptStatus::Empty
            if valid_state_payload(&response) && valid_revision(&response.source_revision) =>
        {
            Ok(state_projection(
                provider,
                TranscriptDocumentState::Empty,
                response.source_revision,
            ))
        }
        AgentTranscriptStatus::Unsupported
            if valid_state_payload(&response)
                && valid_optional_revision(&response.source_revision) =>
        {
            Ok(state_projection(
                provider,
                TranscriptDocumentState::Unsupported,
                response.source_revision,
            ))
        }
        AgentTranscriptStatus::Malformed
            if valid_state_payload(&response)
                && valid_optional_revision(&response.source_revision) =>
        {
            Ok(state_projection(
                provider,
                TranscriptDocumentState::Malformed,
                response.source_revision,
            ))
        }
        AgentTranscriptStatus::TooLarge
            if valid_state_payload(&response) && response.source_revision.is_empty() =>
        {
            Ok(state_projection(
                provider,
                TranscriptDocumentState::TooLarge,
                response.source_revision,
            ))
        }
        AgentTranscriptStatus::Unavailable
            if valid_state_payload(&response) && response.source_revision.is_empty() =>
        {
            Ok(state_projection(
                provider,
                TranscriptDocumentState::Unavailable,
                response.source_revision,
            ))
        }
        AgentTranscriptStatus::Missing
        | AgentTranscriptStatus::Empty
        | AgentTranscriptStatus::Unsupported
        | AgentTranscriptStatus::Malformed
        | AgentTranscriptStatus::TooLarge
        | AgentTranscriptStatus::Unavailable => {
            Err(RemoteTranscriptProjectionError::InvalidPayload)
        }
        AgentTranscriptStatus::InvalidRequest | AgentTranscriptStatus::Unspecified => {
            Err(RemoteTranscriptProjectionError::InvalidStatus)
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TranscriptWatchState {
    revision: Option<String>,
    next_generation: u64,
    in_flight: Option<u64>,
}

impl TranscriptWatchState {
    pub(crate) fn with_revision(revision: Option<String>) -> Self {
        Self {
            revision,
            next_generation: 0,
            in_flight: None,
        }
    }

    pub(crate) fn revision(&self) -> Option<&str> {
        self.revision.as_deref()
    }

    pub(crate) fn is_in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    pub(crate) fn begin_refresh(&mut self) -> Option<u64> {
        if self.in_flight.is_some() {
            return None;
        }
        self.next_generation = self.next_generation.wrapping_add(1).max(1);
        self.in_flight = Some(self.next_generation);
        self.in_flight
    }

    pub(crate) fn finish_refresh(&mut self, generation: u64, revision: Option<String>) -> bool {
        if self.in_flight != Some(generation) {
            return false;
        }
        self.in_flight = None;
        self.revision = revision;
        true
    }

    pub(crate) fn finish_not_modified(&mut self, generation: u64) -> bool {
        if self.in_flight != Some(generation) {
            return false;
        }
        self.in_flight = None;
        true
    }
}

#[cfg(test)]
#[path = "transcript_view_tests.rs"]
mod tests;
