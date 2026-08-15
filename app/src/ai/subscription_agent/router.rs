use super::{AgentCapability, ModelCapability, SubscriptionAgent, SubscriptionTarget};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RoutePreferences {
    pub(crate) agent: Option<SubscriptionAgent>,
    pub(crate) account_id: Option<String>,
    pub(crate) model_id: Option<String>,
    pub(crate) effort: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RouteResult {
    NoReachableAgent,
    NeedsAgentChoice(Vec<SubscriptionAgent>),
    NeedsAccountChoice {
        agent: SubscriptionAgent,
        account_ids: Vec<String>,
    },
    NeedsModelChoice {
        agent: SubscriptionAgent,
        account_id: String,
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

    let selected_agent = preferences
        .agent
        .filter(|agent| agents.contains(agent))
        .or_else(|| (agents.len() == 1).then_some(agents[0]));
    let Some(selected_agent) = selected_agent else {
        return RouteResult::NeedsAgentChoice(agents);
    };
    capabilities.retain(|capability| capability.installation.agent == selected_agent);

    let selected_account = preferences
        .account_id
        .as_deref()
        .and_then(|account_id| {
            capabilities
                .iter()
                .find(|capability| capability.installation.account.id == account_id)
        })
        .or_else(|| (capabilities.len() == 1).then(|| &capabilities[0]));
    let Some(selected_account) = selected_account else {
        let mut account_ids: Vec<_> = capabilities
            .iter()
            .map(|capability| capability.installation.account.id.clone())
            .collect();
        account_ids.sort();
        account_ids.dedup();
        return RouteResult::NeedsAccountChoice {
            agent: selected_agent,
            account_ids,
        };
    };

    let selected_model = select_model(&selected_account.models, preferences.model_id.as_deref());
    let Some(selected_model) = selected_model else {
        return RouteResult::NeedsModelChoice {
            agent: selected_agent,
            account_id: selected_account.installation.account.id.clone(),
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
    preferred_id
        .and_then(|preferred_id| models.iter().find(|model| model.id == preferred_id))
        .or_else(|| {
            let mut defaults = models.iter().filter(|model| model.is_default);
            let default = defaults.next()?;
            defaults.next().is_none().then_some(default)
        })
        .cloned()
}

#[cfg(test)]
#[path = "router_tests.rs"]
mod tests;
