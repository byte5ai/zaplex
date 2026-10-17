use std::sync::atomic::{AtomicUsize, Ordering};
use warpui::{AppContext, SingletonEntity};
use warpui_extras::user_preferences::{Error as PreferencesError, UserPreferences};

use crate::manager::SettingsManager;
use crate::{Setting, SupportedPlatforms, SyncToCloud};

use crate::*;

define_settings_group!(TestSettings, settings: [
    simple_setting: SimpleSetting {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "test.simple_setting",
    },
    key_override_setting: KeyOverrideSetting {
        type: bool,
        default: true,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
        storage_key: "KeyIsOverridden",
    },
    hierarchy_flag: HierarchyFlag {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: false,
        toml_path: "test_section.hierarchy_flag",
    },
]);

define_settings_group!(WriteFailureSettings, settings: [
    fallible_setting: FallibleSetting {
        type: bool,
        default: false,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    },
]);

static CLAMPED_VALIDATION_CALLS: AtomicUsize = AtomicUsize::new(0);

define_settings_group!(ValidationSettings, settings: [
    clamped_setting: ClampedSetting {
        type: u8,
        default: 50,
        supported_platforms: SupportedPlatforms::ALL,
        sync_to_cloud: SyncToCloud::Never,
        private: true,
    },
]);

impl ClampedSetting {
    fn validate(&self, new_value: u8) -> u8 {
        CLAMPED_VALIDATION_CALLS.fetch_add(1, Ordering::Relaxed);
        new_value.clamp(1, 100)
    }
}

static ENUM_VALIDATION_CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(
    Default, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub enum NormalizedEnum {
    #[default]
    Canonical,
    Invalid,
}

impl settings_value::SettingsValue for NormalizedEnum {}

impl NormalizedEnum {
    fn validate(&self, new_value: Self) -> Self {
        ENUM_VALIDATION_CALLS.fetch_add(1, Ordering::Relaxed);
        match new_value {
            Self::Canonical => Self::Canonical,
            Self::Invalid => Self::Canonical,
        }
    }
}

implement_setting_for_enum!(
    NormalizedEnum,
    EnumValidationSettings,
    SupportedPlatforms::ALL,
    SyncToCloud::Never,
    private: true,
);

define_settings_group!(EnumValidationSettings, settings: [
    normalized_enum: NormalizedEnum,
]);

#[derive(Default)]
struct FailingWritePreferences;

impl FailingWritePreferences {
    fn error() -> PreferencesError {
        std::io::Error::other("injected preferences write failure").into()
    }
}

impl UserPreferences for FailingWritePreferences {
    fn write_value(&self, key: &str, value: String) -> std::result::Result<(), PreferencesError> {
        let _ = (key, value);
        Err(Self::error())
    }

    fn read_value(&self, key: &str) -> std::result::Result<Option<String>, PreferencesError> {
        let _ = key;
        Ok(None)
    }

    fn remove_value(&self, key: &str) -> std::result::Result<(), PreferencesError> {
        let _ = key;
        Ok(())
    }

    fn write_value_with_hierarchy(
        &self,
        key: &str,
        value: String,
        hierarchy: Option<&str>,
        max_table_depth: Option<u32>,
    ) -> std::result::Result<(), PreferencesError> {
        let _ = (key, value, hierarchy, max_table_depth);
        Err(Self::error())
    }
}

pub fn init_and_register_preferences(ctx: &mut AppContext) {
    ctx.add_singleton_model(move |_| {
        crate::PublicPreferences::new(Box::<
            warpui_extras::user_preferences::in_memory::InMemoryPreferences,
        >::default())
    });
    ctx.add_singleton_model(move |_| {
        crate::PrivatePreferences::new(Box::<
            warpui_extras::user_preferences::in_memory::InMemoryPreferences,
        >::default())
    });
}

fn init_failing_write_preferences(ctx: &mut AppContext) {
    ctx.add_singleton_model(|_| {
        crate::PublicPreferences::new(Box::<
            warpui_extras::user_preferences::in_memory::InMemoryPreferences,
        >::default())
    });
    ctx.add_singleton_model(|_| {
        crate::PrivatePreferences::new(Box::<FailingWritePreferences>::default())
    });
}

struct WriteFailureEventListener {
    event_count: usize,
}

impl WriteFailureEventListener {
    fn new(ctx: &mut warpui::ModelContext<Self>) -> Self {
        let settings = WriteFailureSettings::handle(ctx);
        ctx.subscribe_to_model(&settings, |listener, event, _ctx| {
            if matches!(
                event,
                WriteFailureSettingsChangedEvent::FallibleSetting { .. }
            ) {
                listener.event_count += 1;
            }
        });
        Self { event_count: 0 }
    }
}

impl warpui::Entity for WriteFailureEventListener {
    type Event = ();
}

struct ValidationEventListener {
    event_count: usize,
}

impl ValidationEventListener {
    fn new(ctx: &mut warpui::ModelContext<Self>) -> Self {
        let settings = ValidationSettings::handle(ctx);
        ctx.subscribe_to_model(&settings, |listener, event, _ctx| {
            if matches!(event, ValidationSettingsChangedEvent::ClampedSetting { .. }) {
                listener.event_count += 1;
            }
        });
        Self { event_count: 0 }
    }
}

impl warpui::Entity for ValidationEventListener {
    type Event = ();
}

#[test]
fn preference_write_failures_preserve_state_and_suppress_events() {
    warpui::App::test((), |mut app| async move {
        app.update(init_failing_write_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        WriteFailureSettings::register(&mut app);
        let listener = app.add_model(WriteFailureEventListener::new);

        let local_error = app
            .update(|ctx| {
                WriteFailureSettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings.fallible_setting.set_value(true, ctx)
                })
            })
            .expect_err("the local preferences write should fail");
        assert!(
            format!("{local_error:#}").contains("injected preferences write failure"),
            "the backend error should be propagated: {local_error:#}"
        );

        let cloud_error = app
            .update(|ctx| {
                WriteFailureSettings::handle(ctx).update(ctx, |settings, ctx| {
                    settings
                        .fallible_setting
                        .set_value_from_cloud_sync(true, ctx)
                })
            })
            .expect_err("the cloud preferences write should fail");
        assert!(
            format!("{cloud_error:#}").contains("injected preferences write failure"),
            "the backend error should be propagated: {cloud_error:#}"
        );

        app.read(|ctx| {
            let setting = &WriteFailureSettings::as_ref(ctx).fallible_setting;
            assert_eq!(*setting.value(), FallibleSetting::default_value());
            assert!(!setting.is_value_explicitly_set());
            assert_eq!(listener.as_ref(ctx).event_count, 0);
        });
    });
}

