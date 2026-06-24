mod glibc;

pub use glibc::{GlibcVersion, RemoteLibc};

use std::time::Duration;

use anyhow::{anyhow, Result};
use warp_core::channel::{Channel, ChannelState};

/// State machine for the remote server install → launch → initialize flow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteServerSetupState {
    /// Checking if the binary exists on remote.
    Checking,
    /// Downloading and installing the binary for the first time on this host.
    Installing { progress_percent: Option<u8> },
    /// Replacing an existing install with a differently-versioned binary.
    /// Rendered as "Updating..." in the UI so the user understands this
    /// isn't a fresh install.
    Updating,
    /// Binary is launched, waiting for InitializeResponse.
    Initializing,
    /// Handshake complete. Ready.
    Ready,
    /// Something failed. Fall back to ControlMaster.
    Failed { error: String },
    /// Preinstall check classified the host as incompatible with the
    /// prebuilt remote-server binary. The controller treats this as a
    /// clean fall-back to the legacy ControlMaster-backed SSH flow,
    /// distinct from `Failed` (which is rendered as a real error).
    Unsupported { reason: UnsupportedReason },
}

impl RemoteServerSetupState {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    pub fn is_unsupported(&self) -> bool {
        matches!(self, Self::Unsupported { .. })
    }

    pub fn is_terminal(&self) -> bool {
        self.is_ready() || self.is_failed() || self.is_unsupported()
    }

    pub fn is_in_progress(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Installing { .. } | Self::Updating | Self::Initializing
        )
    }

    pub fn is_connecting(&self) -> bool {
        matches!(
            self,
            Self::Installing { .. } | Self::Updating | Self::Initializing
        )
    }
}

/// Outcome of [`crate::transport::RemoteTransport::run_preinstall_check`].
///
/// The script runs over the existing SSH socket before any install UI
/// surfaces and reports whether the host can run the prebuilt
/// remote-server binary. The Rust side is intentionally a thin parser
/// over the script's structured stdout (see `preinstall_check.sh`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreinstallCheckResult {
    pub status: PreinstallStatus,
    pub libc: RemoteLibc,
    /// Verbatim, trimmed script stdout. Forwarded to telemetry for
    /// diagnosing `Unknown` outcomes on exotic distros.
    pub raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreinstallStatus {
    Supported,
    Unsupported {
        reason: UnsupportedReason,
    },
    /// Probe ran but couldn't classify the host. Treated as supported
    /// (fail open) by [`PreinstallCheckResult::is_supported`] so we keep
    /// today's install-and-try behavior on hosts where the probe is
    /// unreliable.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnsupportedReason {
    GlibcTooOld {
        detected: GlibcVersion,
        required: GlibcVersion,
    },
    NonGlibc {
        name: String,
    },
}

impl PreinstallCheckResult {
    /// Whether the host is supported. Both `Supported` and `Unknown`
    /// return true — only positive detection of an incompatible libc
    /// triggers the silent fall-back.
    pub fn is_supported(&self) -> bool {
        match self.status {
            PreinstallStatus::Supported | PreinstallStatus::Unknown => true,
            PreinstallStatus::Unsupported { .. } => false,
        }
    }

    /// Parses the structured `key=value` stdout emitted by
    /// `preinstall_check.sh`. Tolerates unknown keys and lines without
    /// `=` (forward-compatibility): future versions of the script can
    /// add new keys without coordinating a client release.
    pub fn parse(stdout: &str) -> Self {
        let mut status_str: Option<&str> = None;
        let mut reason_str: Option<&str> = None;
        let mut libc_family: Option<&str> = None;
        let mut libc_version: Option<&str> = None;
        let mut required_glibc: Option<&str> = None;

        for line in stdout.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "status" => status_str = Some(value.trim()),
                "reason" => reason_str = Some(value.trim()),
                "libc_family" => libc_family = Some(value.trim()),
                "libc_version" => libc_version = Some(value.trim()),
                "required_glibc" => required_glibc = Some(value.trim()),
                _ => {} // ignore unknown keys
            }
        }

        let libc = glibc::parse_libc(libc_family, libc_version);
        let status = parse_status(status_str, reason_str, &libc, required_glibc);

        Self {
            status,
            libc,
            raw: stdout.trim().to_string(),
        }
    }
}

