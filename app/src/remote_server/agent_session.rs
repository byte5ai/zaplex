//! Mapping between the cockpit's `SessionSnapshot` and the wire
//! `AgentSessionInfo` proto (Agent-Cockpit cross-host inventory).
//!
//! The daemon side maps `SessionSnapshot` → `AgentSessionInfo` for the
//! `ListAgentSessions` response ([`snapshot_to_proto`]); the client side maps
//! `AgentSessionInfo` → `SessionSnapshot` when folding a host's rows back into
//! the unified Agent-Inventory tree ([`proto_to_snapshot`]).
//!
//! The `state`/`provider` enums travel as lowercase strings so the wire stays
//! forward-compatible: an unknown future `state` folds to [`SessionState::Idle`]
//! (never "needs me"), and an unknown `provider` folds to [`Provider::Claude`].
//! An empty optional string field round-trips to `None` (honestly unknown), so
//! an older daemon that omits newer identity fields still decodes cleanly and
//! remains unsignalable without a process fingerprint.

use chrono::{TimeZone, Utc};
use zaplex_cockpit::types::{Provider, SessionSnapshot, SessionState};

use super::proto::AgentSessionInfo;

/// Lowercase wire string for a session state.
pub fn state_to_str(state: SessionState) -> &'static str {
    match state {
        SessionState::Active => "active",
        SessionState::Waiting => "waiting",
        SessionState::Monitor => "monitor",
        SessionState::Idle => "idle",
    }
}

/// Parses a wire state string. Unknown/empty values fold to
/// [`SessionState::Idle`] — the safe "not live, never needs me" default that
/// keeps an unrecognized future state from masquerading as an attention signal.
pub fn state_from_str(s: &str) -> SessionState {
    match s {
        "active" => SessionState::Active,
        "waiting" => SessionState::Waiting,
        "monitor" => SessionState::Monitor,
        _ => SessionState::Idle,
    }
}

/// Parses a wire provider string. Unknown/empty values fold to
/// [`Provider::Claude`] (the default provider).
pub fn provider_from_str(s: &str) -> Provider {
    match s {
        "codex" => Provider::Codex,
        _ => Provider::Claude,
    }
}

/// Maps a cockpit `SessionSnapshot` to its wire `AgentSessionInfo`.
pub fn snapshot_to_proto(s: &SessionSnapshot) -> AgentSessionInfo {
    AgentSessionInfo {
        session_id: s.session_id.clone(),
        cwd: s.cwd.clone(),
        name: s.name.clone(),
        state: state_to_str(s.state).to_string(),
        provider: s.provider.as_str().to_string(),
        model: s.model.clone(),
        // Empty string encodes "honestly unknown" (None).
        effort: s.effort.clone().unwrap_or_default(),
        ctx_tokens: s.ctx_tokens,
        project_root: s.project_root.clone(),
        repo_root: s.repo_root.clone(),
        project_name: s.project_name.clone(),
        // Empty string encodes "honestly unknown" (None), like `effort`.
        worktree: s.worktree.clone().unwrap_or_default(),
        branch: s.branch.clone().unwrap_or_default(),
        config_dir: s.config_dir.clone().unwrap_or_default(),
        account_email: s.account_email.clone().unwrap_or_default(),
        last_activity_epoch_millis: s.last_activity.timestamp_millis() as u64,
        pid: s.pid,
        process_fingerprint: s.process_fingerprint.clone().unwrap_or_default(),
    }
}

/// Maps a wire `AgentSessionInfo` back to a cockpit `SessionSnapshot` (client
/// fold). An empty `effort` becomes `None`; an out-of-range timestamp falls
/// back to the epoch rather than panicking.
pub fn proto_to_snapshot(p: &AgentSessionInfo) -> SessionSnapshot {
    let last_activity = Utc
        .timestamp_millis_opt(p.last_activity_epoch_millis as i64)
        .single()
        .unwrap_or_else(|| Utc.timestamp_opt(0, 0).single().unwrap());
    SessionSnapshot {
        session_id: p.session_id.clone(),
        cwd: p.cwd.clone(),
        name: p.name.clone(),
        state: state_from_str(&p.state),
        provider: provider_from_str(&p.provider),
        model: p.model.clone(),
        effort: if p.effort.is_empty() {
            None
        } else {
            Some(p.effort.clone())
        },
        ctx_tokens: p.ctx_tokens,
        project_root: p.project_root.clone(),
        repo_root: p.repo_root.clone(),
        project_name: p.project_name.clone(),
        // Empty string ⇒ None (honestly unknown), symmetric with `effort`.
        worktree: (!p.worktree.is_empty()).then(|| p.worktree.clone()),
        branch: (!p.branch.is_empty()).then(|| p.branch.clone()),
        config_dir: (!p.config_dir.is_empty()).then(|| p.config_dir.clone()),
        // Empty ⇒ None: an older daemon simply doesn't say, and a session that
        // names no account joins none rather than being guessed onto one.
        account_email: (!p.account_email.is_empty()).then(|| p.account_email.clone()),
        process_fingerprint: (!p.process_fingerprint.is_empty())
            .then(|| p.process_fingerprint.clone()),
        last_activity,
        pid: p.pid,
    }
}

#[cfg(test)]
#[path = "agent_session_tests.rs"]
mod tests;
