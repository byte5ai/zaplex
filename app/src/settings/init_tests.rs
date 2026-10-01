use instant::Duration;
use settings::{
    is_settings_file_enabled, set_settings_file_enabled, PrivatePreferences, PublicPreferences,
    Setting, SettingsManager,
};
use settings_value::SettingsValue;
use warp_core::features::FeatureFlag;
use warp_core::settings::{macros::define_settings_group, SupportedPlatforms, SyncToCloud};
use warp_core::user_preferences::GetUserPreferences as _;
use warpui::SingletonEntity;
use warpui_extras::user_preferences;

use crate::terminal::session_settings::{NotificationsMode, NotificationsSettings};

use super::{
    migrate_native_settings_to_settings_file, needs_settings_file_migration,
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

#[test]
#[serial_test::serial]
fn test_migration_copies_public_settings_from_native_store() {
    warpui::App::test((), |mut app| async move {
        // Enable the settings file so `preferences_for_setting` routes
        // public setting writes to the Model singleton (not the private store).
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        let _settings_file_enabled = SettingsFileEnabledGuard::new(true);

        app.update(init_test_app);

        // Seed the native (private) store with values for both settings.
        app.update(|ctx| {
            let native = ctx.private_user_preferences();
            native
                .write_value("PublicSetting", "true".to_owned())
                .unwrap();
            native
                .write_value("PrivateSetting", "true".to_owned())
                .unwrap();
        });

        // Before migration, in-memory values should still be defaults (the
        // public store is empty and registration read from there).
        app.read(|ctx| {
            let settings = MigrationTestSettings::as_ref(ctx);
            assert!(!*settings.public_setting.value());
            assert!(!*settings.private_setting.value());
        });

        // Run the migration.
        app.update(|ctx| {
            migrate_native_settings_to_settings_file(ctx);
        });

        // The public setting should now reflect the native store value.
        app.read(|ctx| {
            let settings = MigrationTestSettings::as_ref(ctx);
            assert!(
                *settings.public_setting.value(),
                "public setting should have been migrated from native store"
            );
        });

        // The public store should now contain the migrated value.
        app.read(|ctx| {
            let public = PublicSetting::preferences_for_setting(ctx);
            let stored = public
                .read_value_with_hierarchy(PublicSetting::storage_key(), PublicSetting::hierarchy())
                .unwrap();
            assert_eq!(stored, Some("true".to_owned()));
        });
        // The private setting should NOT have been touched by migration
        // (it's private, so migration skips it). The in-memory value stays
        // at default because register() read from the public store (empty).
        app.read(|ctx| {
            let settings = MigrationTestSettings::as_ref(ctx);
            assert!(
                !*settings.private_setting.value(),
                "private setting should not be affected by migration"
            );
        });
    });
}

#[test]
fn test_migration_writes_marker_to_native_store() {
    warpui::App::test((), |mut app| async move {
        app.update(init_test_app);

        // No marker before migration.
        app.read(|ctx| {
            let marker = ctx
                .private_user_preferences()
                .read_value(SETTINGS_FILE_MIGRATION_COMPLETE_KEY)
                .unwrap();
            assert!(marker.is_none());
        });

        app.update(|ctx| {
            migrate_native_settings_to_settings_file(ctx);
        });

        // Marker should now be present.
        app.read(|ctx| {
            let marker = ctx
                .private_user_preferences()
                .read_value(SETTINGS_FILE_MIGRATION_COMPLETE_KEY)
                .unwrap();
            assert!(marker.is_some(), "migration marker should be written");
        });
    });
}

#[test]
#[serial_test::serial]
fn test_migration_skips_settings_absent_from_native_store() {
    warpui::App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        let _settings_file_enabled = SettingsFileEnabledGuard::new(true);
        app.update(init_test_app);

        // Don't seed anything in the native store — all settings are absent.

        app.update(|ctx| {
            migrate_native_settings_to_settings_file(ctx);
        });

        // Settings should remain at defaults.
        app.read(|ctx| {
            let settings = MigrationTestSettings::as_ref(ctx);
            assert!(!*settings.public_setting.value());
            assert_eq!(settings.public_string_setting.value().as_str(), "");
        });

        // The public store should have nothing written.
        app.read(|ctx| {
            let public = PublicSetting::preferences_for_setting(ctx);
            assert!(
                public
                    .read_value_with_hierarchy(
                        PublicSetting::storage_key(),
                        PublicSetting::hierarchy(),
                    )
                    .unwrap()
                    .is_none()
            );
            assert!(public
                .read_value_with_hierarchy(
                    PublicStringSetting::storage_key(),
                    PublicStringSetting::hierarchy(),
                )
                .unwrap()
                .is_none());
        });
    });
}

#[test]
fn test_migration_handles_string_setting() {
    warpui::App::test((), |mut app| async move {
        app.update(init_test_app);

        // Seed a JSON-encoded string value in the native store.
        app.update(|ctx| {
            let native = ctx.private_user_preferences();
            native
                .write_value("PublicStringSetting", "\"Fira Code\"".to_owned())
                .unwrap();
        });

        app.update(|ctx| {
            migrate_native_settings_to_settings_file(ctx);
        });

        app.read(|ctx| {
            let settings = MigrationTestSettings::as_ref(ctx);
            assert_eq!(
                settings.public_string_setting.value().as_str(),
                "Fira Code",
                "string setting should have been migrated"
            );
        });
    });
}

