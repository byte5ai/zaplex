use super::{
    AccountIdentity, AgentLifecycle, ModelCapability, SessionIdentity, SubscriptionTarget,
};

const MAX_DIAGNOSTIC_CHARS: usize = 160;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ComposerPolicy {
    Enabled,
    Disabled { reason: String },
}

impl ComposerPolicy {
    pub(crate) fn accepts_prompt(&self) -> bool {
        match self {
            ComposerPolicy::Enabled => true,
            ComposerPolicy::Disabled { .. } => false,
        }
    }

    pub(crate) fn disabled_reason(&self) -> Option<&str> {
        match self {
            ComposerPolicy::Enabled => None,
            ComposerPolicy::Disabled { reason } => Some(reason),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ConversationAction {
    OpenAgentSettings,
    ResolveApproval,
    Retry,
    Resume,
    Restart,
    End,
    NewConversation,
    BackToShell,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConversationPresentation {
    pub(crate) status: String,
    pub(crate) detail: Option<String>,
    pub(crate) composer: ComposerPolicy,
    pub(crate) actions: Vec<ConversationAction>,
}

impl ConversationPresentation {
    pub(crate) fn for_lifecycle(lifecycle: &AgentLifecycle) -> Self {
        let composer = if lifecycle.accepts_prompt() {
            ComposerPolicy::Enabled
        } else {
            ComposerPolicy::Disabled {
                reason: composer_disabled_reason(lifecycle),
            }
        };

        let (status, detail, actions) = match lifecycle {
            AgentLifecycle::NoAgentInstalled => (
                crate::t!("ai-footer-subscription-status-no-agent"),
                Some(crate::t!("ai-footer-subscription-detail-no-agent")),
                vec![
                    ConversationAction::OpenAgentSettings,
                    ConversationAction::BackToShell,
                ],
            ),
            AgentLifecycle::NotSignedIn { agent } => (
                crate::t!(
                    "ai-footer-subscription-status-not-signed-in",
                    agent = agent.display_name()
                ),
                Some(crate::t!(
                    "ai-footer-subscription-detail-not-signed-in",
                    agent = agent.display_name()
                )),
                vec![
                    ConversationAction::OpenAgentSettings,
                    ConversationAction::BackToShell,
                ],
            ),
            AgentLifecycle::Ready => (
                crate::t!("ai-footer-subscription-status-ready"),
                Some(crate::t!("ai-footer-subscription-detail-ready")),
                vec![],
            ),
            AgentLifecycle::SelectionRequired => (
                crate::t!("ai-footer-subscription-selection-required"),
                Some(crate::t!(
                    "ai-footer-subscription-selection-required-detail"
                )),
                vec![],
            ),
            AgentLifecycle::Starting => (
                crate::t!("ai-footer-subscription-status-starting"),
                Some(crate::t!("ai-footer-subscription-detail-starting")),
                vec![],
            ),
            AgentLifecycle::Responding => (
                crate::t!("ai-footer-subscription-status-responding"),
                Some(crate::t!("ai-footer-subscription-detail-responding")),
                vec![],
            ),
            AgentLifecycle::RunningTool { name } => (
                crate::t!("ai-footer-subscription-status-running-tool", tool = name),
                Some(crate::t!(
                    "ai-footer-subscription-detail-running-tool",
                    tool = name
                )),
                vec![],
            ),
            AgentLifecycle::WaitingForApproval { .. } => (
                crate::t!("ai-footer-subscription-status-waiting-for-approval"),
                Some(crate::t!(
                    "ai-footer-subscription-detail-waiting-for-approval"
                )),
                vec![ConversationAction::ResolveApproval],
            ),
            AgentLifecycle::TurnCompleted { .. } => (
                crate::t!("ai-footer-subscription-status-turn-complete"),
                Some(crate::t!("ai-footer-subscription-detail-turn-complete")),
                vec![
                    ConversationAction::Resume,
                    ConversationAction::Restart,
                    ConversationAction::End,
                ],
            ),
            AgentLifecycle::SessionEnded => (
                crate::t!("ai-footer-subscription-status-session-ended"),
                Some(crate::t!("ai-footer-subscription-detail-session-ended")),
                vec![
                    ConversationAction::NewConversation,
                    ConversationAction::BackToShell,
                ],
            ),
            AgentLifecycle::RecoverableError { message, session } => {
                let actions = match session {
                    Some(SessionIdentity::ClaudeCode(_)) | Some(SessionIdentity::Codex(_)) => vec![
                        ConversationAction::Resume,
                        ConversationAction::Restart,
                        ConversationAction::End,
                    ],
                    None => vec![
                        ConversationAction::Retry,
                        ConversationAction::NewConversation,
                        ConversationAction::BackToShell,
                    ],
                };
                (
                    recoverable_status(message),
                    Some(safe_diagnostic(message)),
                    actions,
                )
            }
        };

        Self {
            status,
            detail,
            composer,
            actions,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConversationIdentityField {
    pub(crate) label: String,
    pub(crate) value: String,
}

/// A model label that keeps the CLI argument and the concrete resolved model
/// visible. Provider display names are descriptive, but are not stable launch
/// identities: Claude may report a family alias such as `sonnet` while also
/// resolving it to a versioned model.
pub(crate) fn model_identity_label(model: &ModelCapability) -> String {
    let mut parts = Vec::new();
    if !model.display_name.is_empty() && model.display_name != model.id {
        parts.push(model.display_name.clone());
    }
    parts.push(crate::t!(
        "ai-footer-subscription-identity-id",
        id = model.id.clone()
    ));
    if let Some(resolved) = model
        .resolved_model
        .as_deref()
        .filter(|resolved| !resolved.is_empty() && *resolved != model.id.as_str())
    {
        parts.push(crate::t!(
            "ai-footer-subscription-model-resolved",
            model = resolved
        ));
    }
    parts.join(" · ")
}

pub(crate) fn account_identity_label(account: &AccountIdentity) -> String {
    stable_identity_label(&account.display_name, &account.id)
}

pub(crate) fn host_identity_label(host: &super::HostIdentity) -> String {
    stable_identity_label(&host.display_name, &host.id)
}

pub(crate) fn location_identity_label(location: &super::SubscriptionLocationPreference) -> String {
    crate::t!(
        "ai-footer-subscription-location",
        host = host_identity_label(&location.host),
        directory = location.working_directory.display().to_string()
    )
}

fn stable_identity_label(display_name: &str, id: &str) -> String {
    if display_name.is_empty() || display_name == id {
        crate::t!("ai-footer-subscription-identity-id", id = id)
    } else {
        crate::t!(
            "ai-footer-subscription-identity-named-id",
            name = display_name,
            id = id
        )
    }
}

pub(crate) fn conversation_identity_fields(
    target: &SubscriptionTarget,
    session: Option<&SessionIdentity>,
    lifecycle: &AgentLifecycle,
) -> Vec<ConversationIdentityField> {
    let mut fields = vec![
        ConversationIdentityField {
            label: crate::t!("ai-footer-subscription-field-agent"),
            value: target.installation.agent.display_name().to_string(),
        },
        ConversationIdentityField {
            label: crate::t!("ai-footer-subscription-field-account"),
            value: account_identity_label(&target.installation.account),
        },
        ConversationIdentityField {
            label: crate::t!("ai-footer-subscription-field-host"),
            value: host_identity_label(&target.installation.host),
        },
        ConversationIdentityField {
            label: crate::t!("ai-footer-subscription-field-directory"),
            value: target.working_directory.display().to_string(),
        },
        ConversationIdentityField {
            label: crate::t!("ai-footer-subscription-field-model"),
            value: model_identity_label(&target.model),
        },
    ];
    if let Some(session) = session {
        fields.push(ConversationIdentityField {
            label: crate::t!("ai-footer-subscription-field-session"),
            value: session_label(session),
        });
    }
    fields.push(ConversationIdentityField {
        label: crate::t!("ai-footer-subscription-field-status"),
        value: ConversationPresentation::for_lifecycle(lifecycle).status,
    });
    fields
}

fn recoverable_status(message: &str) -> String {
    let message = message.to_ascii_lowercase();
    if message.contains("does not support the required")
        || message.contains("incompatible cli")
        || message.contains("unsupported cli")
    {
        crate::t!("ai-footer-subscription-status-incompatible-cli")
    } else if (message.contains("remote host")
        && (message.contains("not connected")
            || message.contains("offline")
            || message.contains("unreachable")))
        || (message.contains("ssh")
            && (message.contains("timed out")
                || message.contains("connection refused")
                || message.contains("unreachable")
                || message.contains("could not resolve hostname")))
    {
        crate::t!("ai-footer-subscription-status-remote-unavailable")
    } else if message.contains("model discovery")
        || message.contains("capability discovery")
        || message.contains("did not report models")
        || message.contains("cannot list models")
    {
        crate::t!("ai-footer-subscription-status-model-discovery-failed")
    } else {
        crate::t!("ai-footer-subscription-status-needs-attention")
    }
}

fn composer_disabled_reason(lifecycle: &AgentLifecycle) -> String {
    match lifecycle {
        AgentLifecycle::NoAgentInstalled => {
            crate::t!("ai-footer-subscription-disabled-no-agent")
        }
        AgentLifecycle::NotSignedIn { agent } => {
            crate::t!(
                "ai-footer-subscription-disabled-not-signed-in",
                agent = agent.display_name()
            )
        }
        AgentLifecycle::Ready => String::new(),
        AgentLifecycle::SelectionRequired => {
            crate::t!("ai-footer-subscription-selection-required-disabled")
        }
        AgentLifecycle::Starting => crate::t!("ai-footer-subscription-disabled-starting"),
        AgentLifecycle::Responding => crate::t!("ai-footer-subscription-disabled-responding"),
        AgentLifecycle::RunningTool { name } => {
            crate::t!("ai-footer-subscription-disabled-running-tool", tool = name)
        }
        AgentLifecycle::WaitingForApproval { .. } => {
            crate::t!("ai-footer-subscription-disabled-waiting-for-approval")
        }
        AgentLifecycle::TurnCompleted { .. } => String::new(),
        AgentLifecycle::SessionEnded => {
            crate::t!("ai-footer-subscription-disabled-session-ended")
        }
        AgentLifecycle::RecoverableError { .. } => {
            crate::t!("ai-footer-subscription-disabled-recoverable-error")
        }
    }
}

fn safe_diagnostic(message: &str) -> String {
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return crate::t!("ai-footer-subscription-recoverable-error-fallback");
    }
    if normalized.chars().count() <= MAX_DIAGNOSTIC_CHARS {
        return normalized;
    }
    let mut bounded = normalized
        .chars()
        .take(MAX_DIAGNOSTIC_CHARS - 1)
        .collect::<String>();
    bounded.push('…');
    bounded
}

fn session_label(session: &SessionIdentity) -> String {
    match session {
        SessionIdentity::ClaudeCode(id) => {
            crate::t!("ai-footer-subscription-session-claude", id = id)
        }
        SessionIdentity::Codex(id) => {
            crate::t!("ai-footer-subscription-session-codex", id = id)
        }
    }
}

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod tests;
