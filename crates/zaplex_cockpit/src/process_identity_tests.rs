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

#[cfg(not(unix))]
#[test]
fn unsupported_presence_probe_keeps_current_process_unverified_and_not_cleanable() {
    let pid = std::process::id();
    assert_eq!(process_exists(pid), None);
    let probe = probe_registered_process(pid, None, chrono::Utc::now().timestamp_millis());
    assert_eq!(probe.presence, ProcessPresence::UnverifiedLive);
    assert!(probe.presence.is_live());
    assert!(!probe.presence.allows_registry_cleanup());
    assert_eq!(probe.fingerprint, None);
}

// ── Zaplex terminal link (hookless ownership) ───────────────────────────────

#[test]
fn surface_id_is_the_only_value_taken_from_an_environment_block() {
    let environ =
        b"HOME=/home/me\0ZAPLEX_CONTROL_TOKEN=secret\0ZAPLEX_SURFACE_ID=9a1f-22\0PATH=/bin\0";
    assert_eq!(surface_id_from_environ(environ).as_deref(), Some("9a1f-22"));
    assert_eq!(surface_id_from_environ(b"HOME=/home/me\0PATH=/bin\0"), None);
    // A prefix of another variable is not the surface id.
    assert_eq!(surface_id_from_environ(b"XZAPLEX_SURFACE_ID=1\0"), None);
    // Malformed or oversized values are ignored rather than trusted.
    assert_eq!(surface_id_from_environ(b"ZAPLEX_SURFACE_ID=a b\0"), None);
    let oversized = format!("ZAPLEX_SURFACE_ID={}\0", "a".repeat(129));
    assert_eq!(surface_id_from_environ(oversized.as_bytes()), None);
}

#[test]
fn an_unreadable_environment_is_unknown_never_external() {
    assert_eq!(terminal_link_from_environ(None), TerminalLink::Unknown);
    assert_eq!(terminal_link_from_environ(Some(b"")), TerminalLink::Unknown);
    assert_eq!(
        terminal_link_from_environ(Some(b"HOME=/home/me\0")),
        TerminalLink::External
    );
    assert_eq!(
        terminal_link_from_environ(Some(b"ZAPLEX_SURFACE_ID=s1\0")),
        TerminalLink::Surface("s1".to_string())
    );
}

#[test]
fn merged_links_prefer_a_zaplex_surface_then_external_evidence() {
    let surface = TerminalLink::Surface("s1".to_string());
    assert_eq!(TerminalLink::External.merge(surface.clone()), surface);
    assert_eq!(TerminalLink::Unknown.merge(surface.clone()), surface);
    assert_eq!(
        TerminalLink::Unknown.merge(TerminalLink::External),
        TerminalLink::External
    );
    assert_eq!(
        TerminalLink::Unknown.merge(TerminalLink::Unknown),
        TerminalLink::Unknown
    );
}

#[cfg(target_os = "linux")]
fn sleeping_child_with_surface(surface_id: Option<&str>) -> std::process::Child {
    let mut command = command::blocking::Command::new("/bin/sleep");
    command.arg("60").env_remove(ZAPLEX_SURFACE_ENV);
    if let Some(surface_id) = surface_id {
        command.env(ZAPLEX_SURFACE_ENV, surface_id);
    }
    command.spawn().expect("spawn a harmless agent stand-in")
}

/// A process started in a Zaplex pane carries that pane's surface id even
/// without any hook bridge; one started elsewhere carries none.
#[cfg(target_os = "linux")]
#[test]
fn a_live_process_is_linked_to_the_zaplex_pane_it_was_started_in() {
    let mut inside = sleeping_child_with_surface(Some("surface-1"));
    let mut outside = sleeping_child_with_surface(None);
    let inside_fingerprint = current_process_fingerprint(inside.id());

    let inside_link = terminal_link_for_pid(inside.id(), inside_fingerprint.as_deref());
    let outside_link = terminal_link_for_pid(outside.id(), None);
    stop_child(&mut inside);
    stop_child(&mut outside);

    assert_eq!(inside_link, TerminalLink::Surface("surface-1".to_string()));
    assert_eq!(outside_link, TerminalLink::External);
}

/// PID reuse can never attribute a session to a pane: a process whose start
/// identity differs from the one discovery bound is no evidence at all.
#[cfg(target_os = "linux")]
#[test]
fn a_reused_or_vanished_pid_is_never_linked() {
    let mut child = sleeping_child_with_surface(Some("surface-2"));
    let pid = child.id();
    let fingerprint = current_process_fingerprint(pid).expect("child is inspectable");
    let reused = terminal_link_for_pid(pid, Some(&format!("{fingerprint}:other-process")));
    stop_child(&mut child);

    assert_eq!(reused, TerminalLink::Unknown);
    assert_eq!(
        terminal_link_for_pid(pid, Some(&fingerprint)),
        TerminalLink::Unknown
    );
    assert_eq!(terminal_link_for_pid(0, None), TerminalLink::Unknown);
}
