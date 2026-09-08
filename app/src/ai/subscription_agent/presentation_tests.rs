use super::*;
use crate::ai::subscription_agent::{
    AccountIdentity, HostIdentity, InstallationIdentity, ModelCapability, SubscriptionAgent,
};
use std::path::PathBuf;

fn assert_policy(
    lifecycle: AgentLifecycle,
    status: &str,
    accepts_prompt: bool,
    actions: &[ConversationAction],
) {
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
        "Claude Code is not signed in",
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
        "Running terminal command",
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
fn identity_fields_are_stable_and_omit_a_missing_session() {
    let target = target_with_directory("/a/very/long/project/directory/that/may/wrap");
    let fields = conversation_identity_fields(&target, None, &AgentLifecycle::Ready);
    assert_eq!(
        fields.iter().map(|field| field.label).collect::<Vec<_>>(),
        ["Agent", "Account", "Host", "Directory", "Model", "Status"]
    );
    assert_eq!(
        fields[3].value,
        target.working_directory.display().to_string()
    );
}

#[test]
fn identity_fields_include_the_exact_session_for_resume() {
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
    assert_eq!(fields[5].value, "Codex thread-42");
    assert_eq!(fields[6].value, "Turn complete");
}

#[test]
fn presentations_do_not_leak_between_conversations() {
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
