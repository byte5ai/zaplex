//! Precise operating-system process identity for local agent guardrails.
//!
//! A numeric pid is only a temporary slot. Once the process exits, the kernel
//! can assign that number to an unrelated process, so a later Stop/Kill must
//! prove that it still addresses the process discovered for the session.
//!
//! Linux fingerprints a process with the current boot id plus the raw start
//! ticks from `/proc/<pid>/stat`. Signalling uses a pidfd so the process cannot
//! change between the identity check and `pidfd_send_signal`. macOS uses the
//! microsecond process start time from `proc_pidinfo` and checks it immediately
//! before `kill`. Unsupported or unreadable identity always fails closed.

use std::fmt;

use crate::guardrails::{pid_signalable, GuardrailSignal};

const LINUX_FINGERPRINT_VERSION: &str = "linux-v1";
const MACOS_FINGERPRINT_VERSION: &str = "macos-v1";

/// Result of probing one registry pid during discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessProbe {
    /// Whether the pid currently names a process. An unknown pid (`0`) remains
    /// visible as live because its absence is not proof that the session ended.
    pub alive: bool,
    /// Exact identity only when the current process is bound to Claude's
    /// registry `procStart` value.
    pub fingerprint: Option<String>,
}

/// Why a verified signal was not sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessSignalError {
    InvalidPid,
    IdentityUnavailable(String),
    IdentityChanged,
    SignalFailed(String),
    UnsupportedPlatform,
}

impl fmt::Display for ProcessSignalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProcessSignalError::InvalidPid => f.write_str("invalid process id"),
            ProcessSignalError::IdentityUnavailable(error) => {
                write!(f, "process identity is unavailable: {error}")
            }
            ProcessSignalError::IdentityChanged => {
                f.write_str("the process ended or its id was reused")
            }
            ProcessSignalError::SignalFailed(error) => write!(f, "signal failed: {error}"),
            ProcessSignalError::UnsupportedPlatform => {
                f.write_str("verified process signalling is unsupported on this platform")
            }
        }
    }
}

impl std::error::Error for ProcessSignalError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreciseProcessStart {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Linux { ticks: u64 },
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    MacOs { seconds: u64, microseconds: u64 },
}

/// Reads the exact current identity of `pid`.
///
/// The returned value is opaque to callers; only exact equality is meaningful.
pub fn current_process_fingerprint(pid: u32) -> Option<String> {
    if !pid_signalable(pid) {
        return None;
    }
    match precise_process_start(pid)? {
        PreciseProcessStart::Linux { ticks } => {
            let boot_id = linux_boot_id()?;
            Some(linux_fingerprint(&boot_id, ticks))
        }
        PreciseProcessStart::MacOs {
            seconds,
            microseconds,
        } => {
            let boot_time = system_boot_time()?;
            Some(macos_fingerprint(boot_time, seconds, microseconds))
        }
    }
}

/// Probes a Claude registry process and binds it to the registry's `procStart`.
///
/// A live process with a missing or mismatching registry start stays visible,
/// but receives no fingerprint and is therefore never offered Stop/Kill.
pub fn probe_registered_process(
    pid: u32,
    registry_proc_start: Option<&str>,
    registry_started_at_millis: i64,
) -> ProcessProbe {
    if pid == 0 {
        return ProcessProbe {
            alive: true,
            fingerprint: None,
        };
    }
    if !pid_signalable(pid) {
        return ProcessProbe {
            alive: false,
            fingerprint: None,
        };
    }

    let precise_start = precise_process_start(pid);
    let alive = precise_start.is_some() || process_exists(pid);
    let Some(precise_start) = precise_start else {
        return ProcessProbe {
            alive,
            fingerprint: None,
        };
    };
    let Some(registry_proc_start) = registry_proc_start.filter(|value| !value.trim().is_empty())
    else {
        return ProcessProbe {
            alive,
            fingerprint: None,
        };
    };
    let Some(boot_time) = system_boot_time() else {
        return ProcessProbe {
            alive,
            fingerprint: None,
        };
    };

    let fingerprint = match precise_start {
        PreciseProcessStart::Linux { ticks }
            if linux_registry_binding_matches(
                registry_proc_start,
                ticks,
                registry_started_at_millis,
                boot_time,
            ) =>
        {
            linux_boot_id().map(|boot_id| linux_fingerprint(&boot_id, ticks))
        }
        PreciseProcessStart::MacOs {
            seconds,
            microseconds,
        } if macos_registry_binding_matches(
            registry_proc_start,
            seconds,
            microseconds,
            registry_started_at_millis,
            boot_time,
        ) =>
        {
            Some(macos_fingerprint(boot_time, seconds, microseconds))
        }
        PreciseProcessStart::Linux { .. } | PreciseProcessStart::MacOs { .. } => None,
    };

    ProcessProbe { alive, fingerprint }
}

