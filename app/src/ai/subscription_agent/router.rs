use super::{
    AccountIdentity, AgentCapability, ModelCapability, SubscriptionAgent, SubscriptionTarget,
};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RoutePreferences {
    pub(crate) agent: Option<SubscriptionAgent>,
    pub(crate) account_id: Option<String>,
    pub(crate) account_identity: Option<AccountIdentity>,
    pub(crate) model_id: Option<String>,
    pub(crate) effort: Option<String>,
    pub(crate) require_agent_choice: bool,
    pub(crate) require_account_choice: bool,
    pub(crate) require_model_choice: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum RouteResult {
    NoReachableAgent,
    NeedsAgentChoice(Vec<SubscriptionAgent>),
    NeedsAccountChoice {
        agent: SubscriptionAgent,
        accounts: Vec<AccountIdentity>,
    },
    NeedsModelChoice {
        agent: SubscriptionAgent,
        account: AccountIdentity,
    },
    Ready(SubscriptionTarget),
}

pub(crate) fn route_target(
    capabilities: impl IntoIterator<Item = AgentCapability>,
    preferences: &RoutePreferences,
    working_directory: PathBuf,
) -> RouteResult {
    let mut capabilities: Vec<_> = capabilities.into_iter().collect();
    if capabilities.is_empty() {
        return RouteResult::NoReachableAgent;
    }

    let mut agents: Vec<_> = capabilities
        .iter()
        .map(|capability| capability.installation.agent)
        .collect();
    agents.sort_by_key(|agent| match agent {
        SubscriptionAgent::ClaudeCode => 0,
        SubscriptionAgent::Codex => 1,
    });
    agents.dedup();

    let selected_agent = match (preferences.require_agent_choice, preferences.agent) {
        (true, _) => None,
        (false, Some(agent)) if agents.contains(&agent) => Some(agent),
        // A remembered agent disappearing must become an explicit choice. Even
        // when only one other agent remains, silently switching providers can
        // send a prompt to a different account and model than the user chose.
        (false, Some(_)) => None,
        (false, None) => (agents.len() == 1).then_some(agents[0]),
    };
    let Some(selected_agent) = selected_agent else {
        return RouteResult::NeedsAgentChoice(agents);
    };
    capabilities.retain(|capability| capability.installation.agent == selected_agent);

    let selected_account = match (
        preferences.require_account_choice,
        preferences.account_identity.as_ref(),
        preferences.account_id.as_deref(),
    ) {
        (true, _, _) => None,
        (false, Some(identity), _) => capabilities
            .iter()
            .find(|capability| capability.installation.account == *identity),
        (false, None, Some(account_id)) => capabilities
            .iter()
            .find(|capability| capability.installation.account.id == account_id),
        (false, None, None) if capabilities.len() == 1 => Some(&capabilities[0]),
        (false, None, None) => None,
    };
    let Some(selected_account) = selected_account else {
        // Candidate order is caller policy order. Sorting opaque account ids
        // here would discard any freeness ranking established before capability
        // discovery. Deduplicate the complete identity in place instead: two
        // accounts may share a display/routing id while differing by provider
        // identity or isolated config root.
        let mut accounts = Vec::new();
        for capability in &capabilities {
            let account = capability.installation.account.clone();
            if !accounts.contains(&account) {
                accounts.push(account);
            }
        }
        return RouteResult::NeedsAccountChoice {
            agent: selected_agent,
            accounts,
        };
    };

    let selected_model = (!preferences.require_model_choice)
        .then(|| select_model(&selected_account.models, preferences.model_id.as_deref()))
        .flatten();
    let Some(selected_model) = selected_model else {
        return RouteResult::NeedsModelChoice {
            agent: selected_agent,
            account: selected_account.installation.account.clone(),
        };
    };
    let effort = preferences.effort.clone().filter(|effort| {
        selected_model
            .supported_efforts
            .iter()
            .any(|supported| supported.id == *effort)
    });

    RouteResult::Ready(SubscriptionTarget {
        installation: selected_account.installation.clone(),
        working_directory,
        model: selected_model,
        effort,
    })
}

fn select_model(models: &[ModelCapability], preferred_id: Option<&str>) -> Option<ModelCapability> {
    match preferred_id {
        Some(preferred_id) => models.iter().find(|model| model.id == preferred_id),
        None => {
            let mut defaults = models.iter().filter(|model| model.is_default);
            let default = defaults.next()?;
            defaults.next().is_none().then_some(default)
        }
    }
    .cloned()
}

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
