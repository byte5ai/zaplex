use super::*;

#[test]
fn session_id_roundtrips_through_string() {
    let id = SessionId::from("abc-123".to_string());
    assert_eq!(id.as_str(), "abc-123");
    assert_eq!(String::from(id.clone()), "abc-123");
    assert_eq!(id.to_string(), "abc-123");
}

#[test]
fn new_session_ids_are_unique() {
    assert_ne!(SessionId::new(), SessionId::new());
}

#[test]
fn has_feature_matches_advertised_capabilities() {
    let features = vec![FEATURE_SESSION_HOST.to_string()];
    assert!(has_feature(&features, FEATURE_SESSION_HOST));
    assert!(!has_feature(&features, "nonexistent"));
    assert!(!has_feature(&[], FEATURE_SESSION_HOST));
}

#[test]
fn supported_features_is_empty_in_stage_0() {
    // Stage 0 advertises no session-host capability; update this assertion once
    // Stage 1 lands the implementation.
    assert!(supported_features().is_empty());
}