#[test]
#[serial_test::serial]
fn local_and_cloud_updates_persist_the_once_validated_value() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        ValidationSettings::register(&mut app);
        let listener = app.add_model(ValidationEventListener::new);

        CLAMPED_VALIDATION_CALLS.store(0, Ordering::Relaxed);
        app.update(|ctx| {
            ValidationSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.clamped_setting.set_value(0, ctx).unwrap();
            });
        });
        assert_eq!(CLAMPED_VALIDATION_CALLS.load(Ordering::Relaxed), 1);

        app.read(|ctx| {
            let setting = &ValidationSettings::as_ref(ctx).clamped_setting;
            assert_eq!(*setting.value(), 1);
            assert!(setting.is_value_explicitly_set());
            let stored = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx)
                .read_value(ClampedSetting::storage_key())
                .unwrap();
            assert_eq!(stored.as_deref(), Some("1"));
            assert_eq!(listener.as_ref(ctx).event_count, 1);
        });

        CLAMPED_VALIDATION_CALLS.store(0, Ordering::Relaxed);
        app.update(|ctx| {
            ValidationSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .clamped_setting
                    .set_value_from_cloud_sync(u8::MAX, ctx)
                    .unwrap();
            });
        });
        assert_eq!(CLAMPED_VALIDATION_CALLS.load(Ordering::Relaxed), 1);

        app.read(|ctx| {
            assert_eq!(
                *ValidationSettings::as_ref(ctx).clamped_setting.value(),
                100
            );
            let stored = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx)
                .read_value(ClampedSetting::storage_key())
                .unwrap();
            assert_eq!(stored.as_deref(), Some("100"));
            assert_eq!(listener.as_ref(ctx).event_count, 2);
        });

        CLAMPED_VALIDATION_CALLS.store(0, Ordering::Relaxed);
        app.update(|ctx| {
            ValidationSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .clamped_setting
                    .set_value_from_cloud_sync(u8::MAX, ctx)
                    .unwrap();
            });
        });
        assert_eq!(CLAMPED_VALIDATION_CALLS.load(Ordering::Relaxed), 1);
        app.read(|ctx| {
            assert_eq!(listener.as_ref(ctx).event_count, 2);
        });
    });
}

