//! Precise operating-system process identity for local agent guardrails.
//!
//! A numeric pid is only a temporary slot. Once the process exits, the kernel
//! can assign that number to an unrelated process, so a later Stop/Kill must
//! prove that it still addresses the process discovered for the session.
//!
//! Linux fingerprints a process with the current boot id plus the raw start
//! ticks from `/proc/<pid>/stat`. Signalling uses a pidfd so the process cannot
//! change between the identity check and `pidfd_send_signal`. macOS has no
//! equivalent process-bound signalling primitive, so local signalling is not
//! offered there. Unsupported or unreadable identity always fails closed.

use std::{fmt, sync::OnceLock};

use crate::guardrails::{pid_signalable, GuardrailSignal};

const LINUX_FINGERPRINT_VERSION: &str = "linux-v1";

/// Result of probing one registry pid during discovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessProbe {
    pub presence: ProcessPresence,
    /// Exact identity only when the current process is bound to Claude's
    /// registry `procStart` value.
    pub fingerprint: Option<String>,
}

/// Relationship between a registry entry and the process currently using its pid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessPresence {
    /// The process exists and its current-boot identity matches the registry.
    VerifiedLive,
    /// The process exists or its presence is unknown; no usable identity binding exists.
    UnverifiedLive,
    /// The registry is from a previous boot or its pid now belongs to another process.
    StaleRegistration,
    /// No process currently occupies the registered pid.
    Absent,
}

impl ProcessPresence {
    pub const fn is_live(self) -> bool {
        matches!(self, Self::VerifiedLive | Self::UnverifiedLive)
    }

    pub const fn allows_registry_cleanup(self) -> bool {
        matches!(self, Self::StaleRegistration | Self::Absent)
    }
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

/// Whether this host can signal a process through an identity-bound target.
///
/// Linux support is probed once because old kernels or a seccomp policy can
/// reject the pidfd syscalls even when the binary compiled successfully.
pub fn local_process_signalling_supported() -> bool {
    static SUPPORTED: OnceLock<bool> = OnceLock::new();
    *SUPPORTED.get_or_init(|| process_signalling_supported_with(probe_linux_pidfd_signal_support))
}

fn process_signalling_supported_with(probe: impl FnOnce() -> bool) -> bool {
    #[cfg(target_os = "linux")]
    {
        probe()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = probe;
        false
    }
}

#[cfg(target_os = "linux")]
fn probe_linux_pidfd_signal_support() -> bool {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    // SAFETY: the current process id is positive and flags must be zero.
    let raw_fd = unsafe {
        libc::syscall(
            libc::SYS_pidfd_open,
            std::process::id() as libc::pid_t,
            0 as libc::c_uint,
        )
    };
    if raw_fd < 0 {
        return false;
    }
    // SAFETY: a successful pidfd_open returns a new owned file descriptor.
    let pidfd = unsafe { OwnedFd::from_raw_fd(raw_fd as libc::c_int) };
    // SAFETY: signal 0 performs only permission/existence validation. It has
    // no signal effect; siginfo is null and flags must be zero.
    unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            pidfd.as_raw_fd(),
            0,
            std::ptr::null::<libc::siginfo_t>(),
            0 as libc::c_uint,
        ) == 0
    }
}

#[cfg(not(target_os = "linux"))]
fn probe_linux_pidfd_signal_support() -> bool {
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreciseProcessStart {
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Linux { ticks: u64 },
    #[allow(dead_code)]
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
        PreciseProcessStart::MacOs { .. } => None,
    }
}

