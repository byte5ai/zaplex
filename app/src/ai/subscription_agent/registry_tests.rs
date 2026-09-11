use super::*;
use crate::ai::subscription_agent::{
    AccountIdentity, ApprovalDecision, HostIdentity, InstallationIdentity, ModelCapability,
    SubscriptionAgent,
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
                provider_account_id: None,
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

fn claude_target() -> SubscriptionTarget {
    let mut target = target();
    target.installation.agent = SubscriptionAgent::ClaudeCode;
    target.installation.account.id = "account-2".to_string();
    target.installation.account.display_name = "Claude account".to_string();
    target.installation.executable = "claude".into();
    target.model.id = "claude-model".to_string();
    target.model.display_name = "Claude model".to_string();
    target.effort = Some("high".to_string());
    target
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
fn same_native_request_id_is_routed_to_the_matching_conversation() {
    futures_lite::future::block_on(async {
        let registry = SubscriptionSessionRegistry::default();
        let claude = registry.register_approval(
            "claude-conversation".to_string(),
            "shared-request-id".to_string(),
        );
        let codex = registry.register_approval(
            "codex-conversation".to_string(),
            "shared-request-id".to_string(),
        );

        assert!(registry.resolve_approval(
            "codex-conversation",
            "shared-request-id",
            ApprovalDecision::Cancel,
        ));
        assert!(registry.resolve_approval(
            "claude-conversation",
            "shared-request-id",
            ApprovalDecision::Allow,
        ));

        assert_eq!(claude.await.unwrap(), ApprovalDecision::Allow);
        assert_eq!(codex.await.unwrap(), ApprovalDecision::Cancel);
    });
}

#[test]
fn selecting_an_agent_remembers_it_and_clears_the_pending_choice() {
    let registry = SubscriptionSessionRegistry::default();
    registry.set_agent_choices(
        "conversation-1",
        vec![SubscriptionAgent::ClaudeCode, SubscriptionAgent::Codex],
    );
    registry.set_lifecycle("conversation-1", AgentLifecycle::SelectionRequired);

    assert!(registry.select_agent("conversation-1", SubscriptionAgent::ClaudeCode));

    assert_eq!(
        registry.preferences("conversation-1").agent,
        Some(SubscriptionAgent::ClaudeCode)
    );
    assert_eq!(registry.agent_choices("conversation-1"), Vec::new());
    assert_eq!(
        registry.lifecycle("conversation-1"),
        Some(AgentLifecycle::Ready)
    );
}

#[test]
fn agent_selection_rejects_agents_not_offered_for_this_conversation() {
    let registry = SubscriptionSessionRegistry::default();
    registry.set_agent_choices("conversation-1", vec![SubscriptionAgent::ClaudeCode]);
    registry.set_lifecycle("conversation-1", AgentLifecycle::SelectionRequired);

    assert!(!registry.select_agent("conversation-1", SubscriptionAgent::Codex));
    assert_eq!(registry.preferences("conversation-1").agent, None);
    assert_eq!(registry.agent_choices("conversation-1").len(), 1);
}

#[test]
fn route_defaults_apply_only_to_conversations_that_have_not_started_routing() {
    let registry = SubscriptionSessionRegistry::default();
    assert_eq!(
        registry.preferences("existing-conversation"),
        RoutePreferences::default()
    );
    registry.set_agent_choices("changed-conversation", vec![SubscriptionAgent::ClaudeCode]);
    registry.set_lifecycle("changed-conversation", AgentLifecycle::SelectionRequired);
    assert!(registry.select_agent("changed-conversation", SubscriptionAgent::ClaudeCode));

    assert_eq!(
        registry.preferences("existing-conversation"),
        RoutePreferences::default()
    );
    assert_eq!(
        registry.preferences("new-conversation").agent,
        Some(SubscriptionAgent::ClaudeCode)
    );
}

#[test]
fn a_new_routing_choice_replaces_the_stale_display_target() {
    let registry = SubscriptionSessionRegistry::default();
    registry.set_target("conversation-1", target());
    registry.set_model_choices("conversation-1", vec![target().model]);

    assert_eq!(registry.target("conversation-1"), None);
    assert_eq!(registry.model_choices("conversation-1").len(), 1);
}

#[test]
fn selecting_a_reported_account_clears_model_preferences() {
    let registry = SubscriptionSessionRegistry::default();
    let account = AccountIdentity {
        id: "account-2".to_string(),
        display_name: "Second account".to_string(),
        provider_account_id: None,
        config_dir: None,
    };
    registry.set_account_choices("conversation-1", vec![account.clone()]);
    registry.set_lifecycle("conversation-1", AgentLifecycle::SelectionRequired);

    assert!(registry.select_account("conversation-1", &account));
    assert_eq!(
        registry.preferences("conversation-1").account_id.as_deref(),
        Some("account-2")
    );
    assert_eq!(registry.preferences("conversation-1").model_id, None);
    assert_eq!(registry.preferences("conversation-1").effort, None);
    assert_eq!(registry.account_choices("conversation-1"), Vec::new());
    assert_eq!(
        registry.lifecycle("conversation-1"),
        Some(AgentLifecycle::Ready)
    );
}

#[test]
fn selecting_an_account_preserves_its_complete_routing_identity() {
    let registry = SubscriptionSessionRegistry::default();
    let first = AccountIdentity {
        id: "shared-id".to_string(),
        display_name: "First".to_string(),
        provider_account_id: Some("provider-1".to_string()),
        config_dir: Some("/accounts/one".into()),
    };
    let second = AccountIdentity {
        id: "shared-id".to_string(),
        display_name: "Second".to_string(),
        provider_account_id: Some("provider-2".to_string()),
        config_dir: Some("/accounts/two".into()),
    };
    registry.set_account_choices("conversation-1", vec![first, second.clone()]);
    registry.set_lifecycle("conversation-1", AgentLifecycle::SelectionRequired);

    assert!(registry.select_account("conversation-1", &second));
    assert_eq!(
        registry.preferences("conversation-1").account_identity,
        Some(second)
    );
}

#[test]
fn account_selection_rejects_ids_not_offered_for_this_conversation() {
    let registry = SubscriptionSessionRegistry::default();
    let offered = AccountIdentity {
        id: "account-1".to_string(),
        display_name: "First account".to_string(),
        provider_account_id: None,
        config_dir: None,
    };
    registry.set_account_choices("conversation-1", vec![offered]);
    registry.set_lifecycle("conversation-1", AgentLifecycle::SelectionRequired);

    assert!(!registry.select_account(
        "conversation-1",
        &AccountIdentity {
            id: "account-2".to_string(),
            display_name: "Second account".to_string(),
            provider_account_id: None,
            config_dir: None,
        },
    ));
    assert_eq!(registry.preferences("conversation-1").account_id, None);
    assert_eq!(registry.account_choices("conversation-1").len(), 1);
}

#[test]
fn pending_first_prompt_survives_routing_choice_until_preflight_succeeds() {
    let registry = SubscriptionSessionRegistry::default();
    registry.remember_pending_prompt("conversation-1", "Explain this repository".to_string());
    registry.set_agent_choices("conversation-1", vec![SubscriptionAgent::Codex]);
    registry.set_lifecycle("conversation-1", AgentLifecycle::SelectionRequired);
    assert!(registry.select_agent("conversation-1", SubscriptionAgent::Codex));
    registry.set_account_choices(
        "conversation-1",
        vec![AccountIdentity {
            id: "account-1".to_string(),
            display_name: "Account".to_string(),
            provider_account_id: None,
            config_dir: None,
        }],
    );
    registry.set_lifecycle("conversation-1", AgentLifecycle::SelectionRequired);
    let account = registry.account_choices("conversation-1")[0].clone();
    assert!(registry.select_account("conversation-1", &account));
    registry.set_model_choices("conversation-1", vec![target().model]);
    registry.set_lifecycle("conversation-1", AgentLifecycle::SelectionRequired);
    assert!(registry.select_model("conversation-1", "reported-model"));

    assert_eq!(
        registry.pending_prompt("conversation-1").as_deref(),
        Some("Explain this repository")
    );
    assert_eq!(
        registry.take_pending_prompt("conversation-1").as_deref(),
        Some("Explain this repository")
    );
    assert_eq!(registry.pending_prompt("conversation-1"), None);
}

#[test]
fn selecting_a_reported_model_remembers_exact_id_and_default_effort() {
    let registry = SubscriptionSessionRegistry::default();
    let mut reported_model = target().model;
    reported_model.default_effort = Some("high".to_string());
    registry.set_model_choices("conversation-1", vec![reported_model]);
    registry.set_lifecycle("conversation-1", AgentLifecycle::SelectionRequired);

    assert_eq!(
        registry.select_model("conversation-1", "reported-model"),
        true
    );
    assert_eq!(
        registry.preferences("conversation-1").model_id.as_deref(),
        Some("reported-model")
    );
    assert_eq!(
        registry.preferences("conversation-1").effort.as_deref(),
        Some("high")
    );
    assert_eq!(registry.model_choices("conversation-1"), Vec::new());
}

#[test]
fn beginning_agent_selection_invalidates_execution_state_and_keeps_the_prompt() {
    let registry = SubscriptionSessionRegistry::default();
    let mut current_target = target();
    current_target.effort = Some("high".to_string());
    registry.remember_target("conversation-1", &current_target);
    registry.set_target("conversation-1", current_target.clone());
    registry.store(
        "conversation-1".to_string(),
        current_target,
        SessionIdentity::Codex("thread-1".to_string()),
    );
    registry.remember_pending_prompt("conversation-1", "Keep this prompt".to_string());
    registry.set_lifecycle("conversation-1", AgentLifecycle::Ready);
    let approval =
        registry.register_approval("conversation-1".to_string(), "approval-1".to_string());

    assert!(registry.begin_agent_selection("conversation-1"));

    assert_eq!(
        registry.preferences("conversation-1"),
        RoutePreferences::default()
    );
    assert_eq!(registry.target("conversation-1"), None);
    assert!(registry.get("conversation-1").is_none());
    assert!(futures_lite::future::block_on(approval).is_err());
    assert_eq!(
        registry.pending_prompt("conversation-1").as_deref(),
        Some("Keep this prompt")
    );
    assert_eq!(registry.agent_choices("conversation-1"), Vec::new());
    assert_eq!(registry.account_choices("conversation-1"), Vec::new());
    assert_eq!(registry.model_choices("conversation-1"), Vec::new());
    assert_eq!(
        registry.lifecycle("conversation-1"),
        Some(AgentLifecycle::SelectionRequired)
    );
}

#[test]
fn beginning_account_selection_keeps_only_the_exact_agent() {
    let registry = SubscriptionSessionRegistry::default();
    let current_target = claude_target();
    registry.remember_target("conversation-1", &current_target);
    registry.set_target("conversation-1", current_target);
    registry.set_lifecycle("conversation-1", AgentLifecycle::Ready);

    assert!(registry.begin_account_selection("conversation-1"));

    assert_eq!(
        registry.preferences("conversation-1"),
        RoutePreferences {
            agent: Some(SubscriptionAgent::ClaudeCode),
            account_id: None,
            require_account_choice: true,
            ..RoutePreferences::default()
        }
    );
    assert_eq!(
        registry.lifecycle("conversation-1"),
        Some(AgentLifecycle::SelectionRequired)
    );
}

#[test]
fn beginning_model_selection_keeps_the_exact_agent_and_account() {
    let registry = SubscriptionSessionRegistry::default();
    let current_target = claude_target();
    registry.remember_target("conversation-1", &current_target);
    registry.set_target("conversation-1", current_target);
    registry.set_lifecycle(
        "conversation-1",
        AgentLifecycle::TurnCompleted {
            session: SessionIdentity::ClaudeCode("session-1".to_string()),
        },
    );

    assert!(registry.begin_model_selection("conversation-1"));

    assert_eq!(
        registry.preferences("conversation-1"),
        RoutePreferences {
            agent: Some(SubscriptionAgent::ClaudeCode),
            account_id: Some("account-2".to_string()),
            account_identity: Some(claude_target().installation.account),
            require_model_choice: true,
            ..RoutePreferences::default()
        }
    );
    assert_eq!(
        registry.lifecycle("conversation-1"),
        Some(AgentLifecycle::SelectionRequired)
    );
}

#[test]
fn routing_selection_is_rejected_without_a_target_or_while_a_turn_is_active() {
    let registry = SubscriptionSessionRegistry::default();
    registry.set_lifecycle("missing-target", AgentLifecycle::Ready);
    assert!(!registry.begin_agent_selection("missing-target"));

    registry.set_target("active-conversation", target());
    registry.set_lifecycle("active-conversation", AgentLifecycle::Responding);
    assert!(!registry.begin_account_selection("active-conversation"));
    assert!(registry.target("active-conversation").is_some());
    assert_eq!(
        registry.lifecycle("active-conversation"),
        Some(AgentLifecycle::Responding)
    );
}

#[test]
fn stale_routing_choices_cannot_mutate_an_active_conversation() {
    let registry = SubscriptionSessionRegistry::default();
    registry.set_agent_choices("conversation-1", vec![SubscriptionAgent::ClaudeCode]);
    registry.set_lifecycle("conversation-1", AgentLifecycle::Responding);

    assert!(!registry.select_agent("conversation-1", SubscriptionAgent::ClaudeCode));
    assert_eq!(
        registry.preferences("conversation-1"),
        RoutePreferences::default()
    );
    assert_eq!(registry.agent_choices("conversation-1").len(), 1);
    assert_eq!(
        registry.lifecycle("conversation-1"),
        Some(AgentLifecycle::Responding)
    );
}

#[test]
fn host_change_uses_an_offered_stable_id_and_invalidates_the_old_route() {
    let registry = SubscriptionSessionRegistry::default();
    let old_target = target();
    registry.remember_target("conversation-1", &old_target);
    registry.set_target("conversation-1", old_target.clone());
    registry.store(
        "conversation-1".to_string(),
        old_target,
        SessionIdentity::Codex("thread-1".to_string()),
    );
    registry.remember_pending_prompt("conversation-1", "Keep this prompt".to_string());
    registry.set_lifecycle("conversation-1", AgentLifecycle::Ready);
    registry.set_host_choices(
        "conversation-1",
        vec![
            HostIdentity {
                id: "remote-1".to_string(),
                display_name: "devbox".to_string(),
            },
            HostIdentity {
                id: "local".to_string(),
                display_name: "Local machine".to_string(),
            },
        ],
    );

    assert!(registry.select_host_location(
        "conversation-1",
        "remote-1",
        std::path::PathBuf::from("."),
    ));
    assert_eq!(registry.target("conversation-1"), None);
    assert!(registry.get("conversation-1").is_none());
    assert_eq!(registry.preferences("conversation-1").account_id, None);
    assert_eq!(registry.preferences("conversation-1").model_id, None);
    assert_eq!(
        registry.preferences("conversation-1").agent,
        Some(SubscriptionAgent::Codex)
    );
    assert_eq!(registry.preferences("conversation-1").effort, None);
    assert_eq!(
        registry.pending_prompt("conversation-1").as_deref(),
        Some("Keep this prompt")
    );
    assert_eq!(
        registry.location_preference("conversation-1"),
        Some(SubscriptionLocationPreference {
            host: HostIdentity {
                id: "remote-1".to_string(),
                display_name: "devbox".to_string(),
            },
            working_directory: std::path::PathBuf::from("."),
        })
    );
}

#[test]
fn unknown_host_and_relative_cwd_are_rejected_without_destroying_target() {
    let registry = SubscriptionSessionRegistry::default();
    registry.set_target("conversation-1", target());
    registry.set_lifecycle("conversation-1", AgentLifecycle::Ready);
    registry.set_host_choices(
        "conversation-1",
        vec![HostIdentity {
            id: "local".to_string(),
            display_name: "Local machine".to_string(),
        }],
    );

    assert!(!registry.select_host_location(
        "conversation-1",
        "offline-remote",
        std::path::PathBuf::from("."),
    ));
    assert!(!registry
        .select_working_directory("conversation-1", std::path::PathBuf::from("relative/path"),));
    assert!(registry.target("conversation-1").is_some());
    assert_eq!(registry.location_preference("conversation-1"), None);
}

#[test]
fn inferred_location_does_not_overwrite_an_explicit_selection() {
    let registry = SubscriptionSessionRegistry::default();
    registry.remember_initial_location(
        "conversation-1",
        SubscriptionLocationPreference {
            host: HostIdentity {
                id: "remote-1".to_string(),
                display_name: "devbox".to_string(),
            },
            working_directory: std::path::PathBuf::from("/srv/project"),
        },
    );
    registry.remember_initial_location(
        "conversation-1",
        SubscriptionLocationPreference {
            host: HostIdentity {
                id: "local".to_string(),
                display_name: "Local machine".to_string(),
            },
            working_directory: std::path::PathBuf::from("/tmp"),
        },
    );

    assert_eq!(
        registry.location_preference("conversation-1"),
        Some(SubscriptionLocationPreference {
            host: HostIdentity {
                id: "remote-1".to_string(),
                display_name: "devbox".to_string(),
            },
            working_directory: std::path::PathBuf::from("/srv/project"),
        })
    );
}

#[test]
fn offline_remote_route_changes_to_local_only_after_explicit_selection() {
    let registry = SubscriptionSessionRegistry::default();
    registry.remember_initial_location(
        "conversation-1",
        SubscriptionLocationPreference {
            host: HostIdentity {
                id: "offline-remote".to_string(),
                display_name: "devbox".to_string(),
            },
            working_directory: std::path::PathBuf::from("/srv/project"),
        },
    );
    registry.set_host_choices(
        "conversation-1",
        vec![HostIdentity {
            id: "local".to_string(),
            display_name: "Local machine".to_string(),
        }],
    );
    registry.set_lifecycle(
        "conversation-1",
        AgentLifecycle::RecoverableError {
            message: "remote host offline-remote is not connected".to_string(),
            session: None,
        },
    );

    assert_eq!(
        registry
            .location_preference("conversation-1")
            .map(|location| location.host.id),
        Some("offline-remote".to_string())
    );
    assert!(registry.select_host_location(
        "conversation-1",
        "local",
        std::path::PathBuf::from("/workspace"),
    ));
    assert_eq!(
        registry.location_preference("conversation-1"),
        Some(SubscriptionLocationPreference {
            host: HostIdentity {
                id: "local".to_string(),
                display_name: "Local machine".to_string(),
            },
            working_directory: std::path::PathBuf::from("/workspace"),
        })
    );
}

#[test]
fn cwd_change_preserves_account_and_prompt_but_forces_fresh_model_and_session() {
    let registry = SubscriptionSessionRegistry::default();
    let old_target = target();
    registry.remember_target("conversation-1", &old_target);
    registry.set_target("conversation-1", old_target.clone());
    registry.store(
        "conversation-1".to_string(),
        old_target,
        SessionIdentity::Codex("thread-1".to_string()),
    );
    registry.remember_pending_prompt("conversation-1", "Keep this prompt".to_string());
    let approval =
        registry.register_approval("conversation-1".to_string(), "approval-1".to_string());
    registry.set_lifecycle(
        "conversation-1",
        AgentLifecycle::TurnCompleted {
            session: SessionIdentity::Codex("thread-1".to_string()),
        },
    );

    assert!(registry.select_working_directory(
        "conversation-1",
        std::path::PathBuf::from("/workspace/next"),
    ));
    assert_eq!(registry.target("conversation-1"), None);
    assert!(registry.get("conversation-1").is_none());
    assert_eq!(
        registry.preferences("conversation-1").account_id.as_deref(),
        Some("account-1")
    );
    assert_eq!(
        registry.preferences("conversation-1").agent,
        Some(SubscriptionAgent::Codex)
    );
    assert_eq!(registry.preferences("conversation-1").model_id, None);
    assert_eq!(registry.preferences("conversation-1").effort, None);
    assert!(futures_lite::future::block_on(approval).is_err());
    assert_eq!(
        registry.lifecycle("conversation-1"),
        Some(AgentLifecycle::Ready)
    );
    assert_eq!(
        registry.pending_prompt("conversation-1").as_deref(),
        Some("Keep this prompt")
    );
    assert_eq!(
        registry
            .location_preference("conversation-1")
            .map(|location| location.working_directory),
        Some(std::path::PathBuf::from("/workspace/next"))
    );
}

#[test]
fn cwd_change_keeps_conversation_preferences_isolated_from_new_global_defaults() {
    let registry = SubscriptionSessionRegistry::default();
    let codex_target = target();
    registry.remember_target("codex-conversation", &codex_target);
    registry.set_target("codex-conversation", codex_target.clone());
    registry.store(
        "codex-conversation".to_string(),
        codex_target,
        SessionIdentity::Codex("thread-1".to_string()),
    );
    registry.remember_pending_prompt("codex-conversation", "Keep this prompt".to_string());
    registry.set_lifecycle(
        "codex-conversation",
        AgentLifecycle::TurnCompleted {
            session: SessionIdentity::Codex("thread-1".to_string()),
        },
    );

    let claude_conversation_target = claude_target();
    registry.remember_target("claude-conversation", &claude_conversation_target);
    registry.set_target("claude-conversation", claude_conversation_target);

    assert!(registry.select_working_directory(
        "codex-conversation",
        std::path::PathBuf::from("/workspace/next"),
    ));

    assert_eq!(
        registry.preferences("codex-conversation"),
        RoutePreferences {
            agent: Some(SubscriptionAgent::Codex),
            account_id: Some("account-1".to_string()),
            account_identity: Some(target().installation.account),
            ..RoutePreferences::default()
        }
    );
    assert_eq!(
        registry.preferences("claude-conversation"),
        RoutePreferences {
            agent: Some(SubscriptionAgent::ClaudeCode),
            account_id: Some("account-2".to_string()),
            account_identity: Some(claude_target().installation.account),
            model_id: Some("claude-model".to_string()),
            effort: Some("high".to_string()),
            ..RoutePreferences::default()
        }
    );
    assert_eq!(
        registry.preferences("new-conversation"),
        registry.preferences("claude-conversation")
    );
    assert_eq!(
        registry.pending_prompt("codex-conversation").as_deref(),
        Some("Keep this prompt")
    );
}

#[test]
fn location_change_is_rejected_while_a_turn_is_active() {
    let registry = SubscriptionSessionRegistry::default();
    registry.set_target("conversation-1", target());
    registry.set_lifecycle("conversation-1", AgentLifecycle::Responding);
    registry.set_host_choices(
        "conversation-1",
        vec![HostIdentity {
            id: "remote-1".to_string(),
            display_name: "devbox".to_string(),
        }],
    );

    assert!(!registry.select_host_location(
        "conversation-1",
        "remote-1",
        std::path::PathBuf::from("."),
    ));
    assert!(!registry.select_working_directory(
        "conversation-1",
        std::path::PathBuf::from("/workspace/next"),
    ));
    assert!(registry.target("conversation-1").is_some());
    assert_eq!(
        registry.lifecycle("conversation-1"),
        Some(AgentLifecycle::Responding)
    );
}
