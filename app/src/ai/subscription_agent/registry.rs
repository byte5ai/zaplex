use super::{
    AccountIdentity, AgentLifecycle, HostIdentity, ModelCapability, RoutePreferences,
    SessionIdentity, SubscriptionAgent, SubscriptionLocationPreference, SubscriptionTarget,
};
use futures::channel::oneshot;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use warpui::{Entity, SingletonEntity};

type PendingApprovals = HashMap<(String, String), oneshot::Sender<super::ApprovalDecision>>;

#[derive(Clone)]
pub(crate) struct StoredSubscriptionSession {
    pub(crate) target: SubscriptionTarget,
    pub(crate) session: SessionIdentity,
}

/// Process-independent native session identities keyed by Zaplex conversation.
///
/// Each turn may reconnect to an official CLI process, but resume always uses the real Claude
/// session ID or Codex thread ID captured from that CLI.
#[derive(Clone, Default)]
pub(crate) struct SubscriptionSessionRegistry {
    sessions: Arc<Mutex<HashMap<String, StoredSubscriptionSession>>>,
    targets: Arc<Mutex<HashMap<String, SubscriptionTarget>>>,
    default_preferences: Arc<Mutex<RoutePreferences>>,
    conversation_preferences: Arc<Mutex<HashMap<String, RoutePreferences>>>,
    lifecycle: Arc<Mutex<HashMap<String, AgentLifecycle>>>,
    approvals: Arc<Mutex<PendingApprovals>>,
    pending_prompts: Arc<Mutex<HashMap<String, String>>>,
    host_choices: Arc<Mutex<HashMap<String, Vec<HostIdentity>>>>,
    locations: Arc<Mutex<HashMap<String, SubscriptionLocationPreference>>>,
    agent_choices: Arc<Mutex<HashMap<String, Vec<SubscriptionAgent>>>>,
    account_choices: Arc<Mutex<HashMap<String, Vec<AccountIdentity>>>>,
    model_choices: Arc<Mutex<HashMap<String, Vec<ModelCapability>>>>,
}

impl SubscriptionSessionRegistry {
    pub(crate) fn get(&self, conversation_id: &str) -> Option<StoredSubscriptionSession> {
        self.sessions.lock().get(conversation_id).cloned()
    }

    pub(crate) fn store(
        &self,
        conversation_id: String,
        target: SubscriptionTarget,
        session: SessionIdentity,
    ) {
        self.sessions.lock().insert(
            conversation_id,
            StoredSubscriptionSession { target, session },
        );
    }

    pub(crate) fn clear_session_identity(&self, conversation_id: &str) {
        self.sessions.lock().remove(conversation_id);
    }

    pub(crate) fn remove(&self, conversation_id: &str) {
        self.sessions.lock().remove(conversation_id);
        self.targets.lock().remove(conversation_id);
        self.conversation_preferences.lock().remove(conversation_id);
        self.pending_prompts.lock().remove(conversation_id);
        self.host_choices.lock().remove(conversation_id);
        self.locations.lock().remove(conversation_id);
        self.agent_choices.lock().remove(conversation_id);
        self.account_choices.lock().remove(conversation_id);
        self.model_choices.lock().remove(conversation_id);
        self.lifecycle
            .lock()
            .insert(conversation_id.to_string(), AgentLifecycle::SessionEnded);
    }

    pub(crate) fn preferences(&self, conversation_id: &str) -> RoutePreferences {
        let default_preferences = self.default_preferences.lock();
        self.conversation_preferences
            .lock()
            .entry(conversation_id.to_string())
            .or_insert_with(|| default_preferences.clone())
            .clone()
    }

    pub(crate) fn remember_target(&self, conversation_id: &str, target: &SubscriptionTarget) {
        let remembered = route_preferences_for_target(target);
        let mut default_preferences = self.default_preferences.lock();
        let mut conversation_preferences = self.conversation_preferences.lock();
        *default_preferences = remembered.clone();
        conversation_preferences.insert(conversation_id.to_string(), remembered);
    }

    pub(crate) fn target(&self, conversation_id: &str) -> Option<SubscriptionTarget> {
        self.targets.lock().get(conversation_id).cloned()
    }

    pub(crate) fn set_target(
        &self,
        conversation_id: impl Into<String>,
        target: SubscriptionTarget,
    ) {
        let conversation_id = conversation_id.into();
        self.conversation_preferences.lock().insert(
            conversation_id.clone(),
            route_preferences_for_target(&target),
        );
        self.targets
            .lock()
            .insert(conversation_id.clone(), target.clone());
        let mut locations = self.locations.lock();
        if locations.contains_key(&conversation_id) {
            locations.insert(
                conversation_id.clone(),
                SubscriptionLocationPreference {
                    host: target.installation.host,
                    working_directory: target.working_directory,
                },
            );
        }
        drop(locations);
        self.clear_choices(&conversation_id);
    }

