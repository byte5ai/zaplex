use super::*;
#[cfg(target_os = "linux")]
use std::cell::RefCell;
use std::collections::HashMap;
#[cfg(target_os = "linux")]
use std::rc::Rc;

struct FakeProcfs {
    files: HashMap<PathBuf, io::Result<String>>,
    pids: io::Result<Vec<u32>>,
}

impl Default for FakeProcfs {
    fn default() -> Self {
        Self {
            files: HashMap::new(),
            pids: Ok(Vec::new()),
        }
    }
}

impl FakeProcfs {
    fn with_file(mut self, path: &str, contents: &str) -> Self {
        self.files
            .insert(PathBuf::from(path), Ok(contents.to_string()));
        self
    }

    fn with_failed_file(mut self, path: &str) -> Self {
        self.files.insert(
            PathBuf::from(path),
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")),
        );
        self
    }

    fn with_pids(mut self, pids: Vec<u32>) -> Self {
        self.pids = Ok(pids);
        self
    }
}

impl ProcfsReader for FakeProcfs {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        match self.files.get(path) {
            Some(Ok(contents)) => Ok(contents.clone()),
            Some(Err(error)) => Err(io::Error::new(error.kind(), "redacted test error")),
            None => Err(io::Error::new(io::ErrorKind::NotFound, "missing")),
        }
    }

    fn list_pids(&self) -> io::Result<Vec<u32>> {
        match &self.pids {
            Ok(pids) => Ok(pids.clone()),
            Err(error) => Err(io::Error::new(error.kind(), "redacted test error")),
        }
    }
}

fn stat(pid: u32, process_session_id: i32, start_time: u64, name: &str) -> String {
    // state (3), ppid (4), pgrp (5), session (6), then fields through starttime (22).
    format!(
        "{pid} ({name}) S 1 2 {process_session_id} 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 {start_time} 0"
    )
}

#[test]
fn mem_available_is_measured_from_linux_procfs_with_provenance() {
    let fs = FakeProcfs::default().with_file(
        "/proc/meminfo",
        "MemTotal:       16384 kB\nMemAvailable:    4096 kB\n",
    );

    let snapshot = collect_linux_host_memory(&fs, 42);

    assert_eq!(snapshot.collected_at_epoch_millis, 42);
    assert_eq!(
        snapshot.available.status(),
        MemoryMeasurementStatus::Measured
    );
    assert_eq!(snapshot.available.bytes(), Some(4 * 1024 * 1024));
    assert_eq!(
        snapshot.available.provenance().protocol_name(),
        "linux-proc-memavailable"
    );
    assert_eq!(snapshot.available.diagnostic(), None);
}

#[test]
fn measured_zero_is_distinct_from_unavailable() {
    let measured = parse_kib_field(
        "MemAvailable: 0 kB",
        "MemAvailable:",
        MemoryProvenance::LinuxProcMemAvailable,
    );
    let unavailable = parse_kib_field(
        "MemFree: 0 kB",
        "MemAvailable:",
        MemoryProvenance::LinuxProcMemAvailable,
    );

    assert_eq!(measured.status(), MemoryMeasurementStatus::Measured);
    assert_eq!(measured.bytes(), Some(0));
    assert_eq!(unavailable.status(), MemoryMeasurementStatus::Unavailable);
    assert_eq!(unavailable.bytes(), None);
}

#[test]
fn invalid_units_and_overflow_are_explicit() {
    let invalid = parse_kib_field(
        "MemAvailable: 4 MB",
        "MemAvailable:",
        MemoryProvenance::LinuxProcMemAvailable,
    );
    let overflow = parse_kib_field(
        &format!("Pss: {} kB", u64::MAX),
        "Pss:",
        MemoryProvenance::LinuxProcSmapsRollup,
    );

    assert_eq!(invalid.bytes(), None);
    assert_eq!(invalid.diagnostic(), Some(MemoryDiagnostic::InvalidUnit));
    assert_eq!(overflow.bytes(), None);
    assert_eq!(overflow.diagnostic(), Some(MemoryDiagnostic::Overflow));
}

