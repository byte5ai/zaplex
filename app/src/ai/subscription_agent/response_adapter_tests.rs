use super::*;
use crate::ai::subscription_agent::SessionIdentity;

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
    let Some(api::response_event::Type::Finished(finished)) = &completed[0].r#type else {
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
