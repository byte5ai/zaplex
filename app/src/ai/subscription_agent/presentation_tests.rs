use super::*;
use crate::ai::subscription_agent::{
    AccountIdentity, HostIdentity, InstallationIdentity, ModelCapability, SubscriptionAgent,
    SubscriptionLocationPreference,
};
use std::path::PathBuf;

fn isolated(value: &str) -> String {
    format!("\u{2068}{value}\u{2069}")
}

fn assert_policy(
    lifecycle: AgentLifecycle,
    status: &str,
    accepts_prompt: bool,
    actions: &[ConversationAction],
) {
    crate::i18n::init(Some("en"));
    let presentation = ConversationPresentation::for_lifecycle(&lifecycle);
    assert_eq!(presentation.status, status);
    assert_eq!(presentation.composer.accepts_prompt(), accepts_prompt);
    assert_eq!(presentation.actions, actions);
    assert_eq!(
        presentation.composer.disabled_reason().is_some(),
        !accepts_prompt
    );
}

#[test]
fn no_agent_installed_presentation_requires_setup() {
    assert_policy(
        AgentLifecycle::NoAgentInstalled,
        "No supported agent installed",
        false,
        &[
            ConversationAction::OpenAgentSettings,
            ConversationAction::BackToShell,
        ],
    );
}

#[test]
fn not_signed_in_presentation_requires_authentication() {
    assert_policy(
        AgentLifecycle::NotSignedIn {
            agent: SubscriptionAgent::ClaudeCode,
        },
        &format!("{} is not signed in", isolated("Claude Code")),
        false,
        &[
            ConversationAction::OpenAgentSettings,
            ConversationAction::BackToShell,
        ],
    );
}

#[test]
fn ready_presentation_accepts_a_prompt() {
    assert_policy(AgentLifecycle::Ready, "Ready", true, &[]);
}

#[test]
fn selection_required_presentation_blocks_a_prompt_without_starting() {
    assert_policy(
        AgentLifecycle::SelectionRequired,
        "Selection required",
        false,
        &[],
    );
    let presentation = ConversationPresentation::for_lifecycle(&AgentLifecycle::SelectionRequired);
    assert_eq!(
        presentation.detail.as_deref(),
        Some("Choose an agent, account, or model before sending a prompt.")
    );
}

#[test]
fn starting_presentation_blocks_a_prompt() {
    assert_policy(AgentLifecycle::Starting, "Starting", false, &[]);
}

#[test]
fn responding_presentation_blocks_a_prompt() {
    assert_policy(AgentLifecycle::Responding, "Responding", false, &[]);
}

#[test]
fn running_tool_presentation_names_the_tool() {
    assert_policy(
        AgentLifecycle::RunningTool {
            name: "terminal command".to_string(),
        },
        &format!("Running {}", isolated("terminal command")),
        false,
        &[],
    );
}

#[test]
fn approval_presentation_exposes_only_approval() {
    assert_policy(
        AgentLifecycle::WaitingForApproval {
            request_id: "approval-1".to_string(),
        },
        "Waiting for approval",
        false,
        &[ConversationAction::ResolveApproval],
    );
}

#[test]
fn completed_presentation_can_resume_the_recorded_session() {
    assert_policy(
        AgentLifecycle::TurnCompleted {
            session: SessionIdentity::Codex("thread-1".to_string()),
        },
        "Turn complete",
        true,
        &[
            ConversationAction::Resume,
            ConversationAction::Restart,
            ConversationAction::End,
        ],
    );
}

#[test]
fn ended_presentation_offers_new_conversation_or_shell() {
    assert_policy(
        AgentLifecycle::SessionEnded,
        "Session ended",
        false,
        &[
            ConversationAction::NewConversation,
            ConversationAction::BackToShell,
        ],
    );
}

