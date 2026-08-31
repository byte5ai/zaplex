use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use settings::{PrivatePreferences, PublicPreferences, Setting, SettingsManager};
use warpui::{App, SingletonEntity};
use warpui_extras::user_preferences::{Error as PreferencesError, UserPreferences};

use super::{BackgroundBlurRadius, BackgroundOpacity, WindowSettings};

#[derive(Clone, Default)]
struct SharedPreferences(Arc<Mutex<HashMap<String, String>>>);

impl SharedPreferences {
    fn stored_value(&self, key: &str) -> Option<String> {
        self.0.lock().unwrap().get(key).cloned()
    }
}

impl UserPreferences for SharedPreferences {
    fn write_value(&self, key: &str, value: String) -> std::result::Result<(), PreferencesError> {
        self.0.lock().unwrap().insert(key.to_string(), value);
        Ok(())
    }

    fn read_value(&self, key: &str) -> std::result::Result<Option<String>, PreferencesError> {
        Ok(self.stored_value(key))
    }

    fn remove_value(&self, key: &str) -> std::result::Result<(), PreferencesError> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

#[test]
fn opacity_and_blur_round_trip_their_validated_bounds() {
    App::test((), |mut app| async move {
        let preferences = SharedPreferences::default();
        let public_preferences = preferences.clone();
        let private_preferences = preferences.clone();
        app.add_singleton_model(move |_| PublicPreferences::new(Box::new(public_preferences)));
        app.add_singleton_model(move |_| PrivatePreferences::new(Box::new(private_preferences)));
        app.add_singleton_model(|_| SettingsManager::default());

        preferences
            .write_value(BackgroundOpacity::storage_key(), "0".to_string())
            .unwrap();
        preferences
            .write_value(BackgroundBlurRadius::storage_key(), u8::MAX.to_string())
            .unwrap();

        WindowSettings::register(&mut app);
        app.read(|ctx| {
            let settings = WindowSettings::as_ref(ctx);
            assert_eq!(*settings.background_opacity.value(), BackgroundOpacity::MIN);
            assert_eq!(
                *settings.background_blur_radius.value(),
                BackgroundBlurRadius::MAX
            );
        });

        app.update(|ctx| {
            WindowSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings.background_opacity.set_value(u8::MAX, ctx).unwrap();
                settings
                    .background_blur_radius
                    .set_value_from_cloud_sync(0, ctx)
                    .unwrap();
            });
        });

        assert_eq!(
            preferences.stored_value(BackgroundOpacity::storage_key()),
            Some(BackgroundOpacity::MAX.to_string())
        );
        assert_eq!(
            preferences.stored_value(BackgroundBlurRadius::storage_key()),
            Some(BackgroundBlurRadius::MIN.to_string())
        );

        app.update(|ctx| {
            let opacity = BackgroundOpacity::new_from_storage(ctx);
            assert_eq!(*opacity.value(), BackgroundOpacity::MAX);

            let blur = BackgroundBlurRadius::new_from_storage(ctx);
            assert_eq!(*blur.value(), BackgroundBlurRadius::MIN);
        });
    });
}