/// Probes a Claude registry process and binds it to the registry's `procStart`.
///
/// A live process with a missing or unparseable registry binding stays visible
/// but unsignalable. A previous-boot registration or a numeric Linux start-tick
/// mismatch is stale even when another process now occupies the pid.
pub fn probe_registered_process(
    pid: u32,
    registry_proc_start: Option<&str>,
    registry_started_at_millis: i64,
) -> ProcessProbe {
    if pid == 0 {
        return ProcessProbe {
            presence: ProcessPresence::UnverifiedLive,
            fingerprint: None,
        };
    }
    if !pid_signalable(pid) {
        return ProcessProbe {
            presence: ProcessPresence::Absent,
            fingerprint: None,
        };
    }

    let precise_start = precise_process_start(pid);
    // Unsupported or unreadable probes do not prove that a process exited.
    let process_is_live = precise_start.is_some() || process_exists(pid).unwrap_or(true);
    if !process_is_live {
        return ProcessProbe {
            presence: ProcessPresence::Absent,
            fingerprint: None,
        };
    }

    let boot_time = system_boot_time();
    if boot_time.is_some_and(|boot_time| {
        registry_started_at_millis > 0
            && i128::from(registry_started_at_millis) < i128::from(boot_time) * 1_000
    }) {
        return ProcessProbe {
            presence: ProcessPresence::StaleRegistration,
            fingerprint: None,
        };
    }

    let Some(precise_start) = precise_start else {
        return ProcessProbe {
            presence: ProcessPresence::UnverifiedLive,
            fingerprint: None,
        };
    };
    let Some(registry_proc_start) = registry_proc_start.filter(|value| !value.trim().is_empty())
    else {
        return ProcessProbe {
            presence: ProcessPresence::UnverifiedLive,
            fingerprint: None,
        };
    };

    match precise_start {
        PreciseProcessStart::Linux { ticks } => {
            let Ok(registry_ticks) = registry_proc_start.trim().parse::<u64>() else {
                return ProcessProbe {
                    presence: ProcessPresence::UnverifiedLive,
                    fingerprint: None,
                };
            };
            if registry_ticks != ticks {
                return ProcessProbe {
                    presence: ProcessPresence::StaleRegistration,
                    fingerprint: None,
                };
            }
            let Some(boot_id) = boot_time
                .filter(|boot_time| {
                    registration_is_from_current_boot(registry_started_at_millis, *boot_time)
                })
                .and_then(|_| linux_boot_id())
            else {
                return ProcessProbe {
                    presence: ProcessPresence::UnverifiedLive,
                    fingerprint: None,
                };
            };
            ProcessProbe {
                presence: ProcessPresence::VerifiedLive,
                fingerprint: Some(linux_fingerprint(&boot_id, ticks)),
            }
        }
        PreciseProcessStart::MacOs { .. } => ProcessProbe {
            presence: ProcessPresence::UnverifiedLive,
            fingerprint: None,
        },
    }
}

/// Sends a guardrail signal only to the process identified by `expected`.
///
/// Linux opens a pidfd before re-reading the fingerprint and sends through that
/// descriptor, closing the pid-reuse race completely. Platforms without an
/// identity-bound signalling primitive return [`ProcessSignalError::UnsupportedPlatform`].
pub fn send_verified_process_signal(
    pid: u32,
    expected: &str,
    signal: GuardrailSignal,
) -> Result<(), ProcessSignalError> {
    #[cfg(target_os = "linux")]
    {
        send_verified_linux_signal(pid, expected, signal)
    }
    #[cfg(target_os = "macos")]
    {
        validate_signal_request(pid, expected)?;
        let _ = signal;
        return Err(ProcessSignalError::UnsupportedPlatform);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        validate_signal_request(pid, expected)?;
        let _ = signal;
        Err(ProcessSignalError::UnsupportedPlatform)
    }
}

fn validate_signal_request(pid: u32, expected: &str) -> Result<(), ProcessSignalError> {
    if !pid_signalable(pid) {
        return Err(ProcessSignalError::InvalidPid);
    }
    if expected.trim().is_empty() {
        return Err(ProcessSignalError::IdentityUnavailable(
            "the discovery fingerprint is empty".to_string(),
        ));
    }
    Ok(())
}

/// Re-probes a process identity and reaches `dispatch` only on an exact match.
///
/// `acquire` returns an identity-bound target together with the fingerprint
/// observed while acquiring it. On Linux that target is a pidfd; tests inject
/// a harmless token and a spy dispatcher to prove every rejection fails closed.
fn send_verified_process_signal_with<T>(
    pid: u32,
    expected: &str,
    signal: GuardrailSignal,
    acquire: impl FnOnce(u32) -> Result<(T, String), ProcessSignalError>,
    dispatch: impl FnOnce(T, GuardrailSignal) -> Result<(), ProcessSignalError>,
) -> Result<(), ProcessSignalError> {
    validate_signal_request(pid, expected)?;

    let (target, observed) = acquire(pid)?;
    if observed != expected {
        return Err(ProcessSignalError::IdentityChanged);
    }

    dispatch(target, signal)
}

#[cfg(target_os = "linux")]
fn send_verified_linux_signal(
    pid: u32,
    expected: &str,
    signal: GuardrailSignal,
) -> Result<(), ProcessSignalError> {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    send_verified_process_signal_with(
        pid,
        expected,
        signal,
        |pid| {
            // SAFETY: pid and flags match pidfd_open(2). The shared verifier
            // has guaranteed a positive value that fits pid_t before calling
            // this acquisition closure.
            let raw_fd = unsafe {
                libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0 as libc::c_uint)
            };
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
            Ok((pidfd, observed))
        },
        |pidfd, signal| {
            // SAFETY: pidfd is owned and valid, the signal is one of the two
            // guardrail signals, siginfo is intentionally null, and flags are zero.
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
        },
    )
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