#[test]
fn process_stat_parser_handles_spaces_and_closing_parentheses_in_name() {
    let parsed = parse_process_stat(&stat(42, 42, 9001, "claude worker) one")).unwrap();

    assert_eq!(parsed.pid, 42);
    assert_eq!(parsed.process_session_id, 42);
    assert_eq!(parsed.start_time_ticks, 9001);
}

#[test]
fn process_session_pss_sums_only_the_exact_kernel_session() {
    let fs = FakeProcfs::default()
        .with_pids(vec![42, 43, 43, 99])
        .with_file("/proc/42/stat", &stat(42, 42, 9001, "shell"))
        .with_file("/proc/43/stat", &stat(43, 42, 9002, "claude"))
        .with_file("/proc/99/stat", &stat(99, 99, 12, "other"))
        .with_file("/proc/42/smaps_rollup", "Rss: 30 kB\nPss: 10 kB\n")
        .with_file("/proc/43/smaps_rollup", "Pss: 25 kB\n")
        .with_file("/proc/99/smaps_rollup", "Pss: 5000 kB\n");
    let root = LinuxProcessIdentity {
        pid: 42,
        start_time_ticks: 9001,
        process_session_id: 42,
    };

    let snapshot = collect_linux_process_session_pss(&fs, root, 81);

    assert_eq!(snapshot.collected_at_epoch_millis, 81);
    assert_eq!(snapshot.pss.bytes(), Some(35 * 1024));
    assert_eq!(
        snapshot.pss.provenance(),
        MemoryProvenance::LinuxProcSmapsRollup
    );
}

#[test]
fn process_session_members_are_exact_deduplicated_and_order_the_leader_last() {
    let fs = FakeProcfs::default()
        .with_pids(vec![42, 43, 43, 99])
        .with_file("/proc/42/stat", &stat(42, 42, 9001, "shell"))
        .with_file("/proc/43/stat", &stat(43, 42, 9002, "claude"))
        .with_file("/proc/99/stat", &stat(99, 99, 12, "other"));
    let root = LinuxProcessIdentity {
        pid: 42,
        start_time_ticks: 9001,
        process_session_id: 42,
    };

    let members = linux_process_session_members(&fs, root).unwrap();

    assert_eq!(
        members
            .into_iter()
            .map(|member| member.pid)
            .collect::<Vec<_>>(),
        vec![43, 42]
    );
}

#[test]
fn process_session_members_reject_a_reused_leader_before_selecting_targets() {
    let fs = FakeProcfs::default()
        .with_pids(vec![42, 43])
        .with_file("/proc/42/stat", &stat(42, 42, 9999, "replacement"))
        .with_file("/proc/43/stat", &stat(43, 42, 9002, "claude"));
    let root = LinuxProcessIdentity {
        pid: 42,
        start_time_ticks: 9001,
        process_session_id: 42,
    };

    assert_eq!(
        linux_process_session_members(&fs, root),
        Err(MemoryDiagnostic::ProcessIdentityChanged)
    );
}

#[test]
fn changed_root_identity_fails_before_reading_any_process_memory() {
    let fs = FakeProcfs::default()
        .with_pids(vec![42])
        .with_file("/proc/42/stat", &stat(42, 42, 9999, "replacement"))
        .with_file("/proc/42/smaps_rollup", "Pss: 10 kB\n");
    let root = LinuxProcessIdentity {
        pid: 42,
        start_time_ticks: 9001,
        process_session_id: 42,
    };

    let snapshot = collect_linux_process_session_pss(&fs, root, 81);

    assert_eq!(snapshot.pss.bytes(), None);
    assert_eq!(
        snapshot.pss.diagnostic(),
        Some(MemoryDiagnostic::ProcessIdentityChanged)
    );
}

#[test]
fn managed_process_identity_is_required_instead_of_optionalized() {
    let fs = FakeProcfs::default();

    assert_eq!(
        managed_linux_process_identity(&fs, 42, true),
        Err(MemoryDiagnostic::ReadFailed)
    );
    assert_eq!(managed_linux_process_identity(&fs, 42, false), Ok(None));
}

