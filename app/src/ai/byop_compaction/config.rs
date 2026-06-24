//! Compaction configuration — aligned with opencode `Config.compaction`:
//!
//! ```ts
//! compaction: {
//!   auto?: boolean,                  // default: true
//!   prune?: boolean,                 // default: true
//!   tail_turns?: NonNegativeInt,     // default: 2
//!   preserve_recent_tokens?: NonNegativeInt,
//!   reserved?: NonNegativeInt,
//! }
//! ```
//!
//! On the warp side, this is stored in BYOPCompactionSettings in settings/ai.rs,
//! deserialized, and converted to this struct.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Automatic overflow trigger switch. Default true.
    pub auto: bool,
    /// Tool output prune switch. Default true.
    pub prune: bool,
    /// Preserve the last N user turns as tail. Default 2.
    pub tail_turns: usize,
    /// Force override `preserve_recent_budget` (tokens). None uses opencode formula.
    pub preserve_recent_tokens: Option<usize>,
    /// Force override the reserved buffer in `usable()` (tokens). None uses min(20_000, max_output).
    pub reserved: Option<usize>,
    /// Optional summary-dedicated model reference. If set, use it; otherwise fall back to current conversation model.
    pub compaction_model: Option<CompactionModelRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompactionModelRef {
    pub provider_id: String,
    pub model_id: String,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            auto: true,
            prune: true,
            tail_turns: super::consts::DEFAULT_TAIL_TURNS,
            preserve_recent_tokens: None,
            reserved: None,
            compaction_model: None,
        }
    }
}

impl CompactionConfig {
    /// Calculate the actual preserve_recent_budget — aligned with opencode `compaction.ts:134-139`:
    /// `cfg.preserve_recent_tokens ?? min(MAX, max(MIN, floor(usable * 0.25)))`
    pub fn preserve_recent_budget(&self, usable_tokens: usize) -> usize {
        use super::consts::{MAX_PRESERVE_RECENT_TOKENS, MIN_PRESERVE_RECENT_TOKENS};
        self.preserve_recent_tokens.unwrap_or_else(|| {
            MAX_PRESERVE_RECENT_TOKENS.min(MIN_PRESERVE_RECENT_TOKENS.max(usable_tokens / 4))
        })
    }

    /// Deserialize from `AISettings` (aligned with opencode `Config.compaction.*`).
    ///
    /// Field mapping:
    /// - `byop_compaction_auto` → `auto`
    /// - `byop_compaction_prune` → `prune`
    /// - `byop_compaction_tail_turns` → `tail_turns` (0 is preserved, meaning tail splitting disabled)
    /// - `byop_compaction_preserve_recent_tokens` → `preserve_recent_tokens` (0 → None, use formula)
    /// - `byop_compaction_reserved` → `reserved` (0 → None, use min(20_000, max_output))
    /// - `byop_compaction_model_provider_id` + `byop_compaction_model_id` → `compaction_model`
    ///   (if either is empty → None, fall back to current conversation model)
    pub fn from_settings(app: &warpui::AppContext) -> Self {
        use crate::settings::AISettings;
        use warpui::SingletonEntity as _;
        let s = AISettings::as_ref(app);
        let provider_id = s.byop_compaction_model_provider_id.to_string();
        let model_id = s.byop_compaction_model_id.to_string();
        let compaction_model = if !provider_id.is_empty() && !model_id.is_empty() {
            Some(CompactionModelRef {
                provider_id,
                model_id,
            })
        } else {
            None
        };
        let preserve_raw: u32 = *s.byop_compaction_preserve_recent_tokens;
        let reserved_raw: u32 = *s.byop_compaction_reserved;
        Self {
            auto: *s.byop_compaction_auto,
            prune: *s.byop_compaction_prune,
            tail_turns: *s.byop_compaction_tail_turns as usize,
            preserve_recent_tokens: if preserve_raw == 0 {
                None
            } else {
                Some(preserve_raw as usize)
            },
            reserved: if reserved_raw == 0 {
                None
            } else {
                Some(reserved_raw as usize)
            },
            compaction_model,
        }
    }
}
