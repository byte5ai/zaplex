use super::{AgentLifecycle, SessionIdentity, SubscriptionTarget};

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
                "No supported agent installed".to_string(),
                Some("Install Claude Code or Codex, then return to this conversation.".to_string()),
                vec![
                    ConversationAction::OpenAgentSettings,
                    ConversationAction::BackToShell,
                ],
            ),
            AgentLifecycle::NotSignedIn { agent } => (
                format!("{} is not signed in", agent.display_name()),
                Some(format!(
                    "Sign in with {} before sending this prompt.",
                    agent.display_name()
                )),
                vec![
                    ConversationAction::OpenAgentSettings,
                    ConversationAction::BackToShell,
                ],
            ),
            AgentLifecycle::Ready => (
                "Ready".to_string(),
                Some("The selected agent is ready for your next prompt.".to_string()),
                vec![],
            ),
            AgentLifecycle::Starting => (
                "Starting".to_string(),
                Some("The selected agent session is starting.".to_string()),
                vec![],
            ),
            AgentLifecycle::Responding => (
                "Responding".to_string(),
                Some("The selected agent is responding.".to_string()),
                vec![],
            ),
            AgentLifecycle::RunningTool { name } => (
                format!("Running {name}"),
                Some(format!("The selected agent is running {name}.")),
                vec![],
            ),
            AgentLifecycle::WaitingForApproval { .. } => (
                "Waiting for approval".to_string(),
                Some("Review the requested action before the agent can continue.".to_string()),
                vec![ConversationAction::ResolveApproval],
            ),
            AgentLifecycle::TurnCompleted { .. } => (
                "Turn complete".to_string(),
                Some("Continue the recorded session or start over.".to_string()),
                vec![
                    ConversationAction::Resume,
                    ConversationAction::Restart,
                    ConversationAction::End,
                ],
            ),
            AgentLifecycle::SessionEnded => (
                "Session ended".to_string(),
                Some("This session cannot accept more prompts.".to_string()),
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
                        ConversationAction::NewConversation,
                        ConversationAction::BackToShell,
                    ],
                };
                (
                    "Agent needs attention".to_string(),
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
    pub(crate) label: &'static str,
    pub(crate) value: String,
}

pub(crate) fn conversation_identity_fields(
    target: &SubscriptionTarget,
    session: Option<&SessionIdentity>,
    lifecycle: &AgentLifecycle,
) -> Vec<ConversationIdentityField> {
    let mut fields = vec![
        ConversationIdentityField {
            label: "Agent",
            value: target.installation.agent.display_name().to_string(),
        },
        ConversationIdentityField {
            label: "Account",
            value: target.installation.account.display_name.clone(),
        },
        ConversationIdentityField {
            label: "Host",
            value: target.installation.host.display_name.clone(),
        },
        ConversationIdentityField {
            label: "Directory",
            value: target.working_directory.display().to_string(),
        },
        ConversationIdentityField {
            label: "Model",
            value: target.model.display_name.clone(),
        },
    ];
    if let Some(session) = session {
        fields.push(ConversationIdentityField {
            label: "Session",
            value: session_label(session),
        });
    }
    fields.push(ConversationIdentityField {
        label: "Status",
        value: ConversationPresentation::for_lifecycle(lifecycle).status,
    });
    fields
}

fn composer_disabled_reason(lifecycle: &AgentLifecycle) -> String {
    match lifecycle {
        AgentLifecycle::NoAgentInstalled => {
            "Install a supported agent before sending a prompt.".to_string()
        }
        AgentLifecycle::NotSignedIn { agent } => {
            format!(
                "Sign in with {} before sending a prompt.",
                agent.display_name()
            )
        }
        AgentLifecycle::Ready => String::new(),
        AgentLifecycle::Starting => "Wait for the agent session to start.".to_string(),
        AgentLifecycle::Responding => "Wait for the current response to finish.".to_string(),
        AgentLifecycle::RunningTool { name } => format!("Wait for {name} to finish."),
        AgentLifecycle::WaitingForApproval { .. } => {
            "Resolve the approval request before sending another prompt.".to_string()
        }
        AgentLifecycle::TurnCompleted { .. } => String::new(),
        AgentLifecycle::SessionEnded => {
            "Start a new conversation or return to the shell.".to_string()
        }
        AgentLifecycle::RecoverableError { .. } => {
            "Recover or restart the agent session before sending another prompt.".to_string()
        }
    }
}

fn safe_diagnostic(message: &str) -> String {
    let normalized = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return "The agent reported a recoverable error.".to_string();
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
        SessionIdentity::ClaudeCode(id) => format!("Claude {id}"),
        SessionIdentity::Codex(id) => format!("Codex {id}"),
    }
}

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod tests;