#[test]
fn process_identity_is_revalidated_after_smaps_read() {
    struct ChangingProcfs {
        stat_reads: std::cell::Cell<usize>,
    }

    impl ProcfsReader for ChangingProcfs {
        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            if path == Path::new("/proc/42/stat") {
                let reads = self.stat_reads.get();
                self.stat_reads.set(reads + 1);
                let start_time = if reads < 2 { 9001 } else { 9002 };
                return Ok(stat(42, 42, start_time, "shell"));
            }
            if path == Path::new("/proc/42/smaps_rollup") {
                return Ok("Pss: 10 kB\n".to_string());
            }
            Err(io::Error::new(io::ErrorKind::NotFound, "missing"))
        }

        fn list_pids(&self) -> io::Result<Vec<u32>> {
            Ok(vec![42])
        }
    }

    let fs = ChangingProcfs {
        stat_reads: std::cell::Cell::new(0),
    };
    let root = LinuxProcessIdentity {
        pid: 42,
        start_time_ticks: 9001,
        process_session_id: 42,
    };

    let snapshot = collect_linux_process_session_pss(&fs, root, 81);

    assert_eq!(snapshot.pss.bytes(), None);
    assert_eq!(
        snapshot.pss.diagnostic(),
        Some(MemoryDiagnostic::ProcessIdentityChanged)
    );
}

#[test]
fn unreadable_member_fails_closed_instead_of_undercounting() {
    let fs = FakeProcfs::default()
        .with_pids(vec![42, 43])
        .with_file("/proc/42/stat", &stat(42, 42, 9001, "shell"))
        .with_file("/proc/43/stat", &stat(43, 42, 9002, "claude"))
        .with_file("/proc/42/smaps_rollup", "Pss: 10 kB\n")
        .with_failed_file("/proc/43/smaps_rollup");
    let root = LinuxProcessIdentity {
        pid: 42,
        start_time_ticks: 9001,
        process_session_id: 42,
    };

    let snapshot = collect_linux_process_session_pss(&fs, root, 81);

    assert_eq!(snapshot.pss.bytes(), None);
    assert_eq!(
        snapshot.pss.diagnostic(),
        Some(MemoryDiagnostic::PartialProcessTree)
    );
}

#[test]
fn diagnostics_are_fixed_codes_not_io_messages() {
    let fs = FakeProcfs::default().with_failed_file("/proc/meminfo");

    let snapshot = collect_linux_host_memory(&fs, 0);

    assert_eq!(snapshot.available.bytes(), None);
    assert_eq!(
        snapshot.available.diagnostic().map(|d| d.protocol_code()),
        Some("read-failed")
    );
}

#[test]
fn unsupported_measurement_has_no_numeric_sentinel() {
    let measurement = MemoryMeasurement::unsupported();

    assert_eq!(measurement.status(), MemoryMeasurementStatus::Unsupported);
    assert_eq!(measurement.bytes(), None);
    assert_eq!(
        measurement.provenance(),
        MemoryProvenance::UnsupportedPlatform
    );
}

#[cfg(not(target_os = "linux"))]
#[test]
fn missing_managed_process_root_projects_as_unsupported_off_linux() {
    let measurement = missing_process_root_measurement();

    assert_eq!(measurement.status(), MemoryMeasurementStatus::Unsupported);
    assert_eq!(measurement.bytes(), None);
    assert_eq!(
        measurement.provenance(),
        MemoryProvenance::UnsupportedPlatform
    );
}

#[cfg(target_os = "linux")]
#[test]
fn missing_managed_process_root_is_unavailable_on_linux() {
    let measurement = missing_process_root_measurement();

    assert_eq!(measurement.status(), MemoryMeasurementStatus::Unavailable);
    assert_eq!(measurement.bytes(), None);
    assert_eq!(
        measurement.diagnostic(),
        Some(MemoryDiagnostic::ProcessIdentityChanged)
    );
}

