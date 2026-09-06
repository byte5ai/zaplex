use super::*;
use std::cell::Cell;

fn send_with_spies(
    pid: u32,
    expected: &str,
    acquired: Result<(u32, String), ProcessSignalError>,
    acquire_calls: &Cell<usize>,
    dispatch_calls: &Cell<usize>,
) -> Result<(), ProcessSignalError> {
    send_verified_process_signal_with(
        pid,
        expected,
        GuardrailSignal::Interrupt,
        |_| {
            acquire_calls.set(acquire_calls.get() + 1);
            acquired
        },
        |_, _| {
            dispatch_calls.set(dispatch_calls.get() + 1);
            Ok(())
        },
    )
}

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
fn process_signalling_support_requires_platform_and_runtime_probe() {
    assert_eq!(
        process_signalling_supported_with(|| true),
        cfg!(target_os = "linux")
    );
    assert!(!process_signalling_supported_with(|| false));
}

#[cfg(target_os = "linux")]
#[test]
fn current_linux_process_has_a_boot_scoped_fingerprint() {
    let fingerprint =
        current_process_fingerprint(std::process::id()).expect("current process is inspectable");
    assert!(fingerprint.starts_with("linux-v1:"));
}

#[cfg(target_os = "macos")]
#[test]
fn macos_processes_are_not_exposed_as_signalable_without_a_bound_handle() {
    assert!(!local_process_signalling_supported());
    let pid = std::process::id();
    let proc_start = registry_start_for_process(pid).expect("current process is inspectable");
    let probe = probe_registered_process(
        pid,
        Some(&proc_start),
        chrono::Utc::now().timestamp_millis(),
    );

    assert_eq!(probe.presence, ProcessPresence::UnverifiedLive);
    assert_eq!(probe.fingerprint, None);
    assert_eq!(current_process_fingerprint(pid), None);
    assert_eq!(
        send_verified_process_signal(pid, "macos-v1:unusable", GuardrailSignal::Interrupt),
        Err(ProcessSignalError::UnsupportedPlatform)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn previous_boot_registration_is_stale_even_when_pid_exists() {
    let pid = std::process::id();
    let proc_start = registry_start_for_process(pid).expect("current process is inspectable");
    let boot_time = system_boot_time().expect("system boot time is available");
    let previous_boot_millis =
        i64::try_from(boot_time.saturating_sub(1).saturating_mul(1_000)).unwrap();

    let probe = probe_registered_process(pid, Some(&proc_start), previous_boot_millis);

    assert_eq!(probe.presence, ProcessPresence::StaleRegistration);
    assert_eq!(probe.fingerprint, None);
}

#[cfg(target_os = "linux")]
#[test]
fn matching_current_boot_binding_is_verified_live() {
    let pid = std::process::id();
    let proc_start = registry_start_for_process(pid).expect("current process is inspectable");
    let probe = probe_registered_process(
        pid,
        Some(&proc_start),
        chrono::Utc::now().timestamp_millis(),
    );

    assert_eq!(probe.presence, ProcessPresence::VerifiedLive);
    assert!(probe.fingerprint.is_some());
}

#[cfg(target_os = "linux")]
#[test]
fn missing_or_unparseable_binding_stays_visible_but_unsignalable() {
    let pid = std::process::id();
    let started_at = chrono::Utc::now().timestamp_millis();

    for proc_start in [None, Some("not-a-number")] {
        let probe = probe_registered_process(pid, proc_start, started_at);
        assert_eq!(probe.presence, ProcessPresence::UnverifiedLive);
        assert_eq!(probe.fingerprint, None);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn numeric_process_start_mismatch_is_stale_and_unsignalable() {
    let pid = std::process::id();
    let proc_start = registry_start_for_process(pid)
        .and_then(|value| value.parse::<u64>().ok())
        .expect("current process start ticks are inspectable");
    let mismatching_start = proc_start.saturating_add(1).to_string();

    let probe = probe_registered_process(
        pid,
        Some(&mismatching_start),
        chrono::Utc::now().timestamp_millis(),
    );

    assert_eq!(probe.presence, ProcessPresence::StaleRegistration);
    assert_eq!(probe.fingerprint, None);
}

#[cfg(target_os = "linux")]
fn sleeping_child() -> std::process::Child {
    command::blocking::Command::new("/bin/sleep")
        .arg("60")
        .spawn()
        .expect("spawn a harmless signal target")
}

#[cfg(target_os = "linux")]
fn stop_child(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(target_os = "linux")]
#[test]
fn verified_public_signal_path_controls_the_same_live_process() {
    use std::os::unix::process::ExitStatusExt;

    if !local_process_signalling_supported() {
        return;
    }
    let mut child = sleeping_child();
    let fingerprint = current_process_fingerprint(child.id()).expect("child is inspectable");
    let result = send_verified_process_signal(child.id(), &fingerprint, GuardrailSignal::Kill);
    if result.is_err() {
        stop_child(&mut child);
    }
    assert_eq!(result, Ok(()));
    let status = child.wait().expect("reap signalled child");
    assert_eq!(status.signal(), Some(GuardrailSignal::Kill.signal_number()));
}

#[cfg(target_os = "linux")]
#[test]
fn recycled_identity_is_rejected_by_the_public_signal_path() {
    if !local_process_signalling_supported() {
        return;
    }
    let mut child = sleeping_child();
    let fingerprint = current_process_fingerprint(child.id()).expect("child is inspectable");
    let result = send_verified_process_signal(
        child.id(),
        &format!("{fingerprint}:stale"),
        GuardrailSignal::Kill,
    );
    let child_status = child.try_wait().expect("inspect child");
    if child_status.is_none() {
        stop_child(&mut child);
    }

    assert_eq!(result, Err(ProcessSignalError::IdentityChanged));
    assert_eq!(child_status, None);
}

#[cfg(target_os = "linux")]
#[test]
fn dead_process_is_rejected_by_the_public_signal_path() {
    let mut child = sleeping_child();
    let pid = child.id();
    let fingerprint = current_process_fingerprint(pid).expect("child is inspectable");
    stop_child(&mut child);

    assert!(matches!(
        send_verified_process_signal(pid, &fingerprint, GuardrailSignal::Kill),
        Err(ProcessSignalError::IdentityUnavailable(_)) | Err(ProcessSignalError::IdentityChanged)
    ));
}

#[test]
fn recycled_pid_is_rejected_by_process_identity() {
    let acquire_calls = Cell::new(0);
    let dispatch_calls = Cell::new(0);

    assert_eq!(
        send_with_spies(
            42,
            "fingerprint-at-discovery",
            Ok((42, "fingerprint-after-pid-reuse".to_string())),
            &acquire_calls,
            &dispatch_calls,
        ),
        Err(ProcessSignalError::IdentityChanged),
    );
    assert_eq!(acquire_calls.get(), 1);
    assert_eq!(dispatch_calls.get(), 0);
}

#[test]
fn invalid_pid_never_reaches_the_signal_backend() {
    for pid in [0, i32::MAX as u32 + 1, u32::MAX] {
        let acquire_calls = Cell::new(0);
        let dispatch_calls = Cell::new(0);
        assert_eq!(
            send_with_spies(
                pid,
                "unreachable-fingerprint",
                Ok((pid, "unreachable-fingerprint".to_string())),
                &acquire_calls,
                &dispatch_calls,
            ),
            Err(ProcessSignalError::InvalidPid),
            "pid {pid} must fail before any platform signal operation"
        );
        assert_eq!(acquire_calls.get(), 0);
        assert_eq!(dispatch_calls.get(), 0);
    }
}

#[test]
fn dead_pid_is_rejected_before_signal_dispatch() {
    let acquire_calls = Cell::new(0);
    let dispatch_calls = Cell::new(0);
    let error = ProcessSignalError::IdentityUnavailable("process is dead".to_string());

    let result = send_with_spies(
        42,
        "fingerprint-for-live-process",
        Err(error.clone()),
        &acquire_calls,
        &dispatch_calls,
    );
    assert_eq!(result, Err(error));
    assert_eq!(acquire_calls.get(), 1);
    assert_eq!(dispatch_calls.get(), 0);
}

#[test]
fn missing_process_fingerprint_disables_signal_fail_closed() {
    for expected in ["", "   "] {
        let acquire_calls = Cell::new(0);
        let dispatch_calls = Cell::new(0);
        assert!(matches!(
            send_with_spies(
                42,
                expected,
                Ok((42, "fingerprint".to_string())),
                &acquire_calls,
                &dispatch_calls,
            ),
            Err(ProcessSignalError::IdentityUnavailable(_))
        ));
        assert_eq!(acquire_calls.get(), 0);
        assert_eq!(dispatch_calls.get(), 0);
    }
}

#[test]
fn identical_live_process_reaches_signal_backend_once() {
    let acquire_calls = Cell::new(0);
    let dispatch_calls = Cell::new(0);

    assert_eq!(
        send_with_spies(
            42,
            "same-process",
            Ok((42, "same-process".to_string())),
            &acquire_calls,
            &dispatch_calls,
        ),
        Ok(())
    );
    assert_eq!(acquire_calls.get(), 1);
    assert_eq!(dispatch_calls.get(), 1);
}
