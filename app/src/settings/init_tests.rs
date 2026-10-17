use settings::{
    is_settings_file_enabled, set_settings_file_enabled, PrivatePreferences, PublicPreferences,
    Setting, SettingsManager,
};
use warp_core::features::FeatureFlag;
use warp_core::settings::{macros::define_settings_group, SupportedPlatforms, SyncToCloud};
use warp_core::user_preferences::GetUserPreferences as _;
use warpui::SingletonEntity;
use warpui_extras::user_preferences;

use super::{
    migrate_native_settings_to_settings_file,
    SETTINGS_FILE_MIGRATION_COMPLETE_KEY,
};

// A minimal settings group with one public and one private setting, used to
// verify that migration only copies public settings.
define_settings_group!(MigrationTestSettings, settings: [
    public_setting: PublicSetting {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "migration_test.public_setting",
    },
    public_string_setting: PublicStringSetting {
        type: String,
        default: String::new(),
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "migration_test.public_string_setting",
    },
    private_setting: PrivateSetting {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    },
]);

/// Registers separate InMemoryPreferences singletons for public and private
/// stores, then adds a SettingsManager and the test settings group.
fn init_test_app(ctx: &mut warpui::AppContext) {
    ctx.add_singleton_model(move |_| {
        PublicPreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    ctx.add_singleton_model(move |_| -> PrivatePreferences {
        PrivatePreferences::new(Box::<user_preferences::in_memory::InMemoryPreferences>::default())
    });
    ctx.add_singleton_model(|_| SettingsManager::default());
    MigrationTestSettings::register(ctx);
}

struct SettingsFileEnabledGuard(bool);

impl SettingsFileEnabledGuard {
    fn new(enabled: bool) -> Self {
        let previous = is_settings_file_enabled();
        set_settings_file_enabled(enabled);
        Self(previous)
    }
}

impl Drop for SettingsFileEnabledGuard {
    fn drop(&mut self) {
        set_settings_file_enabled(self.0);
    }
}

// Only tests that toggle the process-global SettingsFile routing flag need to
// run serially.

// ---------------------------------------------------------------------------
// Tests for serde ↔ file-format mismatch during migration
// ---------------------------------------------------------------------------
//
// NotificationsSettings has #[serde(default)] and contains fields whose serde
// and SettingsValue file formats differ:
//   - NotificationsMode: serde uses PascalCase ("Enabled"), file uses snake_case ("enabled")
//   - Duration: serde uses {"secs":N,"nanos":N}, file uses a plain integer
//
// The migration reads serde-format values from the native store and feeds them
// through update_setting_with_storage_key, which tries from_file_value first.
// If from_file_value silently defaults fields (due to #[serde(default)]), the
// serde fallback is never reached and values are lost.

mod notifications_migration {
    use settings::{PrivatePreferences, PublicPreferences, SettingsManager};
    use warp_core::settings::{macros::define_settings_group, SupportedPlatforms, SyncToCloud};
    use warpui_extras::user_preferences;

    use crate::terminal::session_settings::NotificationsSettings;

    define_settings_group!(NotificationsMigrationTestSettings, settings: [
        notifications: MigrationTestNotifications {
            type: NotificationsSettings,
            default: NotificationsSettings::default(),
            supported_platforms: SupportedPlatforms::ALL,
            sync_to_cloud: SyncToCloud::Never,
            private: false,
            toml_path: "migration_test.notifications",
            max_table_depth: 1,
        },
    ]);

    pub fn init_notifications_migration_test_app(ctx: &mut warpui::AppContext) {
        ctx.add_singleton_model(move |_| {
            PublicPreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        ctx.add_singleton_model(move |_| -> PrivatePreferences {
            PrivatePreferences::new(
                Box::<user_preferences::in_memory::InMemoryPreferences>::default(),
            )
        });
        ctx.add_singleton_model(|_| SettingsManager::default());
        NotificationsMigrationTestSettings::register(ctx);
    }
}

// -- Migration integration tests: these demonstrate end-to-end data loss -----

#[test]
fn legacy_ssh_settings_survive_rename_without_eager_write() {
    use crate::settings::ssh::{EnableSshAutoDiscovery, EnableSshWrapper};
    use crate::terminal::zaplexify::settings::{
        AddedSubshellCommands, EnableSshZaplexification, SshExtensionInstallMode,
        SshExtensionInstallModeSetting, SshHostsDenylist, SubshellCommandsDenylist,
        UseSshTmuxWrapper,
    };
    use user_preferences::{toml_backed::TomlBackedUserPreferences, UserPreferences as _};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    let legacy = r#"
[warpify.ssh]
enable_legacy_ssh_wrapper = false
enable_ssh_auto_discovery = false
enable_ssh_warpification = false
ssh_hosts_denylist = ["production"]
use_ssh_tmux_wrapper = true
ssh_extension_install_mode = "never_install"
[warpify.subshells]
added_subshell_commands = ["custom-shell"]
subshell_commands_denylist = ["restricted-shell"]
"#;
    std::fs::write(&path, legacy).unwrap();
    let (prefs, error) = TomlBackedUserPreferences::new(path.clone());
    assert!(error.is_none());
    let prefs = prefs.with_document_migration(super::migrate_legacy_zaplexify_settings);

    assert_eq!(EnableSshWrapper::read_from_preferences(&prefs), Some(false));
    assert_eq!(EnableSshAutoDiscovery::read_from_preferences(&prefs), Some(false));
    assert_eq!(EnableSshZaplexification::read_from_preferences(&prefs), Some(false));
    assert_eq!(UseSshTmuxWrapper::read_from_preferences(&prefs), Some(true));
    assert_eq!(
        SshExtensionInstallModeSetting::read_from_preferences(&prefs),
        Some(SshExtensionInstallMode::NeverInstall),
    );
    assert_eq!(
        SshHostsDenylist::read_from_preferences(&prefs),
        Some(vec!["production".to_owned()]),
    );
    assert_eq!(
        AddedSubshellCommands::read_from_preferences(&prefs),
        Some(vec!["custom-shell".to_owned()]),
    );
    assert_eq!(
        SubshellCommandsDenylist::read_from_preferences(&prefs),
        Some(vec!["restricted-shell".to_owned()]),
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), legacy);

    // Reload must reapply the migration before settings consume the document.
    std::fs::write(&path, legacy.replace("production", "critical")).unwrap();
    prefs.reload_from_disk().unwrap();
    assert_eq!(
        SshHostsDenylist::read_from_preferences(&prefs),
        Some(vec!["critical".to_owned()]),
    );
    prefs
        .write_value_with_hierarchy("font_size", "14".to_owned(), Some("appearance"), None)
        .unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    let (reopened, error) = TomlBackedUserPreferences::new(path.clone());
    assert!(error.is_none());
    assert_eq!(EnableSshZaplexification::read_from_preferences(&reopened), Some(false));
    assert_eq!(
        SshExtensionInstallModeSetting::read_from_preferences(&reopened),
        Some(SshExtensionInstallMode::NeverInstall),
    );
    assert!(!written.contains("enable_ssh_warpification"));
}

#[test]
fn renamed_ssh_settings_win_and_reset_does_not_restore_legacy_value() {
    use crate::terminal::zaplexify::settings::{EnableSshZaplexification, SshHostsDenylist};
    use user_preferences::{toml_backed::TomlBackedUserPreferences, UserPreferences as _};

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    let original = r#"
[warpify.ssh]
enable_ssh_warpification = false
ssh_hosts_denylist = ["old-host"]
[zaplexify.ssh]
enable_ssh_zaplexification = true
ssh_hosts_denylist = []
"#;
    std::fs::write(&path, original).unwrap();
    let (prefs, error) = TomlBackedUserPreferences::new(path.clone());
    assert!(error.is_none());
    let prefs = prefs.with_document_migration(super::migrate_legacy_zaplexify_settings);
    assert_eq!(EnableSshZaplexification::read_from_preferences(&prefs), Some(true));
    assert_eq!(SshHostsDenylist::read_from_preferences(&prefs), Some(Vec::new()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
    prefs.remove_value_with_hierarchy("enable_ssh_zaplexification", Some("zaplexify.ssh")).unwrap();
    prefs.reload_from_disk().unwrap();
    assert_eq!(EnableSshZaplexification::read_from_preferences(&prefs), None);
}

#[test]
fn malformed_new_ssh_parent_and_parse_error_remain_untouched() {
    use user_preferences::{toml_backed::TomlBackedUserPreferences, UserPreferences as _};

    let mut document = r#"
zaplexify = "repair-me"
[warpify.ssh]
enable_ssh_warpification = false
"#.parse::<toml_edit::DocumentMut>().unwrap();
    let original = document.to_string();
    super::migrate_legacy_zaplexify_settings(&mut document);
    assert_eq!(document.to_string(), original);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.toml");
    std::fs::write(&path, "[broken").unwrap();
    let (prefs, error) = TomlBackedUserPreferences::new(path.clone());
    assert!(error.is_some());
    let prefs = prefs.with_document_migration(super::migrate_legacy_zaplexify_settings);
    prefs.write_value("unrelated", "true".to_owned()).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "[broken");
}

#[test]
#[serial_test::serial]
fn native_ssh_read_failure_does_not_write_toml_or_complete_migration() {
    use crate::terminal::zaplexify::settings::{EnableSshZaplexification, ZaplexifySettings};
    use user_preferences::{in_memory::InMemoryPreferences, UserPreferences};

    struct FailedReadPreferences(InMemoryPreferences);
    impl UserPreferences for FailedReadPreferences {
        fn read_value(&self, key: &str) -> Result<Option<String>, user_preferences::Error> {
            if key == "EnableSshWarpification" {
                return Err(std::io::Error::other("injected native read failure").into());
            }
            self.0.read_value(key)
        }
        fn write_value(&self, key: &str, value: String) -> Result<(), user_preferences::Error> {
            self.0.write_value(key, value)
        }
        fn remove_value(&self, key: &str) -> Result<(), user_preferences::Error> {
            self.0.remove_value(key)
        }
    }
    let prefs = FailedReadPreferences(InMemoryPreferences::default());
    assert!(super::read_native_ssh_migration_value(&prefs).is_err());
    prefs.write_value("EnableSshZaplexification", "true".to_owned()).unwrap();
    assert_eq!(
        super::read_native_ssh_migration_value(&prefs).unwrap().as_deref(),
        Some("true"),
    );
    prefs.remove_value("EnableSshZaplexification").unwrap();

    warpui::App::test((), move |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        let _settings_file_enabled = SettingsFileEnabledGuard::new(true);
        app.add_singleton_model(|_| PublicPreferences::new(Box::<InMemoryPreferences>::default()));
        app.add_singleton_model(move |_| PrivatePreferences::new(Box::new(prefs)));
        app.add_singleton_model(|_| SettingsManager::default());
        app.update(ZaplexifySettings::register);
        app.update(migrate_native_settings_to_settings_file);
        app.read(|ctx| {
            assert!(!*ZaplexifySettings::as_ref(ctx).enable_ssh_zaplexification.value());
            assert_eq!(
                ctx.private_user_preferences().read_value(SETTINGS_FILE_MIGRATION_COMPLETE_KEY).unwrap(),
                None,
            );
            assert_eq!(
                EnableSshZaplexification::preferences_for_setting(ctx)
                    .read_value_with_hierarchy(
                        EnableSshZaplexification::storage_key(),
                        EnableSshZaplexification::hierarchy(),
                    )
                    .unwrap(),
                None,
            );
        });
    });
}
