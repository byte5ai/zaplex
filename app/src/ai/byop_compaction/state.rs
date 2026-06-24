//! Compaction sidecar state — attached to `AIConversation`, decoupled from warp `api::Message` protocol.
//!
//! Because warp's `api::Message` comes from external protobuf dependency (`warp_multi_agent_api`),
//! we cannot add fields to mark `is_summary` / `compacted`, etc. This sidecar uses message_id indexing
//! to attach these "compaction metadata" on the conversation side.
//!
//! Schema version [`CompactionState::VERSION`] is manually bumped when the schema evolves;
//! old conversations that fail to deserialize degrade to `Default` (equivalent to "never compacted").

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

/// Source of compaction trigger. `Auto` is triggered only by token-overflow, `Manual` is from /compact /compact-and.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompactionTrigger {
    Manual,
    Auto,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageMarker {
    /// This assistant message is a summary; content is used to replace preceding history during request assembly.
    #[serde(default)]
    pub is_summary: bool,
    /// This user message is a compaction trigger placeholder (opencode `parts.some(p => p.type === "compaction")`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_trigger: Option<CompactionTrigger>,
    /// Output of this ToolCallResult has been pruned, replaced with placeholder during projection. Unix epoch ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_output_compacted_at: Option<u64>,
    /// Marker for synthetic "Continue..." user message synthesized during auto-resume
    /// (aligned with opencode `metadata.compaction_continue`).
    #[serde(default)]
    pub synthetic_continue: bool,
}

/// A completed compaction interval (aligned with opencode `completedCompactions()` return items).
///
/// `user_msg_id` is the user message triggering the summary (with compaction_trigger marker),
/// `assistant_msg_id` is the synthesized summary AgentOutput message. Both are treated as covered
/// in [`CompactionState::hidden_message_ids`] and skipped during projection — but the summary text
/// itself is extracted and filled in place of the head region.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedCompaction {
    pub user_msg_id: String,
    pub assistant_msg_id: String,
    /// Head-region message ids covered by this summary, all hidden when projecting ordinary requests.
    #[serde(default)]
    pub head_message_ids: Vec<String>,
    /// Tail start message id, used for split validation / debug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_start_id: Option<String>,
    /// Summary content (can be extracted from assistant message directly, but cached in state for build_prompt to get previous_summary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_text: Option<String>,
    pub auto: bool,
    pub overflow: bool,
}

/// Sidecar table persisted together with `AIConversation`.
///
/// Default value = empty table = uncompacted state, completely non-intrusive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionState {
    /// Schema version, bumped when evolving.
    #[serde(default = "CompactionState::current_version")]
    pub version: u32,
    #[serde(default)]
    markers: HashMap<String, MessageMarker>,
    #[serde(default)]
    completed: Vec<CompletedCompaction>,
}

impl Default for CompactionState {
    fn default() -> Self {
        Self {
            version: Self::VERSION,
            markers: HashMap::new(),
            completed: Vec::new(),
        }
    }
}

impl CompactionState {
    pub const VERSION: u32 = 2;
    fn current_version() -> u32 {
        Self::VERSION
    }

    pub fn marker(&self, msg_id: &str) -> Option<&MessageMarker> {
        self.markers.get(msg_id)
    }

    /// Write a marker (merge into existing marker, not overwrite the entire marker).
    pub fn upsert_marker(&mut self, msg_id: impl Into<String>, f: impl FnOnce(&mut MessageMarker)) {
        let entry = self.markers.entry(msg_id.into()).or_default();
        f(entry);
    }

    /// Mark that a ToolCallResult's output has been pruned.
    pub fn mark_tool_compacted(&mut self, msg_id: impl Into<String>, now_ms: u64) {
        self.upsert_marker(msg_id, |m| m.tool_output_compacted_at = Some(now_ms));
    }

    /// Push a completed compaction.
    pub fn push_completed(&mut self, c: CompletedCompaction) {
        // Simultaneously mark both user and assistant with markers for individual identification during projection.
        self.upsert_marker(c.user_msg_id.clone(), |m| {
            m.compaction_trigger = Some(if c.auto {
                CompactionTrigger::Auto
            } else {
                CompactionTrigger::Manual
            });
        });
        self.upsert_marker(c.assistant_msg_id.clone(), |m| m.is_summary = true);
        self.completed.push(c);
    }

