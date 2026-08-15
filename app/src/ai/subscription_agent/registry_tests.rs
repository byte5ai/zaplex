use super::*;
use crate::ai::subscription_agent::{
    AccountIdentity, HostIdentity, InstallationIdentity, ModelCapability, SubscriptionAgent,
};

fn target() -> SubscriptionTarget {
    SubscriptionTarget {
        installation: InstallationIdentity {
            agent: SubscriptionAgent::Codex,
            host: HostIdentity {
                id: "local".to_string(),
                display_name: "Local".to_string(),
            },
            account: AccountIdentity {
                id: "account-1".to_string(),
                display_name: "Account".to_string(),
                config_dir: None,
            },
            executable: "codex".into(),
            version: "0.146.0".to_string(),
        },
        working_directory: "/workspace".into(),
        model: ModelCapability {
            id: "reported-model".to_string(),
            display_name: "Reported model".to_string(),
            description: None,
            resolved_model: None,
            is_default: true,
            supported_efforts: Vec::new(),
            default_effort: None,
            context_window: None,
        },
        effort: None,
    }
}

#[test]
fn stores_and_removes_native_session_identity() {
    let registry = SubscriptionSessionRegistry::default();
    registry.store(
        "conversation-1".to_string(),
        target(),
        SessionIdentity::Codex("thread-1".to_string()),
    );

    let stored = registry.get("conversation-1").unwrap();
    assert_eq!(
        stored.session,
        SessionIdentity::Codex("thread-1".to_string())
    );
    assert_eq!(stored.target.model.id, "reported-model");

    registry.remove("conversation-1");
    assert_eq!(registry.get("conversation-1").is_none(), true);
    assert_eq!(
        registry.lifecycle("conversation-1"),
        Some(AgentLifecycle::SessionEnded)
    );
}

#[test]
fn restart_discards_native_identity_but_keeps_target_ready() {
    let registry = SubscriptionSessionRegistry::default();
    let target = target();
    registry.set_target("conversation-1", target.clone());
    registry.store(
        "conversation-1".to_string(),
        target,
        SessionIdentity::Codex("thread-1".to_string()),
    );

    registry.restart("conversation-1");

    assert_eq!(registry.get("conversation-1").is_none(), true);
    assert_eq!(registry.target("conversation-1").is_some(), true);
    assert_eq!(
        registry.lifecycle("conversation-1"),
        Some(AgentLifecycle::Ready)
    );
}

#[test]
fn approvals_are_resolved_by_conversation_and_native_request() {
    futures_lite::future::block_on(async {
        let registry = SubscriptionSessionRegistry::default();
        let receiver =
            registry.register_approval("conversation-1".to_string(), "approval-1".to_string());

        assert_eq!(
            registry.resolve_approval(
                "conversation-1",
                "approval-1",
                ApprovalDecision::AllowForSession,
            ),
            true
        );
        assert_eq!(
            receiver
                .await
                .expect("approval sender must remain connected"),
            ApprovalDecision::AllowForSession
        );
    });
}

#[test]
fn selecting_an_agent_remembers_it_and_clears_the_pending_choice() {
    let registry = SubscriptionSessionRegistry::default();
    registry.set_agent_choices(
        "conversation-1",
        vec![SubscriptionAgent::ClaudeCode, SubscriptionAgent::Codex],
    );

    registry.select_agent("conversation-1", SubscriptionAgent::ClaudeCode);

    assert_eq!(
        registry.preferences().agent,
        Some(SubscriptionAgent::ClaudeCode)
    );
    assert_eq!(registry.agent_choices("conversation-1"), Vec::new());
    assert_eq!(
        registry.lifecycle("conversation-1"),
        Some(AgentLifecycle::Ready)
    );
}

#[test]
fn selecting_a_reported_model_remembers_exact_id_and_default_effort() {
    let registry = SubscriptionSessionRegistry::default();
    let mut reported_model = target().model;
    reported_model.default_effort = Some("high".to_string());
    registry.set_model_choices("conversation-1", vec![reported_model]);

    assert_eq!(
        registry.select_model("conversation-1", "reported-model"),
        true
    );
    assert_eq!(
        registry.preferences().model_id.as_deref(),
        Some("reported-model")
    );
    assert_eq!(registry.preferences().effort.as_deref(), Some("high"));
    assert_eq!(registry.model_choices("conversation-1"), Vec::new());
}