    pub(crate) fn lifecycle(&self, conversation_id: &str) -> Option<AgentLifecycle> {
        self.lifecycle.lock().get(conversation_id).cloned()
    }

    pub(crate) fn set_lifecycle(
        &self,
        conversation_id: impl Into<String>,
        lifecycle: AgentLifecycle,
    ) {
        self.lifecycle
            .lock()
            .insert(conversation_id.into(), lifecycle);
    }

    pub(crate) fn register_approval(
        &self,
        conversation_id: String,
        request_id: String,
    ) -> oneshot::Receiver<super::ApprovalDecision> {
        let (sender, receiver) = oneshot::channel();
        self.approvals
            .lock()
            .insert((conversation_id, request_id), sender);
        receiver
    }

    pub(crate) fn resolve_approval(
        &self,
        conversation_id: &str,
        request_id: &str,
        decision: super::ApprovalDecision,
    ) -> bool {
        self.approvals
            .lock()
            .remove(&(conversation_id.to_string(), request_id.to_string()))
            .is_some_and(|sender| sender.send(decision).is_ok())
    }

    pub(crate) fn clear_approvals(&self, conversation_id: &str) {
        self.approvals
            .lock()
            .retain(|(stored_conversation_id, _), _| stored_conversation_id != conversation_id);
    }

    pub(crate) fn mark_ready_to_resume(&self, conversation_id: &str) {
        self.lifecycle
            .lock()
            .insert(conversation_id.to_string(), AgentLifecycle::Ready);
    }

    pub(crate) fn remember_pending_prompt(
        &self,
        conversation_id: impl Into<String>,
        prompt: String,
    ) {
        self.pending_prompts
            .lock()
            .insert(conversation_id.into(), prompt);
    }

    pub(crate) fn pending_prompt(&self, conversation_id: &str) -> Option<String> {
        self.pending_prompts.lock().get(conversation_id).cloned()
    }

    pub(crate) fn take_pending_prompt(&self, conversation_id: &str) -> Option<String> {
        self.pending_prompts.lock().remove(conversation_id)
    }

    pub(crate) fn set_host_choices(
        &self,
        conversation_id: impl Into<String>,
        mut hosts: Vec<HostIdentity>,
    ) {
        hosts.sort_by(|left, right| left.id.cmp(&right.id));
        hosts.dedup_by(|left, right| left.id == right.id);
        self.host_choices
            .lock()
            .insert(conversation_id.into(), hosts);
    }

