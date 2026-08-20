//! Secret-safe memory accounting for daemon-owned managed agent sessions.
//!
//! Linux procfs is the only supported authoritative source in v1. Every
//! failure is explicit: unavailable and unsupported measurements never become
//! a measured zero and a partially readable process tree is never undercounted.

use std::io;
use std::path::{Path, PathBuf};

const KIB_BYTES: u64 = 1024;
const PROC_ROOT: &str = "/proc";
#[cfg(target_os = "linux")]
const TERMINATION_MAX_PASSES: usize = 16;
#[cfg(target_os = "linux")]
const TERMINATION_PASS_DELAY: std::time::Duration = std::time::Duration::from_millis(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryMeasurementStatus {
    Measured,
    Unavailable,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryProvenance {
    LinuxProcMemAvailable,
    LinuxProcSmapsRollup,
    UnsupportedPlatform,
}

impl MemoryProvenance {
    pub(crate) fn protocol_name(self) -> &'static str {
        match self {
            Self::LinuxProcMemAvailable => "linux-proc-memavailable",
            Self::LinuxProcSmapsRollup => "linux-proc-smaps-rollup",
            Self::UnsupportedPlatform => "unsupported-platform",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MemoryDiagnostic {
    ReadFailed,
    MissingField,
    InvalidUnit,
    InvalidValue,
    Overflow,
    EmptyProcessSet,
    PartialProcessTree,
    ProcessIdentityChanged,
    ProjectIdentityChanged,
    AccountRouteChanged,
    SignalFailed,
    Busy,
    UnsupportedPlatform,
}

impl MemoryDiagnostic {
    pub(crate) fn protocol_code(self) -> &'static str {
        match self {
            Self::ReadFailed => "read-failed",
            Self::MissingField => "missing-field",
            Self::InvalidUnit => "invalid-unit",
            Self::InvalidValue => "invalid-value",
            Self::Overflow => "overflow",
            Self::EmptyProcessSet => "empty-process-set",
            Self::PartialProcessTree => "partial-process-tree",
            Self::ProcessIdentityChanged => "process-identity-changed",
            Self::ProjectIdentityChanged => "project-identity-changed",
            Self::AccountRouteChanged => "account-route-changed",
            Self::SignalFailed => "signal-failed",
            Self::Busy => "busy",
            Self::UnsupportedPlatform => "unsupported-platform",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MemoryMeasurement {
    status: MemoryMeasurementStatus,
    bytes: Option<u64>,
    provenance: MemoryProvenance,
    diagnostic: Option<MemoryDiagnostic>,
}

impl MemoryMeasurement {
    pub(crate) fn measured(bytes: u64, provenance: MemoryProvenance) -> Self {
        Self {
            status: MemoryMeasurementStatus::Measured,
            bytes: Some(bytes),
            provenance,
            diagnostic: None,
        }
    }

    pub(crate) fn unavailable(provenance: MemoryProvenance, diagnostic: MemoryDiagnostic) -> Self {
        Self {
            status: MemoryMeasurementStatus::Unavailable,
            bytes: None,
            provenance,
            diagnostic: Some(diagnostic),
        }
    }

    pub(crate) fn unsupported() -> Self {
        Self {
            status: MemoryMeasurementStatus::Unsupported,
            bytes: None,
            provenance: MemoryProvenance::UnsupportedPlatform,
            diagnostic: Some(MemoryDiagnostic::UnsupportedPlatform),
        }
    }

    pub(crate) fn status(&self) -> MemoryMeasurementStatus {
        self.status
    }

    pub(crate) fn bytes(&self) -> Option<u64> {
        self.bytes
    }

    pub(crate) fn provenance(&self) -> MemoryProvenance {
        self.provenance
    }

    pub(crate) fn diagnostic(&self) -> Option<MemoryDiagnostic> {
        self.diagnostic
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HostMemorySnapshot {
    pub(crate) available: MemoryMeasurement,
    pub(crate) collected_at_epoch_millis: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessMemorySnapshot {
    pub(crate) pss: MemoryMeasurement,
    pub(crate) collected_at_epoch_millis: u64,
}

/// Stable identity for the daemon-owned process-session leader. Linux start
/// time prevents a reused PID from authorizing measurement of another process;
/// the kernel session id scopes descendants even if a child is re-parented.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct LinuxProcessIdentity {
    pub(crate) pid: u32,
    pub(crate) start_time_ticks: u64,
    pub(crate) process_session_id: i32,
}

pub(crate) trait ProcfsReader {
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    fn list_pids(&self) -> io::Result<Vec<u32>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RealProcfs;

impl ProcfsReader for RealProcfs {
    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn list_pids(&self) -> io::Result<Vec<u32>> {
        let mut pids = Vec::new();
        for entry in std::fs::read_dir(PROC_ROOT)? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if let Ok(pid) = name.parse() {
                pids.push(pid);
            }
        }
        Ok(pids)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LinuxProcessStat {
    pid: u32,
    process_session_id: i32,
    start_time_ticks: u64,
    state: u8,
}

impl LinuxProcessStat {
    fn has_exited(self) -> bool {
        matches!(self.state, b'Z' | b'X' | b'x')
    }

    fn same_identity(self, other: Self) -> bool {
        self.pid == other.pid
            && self.process_session_id == other.process_session_id
            && self.start_time_ticks == other.start_time_ticks
    }
}

pub(crate) fn read_linux_process_identity(
    reader: &impl ProcfsReader,
    pid: u32,
) -> Result<LinuxProcessIdentity, MemoryDiagnostic> {
    let stat = read_process_stat(reader, pid)?;
    Ok(LinuxProcessIdentity {
        pid: stat.pid,
        start_time_ticks: stat.start_time_ticks,
        process_session_id: stat.process_session_id,
    })
}

pub(crate) fn managed_linux_process_identity(
    reader: &impl ProcfsReader,
    pid: u32,
    required: bool,
) -> Result<Option<LinuxProcessIdentity>, MemoryDiagnostic> {
    if required {
        read_linux_process_identity(reader, pid).map(Some)
    } else {
        Ok(None)
    }
}

pub(crate) fn collect_linux_host_memory(
    reader: &impl ProcfsReader,
    collected_at_epoch_millis: u64,
) -> HostMemorySnapshot {
    let available = match reader.read_to_string(Path::new("/proc/meminfo")) {
        Ok(contents) => parse_kib_field(
            &contents,
            "MemAvailable:",
            MemoryProvenance::LinuxProcMemAvailable,
        ),
        Err(_) => MemoryMeasurement::unavailable(
            MemoryProvenance::LinuxProcMemAvailable,
            MemoryDiagnostic::ReadFailed,
        ),
    };
    HostMemorySnapshot {
        available,
        collected_at_epoch_millis,
    }
}

pub(crate) fn collect_linux_process_session_pss(
    reader: &impl ProcfsReader,
    root: LinuxProcessIdentity,
    collected_at_epoch_millis: u64,
) -> ProcessMemorySnapshot {
    let pss = collect_linux_process_session_pss_inner(reader, root);
    ProcessMemorySnapshot {
        pss,
        collected_at_epoch_millis,
    }
}

fn collect_linux_process_session_pss_inner(
    reader: &impl ProcfsReader,
    root: LinuxProcessIdentity,
) -> MemoryMeasurement {
    let session_processes = match linux_process_session_members(reader, root) {
        Ok(processes) => processes,
        Err(diagnostic) => {
            return MemoryMeasurement::unavailable(
                MemoryProvenance::LinuxProcSmapsRollup,
                diagnostic,
            )
        }
    };

    let mut total = 0_u64;
    for expected in session_processes {
        let path = PathBuf::from(format!("/proc/{}/smaps_rollup", expected.pid));
        let Ok(contents) = reader.read_to_string(&path) else {
            return MemoryMeasurement::unavailable(
                MemoryProvenance::LinuxProcSmapsRollup,
                MemoryDiagnostic::PartialProcessTree,
            );
        };
        let measurement =
            parse_kib_field(&contents, "Pss:", MemoryProvenance::LinuxProcSmapsRollup);
        let Some(bytes) = measurement.bytes() else {
            return MemoryMeasurement::unavailable(
                MemoryProvenance::LinuxProcSmapsRollup,
                MemoryDiagnostic::PartialProcessTree,
            );
        };
        let Ok(observed) = read_process_stat(reader, expected.pid) else {
            return MemoryMeasurement::unavailable(
                MemoryProvenance::LinuxProcSmapsRollup,
                MemoryDiagnostic::ProcessIdentityChanged,
            );
        };
        if !observed.same_identity(expected) {
            return MemoryMeasurement::unavailable(
                MemoryProvenance::LinuxProcSmapsRollup,
                MemoryDiagnostic::ProcessIdentityChanged,
            );
        }
        let Some(next) = total.checked_add(bytes) else {
            return MemoryMeasurement::unavailable(
                MemoryProvenance::LinuxProcSmapsRollup,
                MemoryDiagnostic::Overflow,
            );
        };
        total = next;
    }
    MemoryMeasurement::measured(total, MemoryProvenance::LinuxProcSmapsRollup)
}

/// Resolves the exact Linux process session owned by one managed PTY.
///
/// The daemon records the shell leader's start time when it creates the PTY.
/// Revalidating that tuple before enumerating descendants prevents a recycled
/// PID from turning a lifecycle action into a signal against another process.
fn linux_process_session_members(
    reader: &impl ProcfsReader,
    root: LinuxProcessIdentity,
) -> Result<Vec<LinuxProcessStat>, MemoryDiagnostic> {
    let current_root = read_process_stat(reader, root.pid)
        .map_err(|_| MemoryDiagnostic::ProcessIdentityChanged)?;
    if current_root.start_time_ticks != root.start_time_ticks
        || current_root.process_session_id != root.process_session_id
    {
        return Err(MemoryDiagnostic::ProcessIdentityChanged);
    }

    let pids = reader
        .list_pids()
        .map_err(|_| MemoryDiagnostic::ReadFailed)?;
    let mut members = Vec::new();
    for pid in pids {
        let Ok(stat) = read_process_stat(reader, pid) else {
            // Processes outside this PTY can disappear while /proc is scanned.
            // A vanished entry cannot remain a live session member to signal.
            continue;
        };
        if stat.process_session_id == root.process_session_id {
            members.push(stat);
        }
    }
    members.sort_unstable_by_key(|stat| (stat.pid == root.pid, stat.pid));
    members.dedup_by_key(|stat| stat.pid);
    if !members
        .iter()
        .any(|stat| stat.pid == root.pid && stat.start_time_ticks == root.start_time_ticks)
    {
        return Err(MemoryDiagnostic::EmptyProcessSet);
    }
    Ok(members)
}

#[cfg(target_os = "linux")]
fn read_process_stat_optional(
    reader: &impl ProcfsReader,
    pid: u32,
) -> Result<Option<LinuxProcessStat>, MemoryDiagnostic> {
    let path = PathBuf::from(format!("/proc/{pid}/stat"));
    match reader.read_to_string(&path) {
        Ok(contents) => parse_process_stat(&contents).map(Some),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(MemoryDiagnostic::ReadFailed),
    }
}

/// Re-scans an already authenticated Linux process session during explicit
/// termination. The original leader may disappear after the first pass, but a
/// reused leader PID is always rejected and every surviving member retains its
/// own start-time identity until it is signalled.
#[cfg(target_os = "linux")]
fn linux_process_session_members_for_termination(
    reader: &impl ProcfsReader,
    root: LinuxProcessIdentity,
) -> Result<Vec<LinuxProcessStat>, MemoryDiagnostic> {
    if let Some(current_root) = read_process_stat_optional(reader, root.pid)? {
        if current_root.start_time_ticks != root.start_time_ticks
            || current_root.process_session_id != root.process_session_id
        {
            return Err(MemoryDiagnostic::ProcessIdentityChanged);
        }
    }

    let pids = reader
        .list_pids()
        .map_err(|_| MemoryDiagnostic::ReadFailed)?;
    let mut members = Vec::new();
    for pid in pids {
        let Some(stat) = read_process_stat_optional(reader, pid)? else {
            continue;
        };
        if stat.pid == root.pid
            && (stat.start_time_ticks != root.start_time_ticks
                || stat.process_session_id != root.process_session_id)
        {
            return Err(MemoryDiagnostic::ProcessIdentityChanged);
        }
        if stat.process_session_id == root.process_session_id && !stat.has_exited() {
            members.push(stat);
        }
    }
    members.sort_unstable_by_key(|stat| (stat.pid == root.pid, stat.pid));
    members.dedup_by_key(|stat| stat.pid);
    Ok(members)
}

#[cfg(target_os = "linux")]
fn terminate_linux_process_session_with<T>(
    reader: &impl ProcfsReader,
    root: LinuxProcessIdentity,
    mut acquire: impl FnMut(LinuxProcessStat) -> Result<Option<(T, LinuxProcessStat)>, MemoryDiagnostic>,
    mut signal: impl FnMut(T) -> io::Result<()>,
    mut after_pass: impl FnMut(),
) -> Result<usize, MemoryDiagnostic> {
    let mut signalled = 0;
    for _ in 0..TERMINATION_MAX_PASSES {
        let members = linux_process_session_members_for_termination(reader, root)?;
        if members.is_empty() {
            return Ok(signalled);
        }
        for expected in members {
            let Some((target, observed)) = acquire(expected)? else {
                continue;
            };
            if !observed.same_identity(expected) {
                return Err(MemoryDiagnostic::ProcessIdentityChanged);
            }
            match signal(target) {
                Ok(()) => signalled += 1,
                Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {}
                Err(_) => return Err(MemoryDiagnostic::SignalFailed),
            }
        }
        after_pass();
    }

    // A successful lifecycle response requires an explicit final proof that
    // no live process remains in the kernel session. This catches children
    // forked between an earlier enumeration and its signal pass.
    if linux_process_session_members_for_termination(reader, root)?.is_empty() {
        Ok(signalled)
    } else {
        Err(MemoryDiagnostic::SignalFailed)
    }
}

/// Terminates every still-matching member of a daemon-owned Linux process
/// session, with the PTY shell leader ordered last. This is used only for an
/// explicit generation-checked managed Stop/Restart; ordinary terminal GC
/// keeps its existing behavior.
#[cfg(target_os = "linux")]
pub(crate) fn terminate_linux_process_session(
    root: LinuxProcessIdentity,
    mut reap_leader: impl FnMut(),
) -> Result<usize, MemoryDiagnostic> {
    use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};

    let reader = RealProcfs;
    terminate_linux_process_session_with(
        &reader,
        root,
        |expected| {
            let pid = i32::try_from(expected.pid).map_err(|_| MemoryDiagnostic::SignalFailed)?;
            // SAFETY: pid is positive and pidfd_open requires zero flags. The
            // returned descriptor binds all later actions to this exact kernel
            // process, even if the numeric PID is reused.
            let raw_fd = unsafe {
                libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0 as libc::c_uint)
            };
            if raw_fd < 0 {
                let error = io::Error::last_os_error();
                return if error.raw_os_error() == Some(libc::ESRCH) {
                    Ok(None)
                } else {
                    Err(MemoryDiagnostic::SignalFailed)
                };
            }
            // SAFETY: successful pidfd_open returns a new owned descriptor.
            let pidfd = unsafe { OwnedFd::from_raw_fd(raw_fd as libc::c_int) };
            let Some(observed) = read_process_stat_optional(&reader, expected.pid)? else {
                return Ok(None);
            };
            Ok(Some((pidfd, observed)))
        },
        |pidfd| {
            // SAFETY: pidfd is owned and valid, SIGKILL has no pointer payload,
            // siginfo is null, and pidfd_send_signal requires zero flags.
            let result = unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    pidfd.as_raw_fd(),
                    libc::SIGKILL,
                    std::ptr::null::<libc::siginfo_t>(),
                    0 as libc::c_uint,
                )
            };
            if result == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        },
        || {
            reap_leader();
            std::thread::sleep(TERMINATION_PASS_DELAY);
        },
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn missing_process_root_measurement() -> MemoryMeasurement {
    MemoryMeasurement::unavailable(
        MemoryProvenance::LinuxProcSmapsRollup,
        MemoryDiagnostic::ProcessIdentityChanged,
    )
}

#[cfg(target_os = "linux")]
pub(crate) fn busy_host_memory_measurement() -> MemoryMeasurement {
    MemoryMeasurement::unavailable(
        MemoryProvenance::LinuxProcMemAvailable,
        MemoryDiagnostic::Busy,
    )
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn busy_host_memory_measurement() -> MemoryMeasurement {
    MemoryMeasurement::unsupported()
}

#[cfg(target_os = "linux")]
pub(crate) fn busy_process_memory_measurement() -> MemoryMeasurement {
    MemoryMeasurement::unavailable(
        MemoryProvenance::LinuxProcSmapsRollup,
        MemoryDiagnostic::Busy,
    )
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn busy_process_memory_measurement() -> MemoryMeasurement {
    MemoryMeasurement::unsupported()
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn missing_process_root_measurement() -> MemoryMeasurement {
    MemoryMeasurement::unsupported()
}

#[cfg(target_os = "linux")]
pub(crate) fn collect_host_memory(collected_at_epoch_millis: u64) -> HostMemorySnapshot {
    collect_linux_host_memory(&RealProcfs, collected_at_epoch_millis)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn collect_host_memory(collected_at_epoch_millis: u64) -> HostMemorySnapshot {
    HostMemorySnapshot {
        available: MemoryMeasurement::unsupported(),
        collected_at_epoch_millis,
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn collect_process_session_pss(
    root: LinuxProcessIdentity,
    collected_at_epoch_millis: u64,
) -> ProcessMemorySnapshot {
    collect_linux_process_session_pss(&RealProcfs, root, collected_at_epoch_millis)
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn collect_process_session_pss(
    _root: LinuxProcessIdentity,
    collected_at_epoch_millis: u64,
) -> ProcessMemorySnapshot {
    ProcessMemorySnapshot {
        pss: MemoryMeasurement::unsupported(),
        collected_at_epoch_millis,
    }
}

fn read_process_stat(
    reader: &impl ProcfsReader,
    pid: u32,
) -> Result<LinuxProcessStat, MemoryDiagnostic> {
    let path = PathBuf::from(format!("/proc/{pid}/stat"));
    let contents = reader
        .read_to_string(&path)
        .map_err(|_| MemoryDiagnostic::ReadFailed)?;
    parse_process_stat(&contents)
}

fn parse_process_stat(contents: &str) -> Result<LinuxProcessStat, MemoryDiagnostic> {
    let open = contents.find('(').ok_or(MemoryDiagnostic::InvalidValue)?;
    let close = contents.rfind(')').ok_or(MemoryDiagnostic::InvalidValue)?;
    if close <= open {
        return Err(MemoryDiagnostic::InvalidValue);
    }
    let pid = contents[..open]
        .trim()
        .parse::<u32>()
        .map_err(|_| MemoryDiagnostic::InvalidValue)?;
    let fields: Vec<&str> = contents[close + 1..].split_whitespace().collect();
    // Fields after comm start at field 3 (state). Session is field 6 (index 3),
    // and starttime is field 22 (index 19).
    if fields.len() <= 19 {
        return Err(MemoryDiagnostic::MissingField);
    }
    let state = match fields[0].as_bytes() {
        [state] => *state,
        _ => return Err(MemoryDiagnostic::InvalidValue),
    };
    let process_session_id = fields[3]
        .parse::<i32>()
        .map_err(|_| MemoryDiagnostic::InvalidValue)?;
    let start_time_ticks = fields[19]
        .parse::<u64>()
        .map_err(|_| MemoryDiagnostic::InvalidValue)?;
    Ok(LinuxProcessStat {
        pid,
        process_session_id,
        start_time_ticks,
        state,
    })
}

fn parse_kib_field(contents: &str, field: &str, provenance: MemoryProvenance) -> MemoryMeasurement {
    let Some(line) = contents.lines().find(|line| {
        line.split_whitespace()
            .next()
            .is_some_and(|name| name == field)
    }) else {
        return MemoryMeasurement::unavailable(provenance, MemoryDiagnostic::MissingField);
    };
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() != 3 {
        return MemoryMeasurement::unavailable(provenance, MemoryDiagnostic::InvalidValue);
    }
    if parts[2] != "kB" {
        return MemoryMeasurement::unavailable(provenance, MemoryDiagnostic::InvalidUnit);
    }
    let Ok(kib) = parts[1].parse::<u64>() else {
        return MemoryMeasurement::unavailable(provenance, MemoryDiagnostic::InvalidValue);
    };
    let Some(bytes) = kib.checked_mul(KIB_BYTES) else {
        return MemoryMeasurement::unavailable(provenance, MemoryDiagnostic::Overflow);
    };
    MemoryMeasurement::measured(bytes, provenance)
}

#[cfg(test)]
#[path = "fleet_memory_tests.rs"]
mod tests;