/// Sends a guardrail signal only to the process identified by `expected`.
///
/// Linux opens a pidfd before re-reading the fingerprint and sends through that
/// descriptor, closing the pid-reuse race completely. macOS has no pidfd, so it
/// performs the precise `proc_pidinfo` re-probe immediately before `kill`.
pub fn send_verified_process_signal(
    pid: u32,
    expected: &str,
    signal: GuardrailSignal,
) -> Result<(), ProcessSignalError> {
    if !pid_signalable(pid) {
        return Err(ProcessSignalError::InvalidPid);
    }
    if expected.is_empty() {
        return Err(ProcessSignalError::IdentityUnavailable(
            "the discovery fingerprint is empty".to_string(),
        ));
    }

    #[cfg(target_os = "linux")]
    {
        return send_verified_linux_signal(pid, expected, signal);
    }
    #[cfg(target_os = "macos")]
    {
        return send_verified_macos_signal(pid, expected, signal);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (pid, expected, signal);
        Err(ProcessSignalError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "linux")]
fn send_verified_linux_signal(
    pid: u32,
    expected: &str,
    signal: GuardrailSignal,
) -> Result<(), ProcessSignalError> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    // SAFETY: pid and flags match pidfd_open(2). `pid_signalable` above has
    // already guaranteed a positive value that fits pid_t.
    let raw_fd =
        unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0 as libc::c_uint) };
    if raw_fd < 0 {
        return Err(ProcessSignalError::IdentityUnavailable(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    // SAFETY: a successful pidfd_open returns a new owned file descriptor.
    let pidfd = unsafe { OwnedFd::from_raw_fd(raw_fd as libc::c_int) };

    let observed = current_process_fingerprint(pid).ok_or_else(|| {
        ProcessSignalError::IdentityUnavailable(
            "the current process fingerprint could not be read".to_string(),
        )
    })?;
    if observed != expected {
        return Err(ProcessSignalError::IdentityChanged);
    }

    // SAFETY: pidfd is owned and valid, the signal is one of the two guardrail
    // signals, siginfo is intentionally null, and flags must be zero.
    let rc = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            signal.signal_number(),
            std::ptr::null::<libc::siginfo_t>(),
            0 as libc::c_uint,
        )
    };
    if rc == 0 {
        Ok(())
    } else {
        Err(ProcessSignalError::SignalFailed(
            std::io::Error::last_os_error().to_string(),
        ))
    }
}

#[cfg(target_os = "macos")]
fn send_verified_macos_signal(
    pid: u32,
    expected: &str,
    signal: GuardrailSignal,
) -> Result<(), ProcessSignalError> {
    let observed = current_process_fingerprint(pid).ok_or_else(|| {
        ProcessSignalError::IdentityUnavailable(
            "the current process fingerprint could not be read".to_string(),
        )
    })?;
    if observed != expected {
        return Err(ProcessSignalError::IdentityChanged);
    }

    // SAFETY: pid is a validated positive pid_t and signal is SIGINT/SIGKILL.
    let rc = unsafe { libc::kill(pid as libc::pid_t, signal.signal_number()) };
    if rc == 0 {
        Ok(())
    } else {
        Err(ProcessSignalError::SignalFailed(
            std::io::Error::last_os_error().to_string(),
        ))
    }
}

#[cfg(target_os = "linux")]
fn precise_process_start(pid: u32) -> Option<PreciseProcessStart> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    parse_linux_proc_start(&stat).map(|ticks| PreciseProcessStart::Linux { ticks })
}

