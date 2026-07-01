use settings::{
    macros::define_settings_group, RespectUserSyncSetting, SupportedPlatforms, SyncToCloud,
};

use super::DriveSortOrder;

pub const HAS_AUTO_OPENED_WELCOME_FOLDER: &str = "HasAutoOpenedWelcomeFolder";

define_settings_group!(WarpDriveSettings, settings: [
    sorting_choice: WarpDriveSortingChoice {
        type: DriveSortOrder,
        default: DriveSortOrder::ByObjectType,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "warp_drive.sorting_choice",
        description: "The sort order for items in Zaplex Drive.",
    },
    sharing_onboarding_block_shown: WarpDriveSharingOnboardingBlockShown {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: true,
    },
    // Controls whether Zaplex Drive appears in the tools panel, command palette, and command search.
    // Zaplex Drive (inherited Warp Drive) is out of scope, so it is disabled by default: this is the
    // master switch that also gates the Drive keybindings (flags::ENABLE_ZAPLEX_DRIVE) and the
    // command-palette / command-search entries. The code is preserved as a template — see
    // docs/superpowers/specs/2026-07-01-self-contained-cleanup-plan.md
    enable_warp_drive: EnableWarpDrive {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Globally(RespectUserSyncSetting::Yes),
        private: false,
        toml_path: "warp_drive.enabled",
        description: "Whether Zaplex Drive is enabled.",
    },
]);

impl WarpDriveSettings {
    /// Returns whether Zaplex Drive should be considered enabled.
    /// Returns `false` when the user is anonymous or fully logged out,
    /// regardless of the user setting.
    pub fn is_warp_drive_enabled(app: &warpui::AppContext) -> bool {
        use warpui::SingletonEntity as _;
        let is_anonymous_or_logged_out = crate::auth::AuthStateProvider::as_ref(app)
            .get()
            .is_anonymous_or_logged_out();
        *Self::as_ref(app).enable_warp_drive && !is_anonymous_or_logged_out
    }
}
