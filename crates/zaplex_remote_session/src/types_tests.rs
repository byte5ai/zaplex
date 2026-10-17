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

#[cfg(unix)]
#[test]
fn pty_binding_requires_negotiated_capability() {
    assert!(has_feature(
        &supported_features(),
        FEATURE_AGENT_PTY_BINDING
    ));
    assert!(!has_feature(&[], FEATURE_AGENT_PTY_BINDING));
    assert!(has_feature(
        &supported_features(),
        FEATURE_AGENT_PTY_BINDING_V2
    ));
    assert!(!has_feature(&[], FEATURE_AGENT_PTY_BINDING_V2));
}

#[cfg(target_os = "linux")]
#[test]
fn supported_features_advertises_managed_agent_fleet_on_linux() {
    assert!(has_feature(
        &supported_features(),
        FEATURE_MANAGED_AGENT_FLEET_V1
    ));
    assert!(has_feature(
        &supported_features(),
        FEATURE_MANAGED_OPEN_ATTACH_V1
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn supported_features_advertises_live_session_metadata_on_linux() {
    assert!(has_feature(
        &supported_features(),
        FEATURE_SESSION_LIVE_METADATA_V1
    ));
}

#[cfg(not(target_os = "linux"))]
#[test]
fn supported_features_omits_live_session_metadata_without_proc() {
    assert!(!has_feature(
        &supported_features(),
        FEATURE_SESSION_LIVE_METADATA_V1
    ));
}

#[cfg(not(target_os = "linux"))]
#[test]
fn supported_features_omits_managed_agent_fleet_when_unsupported() {
    assert!(!has_feature(
        &supported_features(),
        FEATURE_MANAGED_AGENT_FLEET_V1
    ));
    assert!(!has_feature(
        &supported_features(),
        FEATURE_MANAGED_OPEN_ATTACH_V1
    ));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn supported_features_advertises_safe_file_transactions_on_supported_unix() {
    assert!(has_feature(
        &supported_features(),
        FEATURE_SAFE_FILE_TRANSACTIONS_V1
    ));
    assert!(has_feature(
        &supported_features(),
        FEATURE_SAFE_FILE_TRANSACTIONS_V2
    ));
    assert!(has_feature(
        &supported_features(),
        FEATURE_SAFE_FILE_IDENTITY_BATCH_V1
    ));
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn supported_features_omits_safe_file_transactions_when_unsupported() {
    assert!(!has_feature(
        &supported_features(),
        FEATURE_SAFE_FILE_TRANSACTIONS_V1
    ));
    assert!(!has_feature(
        &supported_features(),
        FEATURE_SAFE_FILE_TRANSACTIONS_V2
    ));
    assert!(!has_feature(
        &supported_features(),
        FEATURE_SAFE_FILE_IDENTITY_BATCH_V1
    ));
}

#[cfg(not(target_os = "linux"))]
#[test]
fn supported_features_omits_verified_agent_process_signals_when_unsupported() {
    assert!(!has_feature(
        &supported_features(),
        FEATURE_AGENT_PROCESS_SIGNAL_V1
    ));
}

#[cfg(not(unix))]
#[test]
fn supported_features_omits_session_host_on_non_unix() {
    // Non-unix daemons own no PTYs, so they do not advertise the session host —
    // but they still report agent inventory.
    assert!(!has_feature(&supported_features(), FEATURE_SESSION_HOST));
    assert!(!has_feature(
        &supported_features(),
        FEATURE_STARTUP_COMMAND_ACK
    ));
    assert!(!has_feature(
        &supported_features(),
        FEATURE_LOGICAL_OPEN_ID_V1
    ));
    assert!(!has_feature(
        &supported_features(),
        FEATURE_LOGICAL_OPEN_ATTEMPT_V1
    ));
    assert!(!has_feature(
        &supported_features(),
        FEATURE_AGENT_PTY_BINDING
    ));
    assert!(!has_feature(
        &supported_features(),
        FEATURE_AGENT_PTY_BINDING_V2
    ));
    assert!(!has_feature(
        &supported_features(),
        FEATURE_SAFE_FILE_TRANSACTIONS_V1
    ));
    assert!(!has_feature(
        &supported_features(),
        FEATURE_SAFE_FILE_TRANSACTIONS_V2
    ));
    assert!(!has_feature(
        &supported_features(),
        FEATURE_SAFE_FILE_IDENTITY_BATCH_V1
    ));
    assert!(!has_feature(
        &supported_features(),
        FEATURE_MULTIPLEXER_INVENTORY_V1
    ));
}

#[cfg(unix)]
#[test]
fn session_takeover_is_negotiated_by_both_peers() {
    // A daemon only takes a session over for a client that can render the
    // displaced view's notice, so both sides must advertise the same version.
    assert!(has_feature(
        &supported_features(),
        FEATURE_SESSION_TAKEOVER_V1
    ));
    assert!(has_feature(
        &supported_client_features(),
        FEATURE_SESSION_TAKEOVER_V1
    ));
    assert!(!has_feature(&[], FEATURE_SESSION_TAKEOVER_V1));
}
