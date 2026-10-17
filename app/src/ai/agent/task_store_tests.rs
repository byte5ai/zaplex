use std::collections::HashSet;

use chrono::Local;
use uuid::Uuid;

use crate::ai::{
    agent::{
        task::{Task, TaskId},
        AIAgentExchange, AIAgentExchangeId, AIAgentOutput, AIAgentOutputMessage,
        AIAgentOutputMessageType, AIAgentOutputStatus, FinishedAIAgentOutput, MessageId, Shared,
        SubagentCall,
    },
    llms::LLMId,
};

use super::TaskStore;

fn create_test_exchange() -> AIAgentExchange {
    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: vec![],
        output_status: AIAgentOutputStatus::Streaming { output: None },
        added_message_ids: HashSet::new(),
        start_time: Local::now(),
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory: None,
        model_id: LLMId::from(""),
        request_cost: None,
        coding_model_id: LLMId::from(""),
        cli_agent_model_id: LLMId::from(""),
        computer_use_model_id: LLMId::from(""),
        response_initiator: None,
    }
}

fn create_test_task_with_exchanges(exchange_count: usize) -> Task {
    let mut task = Task::new_optimistic_root();
    for _ in 0..exchange_count {
        task.append_exchange(create_test_exchange());
    }
    task
}

fn create_test_subtask_with_exchanges(exchange_count: usize) -> Task {
    use crate::terminal::model::block::BlockId;
    let mut task = Task::new_optimistic_cli_agent_subtask(BlockId::new());
    for _ in 0..exchange_count {
        task.append_exchange(create_test_exchange());
    }
    task
}

/// Creates an exchange with a finished output containing a subagent call to the given task_id.
fn create_exchange_with_subagent_call(subtask_id: &TaskId) -> AIAgentExchange {
    let output = AIAgentOutput {
        messages: vec![AIAgentOutputMessage {
            id: MessageId::new(Uuid::new_v4().to_string()),
            message: AIAgentOutputMessageType::Subagent(SubagentCall {
                task_id: subtask_id.to_string(),
                subagent_type: crate::ai::agent::SubagentType::Unknown,
            }),
            citations: vec![],
        }],
        citations: vec![],
        server_output_id: None,
        api_metadata_bytes: None,
        suggestions: None,
        telemetry_events: vec![],
        model_info: None,
        request_cost: None,
    };

    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: vec![],
        output_status: AIAgentOutputStatus::Finished {
            finished_output: FinishedAIAgentOutput::Success {
                output: Shared::new(output),
            },
        },
        added_message_ids: HashSet::new(),
        start_time: Local::now(),
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory: None,
        model_id: LLMId::from(""),
        request_cost: None,
        coding_model_id: LLMId::from(""),
        cli_agent_model_id: LLMId::from(""),
        computer_use_model_id: LLMId::from(""),
        response_initiator: None,
    }
}

#[test]
fn test_remove_task() {
    let task = create_test_task_with_exchanges(3);
    let task_id = task.id().clone();
    let mut store = TaskStore::with_root_task(task);

    assert_eq!(store.task_count(), 1);
    assert_eq!(store.exchange_count(), 3);

    let removed = store.remove(&task_id);
    assert!(removed.is_some());
    assert_eq!(store.task_count(), 0);
    assert_eq!(store.exchange_count(), 0);
    assert!(store.get(&task_id).is_none());
}

#[test]
fn test_remove_nonexistent_task() {
    let task = create_test_task_with_exchanges(2);
    let mut store = TaskStore::with_root_task(task);

    let nonexistent_id = TaskId::new(Uuid::new_v4().to_string());
    let removed = store.remove(&nonexistent_id);
    assert!(removed.is_none());
    assert_eq!(store.task_count(), 1);
}

#[test]
fn test_set_root_task_replaces_old() {
    let task1 = create_test_task_with_exchanges(2);
    let task1_id = task1.id().clone();
    let mut store = TaskStore::with_root_task(task1);

    let task2 = create_test_task_with_exchanges(3);
    let task2_id = task2.id().clone();
    let task2_exchange_ids: Vec<_> = task2.exchanges().map(|e| e.id).collect();
    store.set_root_task(task2);

    assert_eq!(store.task_count(), 1);
    assert_eq!(store.exchange_count(), 3);
    assert!(store.get(&task1_id).is_none());
    assert!(store.get(&task2_id).is_some());
    assert_eq!(store.root_task_id(), &task2_id);

    let actual_ids: Vec<_> = store.all_exchanges().map(|e| e.id).collect();
    assert_eq!(actual_ids, task2_exchange_ids);
}

#[test]
fn test_remove_task_exchange() {
    let task = create_test_task_with_exchanges(3);
    let task_id = task.id().clone();
    let exchange_ids: Vec<_> = task.exchanges().map(|e| e.id).collect();
    let mut store = TaskStore::with_root_task(task);

    // First verify initial state
    assert_eq!(store.exchange_count(), 3);

    // Remove the middle exchange
    let removed = store.remove_task_exchange(&task_id, exchange_ids[1]);
    assert!(removed.is_some());
    assert_eq!(removed.unwrap().id, exchange_ids[1]);

    // After removal, we should have 2 exchanges
    assert_eq!(store.exchange_count(), 2);

    // Verify the remaining exchanges are correct
    let remaining_ids: Vec<_> = store.all_exchanges().map(|e| e.id).collect();
    assert_eq!(remaining_ids, vec![exchange_ids[0], exchange_ids[2]]);
}

