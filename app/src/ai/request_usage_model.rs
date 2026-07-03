//! Zaplex (Phase 3c subtask A1): Localized as a permanent "unlimited" stub.
//!
//! Historical responsibility: warp.dev server-side RPC-driven "monthly AI request quota" model.
//! Zaplex uses BYOP (Bring Your Own Provider), where users pay directly to LLM providers
//! and should never be constrained by cloud concepts like "remaining request count / upgrade CTA / buy extra credits".
//!
//! Write constraints:
//! * 30+ UI subscription points (`subscribe_to_model(&AIRequestUsageModel::handle(ctx), ...)`)
//!   are retained, but events are no longer triggered by any code path → subscription callbacks become silent no-ops.
//! * Files that overflow-use `RequestLimitInfo` / `RequestUsageInfo` / `BonusGrant` /
//!   `BonusGrantScope` / `RequestLimitRefreshDuration` /
//!   `BuyCreditsBannerDisplayState` / `AIRequestUsageModelEvent` /
//!   `AMBIENT_AGENT_TRIAL_CREDIT_THRESHOLD` (such as `workspaces/gql_convert.rs`,
//!   `ai_assistant/requests.rs`, `ai_assistant/mod.rs`,
//!   `settings/ai.rs`, `settings/ai_tests.rs`, `workspace/bonus_grant_notification_model.rs`,
//!   `settings_view/ai_page.rs`,
//!   `terminal/view/ambient_agent/first_time_setup.rs`, `agent_view/agent_message_bar.rs`)
//!   are outside this task's write domain → must continue to retain these type definitions and equivalent construction abilities in the stub,
//!   only removing RPC / caching / metering business logic.

use crate::{server_time::ServerTimestamp, workspaces::workspace::WorkspaceUid};
use chrono::{DateTime, Utc};
use instant::Instant;
use serde::{Deserialize, Serialize};
use warpui::{AppContext, Entity, ModelContext, SingletonEntity};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BonusGrantType {
    AmbientOnly,
    Any,
}

