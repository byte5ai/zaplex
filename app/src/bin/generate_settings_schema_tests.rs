use super::*;

#[test]
fn oss_channel_keeps_runtime_and_release_flags_without_internal_tiers() {
    let flags = active_flags_for_channel(SchemaChannel::Oss, [FeatureFlag::SuggestedRules]);

    assert_eq!(flags.contains(&FeatureFlag::SuggestedRules), true);
    assert_eq!(RELEASE_FLAGS.iter().all(|flag| flags.contains(flag)), true);
    assert_eq!(PREVIEW_FLAGS.iter().any(|flag| flags.contains(flag)), false);
    assert_eq!(DOGFOOD_FLAGS.iter().any(|flag| flags.contains(flag)), false);
    assert_eq!(DEBUG_FLAGS.iter().any(|flag| flags.contains(flag)), false);
}

#[test]
fn unknown_channel_is_an_error() {
    let result = parse_arguments([
        "generate_settings_schema".to_string(),
        "--channel".to_string(),
        "osss".to_string(),
        "settings_schema.json".to_string(),
    ]);

    assert_eq!(result.unwrap_err(), "Unknown channel: osss");
}

#[test]
fn channel_option_requires_a_value() {
    let result = parse_arguments([
        "generate_settings_schema".to_string(),
        "--channel".to_string(),
    ]);

    assert_eq!(result.unwrap_err(), "--channel requires a value");
}