#[test]
fn test_migration_does_not_rerun_when_marker_present() {
    warpui::App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);

        app.update(init_test_app);

        // Seed the native store with a public setting.
        app.update(|ctx| {
            let native = ctx.private_user_preferences();
            native
                .write_value("PublicSetting", "true".to_owned())
                .unwrap();
        });

        // Before migration, the guard should allow migration.
        app.read(|ctx| {
            assert!(
                needs_settings_file_migration(ctx),
                "migration should be needed before first run"
            );
        });

        // Run migration.
        app.update(|ctx| {
            migrate_native_settings_to_settings_file(ctx);
        });

        // After migration, the marker should prevent re-migration.
        app.read(|ctx| {
            assert!(
                !needs_settings_file_migration(ctx),
                "migration should not be needed after marker is written"
            );
        });
    });
}

#[test]
#[serial_test::serial]
fn test_migration_with_multiple_setting_types() {
    warpui::App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        let _settings_file_enabled = SettingsFileEnabledGuard::new(true);

        app.update(init_test_app);

        // Seed the native store with values for all three settings.
        app.update(|ctx| {
            let native = ctx.private_user_preferences();
            native
                .write_value("PublicSetting", "true".to_owned())
                .unwrap();
            native
                .write_value("PublicStringSetting", "\"Custom Value\"".to_owned())
                .unwrap();
            native
                .write_value("PrivateSetting", "true".to_owned())
                .unwrap();
        });

        app.update(|ctx| {
            migrate_native_settings_to_settings_file(ctx);
        });

        // Both public settings should have been migrated.
        app.read(|ctx| {
            let settings = MigrationTestSettings::as_ref(ctx);
            assert!(
                *settings.public_setting.value(),
                "public bool should have been migrated"
            );
            assert_eq!(
                settings.public_string_setting.value().as_str(),
                "Custom Value",
                "public string should have been migrated"
            );
        });

        // Public store should contain the migrated bool.
        app.read(|ctx| {
            let public = PublicSetting::preferences_for_setting(ctx);
            assert_eq!(
                public
                    .read_value_with_hierarchy(
                        PublicSetting::storage_key(),
                        PublicSetting::hierarchy(),
                    )
                    .unwrap(),
                Some("true".to_owned())
            );
        });

        // Private setting should NOT have been migrated — in-memory
        // value stays at default because register() read from the
        // public store (which was empty for this key).
        app.read(|ctx| {
            let settings = MigrationTestSettings::as_ref(ctx);
            assert!(
                !*settings.private_setting.value(),
                "private setting should not be affected by migration"
            );
        });

        // The private setting should NOT be in the public store.
        app.read(|ctx| {
            let public = PublicSetting::preferences_for_setting(ctx);
            assert!(
                public
                    .read_value_with_hierarchy(
                        PrivateSetting::storage_key(),
                        PrivateSetting::hierarchy(),
                    )
                    .unwrap()
                    .is_none(),
                "private setting should not appear in public store"
            );
        });
    });
}

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
use notifications_migration::{
    init_notifications_migration_test_app, NotificationsMigrationTestSettings,
};

// -- from_file_value unit tests: these demonstrate the derive-level bug ------

#[test]
fn test_notifications_from_file_value_rejects_serde_format_enum() {
    // serde serializes NotificationsMode::Enabled as "Enabled" (PascalCase),
    // but from_file_value expects "enabled" (snake_case). When the field is
    // present but unparsable, from_file_value should return None — not
    // silently fall back to the #[serde(default)] value (Unset).
    let serde_json_value = serde_json::to_value(NotificationsSettings {
        mode: NotificationsMode::Enabled,
        ..NotificationsSettings::default()
    })
    .unwrap();

    let result = NotificationsSettings::from_file_value(&serde_json_value);
    assert!(
        result.is_none(),
        "from_file_value should reject serde-format enum values, but got: {result:?}"
    );
}

#[test]
fn test_notifications_from_file_value_rejects_serde_format_duration() {
    // serde serializes Duration as {"secs": N, "nanos": N}, but
    // Duration::from_file_value expects a plain integer. Use file-format
    // for mode ("unset") so that the failure is isolated to the Duration field.
    let json = serde_json::json!({
        "mode": "unset",
        "is_long_running_enabled": true,
        "long_running_threshold": {"secs": 60, "nanos": 0},
        "is_password_prompt_enabled": true,
        "is_agent_task_completed_enabled": true,
        "is_needs_attention_enabled": true,
        "play_notification_sound": true,
    });

    let result = NotificationsSettings::from_file_value(&json);
    assert!(
        result.is_none(),
        "from_file_value should reject serde-format Duration, but got: {result:?}"
    );
}

// -- Migration integration tests: these demonstrate end-to-end data loss -----

