//! Scalar policy settings for the cockpit. Accounts are *discovered*, not
//! configured, so no list settings live here (per the Increment 1 design §7).

use serde::{Deserialize, Serialize};
use settings::{
    macros::define_settings_group, RespectUserSyncSetting, SupportedPlatforms, SyncToCloud,
};

/// Controls how a persisted CLI-agent binding is handled when its pane is restored.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    Eq,
    PartialEq,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    settings_value::SettingsValue,
)]
#[schemars(rename_all = "snake_case")]
pub enum CockpitContinuationMode {
    Off,
    #[default]
    Prompt,
    Auto,
}

define_settings_group!(CockpitSettings,
    settings: [
        enabled: CockpitEnabled {
            type: bool,
            default: true,
            supported_platforms: SupportedPlatforms::ALL,
            sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
            private: false,
            storage_key: "CockpitEnabled",
            toml_path: "cockpit.enabled",
            description: "Whether the cockpit account/usage/cost data layer is active.",
        },
        budget_5h: CockpitBudget5h {
            type: u32,
            default: 0,
            supported_platforms: SupportedPlatforms::ALL,
            sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
            private: false,
            storage_key: "CockpitBudget5h",
            toml_path: "cockpit.budget_5h",
            description: "Per-5h-block token budget used for heat (0 = built-in estimate).",
        },
        budget_week: CockpitBudgetWeek {
            type: u32,
            default: 0,
            supported_platforms: SupportedPlatforms::ALL,
            sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
            private: false,
            storage_key: "CockpitBudgetWeek",
            toml_path: "cockpit.budget_week",
            description: "Per-week token budget (0 = built-in estimate). Reserved for later use.",
        },
        oauth_usage: CockpitOauthUsage {
            type: bool,
            default: true,
            supported_platforms: SupportedPlatforms::ALL,
            sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
            private: false,
            storage_key: "CockpitOauthUsage",
            toml_path: "cockpit.oauth_usage",
            description: "Show real Claude subscription utilization from the account's own OAuth usage endpoint (read-only; off = transcript-based estimates only, no requests).",
        },
        continuation_mode: CockpitContinuationModeSetting {
            type: CockpitContinuationMode,
            default: CockpitContinuationMode::Prompt,
            supported_platforms: SupportedPlatforms::DESKTOP,
            sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
            private: false,
            storage_key: "CockpitContinuationMode",
            toml_path: "cockpit.continuation_mode",
            description: "How restored CLI-agent panes continue: off keeps a shell, prompt shows a one-click banner, and auto resumes through the same identity-bound path.",
        },
        attention_dnd: CockpitAttentionDnd {
            type: bool,
            default: false,
            supported_platforms: SupportedPlatforms::ALL,
            sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
            private: false,
            storage_key: "CockpitAttentionDnd",
            toml_path: "cockpit.attention_dnd",
            description: "Focus / do-not-disturb for the attention signal: silence the calm→needy chime entirely. The Dock badge and the Offene-Punkte inbox still update passively (they never make noise).",
        },
        attention_sound: CockpitAttentionSound {
            type: bool,
            default: true,
            supported_platforms: SupportedPlatforms::ALL,
            sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
            private: false,
            storage_key: "CockpitAttentionSound",
            toml_path: "cockpit.attention_sound",
            description: "Play a single subtle chime the moment the fleet goes from 'nothing for me' to 'something for me' (0 → >0 waiting). Never per session. Off = the badge/inbox carry attention silently.",
        },
    ]
);
