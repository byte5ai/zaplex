use super::*;

#[test]
fn linux_stat_parser_handles_spaces_and_closing_parentheses_in_the_name() {
    let mut tail: Vec<String> = (3..=24).map(|field| field.to_string()).collect();
    tail[0] = "S".to_string();
    tail[19] = "987654".to_string();
    let stat = format!("42 (worker ) with spaces) {}", tail.join(" "));

    assert_eq!(parse_linux_proc_start(&stat), Some(987654));
}

#[test]
fn linux_stat_parser_rejects_truncated_or_non_numeric_start_fields() {
    assert_eq!(parse_linux_proc_start("42 worker"), None);
    assert_eq!(parse_linux_proc_start("42 (worker) S 1 2 3"), None);

    let mut tail: Vec<String> = (3..=24).map(|field| field.to_string()).collect();
    tail[0] = "S".to_string();
    tail[19] = "not-a-number".to_string();
    assert_eq!(
        parse_linux_proc_start(&format!("42 (worker) {}", tail.join(" "))),
        None
    );
}

#[test]
fn linux_registry_binding_requires_the_same_ticks_and_current_boot() {
    assert!(linux_registry_binding_matches("987654", 987654, 20_000, 10));
    assert!(!linux_registry_binding_matches(
        "987655", 987654, 20_000, 10
    ));
    assert!(!linux_registry_binding_matches("987654", 987654, 9_999, 10));
    assert!(!linux_registry_binding_matches("", 987654, 20_000, 10));
}

#[test]
fn macos_registry_start_parser_uses_claudes_utc_ps_format() {
    let parsed = parse_macos_registry_start("Thu Jul 23 14:33:02 2026").unwrap();
    let expected = chrono::DateTime::parse_from_rfc3339("2026-07-23T14:33:02Z")
        .unwrap()
        .timestamp();
    assert_eq!(parsed, expected);
}

#[test]
fn macos_registry_binding_uses_microseconds_and_fails_closed() {
    let seconds = parse_macos_registry_start("Thu Jul 23 14:33:02 2026").unwrap() as u64;
    let started_at_millis = seconds as i64 * 1_000 + 500;

    assert!(macos_registry_binding_matches(
        "Thu Jul 23 14:33:02 2026",
        seconds,
        499_999,
        started_at_millis,
        seconds - 100,
    ));
    assert!(!macos_registry_binding_matches(
        "Thu Jul 23 14:33:02 2026",
        seconds,
        500_001,
        started_at_millis,
        seconds - 100,
    ));
    assert!(!macos_registry_binding_matches(
        "Thu Jul 23 14:33:03 2026",
        seconds,
        1,
        started_at_millis,
        seconds - 100,
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn current_linux_process_has_a_boot_scoped_fingerprint() {
    let fingerprint =
        current_process_fingerprint(std::process::id()).expect("current process is inspectable");
    assert!(fingerprint.starts_with("linux-v1:"));
}

#[cfg(target_os = "linux")]
#[test]
fn recycled_pid_is_rejected_by_process_identity() {
    assert_eq!(
        send_verified_process_signal(
            std::process::id(),
            "linux-v1:not-this-process",
            GuardrailSignal::Interrupt,
        ),
        Err(ProcessSignalError::IdentityChanged),
    );
}

#[test]
fn invalid_pid_never_reaches_the_signal_backend() {
    for pid in [0, i32::MAX as u32 + 1, u32::MAX] {
        assert_eq!(
            send_verified_process_signal(pid, "unreachable-fingerprint", GuardrailSignal::Kill),
            Err(ProcessSignalError::InvalidPid),
            "pid {pid} must fail before any platform signal operation"
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn dead_pid_is_rejected_before_signal_dispatch() {
    let result = send_verified_process_signal(
        i32::MAX as u32,
        "fingerprint-for-a-process-that-does-not-exist",
        GuardrailSignal::Interrupt,
    );
    assert!(matches!(
        result,
        Err(ProcessSignalError::IdentityUnavailable(_))
            | Err(ProcessSignalError::IdentityChanged)
    ));
}