fn parse_status(
    status: Option<&str>,
    reason: Option<&str>,
    _libc: &RemoteLibc,
    _required_glibc: Option<&str>,
) -> PreinstallStatus {
    // remote-server is now a static musl binary (see comment at the top of
    // `preinstall_check.sh`), which does not link against the host's dynamic libc.
    // Therefore, `glibc_too_old` / `non_glibc` are no longer valid "unsupported" reasons --
    // any glibc version and musl/uclibc hosts can run this binary. The new script will not
    // emit these two reasons anymore; however, old remote-side versions may still cache the
    // old script, so here we treat these libc gate-keeping reasons as `Supported` rather than
    // `Unsupported` to keep old and new script decisions consistent.
    match status {
        Some("supported") => PreinstallStatus::Supported,
        Some("unsupported") => match reason {
            // Legacy script libc gate-keeping reasons: already obsolete under static binary, treat as supported.
            Some("glibc_too_old") | Some("non_glibc") => PreinstallStatus::Supported,
            // Other unrecognized unsupported reasons: fail open for safety.
            _ => PreinstallStatus::Unknown,
        },
        // status=unknown, missing, or anything else → fail open.
        _ => PreinstallStatus::Unknown,
    }
}

/// The bundled preinstall check script. Loaded as a string so the SSH
/// transport can pipe it through the existing ControlMaster socket via
/// [`crate::ssh::run_ssh_script`].
///
/// The script is intentionally self-contained — the supported-glibc
/// floor is hardcoded inside the script (see `preinstall_check.sh`)
/// rather than templated from Rust.
pub const PREINSTALL_CHECK_SCRIPT: &str = include_str!("preinstall_check.sh");

/// Detected remote platform from `uname -sm` output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemotePlatform {
    pub os: RemoteOs,
    pub arch: RemoteArch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteOs {
    Linux,
    MacOs,
}

impl RemoteOs {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::MacOs => "macos",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteArch {
    X86_64,
    Aarch64,
}

impl RemoteArch {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }
}

/// Parse `uname -sm` output into a `RemotePlatform`.
///
/// The expected format is `<os> <arch>`, e.g. `Linux x86_64` or `Darwin arm64`.
/// Takes the last line to skip any shell initialization output.
pub fn parse_uname_output(output: &str) -> Result<RemotePlatform> {
    let line = output
        .lines()
        .last()
        .ok_or_else(|| anyhow!("empty uname output"))?
        .trim();

    let mut parts = line.split_whitespace();
    let os_str = parts
        .next()
        .ok_or_else(|| anyhow!("missing OS in uname output: {line}"))?;
    let arch_str = parts
        .next()
        .ok_or_else(|| anyhow!("missing arch in uname output: {line}"))?;

    let os = match os_str {
        "Linux" => RemoteOs::Linux,
        "Darwin" => RemoteOs::MacOs,
        other => return Err(anyhow!("unsupported OS: {other}")),
    };

    let arch = match arch_str {
        "x86_64" => RemoteArch::X86_64,
        "aarch64" | "arm64" | "armv8l" => RemoteArch::Aarch64,
        other => return Err(anyhow!("unsupported arch: {other}")),
    };

    Ok(RemotePlatform { os, arch })
}

