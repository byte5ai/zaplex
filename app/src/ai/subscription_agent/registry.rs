use super::{
    AgentLifecycle, ModelCapability, RoutePreferences, SessionIdentity, SubscriptionAgent,
    SubscriptionTarget,
};
use futures::channel::oneshot;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use warpui::{Entity, SingletonEntity};

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
    preferences: Arc<Mutex<RoutePreferences>>,
    lifecycle: Arc<Mutex<HashMap<String, AgentLifecycle>>>,
    approvals: Arc<Mutex<HashMap<(String, String), oneshot::Sender<super::ApprovalDecision>>>>,
    agent_choices: Arc<Mutex<HashMap<String, Vec<SubscriptionAgent>>>>,
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

    pub(crate) fn remove(&self, conversation_id: &str) {
        self.sessions.lock().remove(conversation_id);
        self.targets.lock().remove(conversation_id);
        self.agent_choices.lock().remove(conversation_id);
        self.model_choices.lock().remove(conversation_id);
        self.lifecycle
            .lock()
            .insert(conversation_id.to_string(), AgentLifecycle::SessionEnded);
    }

    pub(crate) fn preferences(&self) -> RoutePreferences {
        self.preferences.lock().clone()
    }

    pub(crate) fn remember_target(&self, target: &SubscriptionTarget) {
        let mut preferences = self.preferences.lock();
        preferences.agent = Some(target.installation.agent);
        preferences.account_id = Some(target.installation.account.id.clone());
        preferences.model_id = Some(target.model.id.clone());
        preferences.effort = target.effort.clone();
    }

    pub(crate) fn target(&self, conversation_id: &str) -> Option<SubscriptionTarget> {
        self.targets.lock().get(conversation_id).cloned()
    }

    pub(crate) fn set_target(
        &self,
        conversation_id: impl Into<String>,
        target: SubscriptionTarget,
    ) {
        self.targets.lock().insert(conversation_id.into(), target);
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
        self.agent_choices
            .lock()
            .insert(conversation_id.into(), agents);
    }

    pub(crate) fn agent_choices(&self, conversation_id: &str) -> Vec<SubscriptionAgent> {
        self.agent_choices
            .lock()
            .get(conversation_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn select_agent(&self, conversation_id: &str, agent: SubscriptionAgent) {
        {
            let mut preferences = self.preferences.lock();
            preferences.agent = Some(agent);
            preferences.account_id = None;
            preferences.model_id = None;
            preferences.effort = None;
        }
        self.agent_choices.lock().remove(conversation_id);
        self.model_choices.lock().remove(conversation_id);
        self.mark_ready_to_resume(conversation_id);
    }

    pub(crate) fn set_model_choices(
        &self,
        conversation_id: impl Into<String>,
        models: Vec<ModelCapability>,
    ) {
        self.model_choices
            .lock()
            .insert(conversation_id.into(), models);
    }

    pub(crate) fn model_choices(&self, conversation_id: &str) -> Vec<ModelCapability> {
        self.model_choices
            .lock()
            .get(conversation_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn select_model(&self, conversation_id: &str, model_id: &str) -> bool {
        let selected = self
            .model_choices
            .lock()
            .get(conversation_id)
            .and_then(|models| models.iter().find(|model| model.id == model_id))
            .cloned();
        let Some(selected) = selected else {
            return false;
        };
        {
            let mut preferences = self.preferences.lock();
            preferences.model_id = Some(selected.id);
            preferences.effort = selected.default_effort;
        }
        self.model_choices.lock().remove(conversation_id);
        self.mark_ready_to_resume(conversation_id);
        true
    }
}

impl Entity for SubscriptionSessionRegistry {
    type Event = ();
}

impl SingletonEntity for SubscriptionSessionRegistry {}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
