//! Adapt warp `api::Message` sequences to the [`MessageRef`] trait for [`super::algorithm`] operations.
//!
//! ## Semantic mapping to opencode `MessageV2.WithParts`
//!
//! opencode: a user/assistant message contains multiple parts (text/tool/file/...);
//! warp: a protobuf `api::Message` is fine-grained (UserQuery / AgentReasoning / AgentOutput / ToolCall / ToolCallResult each independent).
//!
//! This projection maps each warp `api::Message` **one-to-one** to a `MessageRef`.
//! Turn detection still cuts on user message boundaries — a user message followed by consecutive non-user messages forms a turn.
//! This does not affect the correctness of [`super::algorithm::turns`] / [`super::algorithm::select`] algorithms.
//!
//! Prune decisions target `Role::Tool` (ToolCallResult) — each ToolCallResult is a candidate.
//! Callers must pre-index all ToolCalls' `tool_call_id → tool_name` mappings in the conversation into [`ToolNameLookup`].

use std::collections::HashMap;

use warp_multi_agent_api as api;

use super::algorithm::{MessageRef, Role, ToolOutputRef};
use super::state::CompactionState;

/// `tool_call_id → tool_name` index, used during projection for:
/// 1. Annotating ToolCallResult with tool_name (for PRUNE_PROTECTED_TOOLS check)
/// 2. Letting prune decisions skip protected tools (e.g., `skill`)
pub type ToolNameLookup = HashMap<String, String>;

/// Given a group of tasks, extract all ToolCalls' `(tool_call_id, tool_name)` pairs.
pub fn build_tool_name_lookup<'a, I>(messages: I) -> ToolNameLookup
where
    I: IntoIterator<Item = &'a api::Message>,
{
    let mut out = ToolNameLookup::new();
    for msg in messages {
        if let Some(api::message::Message::ToolCall(tc)) = &msg.message {
            // Use the protobuf tool_call.tool enum variant name directly
            let name = tool_name_for(tc).unwrap_or_default();
            out.insert(tc.tool_call_id.clone(), name);
        }
    }
    out
}

/// Extract the "tool name" from a protobuf ToolCall.
///
/// This projection only needs to recognize tools in [`PRUNE_PROTECTED_TOOLS`](`super::consts::PRUNE_PROTECTED_TOOLS`)
/// (currently only "skill", corresponding to warp's `Tool::ReadSkill`); other tools return an empty string —
/// in prune decisions, an empty string matches no protected entry, so behavior is correct (allowed to prune).
fn tool_name_for(tc: &api::message::ToolCall) -> Option<String> {
    use api::message::tool_call::Tool;
    let t = tc.tool.as_ref()?;
    let s = match t {
        Tool::ReadSkill(_) => "skill",
        _ => "",
    };
    Some(s.to_string())
}

/// View of a single `api::Message`.
#[derive(Clone, Copy)]
pub struct WarpMessageView<'a> {
    pub msg: &'a api::Message,
    pub state: &'a CompactionState,
    pub tool_names: &'a ToolNameLookup,
}

/// Estimate the token usage of a single message — sum visible text character count / 4.
fn estimate_message(msg: &api::Message) -> usize {
    use super::token::estimate;
    use api::message::Message as M;
    let chars = msg
        .message
        .as_ref()
        .map(|inner| match inner {
            M::UserQuery(u) => u.query.chars().count(),
            M::AgentOutput(a) => a.text.chars().count(),
            M::AgentReasoning(r) => r.reasoning.chars().count(),
            M::ToolCall(_) => msg.server_message_data.chars().count().max(64),
            M::ToolCallResult(tcr) => {
                // Prefer estimate from result oneof; fall back to server_message_data.
                // Simplification: all use character count; result.estimate uses Debug repr.
                let from_oneof = tcr
                    .result
                    .as_ref()
                    .map(|r| format!("{r:?}").chars().count())
                    .unwrap_or(0);
                from_oneof
                    .max(msg.server_message_data.chars().count())
                    .max(32)
            }
            _ => 0,
        })
        .unwrap_or(0);
    // Same algorithm as opencode: chars / 4 rounded.
    estimate(&" ".repeat(chars))
}

impl<'a> MessageRef for WarpMessageView<'a> {
    type Id = String;
    type CallId = String;

    fn id(&self) -> String {
        self.msg.id.clone()
    }

    fn role(&self) -> Role {
        use api::message::Message as M;
        match &self.msg.message {
            Some(M::UserQuery(_)) => Role::User,
            Some(M::ToolCallResult(_)) => Role::Tool,
            // AgentOutput / AgentReasoning / ToolCall / others → Assistant
            _ => Role::Assistant,
        }
    }

    fn is_compaction_marker(&self) -> bool {
        // Only user messages with a compaction_trigger marker count
        if self.role() != Role::User {
            return false;
        }
        self.state
            .marker(&self.msg.id)
            .map(|m| m.compaction_trigger.is_some())
            .unwrap_or(false)
    }

    fn is_summary(&self) -> bool {
        // Only assistant messages can be summaries
        if self.role() != Role::Assistant {
            return false;
        }
        self.state
            .marker(&self.msg.id)
            .map(|m| m.is_summary)
            .unwrap_or(false)
    }

    fn estimate_size(&self) -> usize {
        estimate_message(self.msg)
    }

    fn tool_outputs(&self) -> Vec<ToolOutputRef<String>> {
        let Some(api::message::Message::ToolCallResult(tcr)) = &self.msg.message else {
            return Vec::new();
        };
        let tool_name = self
            .tool_names
            .get(&tcr.tool_call_id)
            .cloned()
            .unwrap_or_default();
        let already_compacted = self
            .state
            .marker(&self.msg.id)
            .and_then(|m| m.tool_output_compacted_at)
            .is_some();
        // output_size reuses estimate_message — ToolCallResult path uses character count from result/server_message_data
        let output_size = estimate_message(self.msg);
        vec![ToolOutputRef {
            call_id: tcr.tool_call_id.clone(),
            tool_name,
            output_size,
            completed: tcr.result.is_some() || !self.msg.server_message_data.is_empty(),
            already_compacted,
        }]
    }
}

/// Project a group of messages into `Vec<WarpMessageView>`, sorted by timestamp in ascending order —
/// consistent with the sort order in [`crate::ai::agent_providers::chat_stream::build_chat_request`].
pub fn project<'a>(
    messages: &'a [&'a api::Message],
    state: &'a CompactionState,
    tool_names: &'a ToolNameLookup,
) -> Vec<WarpMessageView<'a>> {
    let mut sorted: Vec<&api::Message> = messages.to_vec();
    sorted.sort_by_key(|m| {
        m.timestamp
            .as_ref()
            .map(|ts| (ts.seconds, ts.nanos))
            .unwrap_or((0, 0))
    });
    sorted
        .into_iter()
        .map(|msg| WarpMessageView {
            msg,
            state,
            tool_names,
        })
        .collect()
}
