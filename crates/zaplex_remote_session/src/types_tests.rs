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

#[cfg(unix)]
#[test]
fn supported_features_advertises_session_host_on_unix() {
    // Stage 1: unix daemons own PTYs and advertise the session host.
    assert!(has_feature(&supported_features(), FEATURE_SESSION_HOST));
}

#[cfg(unix)]
#[test]
fn supported_features_advertises_retry_safe_startup_delivery_on_unix() {
    assert!(has_feature(
        &supported_features(),
        FEATURE_STARTUP_COMMAND_ACK
    ));
}

#[cfg(unix)]
#[test]
fn supported_features_advertises_typed_multiplexer_inventory_on_unix() {
    assert!(has_feature(
        &supported_features(),
        FEATURE_MULTIPLEXER_INVENTORY_V1
    ));
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn supported_features_advertises_safe_file_transactions_on_supported_unix() {
    assert!(has_feature(
        &supported_features(),
        FEATURE_SAFE_FILE_TRANSACTIONS_V1
    ));
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[test]
fn supported_features_omits_safe_file_transactions_when_unsupported() {
    assert!(!has_feature(
        &supported_features(),
        FEATURE_SAFE_FILE_TRANSACTIONS_V1
    ));
}

#[test]
fn supported_features_advertises_agent_inventory_on_all_platforms() {
    // Agent-session inventory is filesystem-based (no PTY), so it is advertised
    // regardless of platform.
    assert!(has_feature(&supported_features(), FEATURE_AGENT_INVENTORY));
}

#[test]
fn supported_features_advertises_host_exec_on_all_platforms() {
    // Session-less host-exec runs in a forked subshell (no PTY), so it is
    // advertised regardless of platform for its non-guardrail callers.
    assert!(has_feature(&supported_features(), FEATURE_HOST_EXEC));
}

#[cfg(target_os = "linux")]
#[test]
fn supported_features_advertises_verified_agent_process_signals() {
    assert!(has_feature(
        &supported_features(),
        FEATURE_AGENT_PROCESS_SIGNAL_V1
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

#[test]
fn supported_client_features_are_explicit_and_platform_independent() {
    let features = supported_client_features();
    for feature in [
        FEATURE_SESSION_HOST,
        FEATURE_STARTUP_COMMAND_ACK,
        FEATURE_AGENT_INVENTORY,
        FEATURE_AGENT_PROCESS_SIGNAL_V1,
        FEATURE_AGENT_PTY_BINDING,
        FEATURE_AGENT_PTY_BINDING_V2,
        FEATURE_SAFE_FILE_TRANSACTIONS_V1,
        FEATURE_HOST_EXEC,
        FEATURE_MULTIPLEXER_INVENTORY_V1,
    ] {
        assert!(has_feature(&features, feature), "missing {feature}");
    }
    assert!(!has_feature(&features, FEATURE_UDP_TRANSPORT));
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
        FEATURE_MULTIPLEXER_INVENTORY_V1
    ));
}
