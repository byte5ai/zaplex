use crate::ai::agent::ReceivedMessageInput;
use crate::ai::agent_events::AgentRunEvent;

/// Zap local builds no longer fetch message bodies from cloud mailbox or send delivery receipts.
/// This type preserves side-effect-free compatible semantics for the local harness bridging call surface.
#[derive(Clone)]
pub(crate) struct MessageHydrator;

impl MessageHydrator {
    pub(crate) fn new() -> Self {
        Self
    }

    pub(crate) async fn hydrate_event_for_recipient(
        &self,
        event: &AgentRunEvent,
        recipient_run_id: &str,
    ) -> Option<ReceivedMessageInput> {
        if event.event_type != "new_message" || event.run_id != recipient_run_id {
            return None;
        }

        None
    }

    pub(crate) async fn mark_messages_delivered_best_effort<'a, I>(
        &self,
        _message_ids: I,
    ) -> Vec<(String, anyhow::Error)>
    where
        I: IntoIterator<Item = &'a str>,
    {
        Vec::new()
    }
}