#[test]
fn recoverable_error_presentation_preserves_a_safe_diagnostic() {
    crate::i18n::init(Some("en"));
    let lifecycle = AgentLifecycle::RecoverableError {
        message: format!("connection\nfailed {}", "x".repeat(200)),
        session: Some(SessionIdentity::ClaudeCode("session-1".to_string())),
    };
    let presentation = ConversationPresentation::for_lifecycle(&lifecycle);
    assert_eq!(presentation.status, "Agent needs attention");
    assert_eq!(presentation.composer.accepts_prompt(), false);
    assert_eq!(
        presentation.actions,
        [
            ConversationAction::Resume,
            ConversationAction::Restart,
            ConversationAction::End,
        ]
    );
    let detail = presentation.detail.expect("error detail");
    assert_eq!(detail.contains('\n'), false);
    assert_eq!(detail.chars().count(), MAX_DIAGNOSTIC_CHARS);
    assert!(detail.ends_with('…'));
}

#[test]
fn recoverable_initial_error_offers_retry_without_discarding_the_conversation() {
    let presentation = ConversationPresentation::for_lifecycle(&AgentLifecycle::RecoverableError {
        message: "Remote discovery timed out".to_string(),
        session: None,
    });

    assert_eq!(
        presentation.actions,
        [
            ConversationAction::Retry,
            ConversationAction::NewConversation,
            ConversationAction::BackToShell,
        ]
    );
}

#[test]
fn identity_fields_are_stable_and_omit_a_missing_session() {
    crate::i18n::init(Some("en"));
    let target = target_with_directory("/a/very/long/project/directory/that/may/wrap");
    let fields = conversation_identity_fields(&target, None, &AgentLifecycle::Ready);
    assert_eq!(
        fields
            .iter()
            .map(|field| field.label.as_str())
            .collect::<Vec<_>>(),
        ["Agent", "Account", "Host", "Directory", "Model", "Status"]
    );
    assert_eq!(fields[0].value, "Codex");
    assert_eq!(
        fields[1].value,
        format!("{} · ID {}", isolated("Work account"), isolated("work"))
    );
    assert_eq!(
        fields[2].value,
        format!("{} · ID {}", isolated("Local machine"), isolated("local"))
    );
    assert_eq!(
        fields[3].value,
        target.working_directory.display().to_string()
    );
    assert_eq!(fields[4].value, "GPT-5 · ID gpt-5");
}

#[test]
fn model_identity_names_alias_launch_id_and_resolved_version() {
    crate::i18n::init(Some("en"));
    let mut target = target_with_directory("/project");
    target.model.id = "sonnet".to_string();
    target.model.display_name = "Claude Sonnet".to_string();
    target.model.resolved_model = Some("claude-sonnet-4-5-20250929".to_string());

    assert_eq!(
        model_identity_label(&target.model),
        format!(
            "Claude Sonnet · ID {} · resolved {}",
            isolated("sonnet"),
            isolated("claude-sonnet-4-5-20250929")
        )
    );
    let fields = conversation_identity_fields(&target, None, &AgentLifecycle::Ready);
    assert_eq!(fields[4].value, model_identity_label(&target.model));
}

#[test]
fn model_identity_does_not_duplicate_equal_names_or_resolved_ids() {
    crate::i18n::init(Some("en"));
    let mut model = target_with_directory("/project").model;
    model.display_name = model.id.clone();
    model.resolved_model = Some(model.id.clone());

    assert_eq!(
        model_identity_label(&model),
        format!("ID {}", isolated("gpt-5"))
    );
}

#[test]
fn identity_fields_include_the_exact_session_for_resume() {
    crate::i18n::init(Some("en"));
    let target = target_with_directory("/project");
    let session = SessionIdentity::Codex("thread-42".to_string());
    let fields = conversation_identity_fields(
        &target,
        Some(&session),
        &AgentLifecycle::TurnCompleted {
            session: session.clone(),
        },
    );
    assert_eq!(fields[5].label, "Session");
    assert_eq!(fields[5].value, format!("Codex {}", isolated("thread-42")));
    assert_eq!(fields[6].value, "Turn complete");
}