/// Returns the remote binary installation directory, isolated by channel.
///
/// - stable:      `~/.warp/remote-server`
/// - preview:     `~/.warp-preview/remote-server`
/// - dev:         `~/.warp-dev/remote-server`
/// - local:       `~/.warp-local/remote-server`
/// - integration: `~/.warp-dev/remote-server`
/// - warp-oss:    `~/.zap/remote-server`
pub fn remote_server_dir() -> String {
    let warp_dir = match ChannelState::channel() {
        Channel::Stable => ".warp",
        Channel::Preview => ".warp-preview",
        Channel::Dev | Channel::Integration => ".warp-dev",
        Channel::Local => ".warp-local",
        Channel::Oss => ".zap",
    };
    format!("~/{warp_dir}/remote-server")
}

/// Returns a directory name for the remote-server identity key that is safe to include in a path.
///
/// The identity key is not a cryptographic key, but may contain bytes that are unsafe or ambiguous
/// in paths. ASCII alphanumerics and `-` / `_` are preserved; other UTF-8 bytes are percent-encoded.
pub fn remote_server_identity_dir_name(identity_key: &str) -> String {
    if identity_key.is_empty() {
        return "empty".to_string();
    }

    let mut encoded = String::with_capacity(identity_key.len());
    for byte in identity_key.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Returns the remote directory isolated by identity, used for daemon socket and PID files.
pub fn remote_server_daemon_dir(identity_key: &str) -> String {
    format!(
        "{}/{}",
        remote_server_dir(),
        remote_server_identity_dir_name(identity_key)
    )
}

/// Returns the remote-server binary filename.
pub fn binary_name() -> &'static str {
    ChannelState::channel().cli_command_name()
}

/// Returns the full path to the remote binary corresponding to the current channel and client version.
///
/// Local builds preserve an unversioned suffix path so that `script/deploy_remote_server` can
/// overwrite the same development slot. Zap release builds with `GIT_RELEASE_TAG` use a versioned
/// suffix, allowing new versions to naturally trigger reinstalls. Source-built local builds without
/// a release tag still use an unversioned path.
pub fn remote_server_binary() -> String {
    let dir = remote_server_dir();
    let name = binary_name();
    match ChannelState::channel() {
        Channel::Local => format!("{dir}/{name}"),
        Channel::Oss if ChannelState::app_version().is_none() => format!("{dir}/{name}"),
        Channel::Oss => format!("{dir}/{name}-{}", pinned_version()),
        Channel::Stable | Channel::Preview | Channel::Dev | Channel::Integration => {
            format!("{dir}/{name}-{}", pinned_version())
        }
    }
}

/// Returns the shell command to check if the remote remote-server binary exists and is executable.
///
/// Consistent with upstream, this actually runs `--version` rather than just `test -x`;
/// this allows early detection of corrupted or parameter-unparseable binaries.
pub fn binary_check_command() -> String {
    format!("{} --version", remote_server_binary())
}

/// Returns the version number used for versioned installation paths. Prefers the compile-time injected
/// `GIT_RELEASE_TAG`; falls back to `CARGO_PKG_VERSION` when there is no release tag,
/// keeping channels that need versioned paths deterministic and failing clearly when corresponding
/// release assets are missing, rather than accidentally using unversioned paths.
fn pinned_version() -> &'static str {
    ChannelState::app_version().unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// The install script template is kept in a separate `.sh` file for easy maintenance.
/// Placeholders like `{download_base_url}` are replaced by [`install_script`].
const INSTALL_SCRIPT_TEMPLATE: &str = include_str!("install_remote_server.sh");

/// Returns the install script. When `staging_tarball_path` is non-empty, the script skips remote
/// download and instead extracts a tarball pre-uploaded by the client via SCP.
pub fn install_script(staging_tarball_path: Option<&str>) -> String {
    let version_suffix = version_suffix();
    INSTALL_SCRIPT_TEMPLATE
        .replace("{download_base_url}", &download_url())
        .replace("{install_dir}", &remote_server_dir())
        .replace("{binary_name}", binary_name())
        .replace("{version_suffix}", &version_suffix)
        .replace("{staging_tarball_path}", staging_tarball_path.unwrap_or(""))
}