#[cfg(target_os = "linux")]
#[test]
fn busy_memory_measurements_are_typed_and_secret_free() {
    for measurement in [
        busy_host_memory_measurement(),
        busy_process_memory_measurement(),
    ] {
        assert_eq!(measurement.status(), MemoryMeasurementStatus::Unavailable);
        assert_eq!(measurement.bytes(), None);
        assert_eq!(measurement.diagnostic(), Some(MemoryDiagnostic::Busy));
        assert_eq!(
            measurement
                .diagnostic()
                .map(MemoryDiagnostic::protocol_code),
            Some("busy")
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
fn termination_reaches_a_fixed_point_after_a_child_spawn_race() {
    #[derive(Default)]
    struct MutableProcfsState {
        files: HashMap<PathBuf, String>,
        pids: Vec<u32>,
        signals: Vec<u32>,
    }

    #[derive(Clone)]
    struct MutableProcfs(Rc<RefCell<MutableProcfsState>>);

    impl ProcfsReader for MutableProcfs {
        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            self.0
                .borrow()
                .files
                .get(path)
                .cloned()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing"))
        }

        fn list_pids(&self) -> io::Result<Vec<u32>> {
            Ok(self.0.borrow().pids.clone())
        }
    }

    let state = Rc::new(RefCell::new(MutableProcfsState {
        files: HashMap::from([
            (PathBuf::from("/proc/42/stat"), stat(42, 42, 9001, "shell")),
            (PathBuf::from("/proc/43/stat"), stat(43, 42, 9002, "agent")),
        ]),
        pids: vec![42, 43],
        signals: Vec::new(),
    }));
    let reader = MutableProcfs(state.clone());
    let root = LinuxProcessIdentity {
        pid: 42,
        start_time_ticks: 9001,
        process_session_id: 42,
    };

    let signalled = terminate_linux_process_session_with(
        &reader,
        root,
        |expected| {
            Ok(read_process_stat_optional(&reader, expected.pid)?
                .map(|observed| (expected.pid, observed)))
        },
        |pid| {
            let mut state = state.borrow_mut();
            state.signals.push(pid);
            state
                .files
                .remove(&PathBuf::from(format!("/proc/{pid}/stat")));
            state.pids.retain(|candidate| *candidate != pid);
            if pid == 43 {
                state.files.insert(
                    PathBuf::from("/proc/44/stat"),
                    stat(44, 42, 9003, "late-child"),
                );
                state.pids.push(44);
            }
            Ok(())
        },
        || {},
    )
    .unwrap();

    assert_eq!(signalled, 3);
    assert_eq!(state.borrow().signals, vec![43, 42, 44]);
    assert!(state.borrow().pids.is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn termination_fails_when_the_session_never_becomes_empty() {
    let fs = FakeProcfs::default()
        .with_pids(vec![42])
        .with_file("/proc/42/stat", &stat(42, 42, 9001, "shell"));
    let root = LinuxProcessIdentity {
        pid: 42,
        start_time_ticks: 9001,
        process_session_id: 42,
    };

    let result = terminate_linux_process_session_with(
        &fs,
        root,
        |expected| {
            Ok(read_process_stat_optional(&fs, expected.pid)?
                .map(|observed| (expected.pid, observed)))
        },
        |_| Ok(()),
        || {},
    );

    assert_eq!(result, Err(MemoryDiagnostic::SignalFailed));
}

#[cfg(target_os = "linux")]
#[test]
fn pid_reuse_during_target_acquisition_never_reaches_signal_dispatch() {
    let fs = FakeProcfs::default()
        .with_pids(vec![42])
        .with_file("/proc/42/stat", &stat(42, 42, 9001, "managed"));
    let root = LinuxProcessIdentity {
        pid: 42,
        start_time_ticks: 9001,
        process_session_id: 42,
    };
    let mut dispatched = false;

    let result = terminate_linux_process_session_with(
        &fs,
        root,
        |expected| {
            Ok(Some((
                expected.pid,
                LinuxProcessStat {
                    start_time_ticks: 9002,
                    ..expected
                },
            )))
        },
        |_| {
            dispatched = true;
            Ok(())
        },
        || {},
    );

    assert_eq!(result, Err(MemoryDiagnostic::ProcessIdentityChanged));
    assert!(!dispatched);
}