#[test]
#[serial_test::serial]
fn enum_updates_persist_the_once_validated_value() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        EnumValidationSettings::register(&mut app);

        ENUM_VALIDATION_CALLS.store(0, Ordering::Relaxed);
        app.update(|ctx| {
            EnumValidationSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .normalized_enum
                    .set_value(NormalizedEnum::Invalid, ctx)
            })
        })
        .unwrap();
        assert_eq!(ENUM_VALIDATION_CALLS.load(Ordering::Relaxed), 1);

        app.read(|ctx| {
            assert_eq!(
                EnumValidationSettings::as_ref(ctx).normalized_enum,
                NormalizedEnum::Canonical
            );
            let stored = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx)
                .read_value(NormalizedEnum::storage_key())
                .unwrap();
            assert_eq!(
                stored,
                Some(serde_json::to_string(&NormalizedEnum::Canonical).unwrap())
            );
        });

        app.update(|ctx| {
            <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx)
                .remove_value(NormalizedEnum::storage_key())
                .unwrap();
        });
        ENUM_VALIDATION_CALLS.store(0, Ordering::Relaxed);
        app.update(|ctx| {
            EnumValidationSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .normalized_enum
                    .set_value_from_cloud_sync(NormalizedEnum::Invalid, ctx)
            })
        })
        .unwrap();
        assert_eq!(ENUM_VALIDATION_CALLS.load(Ordering::Relaxed), 1);
    });
}

#[test]
fn test_setting_name_is_struct_name() {
    assert_eq!(SimpleSetting::setting_name(), "SimpleSetting");
}

#[test]
fn test_explicit_value_tracking_after_clear_value() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        // Register our TestSettings settings group with the app.
        TestSettings::register(&mut app);

        // Set a value first
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings.simple_setting.set_value(true, ctx);
            });
        });

        // Verify it's marked as explicitly set
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(settings.simple_setting.is_value_explicitly_set());
        });

        // Clear the value
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings.simple_setting.clear_value(ctx);
            });
        });

        // After clearing, it should no longer be marked as explicitly set
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(!settings.simple_setting.is_value_explicitly_set());
            assert_eq!(
                *settings.simple_setting.value(),
                SimpleSetting::default_value()
            );
        });
    });
}

#[test]
fn test_toml_path_returns_none_for_private_setting() {
    assert_eq!(KeyOverrideSetting::toml_path(), None);
}

#[test]
fn test_explicit_value_tracking_cloud_sync() {
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());

        // Register our TestSettings settings group with the app.
        TestSettings::register(&mut app);

        // Initially not explicitly set
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(!settings.simple_setting.is_value_explicitly_set());
        });

        // Set value from cloud sync
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                let _ = test_settings
                    .simple_setting
                    .set_value_from_cloud_sync(true, ctx);
            });
        });

        // Should now be marked as explicitly set
        app.read(|ctx| {
            let settings = TestSettings::as_ref(ctx);
            assert!(settings.simple_setting.is_value_explicitly_set());
            assert!(*settings.simple_setting.value());
        });
    });
}

// ---------------------------------------------------------------------------
// Private / public settings split tests
// ---------------------------------------------------------------------------

// Tests that call `set_settings_file_enabled` are marked `#[serial_test::serial]`
// because they mutate the process-global `SETTINGS_FILE_ENABLED` AtomicBool and
// would race under `cargo test` (thread-based parallelism). This can be removed
// when the SettingsFile feature flag is cleaned up and the global flag is deleted.

// ---------------------------------------------------------------------------
// SettingsManager private/public routing tests
// ---------------------------------------------------------------------------