/// Threshold of ambient-only credits at which we surface upgrade/CTA UI.
///
/// Zaplex: In the local scenario, this will never be reached (because `ambient_only_credits_remaining` is always `None`),
/// but the constant definition is retained for compatibility with external imports.
pub const AMBIENT_AGENT_TRIAL_CREDIT_THRESHOLD: i32 = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BonusGrantScope {
    User,
    Workspace(WorkspaceUid),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum BuyCreditsBannerDisplayState {
    #[default]
    Hidden,
    OutOfCredits,
    MonthlyLimitReached,
}

#[derive(Clone, Debug)]
pub struct BonusGrant {
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub cost_cents: i32,
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
    pub grant_type: BonusGrantType,
    pub reason: String,
    pub user_facing_message: Option<String>,
    pub request_credits_granted: i32,
    pub request_credits_remaining: i32,
    pub scope: BonusGrantScope,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum RequestLimitRefreshDuration {
    Weekly,
    Monthly,
    EveryTwoWeeks,
}

/// Historical: Server-issued snapshot of "monthly request quota".
/// Zaplex: Retained only as a type shell (`AISettings::update_quota_info` / `ai_assistant/requests.rs`
/// and other out-of-domain files still construct this structure). `AIRequestUsageModel` no longer holds / caches / updates it.
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct RequestLimitInfo {
    pub limit: usize,
    pub num_requests_used_since_refresh: usize,
    pub next_refresh_time: ServerTimestamp,
    pub is_unlimited: bool,
    pub request_limit_refresh_duration: RequestLimitRefreshDuration,
    pub is_unlimited_voice: bool,
    #[serde(default)]
    pub voice_request_limit: usize,
    #[serde(default)]
    pub voice_requests_used_since_last_refresh: usize,
    #[serde(default)]
    pub max_files_per_repo: usize,
    #[serde(default)]
    pub embedding_generation_batch_size: usize,
}

fn default_voice_requests_limit() -> usize {
    10000
}

impl Default for RequestLimitInfo {
    /// Zaplex: No cloud-side quota; default value is treated as "unlimited".
    fn default() -> Self {
        Self {
            limit: usize::MAX,
            num_requests_used_since_refresh: 0,
            next_refresh_time: ServerTimestamp::new(Utc::now() + chrono::Duration::days(365)),
            is_unlimited: true,
            request_limit_refresh_duration: RequestLimitRefreshDuration::Monthly,
            is_unlimited_voice: true,
            voice_request_limit: default_voice_requests_limit(),
            voice_requests_used_since_last_refresh: 0,
            max_files_per_repo: usize::MAX,
            embedding_generation_batch_size: 100,
        }
    }
}

#[cfg(test)]
impl RequestLimitInfo {
    pub fn new_for_test(limit: usize, num_requests_used_since_refresh: usize) -> Self {
        Self {
            limit,
            num_requests_used_since_refresh,
            ..Self::default()
        }
    }
}

/// Historical: Aggregate structure returned by server's `getRequestLimitInfo`.
/// Zaplex: Retained only as a type shell (`ai_assistant/requests.rs` still constructs this type).
/// `AIRequestUsageModel` no longer consumes it.
pub struct RequestUsageInfo {
    pub request_limit_info: RequestLimitInfo,
    pub bonus_grants: Vec<BonusGrant>,
}

/// Zaplex: Model no longer holds any state.
pub struct AIRequestUsageModel;

impl Entity for AIRequestUsageModel {
    type Event = AIRequestUsageModelEvent;
}

/// Zaplex: Enum definition is retained to be compatible with subscription callback `match` patterns;
/// after localization, `AIRequestUsageModel` no longer emits any variants → all subscription callbacks become silent no-ops.
pub enum AIRequestUsageModelEvent {
    RequestUsageUpdated,
    RequestBonusRefunded {
        requests_refunded: i32,
        server_conversation_id: String,
        request_id: String,
    },
}

impl AIRequestUsageModel {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self
    }

    #[cfg(test)]
    pub fn new_for_test(_ctx: &mut ModelContext<Self>) -> Self {
        Self
    }

    pub fn last_update_time(&self) -> Option<Instant> {
        None
    }

    /// Zaplex: No cloud backend, no-op.
    pub fn refresh_request_usage_async(&mut self, _ctx: &mut ModelContext<Self>) {}

    /// Zaplex (localized): Always returns true; BYOP local runs are not constrained by cloud limits.
    pub fn has_requests_remaining(&self) -> bool {
        true
    }

    /// Zaplex (localized): Always returns true.
    /// AI availability depends only on whether the user has configured an API key (managed independently by `ApiKeyManager`),
    /// not on cloud-side metering components like `request_limit_info`.
    pub fn has_any_ai_remaining(&self, _ctx: &AppContext) -> bool {
        true
    }

    /// Zaplex (localized): No cloud-side metering; always returns 0.
    pub fn requests_used(&self) -> usize {
        0
    }

    /// Zaplex (localized): No cloud-side metering; always returns 0.0.
    pub fn request_percentage_used(&self) -> f32 {
        0.0
    }

    /// Zaplex (localized): No cloud-side limit; always returns `usize::MAX`.
    pub fn request_limit(&self) -> usize {
        usize::MAX
    }

    /// Zaplex (localized): Far-future placeholder time.
    pub fn next_refresh_time(&self) -> DateTime<Utc> {
        Utc::now() + chrono::Duration::days(365)
    }

    /// Zaplex (localized): Always unlimited.
    pub fn is_unlimited(&self) -> bool {
        true
    }

    pub fn refresh_duration_to_string(&self) -> String {
        "monthly".to_string()
    }

    /// Zaplex (localized): Local users have no bonus grants.
    pub fn bonus_grants(&self) -> &[BonusGrant] {
        &[]
    }

    /// Zaplex (localized): Local users have no concept of ambient-only credits.
    pub fn ambient_only_credits_remaining(&self) -> Option<i32> {
        None
    }

    /// Zaplex (localized): Local users have no concept of workspace bonus credits.
    pub fn total_workspace_bonus_credits_remaining(&self, _uid: WorkspaceUid) -> i32 {
        0
    }

    /// Zaplex (localized): Local users have no concept of workspace bonus credits.
    pub fn total_current_workspace_bonus_credits_remaining(&self, _ctx: &AppContext) -> i32 {
        0
    }

    /// Zaplex (localized): Purchasing extra credits does not apply.
    pub fn compute_buy_addon_credits_banner_display_state(
        &self,
        _ctx: &AppContext,
    ) -> BuyCreditsBannerDisplayState {
        BuyCreditsBannerDisplayState::Hidden
    }

    /// Zaplex (localized): No-op.
    pub fn dismiss_buy_credits_banner(&mut self, _ctx: &mut ModelContext<Self>) {}

    /// Zaplex (localized): No-op.
    pub fn enable_buy_credits_banner(&mut self, _ctx: &mut ModelContext<Self>) {}

    /// Zaplex (localized): Voice input is not constrained by cloud-side quota; always returns true.
    pub fn can_request_voice(&self) -> bool {
        true
    }
}

impl SingletonEntity for AIRequestUsageModel {}