/// Constructs the base URL for downloading Zap CLI release assets.
fn download_url() -> String {
    let release_path = match ChannelState::app_version() {
        Some(tag) => format!("download/{tag}"),
        None => "latest/download".to_string(),
    };
    format!("https://github.com/zerx-lab/warp/releases/{release_path}")
}

fn version_suffix() -> String {
    match ChannelState::channel() {
        Channel::Local => String::new(),
        Channel::Oss if ChannelState::app_version().is_none() => String::new(),
        Channel::Oss | Channel::Stable | Channel::Preview | Channel::Dev | Channel::Integration => {
            format!("-{}", pinned_version())
        }
    }
}

/// Returns the Zap CLI tarball URL for the specified remote platform.
pub fn download_tarball_url(platform: &RemotePlatform) -> String {
    format!(
        "{}/zap-{}-{}.tar.gz",
        download_url(),
        platform.os.as_str(),
        platform.arch.as_str(),
    )
}

/// Zap fork: In development mode (DEBUG source builds without release tags),
/// the SSH transport no longer downloads stale releases from GitHub. Instead, it cross-compiles
/// the current `warp` binary locally and uploads it. The constants below describe the cross-compilation
/// artifacts, coordinated with `script/deploy_remote_server` (same profile / features / target)
/// to avoid divergence.
///
/// Cross-compilation target triple.
pub const DEV_MUSL_TARGET: &str = "x86_64-unknown-linux-musl";

/// Cargo profile used for cross-compilation. Corresponds to `[profile.dev-remote]` in `Cargo.toml`,
/// which inherits from `dev` and strips symbols to reduce size and speed up uploads.
pub const DEV_REMOTE_PROFILE: &str = "dev-remote";

/// Features enabled for cross-compilation, consistent with `script/deploy_remote_server`.
pub const DEV_REMOTE_FEATURES: &str = "release_bundle,crash_reporting,standalone,agent_mode_debug";

/// Determines whether we are currently in the "development mode remote-server installation" path.
///
/// Default condition: DEBUG build (`debug_assertions`) with no injected `GIT_RELEASE_TAG`
/// (`app_version().is_none()`, i.e., source-built locally, not a release). This matches
/// the standard used in `remote_server_binary()` / `download_url()` for "no release tag".
/// Release builds always return `false`, with unchanged behavior.
///
/// Explicit override: set `WARP_REMOTE_SERVER_FROM_LOCAL=1` to force the local cross-compilation path
/// (`0` or unset means disabled). Used for temporary local remote-server debugging in release builds.
pub fn is_dev_source_build() -> bool {
    if let Some(raw) = std::env::var_os("WARP_REMOTE_SERVER_FROM_LOCAL") {
        let lossy = raw.to_string_lossy();
        let trimmed = lossy.trim();
        let disabled =
            trimmed.is_empty() || trimmed == "0" || trimmed.eq_ignore_ascii_case("false");
        if !disabled {
            return true;
        }
    }
    cfg!(debug_assertions) && ChannelState::app_version().is_none()
}

/// Timeout for checking if the binary exists.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(10);

/// Standard remote install script timeout.
pub const INSTALL_TIMEOUT: Duration = Duration::from_secs(60);

/// SCP fallback includes local download, upload, and remote extraction, so we use a more generous timeout.
pub const SCP_INSTALL_TIMEOUT: Duration = Duration::from_secs(120);

/// Development mode cross-compilation may need to compile the entire crate graph from scratch, so we use a very generous timeout.
pub const DEV_CROSS_COMPILE_TIMEOUT: Duration = Duration::from_secs(900);

/// Timeout for uploading development mode cross-compiled artifacts. The dev binary (unoptimized + debug info)
/// can be hundreds of MB, and even with SCP's `-C` compression, uploading over the network may take minutes,
/// so we use a much more generous limit than `SCP_INSTALL_TIMEOUT`.
pub const DEV_UPLOAD_TIMEOUT: Duration = Duration::from_secs(1800);

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
