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
                provider_account_id: None,
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
fn zero_agents_has_no_reachable_route() {
    assert_eq!(
        route_target([], &RoutePreferences::default(), "/workspace".into()),
        RouteResult::NoReachableAgent
    );
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
fn removed_preferred_agent_never_silently_switches_provider() {
    let result = route_target(
        [capability(
            SubscriptionAgent::Codex,
            "codex-account",
            &[("gpt-current", true)],
        )],
        &RoutePreferences {
            agent: Some(SubscriptionAgent::ClaudeCode),
            ..RoutePreferences::default()
        },
        "/workspace".into(),
    );

    assert_eq!(
        result,
        RouteResult::NeedsAgentChoice(vec![SubscriptionAgent::Codex])
    );
}

#[test]
fn account_choices_preserve_policy_order_instead_of_sorting_opaque_ids() {
    let result = route_target(
        [
            capability(
                SubscriptionAgent::ClaudeCode,
                "z-freest-account",
                &[("claude-current", true)],
            ),
            capability(
                SubscriptionAgent::ClaudeCode,
                "a-busy-account",
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
            accounts: vec![
                AccountIdentity {
                    id: "z-freest-account".to_string(),
                    display_name: "z-freest-account".to_string(),
                    provider_account_id: None,
                    config_dir: None,
                },
                AccountIdentity {
                    id: "a-busy-account".to_string(),
                    display_name: "a-busy-account".to_string(),
                    provider_account_id: None,
                    config_dir: None,
                },
            ],
        }
    );
}

#[test]
fn removed_preferred_account_never_falls_back_to_remaining_login() {
    let result = route_target(
        [capability(
            SubscriptionAgent::ClaudeCode,
            "remaining-account",
            &[("claude-current", true)],
        )],
        &RoutePreferences {
            agent: Some(SubscriptionAgent::ClaudeCode),
            account_id: Some("signed-out-account".to_string()),
            ..RoutePreferences::default()
        },
        "/workspace".into(),
    );

    assert_eq!(
        result,
        RouteResult::NeedsAccountChoice {
            agent: SubscriptionAgent::ClaudeCode,
            accounts: vec![AccountIdentity {
                id: "remaining-account".to_string(),
                display_name: "remaining-account".to_string(),
                provider_account_id: None,
                config_dir: None,
            }],
        }
    );
}

#[test]
fn account_choices_deduplicate_only_the_complete_account_identity() {
    let mut first = capability(
        SubscriptionAgent::ClaudeCode,
        "shared-routing-id",
        &[("claude-current", true)],
    );
    first.installation.account.provider_account_id = Some("provider-1".to_string());
    first.installation.account.config_dir = Some("/accounts/one".into());
    let duplicate = first.clone();
    let mut second = first.clone();
    second.installation.account.provider_account_id = Some("provider-2".to_string());
    second.installation.account.config_dir = Some("/accounts/two".into());

    let result = route_target(
        [first.clone(), duplicate, second.clone()],
        &RoutePreferences::default(),
        "/workspace".into(),
    );

    assert_eq!(
        result,
        RouteResult::NeedsAccountChoice {
            agent: SubscriptionAgent::ClaudeCode,
            accounts: vec![first.installation.account, second.installation.account],
        }
    );
}

#[test]
fn exact_account_identity_disambiguates_shared_routing_ids() {
    let mut first = capability(
        SubscriptionAgent::ClaudeCode,
        "shared-routing-id",
        &[("claude-current", true)],
    );
    first.installation.account.provider_account_id = Some("provider-1".to_string());
    first.installation.account.config_dir = Some("/accounts/one".into());
    let mut second = first.clone();
    second.installation.account.provider_account_id = Some("provider-2".to_string());
    second.installation.account.config_dir = Some("/accounts/two".into());

    let result = route_target(
        [first, second.clone()],
        &RoutePreferences {
            agent: Some(SubscriptionAgent::ClaudeCode),
            account_id: Some("shared-routing-id".to_string()),
            account_identity: Some(second.installation.account.clone()),
            ..RoutePreferences::default()
        },
        "/workspace".into(),
    );

    let RouteResult::Ready(target) = result else {
        panic!("expected an exact ready route");
    };
    assert_eq!(target.installation.account, second.installation.account);
}

#[test]
fn explicit_change_requests_never_auto_select_the_only_choice() {
    let candidate = capability(
        SubscriptionAgent::Codex,
        "account-1",
        &[("gpt-current", true)],
    );

    assert!(matches!(
        route_target(
            [candidate.clone()],
            &RoutePreferences {
                require_agent_choice: true,
                ..RoutePreferences::default()
            },
            "/workspace".into(),
        ),
        RouteResult::NeedsAgentChoice(_)
    ));
    assert!(matches!(
        route_target(
            [candidate.clone()],
            &RoutePreferences {
                agent: Some(SubscriptionAgent::Codex),
                require_account_choice: true,
                ..RoutePreferences::default()
            },
            "/workspace".into(),
        ),
        RouteResult::NeedsAccountChoice { .. }
    ));
    assert!(matches!(
        route_target(
            [candidate.clone()],
            &RoutePreferences {
                agent: Some(SubscriptionAgent::Codex),
                account_identity: Some(candidate.installation.account),
                require_model_choice: true,
                ..RoutePreferences::default()
            },
            "/workspace".into(),
        ),
        RouteResult::NeedsModelChoice { .. }
    ));
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
            account: capability(
                SubscriptionAgent::Codex,
                "account-1",
                &[("gpt-current", false)],
            )
            .installation
            .account,
        }
    );
}

#[test]
fn removed_model_never_falls_back_to_a_different_reported_default() {
    let result = route_target(
        [capability(
            SubscriptionAgent::Codex,
            "account-1",
            &[("new-default", true)],
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
            account: capability(
                SubscriptionAgent::Codex,
                "account-1",
                &[("new-default", true)],
            )
            .installation
            .account,
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