    /// Mark a synthetic "Continue..." user message (synthesized by auto+overflow path).
    pub fn mark_synthetic_continue(&mut self, msg_id: impl Into<String>) {
        self.upsert_marker(msg_id, |m| m.synthetic_continue = true);
    }

    /// Get the last completed compaction (used as incremental summary anchor for [`super::prompt::build_prompt`]).
    pub fn previous_summary(&self) -> Option<&str> {
        self.completed
            .last()
            .and_then(|c| c.summary_text.as_deref())
    }

    pub fn completed(&self) -> &[CompletedCompaction] {
        &self.completed
    }

    /// All message ids that should be skipped when assembling requests (aligned with opencode `hidden`):
    /// head_message_ids + user_msg_id + assistant_msg_id for each completed compaction interval.
    ///
    /// Note: this is just the set of "message ids originally to be hidden from history", **not** including
    /// the summary itself — summary text is inserted by request projection at the compaction trigger user_msg_id position.
    pub fn hidden_message_ids(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for c in &self.completed {
            out.extend(c.head_message_ids.iter().cloned());
            out.insert(c.user_msg_id.clone());
            out.insert(c.assistant_msg_id.clone());
        }
        out
    }

    /// Debug / test entry point: check if a marker exists.
    #[cfg(test)]
    pub(crate) fn marker_count(&self) -> usize {
        self.markers.len()
    }
}

#[cfg(test)]
mod state_tests {
    use super::*;

    fn cc(uid: &str, aid: &str, auto: bool) -> CompletedCompaction {
        CompletedCompaction {
            user_msg_id: uid.to_string(),
            assistant_msg_id: aid.to_string(),
            head_message_ids: Vec::new(),
            tail_start_id: None,
            summary_text: Some(format!("summary-{aid}")),
            auto,
            overflow: false,
        }
    }

    #[test]
    fn push_completed_marks_both_messages() {
        let mut s = CompactionState::default();
        s.push_completed(cc("u1", "a1", true));
        assert!(s.marker("u1").unwrap().compaction_trigger == Some(CompactionTrigger::Auto));
        assert!(s.marker("a1").unwrap().is_summary);
    }

    #[test]
    fn previous_summary_returns_last() {
        let mut s = CompactionState::default();
        s.push_completed(cc("u1", "a1", false));
        s.push_completed(cc("u2", "a2", false));
        assert_eq!(s.previous_summary(), Some("summary-a2"));
    }

    #[test]
    fn hidden_message_ids_covers_all_completed() {
        let mut s = CompactionState::default();
        s.push_completed(cc("u1", "a1", false));
        s.push_completed(cc("u2", "a2", false));
        let h = s.hidden_message_ids();
        assert!(h.contains("u1"));
        assert!(h.contains("a1"));
        assert!(h.contains("u2"));
        assert!(h.contains("a2"));
        assert_eq!(h.len(), 4);
    }

    #[test]
    fn hidden_message_ids_includes_head_message_ids() {
        let mut s = CompactionState::default();
        let mut c = cc("u1", "a1", false);
        c.head_message_ids = vec!["h1".to_string(), "h2".to_string(), "u1".to_string()];
        s.push_completed(c);
        let h = s.hidden_message_ids();
        assert!(h.contains("h1"));
        assert!(h.contains("h2"));
        assert!(h.contains("u1"));
        assert!(h.contains("a1"));
        assert_eq!(h.len(), 4);
    }

    #[test]
    fn v1_completed_compaction_deserializes_to_empty_head_message_ids() {
        let json = r#"{
            "user_msg_id":"u1",
            "assistant_msg_id":"a1",
            "tail_start_id":null,
            "summary_text":"summary",
            "auto":false,
            "overflow":false
        }"#;
        let c: CompletedCompaction = serde_json::from_str(json).unwrap();
        assert!(c.head_message_ids.is_empty());
    }

    #[test]
    fn upsert_marker_merges() {
        let mut s = CompactionState::default();
        s.upsert_marker("m1", |m| m.is_summary = true);
        s.upsert_marker("m1", |m| m.synthetic_continue = true);
        let m = s.marker("m1").unwrap();
        assert!(m.is_summary);
        assert!(m.synthetic_continue);
        assert_eq!(s.marker_count(), 1);
    }

    #[test]
    fn default_serializable_roundtrip() {
        let s = CompactionState::default();
        let j = serde_json::to_string(&s).unwrap();
        let back: CompactionState = serde_json::from_str(&j).unwrap();
        assert_eq!(back.version, CompactionState::VERSION);
        assert!(back.completed.is_empty());
    }
}
