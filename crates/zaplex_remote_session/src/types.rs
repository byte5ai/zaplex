//! Transport-agnostic types and capability constants shared across the zaplex
//! remote-session layer.
//!
//! This module is used by both the daemon (server) and the client, so it only
//! holds pure data types — no tokio/PTY or other side-specific implementation
//! details.

use serde::{Deserialize, Serialize};

/// Capability identifier advertised by the daemon in `InitializeResponse.features`:
/// it signals that the daemon has the native zaplex session host built in
/// (PTY ownership + reconnect replay).
///
/// The client uses it to decide whether it may take the
/// `OpenSession`/`AttachSession` path instead of falling back to the legacy
/// "SSH PTY + no persistence" behaviour.
pub const FEATURE_SESSION_HOST: &str = "session-host";

/// Reserved capability name for the Phase B3 native UDP transport (mosh-grade
/// roaming + low latency). **Not yet advertised** by [`supported_features`] —
/// the transport is unimplemented; this only reserves the negotiation name so
/// client and daemon agree on it once it lands, keeping the capability handshake
/// honest (never advertise what we can't fulfil). See
/// `docs/superpowers/specs/2026-06-28-stage-b3-udp-transport-design.md`.
pub const FEATURE_UDP_TRANSPORT: &str = "udp-transport";

/// Capability identifier advertised by the daemon in `InitializeResponse.features`:
/// it signals that the daemon can report its host's **agent-session inventory**
/// (Claude/Codex CLI conversations discovered on the daemon's filesystem) via
/// `ListAgentSessions` → `AgentSessionList` — the Agent-Cockpit's cross-host tree.
///
/// A client talking to an old daemon that omits this feature must skip the
/// `ListAgentSessions` call and treat that host as contributing zero
/// agent-sessions, never erroring the whole tree.
///
/// Unlike [`FEATURE_SESSION_HOST`] this is filesystem-based (no PTY ownership),
/// so it is advertised on all platforms.
pub const FEATURE_AGENT_INVENTORY: &str = "agent-inventory";

/// A persistent session identifier assigned by the daemon.
///
/// Unlike the protocol's existing `session_id: uint64` (which is the client's
/// tab/connection dimension), this is the daemon-side session key: it stays
/// stable across reconnects and across client app restarts, hence a UUID string
/// rather than an in-process counter.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(String);

impl SessionId {
    /// Generates a fresh random session identifier (called by the daemon on
    /// `OpenSession`).
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }

    /// Returns the underlying string view.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<SessionId> for String {
    fn from(id: SessionId) -> Self {
        id.0
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Returns the set of capabilities this daemon binary actually supports, used
/// to populate `InitializeResponse.features`.
///
/// Unix daemons advertise [`FEATURE_SESSION_HOST`] (Stage 1 PTY session host).
/// Non-unix targets do not own PTYs, so they advertise nothing — honest
/// advertisement: never claim a capability we cannot fulfil.
pub fn supported_features() -> Vec<String> {
    // Agent-session inventory is filesystem-based (no PTY), so every daemon
    // build advertises it regardless of platform.
    let mut features = vec![FEATURE_AGENT_INVENTORY.to_string()];
    // The native PTY session host is unix-only: non-unix targets do not own
    // PTYs, so they advertise nothing more — honest advertisement, never claim
    // a capability we cannot fulfil.
    #[cfg(unix)]
    features.push(FEATURE_SESSION_HOST.to_string());
    features
}

/// Returns whether `feature` appears in the daemon-advertised `features` list.
pub fn has_feature(features: &[String], feature: &str) -> bool {
    features.iter().any(|f| f == feature)
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