    pub(crate) fn host_choices(&self, conversation_id: &str) -> Vec<HostIdentity> {
        self.host_choices
            .lock()
            .get(conversation_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn location_preference(
        &self,
        conversation_id: &str,
    ) -> Option<SubscriptionLocationPreference> {
        self.locations.lock().get(conversation_id).cloned()
    }

    pub(crate) fn remember_initial_location(
        &self,
        conversation_id: impl Into<String>,
        location: SubscriptionLocationPreference,
    ) {
        self.locations
            .lock()
            .entry(conversation_id.into())
            .or_insert(location);
    }

    pub(crate) fn select_host_location(
        &self,
        conversation_id: &str,
        host_id: &str,
        working_directory: std::path::PathBuf,
    ) -> bool {
        if !self.location_change_allowed(conversation_id)
            || !valid_working_directory(&working_directory)
        {
            return false;
        }
        let selected_host = self
            .host_choices
            .lock()
            .get(conversation_id)
            .and_then(|hosts| hosts.iter().find(|host| host.id == host_id))
            .cloned();
        let Some(host) = selected_host else {
            return false;
        };
        self.locations.lock().insert(
            conversation_id.to_string(),
            SubscriptionLocationPreference {
                host,
                working_directory,
            },
        );
        self.invalidate_execution_target(conversation_id, true);
        true
    }

    pub(crate) fn select_working_directory(
        &self,
        conversation_id: &str,
        working_directory: std::path::PathBuf,
    ) -> bool {
        if !self.location_change_allowed(conversation_id)
            || !valid_working_directory(&working_directory)
        {
            return false;
        }
        let host = self
            .location_preference(conversation_id)
            .map(|location| location.host)
            .or_else(|| {
                self.target(conversation_id)
                    .map(|target| target.installation.host)
            });
        let Some(host) = host else {
            return false;
        };
        self.locations.lock().insert(
            conversation_id.to_string(),
            SubscriptionLocationPreference {
                host,
                working_directory,
            },
        );
        self.invalidate_execution_target(conversation_id, false);
        true
    }

    pub(crate) fn restart(&self, conversation_id: &str) {
        self.sessions.lock().remove(conversation_id);
        self.clear_approvals(conversation_id);
        self.mark_ready_to_resume(conversation_id);
    }

    pub(crate) fn set_agent_choices(
        &self,
        conversation_id: impl Into<String>,
        agents: Vec<SubscriptionAgent>,
    ) {
        let conversation_id = conversation_id.into();
        self.ensure_conversation_preferences(&conversation_id);
        self.targets.lock().remove(&conversation_id);
        self.agent_choices
            .lock()
            .insert(conversation_id.clone(), agents);
        self.account_choices.lock().remove(&conversation_id);
        self.model_choices.lock().remove(&conversation_id);
    }

    pub(crate) fn agent_choices(&self, conversation_id: &str) -> Vec<SubscriptionAgent> {
        self.agent_choices
            .lock()
            .get(conversation_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn begin_agent_selection(&self, conversation_id: &str) -> bool {
        self.begin_routing_selection(conversation_id, |preferences| {
            preferences.agent = None;
            preferences.account_id = None;
            preferences.account_identity = None;
            preferences.model_id = None;
            preferences.effort = None;
            preferences.require_agent_choice = true;
            preferences.require_account_choice = false;
            preferences.require_model_choice = false;
        })
    }

    pub(crate) fn begin_account_selection(&self, conversation_id: &str) -> bool {
        self.begin_routing_selection(conversation_id, |preferences| {
            preferences.account_id = None;
            preferences.account_identity = None;
            preferences.model_id = None;
            preferences.effort = None;
            preferences.require_agent_choice = false;
            preferences.require_account_choice = true;
            preferences.require_model_choice = false;
        })
    }

    pub(crate) fn begin_model_selection(&self, conversation_id: &str) -> bool {
        self.begin_routing_selection(conversation_id, |preferences| {
            preferences.model_id = None;
            preferences.effort = None;
            preferences.require_agent_choice = false;
            preferences.require_account_choice = false;
            preferences.require_model_choice = true;
        })
    }

    pub(crate) fn select_agent(&self, conversation_id: &str, agent: SubscriptionAgent) -> bool {
        if !self.routing_choice_allowed(conversation_id) {
            return false;
        }
        let selected = self
            .agent_choices
            .lock()
            .get(conversation_id)
            .is_some_and(|agents| agents.contains(&agent));
        if !selected {
            return false;
        }
        self.update_preferences(conversation_id, |preferences| {
            preferences.agent = Some(agent);
            preferences.account_id = None;
            preferences.account_identity = None;
            preferences.model_id = None;
            preferences.effort = None;
            preferences.require_agent_choice = false;
            preferences.require_account_choice = false;
            preferences.require_model_choice = false;
        });
        self.agent_choices.lock().remove(conversation_id);
        self.account_choices.lock().remove(conversation_id);
        self.model_choices.lock().remove(conversation_id);
        self.mark_ready_to_resume(conversation_id);
        true
    }

    pub(crate) fn set_account_choices(
        &self,
        conversation_id: impl Into<String>,
        accounts: Vec<AccountIdentity>,
    ) {
        let conversation_id = conversation_id.into();
        self.ensure_conversation_preferences(&conversation_id);
        self.targets.lock().remove(&conversation_id);
        self.account_choices
            .lock()
            .insert(conversation_id.clone(), accounts);
        self.agent_choices.lock().remove(&conversation_id);
        self.model_choices.lock().remove(&conversation_id);
    }

    pub(crate) fn account_choices(&self, conversation_id: &str) -> Vec<AccountIdentity> {
        self.account_choices
            .lock()
            .get(conversation_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn select_account(&self, conversation_id: &str, account: &AccountIdentity) -> bool {
        if !self.routing_choice_allowed(conversation_id) {
            return false;
        }
        let selected = self
            .account_choices
            .lock()
            .get(conversation_id)
            .is_some_and(|accounts| accounts.contains(account));
        if !selected {
            return false;
        }
        self.update_preferences(conversation_id, |preferences| {
            preferences.account_id = Some(account.id.clone());
            preferences.account_identity = Some(account.clone());
            preferences.model_id = None;
            preferences.effort = None;
            preferences.require_account_choice = false;
            preferences.require_model_choice = false;
        });
        self.account_choices.lock().remove(conversation_id);
        self.model_choices.lock().remove(conversation_id);
        self.mark_ready_to_resume(conversation_id);
        true
    }

    pub(crate) fn set_model_choices(
        &self,
        conversation_id: impl Into<String>,
        models: Vec<ModelCapability>,
    ) {
        let conversation_id = conversation_id.into();
        self.ensure_conversation_preferences(&conversation_id);
        self.targets.lock().remove(&conversation_id);
        self.model_choices
            .lock()
            .insert(conversation_id.clone(), models);
        self.agent_choices.lock().remove(&conversation_id);
        self.account_choices.lock().remove(&conversation_id);
    }

    pub(crate) fn model_choices(&self, conversation_id: &str) -> Vec<ModelCapability> {
        self.model_choices
            .lock()
            .get(conversation_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn select_model(&self, conversation_id: &str, model_id: &str) -> bool {
        if !self.routing_choice_allowed(conversation_id) {
            return false;
        }
        let selected = self
            .model_choices
            .lock()
            .get(conversation_id)
            .and_then(|models| models.iter().find(|model| model.id == model_id))
            .cloned();
        let Some(selected) = selected else {
            return false;
        };
        self.update_preferences(conversation_id, |preferences| {
            preferences.model_id = Some(selected.id);
            preferences.effort = selected.default_effort;
            preferences.require_model_choice = false;
        });
        self.model_choices.lock().remove(conversation_id);
        self.mark_ready_to_resume(conversation_id);
        true
    }

    fn clear_choices(&self, conversation_id: &str) {
        self.agent_choices.lock().remove(conversation_id);
        self.account_choices.lock().remove(conversation_id);
        self.model_choices.lock().remove(conversation_id);
    }

    fn begin_routing_selection(
        &self,
        conversation_id: &str,
        reset: impl FnOnce(&mut RoutePreferences),
    ) -> bool {
        if !self.location_change_allowed(conversation_id) {
            return false;
        }
        let Some(target) = self.target(conversation_id) else {
            return false;
        };
        self.conversation_preferences.lock().insert(
            conversation_id.to_string(),
            route_preferences_for_target(&target),
        );
        self.sessions.lock().remove(conversation_id);
        self.targets.lock().remove(conversation_id);
        self.clear_approvals(conversation_id);
        self.clear_choices(conversation_id);
        self.update_conversation_preferences(conversation_id, reset);
        self.set_lifecycle(conversation_id, AgentLifecycle::SelectionRequired);
        true
    }

    fn invalidate_execution_target(&self, conversation_id: &str, host_changed: bool) {
        self.sessions.lock().remove(conversation_id);
        self.targets.lock().remove(conversation_id);
        self.clear_approvals(conversation_id);
        self.clear_choices(conversation_id);
        self.update_conversation_preferences(conversation_id, |preferences| {
            if host_changed {
                preferences.account_id = None;
                preferences.account_identity = None;
            }
            preferences.model_id = None;
            preferences.effort = None;
            preferences.require_agent_choice = false;
            preferences.require_account_choice = false;
            preferences.require_model_choice = false;
        });
        self.mark_ready_to_resume(conversation_id);
    }

    fn ensure_conversation_preferences(&self, conversation_id: &str) {
        let default_preferences = self.default_preferences.lock();
        self.conversation_preferences
            .lock()
            .entry(conversation_id.to_string())
            .or_insert_with(|| default_preferences.clone());
    }

    fn update_preferences(
        &self,
        conversation_id: &str,
        update: impl FnOnce(&mut RoutePreferences),
    ) {
        let mut default_preferences = self.default_preferences.lock();
        let mut conversation_preferences = self.conversation_preferences.lock();
        let conversation_preferences = conversation_preferences
            .entry(conversation_id.to_string())
            .or_insert_with(|| default_preferences.clone());
        update(conversation_preferences);
        *default_preferences = conversation_preferences.clone();
    }

    fn update_conversation_preferences(
        &self,
        conversation_id: &str,
        update: impl FnOnce(&mut RoutePreferences),
    ) {
        let default_preferences = self.default_preferences.lock();
        let mut conversation_preferences = self.conversation_preferences.lock();
        let conversation_preferences = conversation_preferences
            .entry(conversation_id.to_string())
            .or_insert_with(|| default_preferences.clone());
        update(conversation_preferences);
    }

    fn location_change_allowed(&self, conversation_id: &str) -> bool {
        self.lifecycle
            .lock()
            .get(conversation_id)
            .is_some_and(AgentLifecycle::can_change_location)
    }

    fn routing_choice_allowed(&self, conversation_id: &str) -> bool {
        self.lifecycle
            .lock()
            .get(conversation_id)
            .is_some_and(|lifecycle| matches!(lifecycle, AgentLifecycle::SelectionRequired))
    }
}

fn route_preferences_for_target(target: &SubscriptionTarget) -> RoutePreferences {
    RoutePreferences {
        agent: Some(target.installation.agent),
        account_id: Some(target.installation.account.id.clone()),
        account_identity: Some(target.installation.account.clone()),
        model_id: Some(target.model.id.clone()),
        effort: target.effort.clone(),
        require_agent_choice: false,
        require_account_choice: false,
        require_model_choice: false,
    }
}

fn valid_working_directory(path: &std::path::Path) -> bool {
    path == std::path::Path::new(".") || path.is_absolute()
}

impl Entity for SubscriptionSessionRegistry {
    type Event = ();
}

impl SingletonEntity for SubscriptionSessionRegistry {}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
