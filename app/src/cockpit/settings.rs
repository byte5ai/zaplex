//! Scalar policy settings for the cockpit. Accounts are *discovered*, not
//! configured, so no list settings live here (per the Increment 1 design §7).

use settings::{
    macros::define_settings_group, RespectUserSyncSetting, SupportedPlatforms, SyncToCloud,
};

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
    ]
);
