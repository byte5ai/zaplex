use super::{
    discovery_failure_lifecycle, legacy_ssh_candidates, AccountIdentity, AgentLifecycle,
    HostIdentity, InstallationIdentity, ProcessLocation, SubscriptionAgent,
    SubscriptionSessionRegistry, SubscriptionTarget,
};
use crate::ai::subscription_agent::{ModelCapability, SessionIdentity};
use crate::terminal::ssh::util::InteractiveSshCommand;

fn target(agent: SubscriptionAgent) -> SubscriptionTarget {
    SubscriptionTarget {
        installation: InstallationIdentity {
            agent,
            host: HostIdentity {
                id: "local".to_string(),
                display_name: "Local".to_string(),
            },
            account: AccountIdentity {
                id: "account".to_string(),
                display_name: "Account".to_string(),
                config_dir: None,
            },
            executable: agent.display_name().into(),
            version: "1.0.0".to_string(),
        },
        working_directory: "/workspace".into(),
        model: ModelCapability {
            id: "model".to_string(),
            display_name: "Model".to_string(),
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
fn signed_out_agents_are_not_recoverable_even_with_a_session_identity() {
    for (agent, message, session) in [
        (
            SubscriptionAgent::ClaudeCode,
            "Not logged in · Please run /login",
            SessionIdentity::ClaudeCode("session-1".to_string()),
        ),
        (
            SubscriptionAgent::Codex,
            "Codex is not using a ChatGPT subscription account",
            SessionIdentity::Codex("thread-1".to_string()),
        ),
    ] {
        let registry = SubscriptionSessionRegistry::default();
        registry.store("conversation".to_string(), target(agent), session);

        let lifecycle =
            discovery_failure_lifecycle(&[agent], message.to_string(), &registry, "conversation");

        assert_eq!(lifecycle, AgentLifecycle::NotSignedIn { agent });
        assert!(!lifecycle.accepts_prompt());
        assert!(!lifecycle.can_resume());
        assert!(registry.get("conversation").is_none());
    }
}

#[test]
fn legacy_ssh_candidates_run_both_agents_on_the_active_host() {
    let candidates = legacy_ssh_candidates(&InteractiveSshCommand {
        host: Some("developer@ssh.example.test".to_string()),
        port: Some("2222".to_string()),
    })
    .unwrap();

    assert_eq!(candidates.len(), 2);
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.installation.agent)
            .collect::<Vec<_>>(),
        vec![SubscriptionAgent::ClaudeCode, SubscriptionAgent::Codex],
    );
    for candidate in candidates {
        assert_eq!(
            candidate.installation.host.id,
            "legacy-ssh:developer@ssh.example.test:2222"
        );
        assert_eq!(
            candidate.location,
            ProcessLocation::Remote {
                ssh_argv: vec![
                    "ssh".to_string(),
                    "-o".to_string(),
                    "StrictHostKeyChecking=ask".to_string(),
                    "-p".to_string(),
                    "2222".to_string(),
                    "--".to_string(),
                    "developer@ssh.example.test".to_string(),
                ],
            }
        );
    }
}

#[test]
fn legacy_ssh_candidates_require_a_reusable_host() {
    let error = legacy_ssh_candidates(&InteractiveSshCommand::default())
        .err()
        .expect("missing SSH host must fail closed");

    assert_eq!(
        error.to_string(),
        "the active SSH session has no reusable host"
    );
}
