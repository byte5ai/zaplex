use super::{SubscriptionEvent, Usage};
use crate::ai::agent::api::convert_conversation::TurnOutcome;
use std::time::SystemTime;
use uuid::Uuid;
use warp_multi_agent_api as api;

enum AppendKind {
    Text(String),
    Reasoning(String),
}

/// Converts official CLI protocol events into the established persisted conversation contract.
pub(crate) struct ResponseEventAdapter {
    task_id: String,
    request_id: String,
    text_message_id: Option<String>,
    reasoning_message_id: Option<String>,
    latest_usage: Option<Usage>,
    context_window: Option<u64>,
}

impl ResponseEventAdapter {
    pub(crate) fn new(task_id: String, context_window: Option<u64>) -> Self {
        Self {
            task_id,
            request_id: Uuid::new_v4().to_string(),
            text_message_id: None,
            reasoning_message_id: None,
            latest_usage: None,
            context_window,
        }
    }

    pub(crate) fn stream_init(&self) -> api::ResponseEvent {
        api::ResponseEvent {
            r#type: Some(api::response_event::Type::Init(
                api::response_event::StreamInit {
                    request_id: self.request_id.clone(),
                    conversation_id: String::new(),
                    run_id: String::new(),
                },
            )),
        }
    }

    pub(crate) fn create_task(&self) -> api::ResponseEvent {
        api::ResponseEvent {
            r#type: Some(api::response_event::Type::ClientActions(
                api::response_event::ClientActions {
                    actions: vec![api::ClientAction {
                        action: Some(api::client_action::Action::CreateTask(
                            api::client_action::CreateTask {
                                task: Some(api::Task {
                                    id: self.task_id.clone(),
                                    description: String::new(),
                                    dependencies: None,
                                    messages: Vec::new(),
                                    summary: String::new(),
                                    server_data: String::new(),
                                }),
                            },
                        )),
                    }],
                },
            )),
        }
    }

    pub(crate) fn persist_user_query(&self, query: String) -> api::ResponseEvent {
        self.add_messages(vec![api::Message {
            id: Uuid::new_v4().to_string(),
            task_id: self.task_id.clone(),
            server_message_data: String::new(),
            citations: Vec::new(),
            message: Some(api::message::Message::UserQuery(api::message::UserQuery {
                query,
                context: None,
                ..Default::default()
            })),
            request_id: self.request_id.clone(),
            timestamp: Some(SystemTime::now().into()),
        }])
    }

    pub(crate) fn target(&self, target: &super::SubscriptionTarget) -> api::ResponseEvent {
        let session_target = crate::t!(
            "ai-footer-subscription-session-target",
            agent = target.installation.agent.display_name(),
            account = target.installation.account.display_name.as_str(),
            host = target.installation.host.display_name.as_str(),
            directory = target.working_directory.display().to_string(),
            model = target.model.display_name.as_str()
        );
        self.add_messages(vec![agent_output(
            &self.task_id,
            &self.request_id,
            session_target,
        )])
    }

    pub(crate) fn adapt(&mut self, event: SubscriptionEvent) -> Vec<api::ResponseEvent> {
        match event {
            SubscriptionEvent::SessionStarted(_) => Vec::new(),
            SubscriptionEvent::TextDelta(text) => vec![self.append_text(text)],
            SubscriptionEvent::ReasoningDelta(reasoning) => {
                vec![self.append_reasoning(reasoning)]
            }
            SubscriptionEvent::ToolStarted { id, name, input } => {
                vec![self.add_messages(vec![api::Message {
                    id: Uuid::new_v4().to_string(),
                    task_id: self.task_id.clone(),
                    server_message_data: format!("{name}\n{input}"),
                    citations: Vec::new(),
                    message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                        tool_call_id: id,
                        tool: None,
                    })),
                    request_id: self.request_id.clone(),
                    timestamp: Some(SystemTime::now().into()),
                }])]
            }
            SubscriptionEvent::ToolOutput {
                id,
                output,
                is_error,
            } => vec![self.add_messages(vec![api::Message {
                id: Uuid::new_v4().to_string(),
                task_id: self.task_id.clone(),
                server_message_data: if is_error {
                    format!("Error: {output}")
                } else {
                    output
                },
                citations: Vec::new(),
                message: Some(api::message::Message::ToolCallResult(
                    api::message::ToolCallResult {
                        tool_call_id: id,
                        context: None,
                        result: None,
                    },
                )),
                request_id: self.request_id.clone(),
                timestamp: Some(SystemTime::now().into()),
            }])],
            SubscriptionEvent::Diff(diff) => vec![self.add_messages(vec![agent_output(
                &self.task_id,
                &self.request_id,
                format!("```diff\n{diff}\n```"),
            )])],
            SubscriptionEvent::ApprovalRequested {
                kind, description, ..
            } => vec![self.add_messages(vec![agent_output(
                &self.task_id,
                &self.request_id,
                crate::t!(
                    "ai-footer-subscription-approval-required",
                    kind = kind,
                    description = description
                ),
            )])],
            SubscriptionEvent::Usage(usage) => {
                self.latest_usage = Some(usage);
                Vec::new()
            }
            SubscriptionEvent::TurnCompleted { .. } => {
                vec![self.turn_outcome(TurnOutcome::Completed), self.finished()]
            }
            SubscriptionEvent::Error { message, .. } => vec![self.turn_failed(message)],
        }
    }

    fn append_text(&mut self, text: String) -> api::ResponseEvent {
        match self.text_message_id.clone() {
            Some(message_id) => self.append(message_id, AppendKind::Text(text)),
            None => {
                let message_id = Uuid::new_v4().to_string();
                self.text_message_id = Some(message_id.clone());
                self.add_messages(vec![api::Message {
                    id: message_id,
                    task_id: self.task_id.clone(),
                    server_message_data: String::new(),
                    citations: Vec::new(),
                    message: Some(api::message::Message::AgentOutput(
                        api::message::AgentOutput { text },
                    )),
                    request_id: self.request_id.clone(),
                    timestamp: Some(SystemTime::now().into()),
                }])
            }
        }
    }

    fn append_reasoning(&mut self, reasoning: String) -> api::ResponseEvent {
        match self.reasoning_message_id.clone() {
            Some(message_id) => self.append(message_id, AppendKind::Reasoning(reasoning)),
            None => {
                let message_id = Uuid::new_v4().to_string();
                self.reasoning_message_id = Some(message_id.clone());
                self.add_messages(vec![api::Message {
                    id: message_id,
                    task_id: self.task_id.clone(),
                    server_message_data: String::new(),
                    citations: Vec::new(),
                    message: Some(api::message::Message::AgentReasoning(
                        api::message::AgentReasoning {
                            reasoning,
                            finished_duration: None,
                        },
                    )),
                    request_id: self.request_id.clone(),
                    timestamp: Some(SystemTime::now().into()),
                }])
            }
        }
    }

    /// Records that the turn failed. Persisted before the error ends the stream, so a
    /// restored conversation shows the failure and when it happened.
    pub(crate) fn turn_failed(&self, error_message: String) -> api::ResponseEvent {
        self.turn_outcome(TurnOutcome::Failed { error_message })
    }

    /// The turn's last persisted message: it keeps the outcome, and its timestamp
    /// is the turn's finish time, which appended streaming text cannot update.
    fn turn_outcome(&self, outcome: TurnOutcome) -> api::ResponseEvent {
        self.add_messages(vec![api::Message {
            id: Uuid::new_v4().to_string(),
            task_id: self.task_id.clone(),
            server_message_data: String::new(),
            citations: Vec::new(),
            message: Some(api::message::Message::ServerEvent(outcome.server_event())),
            request_id: self.request_id.clone(),
            timestamp: Some(SystemTime::now().into()),
        }])
    }

    fn add_messages(&self, messages: Vec<api::Message>) -> api::ResponseEvent {
        api::ResponseEvent {
            r#type: Some(api::response_event::Type::ClientActions(
                api::response_event::ClientActions {
                    actions: vec![api::ClientAction {
                        action: Some(api::client_action::Action::AddMessagesToTask(
                            api::client_action::AddMessagesToTask {
                                task_id: self.task_id.clone(),
                                messages,
                            },
                        )),
                    }],
                },
            )),
        }
    }

    fn append(&self, message_id: String, kind: AppendKind) -> api::ResponseEvent {
        let (message, mask_path) = match kind {
            AppendKind::Text(text) => (
                api::message::Message::AgentOutput(api::message::AgentOutput { text }),
                "agent_output.text",
            ),
            AppendKind::Reasoning(reasoning) => (
                api::message::Message::AgentReasoning(api::message::AgentReasoning {
                    reasoning,
                    finished_duration: None,
                }),
                "agent_reasoning.reasoning",
            ),
        };
        api::ResponseEvent {
            r#type: Some(api::response_event::Type::ClientActions(
                api::response_event::ClientActions {
                    actions: vec![api::ClientAction {
                        action: Some(api::client_action::Action::AppendToMessageContent(
                            api::client_action::AppendToMessageContent {
                                task_id: self.task_id.clone(),
                                message: Some(api::Message {
                                    id: message_id,
                                    task_id: self.task_id.clone(),
                                    server_message_data: String::new(),
                                    citations: Vec::new(),
                                    message: Some(message),
                                    request_id: self.request_id.clone(),
                                    timestamp: None,
                                }),
                                mask: Some(prost_types::FieldMask {
                                    paths: vec![mask_path.to_string()],
                                }),
                            },
                        )),
                    }],
                },
            )),
        }
    }

    fn finished(&self) -> api::ResponseEvent {
        let usage = self
            .latest_usage
            .as_ref()
            .zip(self.context_window)
            .and_then(|(usage, context_window)| {
                if context_window == 0 {
                    return None;
                }
                let used = usage.input_tokens + usage.output_tokens;
                Some(
                    api::response_event::stream_finished::ConversationUsageMetadata {
                        context_window_usage: (used as f32 / context_window as f32).clamp(0.0, 1.0),
                        summarized: false,
                        credits_spent: 0.0,
                        #[allow(deprecated)]
                        token_usage: Vec::new(),
                        tool_usage_metadata: None,
                        warp_token_usage: std::collections::HashMap::new(),
                        byok_token_usage: std::collections::HashMap::new(),
                    },
                )
            });
        api::ResponseEvent {
            r#type: Some(api::response_event::Type::Finished(
                api::response_event::StreamFinished {
                    reason: Some(api::response_event::stream_finished::Reason::Done(
                        api::response_event::stream_finished::Done {},
                    )),
                    conversation_usage_metadata: usage,
                    token_usage: Vec::new(),
                    should_refresh_model_config: false,
                    request_cost: None,
                },
            )),
        }
    }
}

fn agent_output(task_id: &str, request_id: &str, text: String) -> api::Message {
    api::Message {
        id: Uuid::new_v4().to_string(),
        task_id: task_id.to_string(),
        server_message_data: String::new(),
        citations: Vec::new(),
        message: Some(api::message::Message::AgentOutput(
            api::message::AgentOutput { text },
        )),
        request_id: request_id.to_string(),
        timestamp: Some(SystemTime::now().into()),
    }
}

#[cfg(test)]
#[path = "response_adapter_tests.rs"]
mod tests;
