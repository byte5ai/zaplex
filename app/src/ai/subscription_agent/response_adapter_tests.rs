use super::*;
use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::{AIAgentOutputStatus, FinishedAIAgentOutput};
use crate::ai::subscription_agent::SessionIdentity;
use chrono::{DateTime, Local};

/// The messages a turn adds to its task, in the order the client applies them.
fn persisted_messages(events: &[api::ResponseEvent]) -> Vec<api::Message> {
    let mut messages = Vec::new();
    for event in events {
        let Some(api::response_event::Type::ClientActions(actions)) = &event.r#type else {
            continue;
        };
        for action in &actions.actions {
            if let Some(api::client_action::Action::AddMessagesToTask(added)) = &action.action {
                messages.extend(added.messages.iter().cloned());
            }
        }
    }
    messages
}

/// Restores `messages` with a fallback time outside every test turn, so only the
/// persisted timestamps can place a restored exchange inside one.
fn restored(messages: Vec<api::Message>) -> AIConversation {
    AIConversation::new_restored(
        AIConversationId::new(),
        vec![api::Task {
            id: "task-1".to_string(),
            messages,
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }],
        None,
        DateTime::<Local>::default(),
    )
    .unwrap()
}

#[test]
fn text_chunks_create_then_append_one_message() {
    let mut adapter = ResponseEventAdapter::new("task-1".to_string(), None);
    let first = adapter.adapt(SubscriptionEvent::TextDelta("Hello".to_string()));
    let second = adapter.adapt(SubscriptionEvent::TextDelta(" world".to_string()));

    assert!(matches!(
        first[0].r#type,
        Some(api::response_event::Type::ClientActions(_))
    ));
    let Some(api::response_event::Type::ClientActions(actions)) = &second[0].r#type else {
        panic!("expected client actions");
    };
    assert!(matches!(
        actions.actions[0].action,
        Some(api::client_action::Action::AppendToMessageContent(_))
    ));
}

#[test]
fn completion_carries_reported_context_usage() {
    let mut adapter = ResponseEventAdapter::new("task-1".to_string(), Some(100));
    assert_eq!(
        adapter
            .adapt(SubscriptionEvent::Usage(Usage {
                input_tokens: 20,
                cached_input_tokens: 10,
                output_tokens: 5,
            }))
            .is_empty(),
        true
    );
    let completed = adapter.adapt(SubscriptionEvent::TurnCompleted {
        session: SessionIdentity::Codex("thread-1".to_string()),
    });
    let Some(api::response_event::Type::Finished(finished)) = &completed.last().unwrap().r#type
    else {
        panic!("expected finished event");
    };
    assert_eq!(
        finished
            .conversation_usage_metadata
            .as_ref()
            .map(|usage| usage.context_window_usage),
        Some(0.25)
    );
}

#[test]
fn restored_subscription_conversation_keeps_real_timestamp() {
    let started = Local::now();
    let mut adapter = ResponseEventAdapter::new("task-1".to_string(), None);
    let mut events = vec![adapter.persist_user_query("Summarize the diff".to_string())];
    events.extend(adapter.adapt(SubscriptionEvent::TextDelta("Done".to_string())));
    events.extend(adapter.adapt(SubscriptionEvent::TurnCompleted {
        session: SessionIdentity::ClaudeCode("session-1".to_string()),
    }));
    let ended = Local::now();

    let conversation = restored(persisted_messages(&events));
    let start_time = conversation.latest_exchange().unwrap().start_time;

    assert!(
        started <= start_time && start_time <= ended,
        "restored start {start_time} is outside the turn ({started} – {ended})"
    );
}

#[test]
fn restored_failed_subscription_turn_is_not_success() {
    let mut adapter = ResponseEventAdapter::new("task-1".to_string(), None);
    let mut events = vec![adapter.persist_user_query("Hi".to_string())];
    events.extend(adapter.adapt(SubscriptionEvent::TextDelta("Checking".to_string())));
    events.extend(adapter.adapt(SubscriptionEvent::Error {
        message: "Not logged in · Please run /login".to_string(),
        recoverable: false,
        session: None,
    }));

    let conversation = restored(persisted_messages(&events));

    assert_eq!(conversation.status(), &ConversationStatus::Error);
    let Some(AIAgentOutputStatus::Finished {
        finished_output: FinishedAIAgentOutput::Error { error, .. },
    }) = conversation
        .latest_exchange()
        .map(|exchange| &exchange.output_status)
    else {
        panic!("the restored turn must keep its failure");
    };
    assert_eq!(error.to_string(), "Not logged in · Please run /login");
}