#[cfg(target_os = "macos")]
fn precise_process_start(pid: u32) -> Option<PreciseProcessStart> {
    use std::mem::{size_of, MaybeUninit};

    let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = size_of::<libc::proc_bsdinfo>();
    // SAFETY: `info` points to a writable buffer of exactly `size` bytes.
    let written = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size as libc::c_int,
        )
    };
    if written != size as libc::c_int {
        return None;
    }
    // SAFETY: proc_pidinfo reported that it initialized the full struct.
    let info = unsafe { info.assume_init() };
    if info.pbi_pid != pid || info.pbi_start_tvusec >= 1_000_000 {
        return None;
    }
    Some(PreciseProcessStart::MacOs {
        seconds: info.pbi_start_tvsec,
        microseconds: info.pbi_start_tvusec,
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn precise_process_start(_pid: u32) -> Option<PreciseProcessStart> {
    None
}

fn process_exists(pid: u32) -> bool {
    #[cfg(unix)]
    {
        // SAFETY: signal 0 checks existence/permission without sending a signal.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        rc == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

fn system_boot_time() -> Option<u64> {
    let boot_time = sysinfo::System::boot_time();
    (boot_time > 0).then_some(boot_time)
}

#[cfg(target_os = "linux")]
fn linux_boot_id() -> Option<String> {
    let boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
    let boot_id = boot_id.trim();
    (!boot_id.is_empty()).then(|| boot_id.to_string())
}

#[cfg(not(target_os = "linux"))]
fn linux_boot_id() -> Option<String> {
    None
}

fn linux_fingerprint(boot_id: &str, ticks: u64) -> String {
    format!("{LINUX_FINGERPRINT_VERSION}:{boot_id}:{ticks}")
}

fn macos_fingerprint(boot_time: u64, seconds: u64, microseconds: u64) -> String {
    format!("{MACOS_FINGERPRINT_VERSION}:{boot_time}:{seconds}:{microseconds}")
}

fn registration_is_from_current_boot(started_at_millis: i64, boot_time: u64) -> bool {
    started_at_millis > 0 && i128::from(started_at_millis) >= i128::from(boot_time) * 1_000
}

fn linux_registry_binding_matches(
    registry_proc_start: &str,
    ticks: u64,
    started_at_millis: i64,
    boot_time: u64,
) -> bool {
    registration_is_from_current_boot(started_at_millis, boot_time)
        && registry_proc_start.trim().parse::<u64>().ok() == Some(ticks)
}

fn parse_macos_registry_start(value: &str) -> Option<i64> {
    chrono::NaiveDateTime::parse_from_str(value.trim(), "%a %b %e %H:%M:%S %Y")
        .ok()
        .map(|value| value.and_utc().timestamp())
}

fn macos_registry_binding_matches(
    registry_proc_start: &str,
    seconds: u64,
    microseconds: u64,
    started_at_millis: i64,
    boot_time: u64,
) -> bool {
    if !registration_is_from_current_boot(started_at_millis, boot_time)
        || parse_macos_registry_start(registry_proc_start) != i64::try_from(seconds).ok()
    {
        return false;
    }
    let process_start_micros = i128::from(seconds) * 1_000_000 + i128::from(microseconds);
    process_start_micros <= i128::from(started_at_millis) * 1_000
}

fn parse_linux_proc_start(stat: &str) -> Option<u64> {
    // `/proc/<pid>/stat` is `pid (comm) state ...`; `comm` may itself contain
    // spaces or `)`, so split after its final closing delimiter. Field 22 is
    // token 19 when counting from field 3 (`state`) in the remaining tail.
    let tail = stat.rsplit_once(") ")?.1;
    tail.split_whitespace().nth(19)?.parse().ok()
}

/// Produces the same start marker Claude writes as `procStart`.
///
/// This stays crate-visible for deterministic discovery fixtures; production
/// discovery only consumes the marker already present in Claude's registry.
#[cfg(test)]
pub(crate) fn registry_start_for_process(pid: u32) -> Option<String> {
    match precise_process_start(pid)? {
        PreciseProcessStart::Linux { ticks } => Some(ticks.to_string()),
        PreciseProcessStart::MacOs { seconds, .. } => {
            let seconds = i64::try_from(seconds).ok()?;
            chrono::DateTime::<chrono::Utc>::from_timestamp(seconds, 0)
                .map(|value| value.format("%a %b %e %H:%M:%S %Y").to_string())
        }
    }
}

#[cfg(test)]
#[path = "process_identity_tests.rs"]
mod tests;
