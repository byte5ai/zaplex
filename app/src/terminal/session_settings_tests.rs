use settings_value::SettingsValue as _;

use super::*;

/// Preferences stored before the agent triggers were removed.
fn stored_with_removed_agent_keys(stored: &NotificationsSettings) -> serde_json::Value {
    let mut value = stored.to_file_value();
    let object = value
        .as_object_mut()
        .expect("notification preferences are a table");
    object.insert(
        "is_agent_task_completed_enabled".to_string(),
        serde_json::Value::Bool(true),
    );
    object.insert(
        "is_needs_attention_enabled".to_string(),
        serde_json::Value::Bool(true),
    );
    value
}

#[test]
fn stored_preferences_with_removed_agent_triggers_still_load() {
    let stored = NotificationsSettings {
        mode: NotificationsMode::Enabled,
        is_long_running_enabled: false,
        long_running_threshold: Duration::from_secs(90),
        is_password_prompt_enabled: false,
        play_notification_sound: false,
    };
    let value = stored_with_removed_agent_keys(&stored);

    assert_eq!(
        NotificationsSettings::from_file_value(&value),
        Some(stored.clone()),
        "the settings file keeps every remaining preference"
    );
    let synced: NotificationsSettings = serde_json::from_value(
        serde_json::to_value(&stored)
            .map(|mut synced| {
                let object = synced.as_object_mut().unwrap();
                object.insert(
                    "is_needs_attention_enabled".to_string(),
                    serde_json::Value::Bool(true),
                );
                synced
            })
            .unwrap(),
    )
    .expect("synced preferences with old keys deserialize");
    assert_eq!(synced, stored);
}
