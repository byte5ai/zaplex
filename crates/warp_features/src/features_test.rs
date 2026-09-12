use super::*;

#[test]
#[ignore = "CORE-3768 - need to clean up PREVIEW_FLAGS, but this is a temporary fix for the cluttered changelog"]
fn test_all_preview_flags_have_a_description() {
    for flag in PREVIEW_FLAGS {
        assert!(
            flag.flag_description()
                .is_some_and(|description| !description.is_empty()),
            "Missing description for preview-enabled flag {flag:?}"
        );
    }
}

#[test]
fn oss_notification_features_match_manifest_and_runtime_wiring() {
    let manifest = include_str!("../../../app/Cargo.toml");
    let app_features = include_str!("../../../app/src/lib.rs");

    assert!(manifest.contains("\"gemini_notifications\""));
    assert!(manifest.contains("gemini_notifications = []"));
    assert!(app_features.contains(
        "#[cfg(feature = \"gemini_notifications\")]\n        FeatureFlag::GeminiNotifications,"
    ));

    assert!(!manifest.contains("hoa_remote_control"));
    assert!(!app_features.contains("HOARemoteControl"));
}
