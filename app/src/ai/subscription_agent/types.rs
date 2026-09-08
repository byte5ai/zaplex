use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// An installed subscription agent supported by the in-app conversation surface.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SubscriptionAgent {
    ClaudeCode,
    Codex,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct SubscriptionAuthenticationError {
    pub(crate) message: String,
}

impl SubscriptionAgent {
    pub(crate) fn display_name(self) -> &'static str {
        match self {
            SubscriptionAgent::ClaudeCode => "Claude Code",
            SubscriptionAgent::Codex => "Codex",
        }
    }
}

/// Stable identity for the machine on which the official CLI process runs.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub(crate) struct HostIdentity {
    pub(crate) id: String,
    pub(crate) display_name: String,
}

/// Account identity reported by the CLI, coupled to its isolated config directory.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub(crate) struct AccountIdentity {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) config_dir: Option<PathBuf>,
}

/// A concrete CLI installation. The version participates in capability-cache identity.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub(crate) struct InstallationIdentity {
    pub(crate) agent: SubscriptionAgent,
    pub(crate) host: HostIdentity,
    pub(crate) account: AccountIdentity,
    pub(crate) executable: PathBuf,
    pub(crate) version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ModelEffort {
    pub(crate) id: String,
    pub(crate) display_name: String,
}

/// Exact model metadata returned by the selected installed CLI.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ModelCapability {
    pub(crate) id: String,
    pub(crate) display_name: String,
    pub(crate) description: Option<String>,
    pub(crate) resolved_model: Option<String>,
    pub(crate) is_default: bool,
    pub(crate) supported_efforts: Vec<ModelEffort>,
    pub(crate) default_effort: Option<String>,
    pub(crate) context_window: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AgentCapability {
    pub(crate) installation: InstallationIdentity,
    pub(crate) models: Vec<ModelCapability>,
}

/// Every field required to explain where a turn will run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct SubscriptionTarget {
    pub(crate) installation: InstallationIdentity,
    pub(crate) working_directory: PathBuf,
    pub(crate) model: ModelCapability,
    pub(crate) effort: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "agent", content = "id", rename_all = "snake_case")]
pub(crate) enum SessionIdentity {
    ClaudeCode(String),
    Codex(String),
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Usage {
    pub(crate) input_tokens: u64,
    pub(crate) cached_input_tokens: u64,
    pub(crate) output_tokens: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ApprovalDecision {
    Allow,
    AllowForSession,
    Deny,
    Cancel,
}

/// Provider-neutral events emitted by either official structured protocol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SubscriptionEvent {
    SessionStarted(SessionIdentity),
    TextDelta(String),
    ReasoningDelta(String),
    ToolStarted {
        id: String,
        name: String,
        input: Value,
    },
    ToolOutput {
        id: String,
        output: String,
        is_error: bool,
    },
    Diff(String),
    ApprovalRequested {
        request_id: String,
        kind: String,
        description: String,
        input: Value,
    },
    Usage(Usage),
    TurnCompleted {
        session: SessionIdentity,
    },
    Error {
        message: String,
        recoverable: bool,
        session: Option<SessionIdentity>,
    },
}

/// Presentation state for the complete native-agent session lifecycle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AgentLifecycle {
    NoAgentInstalled,
    NotSignedIn {
        agent: SubscriptionAgent,
    },
    Ready,
    Starting,
    Responding,
    RunningTool {
        name: String,
    },
    WaitingForApproval {
        request_id: String,
    },
    TurnCompleted {
        session: SessionIdentity,
    },
    SessionEnded,
    RecoverableError {
        message: String,
        session: Option<SessionIdentity>,
    },
}

impl AgentLifecycle {
    pub(crate) fn accepts_prompt(&self) -> bool {
        matches!(
            self,
            AgentLifecycle::Ready | AgentLifecycle::TurnCompleted { .. }
        )
    }

    pub(crate) fn can_resume(&self) -> bool {
        matches!(
            self,
            AgentLifecycle::TurnCompleted { .. }
                | AgentLifecycle::RecoverableError {
                    session: Some(_),
                    ..
                }
        )
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