#[test]
fn presentations_do_not_leak_between_conversations() {
    crate::i18n::init(Some("en"));
    let first = ConversationPresentation::for_lifecycle(&AgentLifecycle::Responding);
    let second = ConversationPresentation::for_lifecycle(&AgentLifecycle::SessionEnded);
    assert_eq!(first.status, "Responding");
    assert_eq!(first.actions, []);
    assert_eq!(second.status, "Session ended");
    assert_eq!(
        second.actions,
        [
            ConversationAction::NewConversation,
            ConversationAction::BackToShell,
        ]
    );
}

#[test]
fn incompatible_cli_and_remote_offline_have_concrete_statuses() {
    crate::i18n::init(Some("en"));
    let incompatible = ConversationPresentation::for_lifecycle(&AgentLifecycle::RecoverableError {
        message: "installed Claude Code does not support the required structured protocol"
            .to_string(),
        session: None,
    });
    assert_eq!(incompatible.status, "Incompatible agent CLI");

    let offline = ConversationPresentation::for_lifecycle(&AgentLifecycle::RecoverableError {
        message: "ssh connection refused while opening remote host".to_string(),
        session: None,
    });
    assert_eq!(offline.status, "Remote host unavailable");

    let disconnected = ConversationPresentation::for_lifecycle(&AgentLifecycle::RecoverableError {
        message: "remote host host-1 is not connected".to_string(),
        session: None,
    });
    assert_eq!(disconnected.status, "Remote host unavailable");

    let discovery = ConversationPresentation::for_lifecycle(&AgentLifecycle::RecoverableError {
        message: "subscription agent capability discovery timed out".to_string(),
        session: None,
    });
    assert_eq!(discovery.status, "Model discovery failed");
}

#[test]
fn location_changes_are_available_only_when_no_turn_is_active() {
    for lifecycle in [
        AgentLifecycle::NoAgentInstalled,
        AgentLifecycle::NotSignedIn {
            agent: SubscriptionAgent::Codex,
        },
        AgentLifecycle::Ready,
        AgentLifecycle::SelectionRequired,
        AgentLifecycle::TurnCompleted {
            session: SessionIdentity::Codex("thread-1".to_string()),
        },
        AgentLifecycle::RecoverableError {
            message: "offline".to_string(),
            session: None,
        },
    ] {
        assert!(lifecycle.can_change_location());
    }

    for lifecycle in [
        AgentLifecycle::Starting,
        AgentLifecycle::Responding,
        AgentLifecycle::RunningTool {
            name: "command".to_string(),
        },
        AgentLifecycle::WaitingForApproval {
            request_id: "approval-1".to_string(),
        },
        AgentLifecycle::SessionEnded,
    ] {
        assert!(!lifecycle.can_change_location());
    }
}

#[test]
fn selected_location_exposes_stable_host_id_and_exact_directory() {
    crate::i18n::init(Some("en"));
    let location = SubscriptionLocationPreference {
        host: HostIdentity {
            id: "daemon-42".to_string(),
            display_name: "devhost".to_string(),
        },
        working_directory: PathBuf::from("/srv/project"),
    };

    assert_eq!(
        location_identity_label(&location),
        format!(
            "Host {} · Directory {}",
            isolated(&format!(
                "{} · ID {}",
                isolated("devhost"),
                isolated("daemon-42")
            )),
            isolated("/srv/project")
        )
    );
}

fn target_with_directory(directory: &str) -> SubscriptionTarget {
    SubscriptionTarget {
        installation: InstallationIdentity {
            agent: SubscriptionAgent::Codex,
            host: HostIdentity {
                id: "local".to_string(),
                display_name: "Local machine".to_string(),
            },
            account: AccountIdentity {
                id: "work".to_string(),
                display_name: "Work account".to_string(),
                provider_account_id: None,
                config_dir: None,
            },
            executable: PathBuf::from("codex"),
            version: "1.0.0".to_string(),
        },
        working_directory: PathBuf::from(directory),
        model: ModelCapability {
            id: "gpt-5".to_string(),
            display_name: "GPT-5".to_string(),
            description: None,
            resolved_model: None,
            is_default: true,
            supported_efforts: vec![],
            default_effort: None,
            context_window: None,
        },
        effort: None,
    }
}