#[test]
fn test_remove_exchange_from_nonexistent_task() {
    let task = create_test_task_with_exchanges(1);
    let exchange_id = task.exchanges().next().unwrap().id;
    let mut store = TaskStore::with_root_task(task);

    let nonexistent_task_id = TaskId::new(Uuid::new_v4().to_string());
    let result = store.remove_task_exchange(&nonexistent_task_id, exchange_id);
    assert!(result.is_none());
    assert_eq!(store.exchange_count(), 1);
}

#[test]
fn test_multiple_tasks_exchange_order() {
    // Create root task with 1 exchange, then add subagent call exchange
    let root_task = create_test_task_with_exchanges(1);
    let root_task_id = root_task.id().clone();
    let first_root_exchange_id = root_task.exchanges().next().unwrap().id;
    let mut store = TaskStore::with_root_task(root_task);

    // Create subtask with 2 exchanges
    let subtask = create_test_subtask_with_exchanges(2);
    let subtask_id = subtask.id().clone();
    let subtask_exchange_ids: Vec<_> = subtask.exchanges().map(|e| e.id).collect();

    // Add subagent call exchange to root
    let subagent_exchange = create_exchange_with_subagent_call(&subtask_id);
    let subagent_exchange_id = subagent_exchange.id;
    store.append_exchange(&root_task_id, subagent_exchange);

    // Insert subtask - now linked via subagent call
    store.insert(subtask);

    // 1 root + 1 subagent call + 2 subtask = 4 exchanges
    assert_eq!(store.exchange_count(), 4);

    // First/last should still work
    assert_eq!(
        store.first_exchange().map(|e| e.id),
        Some(first_root_exchange_id)
    );
    assert_eq!(
        store.latest_exchange().map(|e| e.id),
        Some(subtask_exchange_ids[1])
    );

    // Order: root[0], root[subagent_call], subtask[0], subtask[1]
    let all_ids: Vec<_> = store.all_exchanges().map(|e| e.id).collect();
    assert_eq!(all_ids.len(), 4);
    assert_eq!(all_ids[0], first_root_exchange_id);
    assert_eq!(all_ids[1], subagent_exchange_id);
    assert_eq!(all_ids[2], subtask_exchange_ids[0]);
    assert_eq!(all_ids[3], subtask_exchange_ids[1]);
}

#[test]
fn test_modify_task_conditional_rebuild() {
    let task = create_test_task_with_exchanges(2);
    let task_id = task.id().clone();
    let original_exchange_ids: Vec<_> = task.exchanges().map(|e| e.id).collect();
    let mut store = TaskStore::with_root_task(task);

    // Verify initial state
    assert_eq!(store.exchange_count(), 2);

    // modify_task with no exchange change should still work
    let result = store.modify_task(&task_id, |task| {
        assert_eq!(task.exchanges_len(), 2);
        "no change"
    });
    assert_eq!(result, Some("no change"));
    assert_eq!(store.exchange_count(), 2);

    // modify_task that adds an exchange should update the index
    let new_exchange = create_test_exchange();
    let new_exchange_id = new_exchange.id;
    store.modify_task(&task_id, |task| {
        task.append_exchange(new_exchange);
    });
    assert_eq!(store.exchange_count(), 3);
    assert_eq!(store.latest_exchange().map(|e| e.id), Some(new_exchange_id));

    // Verify all exchanges are in the index
    let all_ids: Vec<_> = store.all_exchanges().map(|e| e.id).collect();
    assert_eq!(all_ids.len(), 3);
    assert_eq!(all_ids[0], original_exchange_ids[0]);
    assert_eq!(all_ids[1], original_exchange_ids[1]);
    assert_eq!(all_ids[2], new_exchange_id);
}

// =============================================================================
// Subtask Linearization Tests
// =============================================================================

#[test]
fn test_all_exchanges_by_task_with_subtasks() {
    // Create a subtask
    let subtask = create_test_subtask_with_exchanges(2);
    let subtask_id = subtask.id().clone();
    let subtask_exchange_ids: Vec<_> = subtask.exchanges().map(|e| e.id).collect();

    // Create root task with a call to subtask
    let mut root_task = Task::new_optimistic_root();
    let root_id = root_task.id().clone();

    let root_exchange1 = create_test_exchange();
    let root_exchange1_id = root_exchange1.id;
    root_task.append_exchange(root_exchange1);

    let call_to_subtask = create_exchange_with_subagent_call(&subtask_id);
    let call_exchange_id = call_to_subtask.id;
    root_task.append_exchange(call_to_subtask);

    let root_exchange3 = create_test_exchange();
    let root_exchange3_id = root_exchange3.id;
    root_task.append_exchange(root_exchange3);

    // Build the store
    let mut store = TaskStore::with_root_task(root_task);
    store.insert(subtask);

    // Check all_exchanges_by_task grouping
    let by_task = store.all_exchanges_by_task();

    // Should have 3 groups: root[0-1], subtask[0-1], root[2]
    assert_eq!(by_task.len(), 3);

    // First group: root task's first two exchanges
    assert_eq!(by_task[0].0, root_id);
    assert_eq!(by_task[0].1.len(), 2);
    assert_eq!(by_task[0].1[0].id, root_exchange1_id);
    assert_eq!(by_task[0].1[1].id, call_exchange_id);

    // Second group: subtask's exchanges
    assert_eq!(by_task[1].0, subtask_id);
    assert_eq!(by_task[1].1.len(), 2);
    assert_eq!(by_task[1].1[0].id, subtask_exchange_ids[0]);
    assert_eq!(by_task[1].1[1].id, subtask_exchange_ids[1]);

    // Third group: root task's last exchange
    assert_eq!(by_task[2].0, root_id);
    assert_eq!(by_task[2].1.len(), 1);
    assert_eq!(by_task[2].1[0].id, root_exchange3_id);
}
