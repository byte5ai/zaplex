use super::*;
use crate::ai::subscription_agent::{
    AccountIdentity, HostIdentity, InstallationIdentity, ModelEffort,
};

fn capability(
    agent: SubscriptionAgent,
    account_id: &str,
    models: &[(&str, bool)],
) -> AgentCapability {
    AgentCapability {
        installation: InstallationIdentity {
            agent,
            host: HostIdentity {
                id: "local".to_string(),
                display_name: "Local".to_string(),
            },
            account: AccountIdentity {
                id: account_id.to_string(),
                display_name: account_id.to_string(),
                config_dir: None,
            },
            executable: agent.display_name().into(),
            version: "1.0".to_string(),
        },
        models: models
            .iter()
            .map(|(id, is_default)| ModelCapability {
                id: (*id).to_string(),
                display_name: (*id).to_string(),
                description: None,
                resolved_model: None,
                is_default: *is_default,
                supported_efforts: vec![ModelEffort {
                    id: "high".to_string(),
                    display_name: "High".to_string(),
                }],
                default_effort: Some("high".to_string()),
                context_window: None,
            })
            .collect(),
    }
}

#[test]
fn one_agent_account_and_reported_default_route_automatically() {
    let result = route_target(
        [capability(
            SubscriptionAgent::Codex,
            "account-1",
            &[("gpt-current", true)],
        )],
        &RoutePreferences::default(),
        "/workspace".into(),
    );

    let RouteResult::Ready(target) = result else {
        panic!("expected a ready route");
    };
    assert_eq!(target.model.id, "gpt-current");
    assert_eq!(target.installation.account.id, "account-1");
}

#[test]
fn two_agents_require_choice_without_explicit_default() {
    let result = route_target(
        [
            capability(
                SubscriptionAgent::ClaudeCode,
                "claude-account",
                &[("claude-current", true)],
            ),
            capability(
                SubscriptionAgent::Codex,
                "codex-account",
                &[("gpt-current", true)],
            ),
        ],
        &RoutePreferences::default(),
        "/workspace".into(),
    );

    assert_eq!(
        result,
        RouteResult::NeedsAgentChoice(vec![
            SubscriptionAgent::ClaudeCode,
            SubscriptionAgent::Codex,
        ])
    );
}

#[test]
fn multiple_accounts_are_not_ranked_by_plan_or_name() {
    let result = route_target(
        [
            capability(
                SubscriptionAgent::ClaudeCode,
                "paid-looking-account",
                &[("claude-current", true)],
            ),
            capability(
                SubscriptionAgent::ClaudeCode,
                "free-looking-account",
                &[("claude-current", true)],
            ),
        ],
        &RoutePreferences::default(),
        "/workspace".into(),
    );

    assert_eq!(
        result,
        RouteResult::NeedsAccountChoice {
            agent: SubscriptionAgent::ClaudeCode,
            account_ids: vec![
                "free-looking-account".to_string(),
                "paid-looking-account".to_string(),
            ],
        }
    );
}

#[test]
fn invalid_model_preference_does_not_fall_back_to_non_default() {
    let result = route_target(
        [capability(
            SubscriptionAgent::Codex,
            "account-1",
            &[("gpt-current", false)],
        )],
        &RoutePreferences {
            model_id: Some("removed-model".to_string()),
            ..RoutePreferences::default()
        },
        "/workspace".into(),
    );

    assert_eq!(
        result,
        RouteResult::NeedsModelChoice {
            agent: SubscriptionAgent::Codex,
            account_id: "account-1".to_string(),
        }
    );
}

#[test]
fn unsupported_effort_is_cleared() {
    let result = route_target(
        [capability(
            SubscriptionAgent::Codex,
            "account-1",
            &[("gpt-current", true)],
        )],
        &RoutePreferences {
            effort: Some("extra-high".to_string()),
            ..RoutePreferences::default()
        },
        "/workspace".into(),
    );

    let RouteResult::Ready(target) = result else {
        panic!("expected a ready route");
    };
    assert_eq!(target.effort, None);
}