#[test]
#[serial_test::serial]
fn test_manager_read_local_setting_value_routes_when_flag_enabled() {
    crate::set_settings_file_enabled(true);
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Write directly to the correct backends.
        app.update(|ctx| {
            let public = <crate::PublicPreferences as SingletonEntity>::as_ref(ctx);
            public
                .as_preferences()
                .write_value("SimpleSetting", "true".to_string())
                .unwrap();

            let private = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx);
            private
                .0
                .write_value("KeyIsOverridden", "false".to_string())
                .unwrap();
        });

        app.read(|ctx| {
            let manager = SettingsManager::as_ref(ctx);

            let public_val = manager
                .read_local_setting_value("SimpleSetting", ctx)
                .unwrap();
            assert_eq!(
                public_val,
                Some("true".to_string()),
                "manager should read public setting from public backend"
            );

            let private_val = manager
                .read_local_setting_value("KeyIsOverridden", ctx)
                .unwrap();
            assert_eq!(
                private_val,
                Some("false".to_string()),
                "manager should read private setting from private backend"
            );
        });
    });
}

#[test]
#[serial_test::serial]
fn test_manager_read_local_setting_value_falls_back_when_flag_disabled() {
    crate::set_settings_file_enabled(false);
    warpui::App::test((), |mut app| async move {
        app.update(init_and_register_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Write both values to the private backend.
        app.update(|ctx| {
            let private = <crate::PrivatePreferences as SingletonEntity>::as_ref(ctx);
            private
                .0
                .write_value("SimpleSetting", "true".to_string())
                .unwrap();
            private
                .0
                .write_value("KeyIsOverridden", "false".to_string())
                .unwrap();
        });

        app.read(|ctx| {
            let manager = SettingsManager::as_ref(ctx);

            let public_val = manager
                .read_local_setting_value("SimpleSetting", ctx)
                .unwrap();
            assert_eq!(
                public_val,
                Some("true".to_string()),
                "public setting should fall back to private backend"
            );

            let private_val = manager
                .read_local_setting_value("KeyIsOverridden", ctx)
                .unwrap();
            assert_eq!(
                private_val,
                Some("false".to_string()),
                "private setting should read from private backend"
            );
        });
    });
}

/// Regression test for the settings sync disappearing on restart bug: when
/// the TOML settings file is enabled, `read_local_setting_value` must
/// forward the setting's hierarchy, otherwise values stored under a
/// section like `[account]` are invisible to the SettingsManager and the
/// cloud preferences syncer clobbers them with stale cloud state.
#[test]
#[serial_test::serial]
fn test_manager_read_local_setting_value_respects_hierarchy_with_settings_file() {
    use warpui_extras::user_preferences::toml_backed::TomlBackedUserPreferences;

    crate::set_settings_file_enabled(true);
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("settings.toml");

    warpui::App::test((), |mut app| async move {
        // Use the TOML-backed store for public preferences so the hierarchy
        // routing actually matters; in-memory preferences ignore hierarchy
        // entirely and would hide this bug.
        let file_path_for_public = file_path.clone();
        app.add_singleton_model(move |_| {
            let (prefs, _) = TomlBackedUserPreferences::new(file_path_for_public);
            crate::PublicPreferences::new(Box::new(prefs))
        });
        app.add_singleton_model(|_| {
            crate::PrivatePreferences::new(Box::<
                warpui_extras::user_preferences::in_memory::InMemoryPreferences,
            >::default())
        });
        app.add_singleton_model(|_| SettingsManager::default());
        TestSettings::register(&mut app);

        // Toggle a public, hierarchy-scoped setting via the normal write
        // path. `set_value` writes to the TOML under `[test_section]`.
        app.update(|ctx| {
            TestSettings::handle(ctx).update(ctx, |test_settings, ctx| {
                test_settings.hierarchy_flag.set_value(true, ctx).unwrap();
            });
        });

        // The SettingsManager should see the value we just wrote. Without
        // the fix, this returned None because the read path looked at the
        // root table instead of `[test_section]`.
        app.read(|ctx| {
            let manager = SettingsManager::as_ref(ctx);
            let value = manager
                .read_local_setting_value("HierarchyFlag", ctx)
                .unwrap();
            assert_eq!(
                value,
                Some("true".to_string()),
                "SettingsManager must forward hierarchy when reading from \
                 the TOML-backed store"
            );
        });
    });
}