fn process_exists(pid: u32) -> Option<bool> {
    #[cfg(unix)]
    {
        // SAFETY: signal 0 checks existence/permission without sending a signal.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            return Some(true);
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::EPERM) => Some(true),
            Some(libc::ESRCH) => Some(false),
            _ => None,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        None
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

fn registration_is_from_current_boot(started_at_millis: i64, boot_time: u64) -> bool {
    started_at_millis > 0 && i128::from(started_at_millis) >= i128::from(boot_time) * 1_000
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

/// Environment variable Zaplex injects into every local terminal PTY: one
/// random, non-secret id per pane (the control surface id). An agent process
/// started in that pane inherits it, which links the process to the pane even
/// when no hook bridge is installed.
pub const ZAPLEX_SURFACE_ENV: &str = "ZAPLEX_SURFACE_ID";
const MAX_SURFACE_ID_BYTES: usize = 128;
/// Upper bound for one process environment read.
const MAX_ENVIRON_BYTES: u64 = 1024 * 1024;

/// What the operating system says about the Zaplex terminal a live local agent
/// process runs in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalLink {
    /// The process inherited this Zaplex terminal surface id.
    Surface(String),
    /// The process environment was readable and carries no Zaplex surface: the
    /// agent was started outside Zaplex.
    External,
    /// No usable evidence: unsupported platform, unreadable environment, a
    /// process that changed or vanished while being read, or no process at all.
    /// Callers must not treat this as "external".
    Unknown,
}

impl TerminalLink {
    pub fn surface_id(&self) -> Option<&str> {
        match self {
            TerminalLink::Surface(surface_id) => Some(surface_id),
            TerminalLink::External | TerminalLink::Unknown => None,
        }
    }

    /// Combine the links of several processes holding the same session: a
    /// Zaplex surface wins, then positive external evidence.
    pub fn merge(self, other: TerminalLink) -> TerminalLink {
        match (self, other) {
            (TerminalLink::Surface(surface_id), _) | (_, TerminalLink::Surface(surface_id)) => {
                TerminalLink::Surface(surface_id)
            }
            (TerminalLink::External, _) | (_, TerminalLink::External) => TerminalLink::External,
            (TerminalLink::Unknown, TerminalLink::Unknown) => TerminalLink::Unknown,
        }
    }
}

/// The Zaplex surface id carried by a NUL-separated environment block, if it
/// is present and well formed. Only this one variable is ever extracted; the
/// rest of the environment (which may hold credentials) is neither parsed
/// further nor retained.
pub fn surface_id_from_environ(environ: &[u8]) -> Option<String> {
    let prefix = format!("{ZAPLEX_SURFACE_ENV}=");
    environ
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(prefix.as_bytes()))
        .filter(|value| {
            !value.is_empty()
                && value.len() <= MAX_SURFACE_ID_BYTES
                && value
                    .iter()
                    .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
        })
        .and_then(|value| std::str::from_utf8(value).ok())
        .map(str::to_string)
}

/// Decide the link from an environment read. An empty block is no evidence:
/// every real process has some environment, so it means the read failed.
pub fn terminal_link_from_environ(environ: Option<&[u8]>) -> TerminalLink {
    match environ {
        Some(environ) if !environ.is_empty() => surface_id_from_environ(environ)
            .map(TerminalLink::Surface)
            .unwrap_or(TerminalLink::External),
        Some(_) | None => TerminalLink::Unknown,
    }
}

/// Which Zaplex terminal, if any, the live local process `pid` runs in.
///
/// PID reuse can never misattribute a session: the process start identity is
/// read before and after the environment and must not change, and when
/// discovery bound the session to an exact fingerprint (`expected_fingerprint`,
/// Linux) the current process must still carry it. Anything else is
/// [`TerminalLink::Unknown`].
pub fn terminal_link_for_pid(pid: u32, expected_fingerprint: Option<&str>) -> TerminalLink {
    if !pid_signalable(pid) {
        return TerminalLink::Unknown;
    }
    let before = precise_process_start(pid);
    let environ = read_process_environ(pid);
    let after = precise_process_start(pid);
    if before.is_none() || before != after {
        return TerminalLink::Unknown;
    }
    if let Some(expected) = expected_fingerprint {
        if current_process_fingerprint(pid).as_deref() != Some(expected) {
            return TerminalLink::Unknown;
        }
    }
    terminal_link_from_environ(environ.as_deref())
}

#[cfg(target_os = "linux")]
fn read_process_environ(pid: u32) -> Option<Vec<u8>> {
    use std::io::Read as _;

    let mut environ = Vec::new();
    std::fs::File::open(format!("/proc/{pid}/environ"))
        .ok()?
        .take(MAX_ENVIRON_BYTES)
        .read_to_end(&mut environ)
        .ok()?;
    Some(environ)
}

#[cfg(target_os = "macos")]
fn read_process_environ(pid: u32) -> Option<Vec<u8>> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_environ(UpdateKind::Always),
    );
    let mut environ = Vec::new();
    for entry in system.process(pid)?.environ() {
        environ.extend_from_slice(entry.as_encoded_bytes());
        environ.push(0);
        if environ.len() as u64 > MAX_ENVIRON_BYTES {
            return None;
        }
    }
    Some(environ)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn read_process_environ(_pid: u32) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
#[path = "process_identity_tests.rs"]
mod tests;
