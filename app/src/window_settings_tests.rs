use settings::{PrivatePreferences, PublicPreferences, Setting, SettingsManager};
use warpui::{AppContext, SingletonEntity};
use warpui_extras::user_preferences::in_memory::InMemoryPreferences;

use super::{BackgroundBlurRadius, BackgroundOpacity, WindowSettings};

struct SettingsFileEnabledGuard(bool);

impl SettingsFileEnabledGuard {
    fn disabled() -> Self {
        let previous = settings::is_settings_file_enabled();
        settings::set_settings_file_enabled(false);
        Self(previous)
    }
}

impl Drop for SettingsFileEnabledGuard {
    fn drop(&mut self) {
        settings::set_settings_file_enabled(self.0);
    }
}

fn init_preferences(ctx: &mut AppContext) {
    ctx.add_singleton_model(move |_| PublicPreferences::new(Box::<InMemoryPreferences>::default()));
    ctx.add_singleton_model(
        move |_| PrivatePreferences::new(Box::<InMemoryPreferences>::default()),
    );
}

#[test]
#[serial_test::serial]
fn test_background_values_round_trip_at_validated_bounds() {
    warpui::App::test((), |mut app| async move {
        let _settings_file_enabled = SettingsFileEnabledGuard::disabled();
        app.update(init_preferences);
        app.add_singleton_model(|_| SettingsManager::default());
        WindowSettings::register(&mut app);

        app.update(|ctx| {
            WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
                window_settings
                    .background_opacity
                    .set_value(0, ctx)
                    .unwrap();
                window_settings
                    .background_blur_radius
                    .set_value(0, ctx)
                    .unwrap();
            });
        });

        app.update(|ctx| {
            let opacity = BackgroundOpacity::new_from_storage(ctx);
            assert_eq!(*opacity.value(), BackgroundOpacity::MIN);
            let stored_opacity = BackgroundOpacity::preferences_for_setting(ctx)
                .read_value(BackgroundOpacity::storage_key())
                .unwrap();
            assert_eq!(stored_opacity.as_deref(), Some("1"));

            let blur_radius = BackgroundBlurRadius::new_from_storage(ctx);
            assert_eq!(*blur_radius.value(), BackgroundBlurRadius::MIN);
            let stored_blur_radius = BackgroundBlurRadius::preferences_for_setting(ctx)
                .read_value(BackgroundBlurRadius::storage_key())
                .unwrap();
            assert_eq!(stored_blur_radius.as_deref(), Some("1"));
        });

        app.update(|ctx| {
            WindowSettings::handle(ctx).update(ctx, |window_settings, ctx| {
                window_settings
                    .background_opacity
                    .set_value_from_cloud_sync(BackgroundOpacity::MAX + 1, ctx)
                    .unwrap();
                window_settings
                    .background_blur_radius
                    .set_value_from_cloud_sync(BackgroundBlurRadius::MAX + 1, ctx)
                    .unwrap();
            });
        });

        app.update(|ctx| {
            let opacity = BackgroundOpacity::new_from_storage(ctx);
            assert_eq!(*opacity.value(), BackgroundOpacity::MAX);
            let stored_opacity = BackgroundOpacity::preferences_for_setting(ctx)
                .read_value(BackgroundOpacity::storage_key())
                .unwrap();
            assert_eq!(stored_opacity.as_deref(), Some("100"));

            let blur_radius = BackgroundBlurRadius::new_from_storage(ctx);
            assert_eq!(*blur_radius.value(), BackgroundBlurRadius::MAX);
            let stored_blur_radius = BackgroundBlurRadius::preferences_for_setting(ctx)
                .read_value(BackgroundBlurRadius::storage_key())
                .unwrap();
            assert_eq!(stored_blur_radius.as_deref(), Some("64"));
        });
    });
}