#[test]
#[serial_test::serial]
fn test_migration_preserves_notifications_mode() {
    warpui::App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        let _settings_file_enabled = SettingsFileEnabledGuard::new(true);

        app.update(init_notifications_migration_test_app);

        // Seed the native store with serde-serialized NotificationsSettings
        // where mode is Enabled. In serde format: {"mode":"Enabled",...}.
        app.update(|ctx| {
            let native = ctx.private_user_preferences();
            let serde_value = serde_json::to_string(&NotificationsSettings {
                mode: NotificationsMode::Enabled,
                ..NotificationsSettings::default()
            })
            .unwrap();
            native
                .write_value("MigrationTestNotifications", serde_value)
                .unwrap();
        });

        app.update(|ctx| {
            migrate_native_settings_to_settings_file(ctx);
        });

        // Mode should be preserved as Enabled, not silently defaulted to Unset.
        app.read(|ctx| {
            let settings = NotificationsMigrationTestSettings::as_ref(ctx);
            assert_eq!(
                settings.notifications.value().mode,
                NotificationsMode::Enabled,
                "NotificationsMode should be preserved during migration"
            );
        });
    });
}

#[test]
#[serial_test::serial]
fn test_migration_preserves_custom_long_running_threshold() {
    warpui::App::test((), |mut app| async move {
        let _guard = FeatureFlag::SettingsFile.override_enabled(true);
        let _settings_file_enabled = SettingsFileEnabledGuard::new(true);

        app.update(init_notifications_migration_test_app);

        // Seed with a non-default threshold (60s instead of default 30s).
        // serde serializes Duration as {"secs":60,"nanos":0}, which differs
        // from the file format (plain integer 60).
        let custom_threshold = Duration::from_secs(60);
        app.update(|ctx| {
            let native = ctx.private_user_preferences();
            let serde_value = serde_json::to_string(&NotificationsSettings {
                long_running_threshold: custom_threshold,
                ..NotificationsSettings::default()
            })
            .unwrap();
            native
                .write_value("MigrationTestNotifications", serde_value)
                .unwrap();
        });

        app.update(|ctx| {
            migrate_native_settings_to_settings_file(ctx);
        });

        app.read(|ctx| {
            let settings = NotificationsMigrationTestSettings::as_ref(ctx);
            assert_eq!(
                settings.notifications.value().long_running_threshold,
                custom_threshold,
                "custom long_running_threshold should be preserved during migration"
            );
        });
    });
}

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
fn native_ssh_alias_migration_preserves_choices_and_removes_old_key() {
    use user_preferences::{in_memory::InMemoryPreferences, UserPreferences as _};

    for new_value in [None, Some("true"), Some("invalid-but-explicit")] {
        let prefs = InMemoryPreferences::default();
        prefs.write_value("EnableSshWarpification", "false".to_owned()).unwrap();
        if let Some(value) = new_value {
            prefs.write_value("EnableSshZaplexification", value.to_owned()).unwrap();
        }
        super::migrate_legacy_native_ssh_setting(&prefs).unwrap();
        assert_eq!(prefs.read_value("EnableSshWarpification").unwrap(), None);
        assert_eq!(
            prefs.read_value("EnableSshZaplexification").unwrap().as_deref(),
            Some(new_value.unwrap_or("false")),
        );
        prefs.remove_value("EnableSshZaplexification").unwrap();
        super::migrate_legacy_native_ssh_setting(&prefs).unwrap();
        assert_eq!(prefs.read_value("EnableSshZaplexification").unwrap(), None);
    }
}

#[test]
fn native_ssh_alias_is_retained_when_replacement_write_fails() {
    use user_preferences::{in_memory::InMemoryPreferences, UserPreferences};

    struct FailedWritePreferences(InMemoryPreferences);
    impl UserPreferences for FailedWritePreferences {
        fn read_value(&self, key: &str) -> Result<Option<String>, user_preferences::Error> {
            self.0.read_value(key)
        }
        fn write_value(&self, _key: &str, _value: String) -> Result<(), user_preferences::Error> {
            Err(std::io::Error::other("injected native write failure").into())
        }
        fn remove_value(&self, key: &str) -> Result<(), user_preferences::Error> {
            self.0.remove_value(key)
        }
    }
    let prefs = FailedWritePreferences(InMemoryPreferences::default());
    prefs.0.write_value("EnableSshWarpification", "false".to_owned()).unwrap();
    assert!(super::migrate_legacy_native_ssh_setting(&prefs).is_err());
    assert_eq!(
        prefs.read_value("EnableSshWarpification").unwrap().as_deref(),
        Some("false"),
    );
    assert_eq!(prefs.read_value("EnableSshZaplexification").unwrap(), None);
}

#[test]
fn unsupported_inline_legacy_ssh_table_is_not_partially_migrated() {
    let mut document = "warpify = { ssh = { enable_ssh_warpification = false } }\n"
        .parse::<toml_edit::DocumentMut>().unwrap();
    let original = document.to_string();
    super::migrate_legacy_zaplexify_settings(&mut document);
    assert_eq!(document.to_string(), original);
    assert!(document.get("zaplexify").is_none());
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
