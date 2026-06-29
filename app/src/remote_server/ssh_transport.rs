//! SSH-specific implementation of [`RemoteTransport`].
//!
//! [`SshTransport`] uses an existing SSH ControlMaster socket to check/install
//! the remote server binary and to launch the `remote-server-proxy` process
//! whose stdin/stdout become the protocol channel.
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use warpui::r#async::{executor, FutureExt as _};

use remote_server::auth::RemoteServerAuthContext;
use remote_server::client::RemoteServerClient;
use remote_server::setup::{
    parse_uname_output, remote_server_daemon_dir, PreinstallCheckResult, RemotePlatform,
};
use remote_server::ssh::ssh_args;
use remote_server::transport::{Connection, RemoteTransport};

/// SSH transport: connects via a ControlMaster socket.
///
/// `socket_path` is the local Unix socket created by the ControlMaster
/// process (`ssh -N -o ControlMaster=yes -o ControlPath=<path>`). All SSH
/// commands (binary check, install, proxy launch) are multiplexed through
/// this socket without re-authenticating.
#[derive(Clone)]
pub struct SshTransport {
    socket_path: PathBuf,
    auth_context: Arc<RemoteServerAuthContext>,
}

impl fmt::Debug for SshTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshTransport")
            .field("socket_path", &self.socket_path)
            .finish_non_exhaustive()
    }
}

impl SshTransport {
    pub fn new(socket_path: PathBuf, auth_context: Arc<RemoteServerAuthContext>) -> Self {
        Self {
            socket_path,
            auth_context,
        }
    }

    pub fn socket_path(&self) -> &PathBuf {
        &self.socket_path
    }

    pub fn remote_daemon_socket_path(&self) -> String {
        format!(
            "{}/server.sock",
            remote_server_daemon_dir(&self.auth_context.remote_server_identity_key())
        )
    }

    pub fn remote_daemon_pid_path(&self) -> String {
        format!(
            "{}/server.pid",
            remote_server_daemon_dir(&self.auth_context.remote_server_identity_key())
        )
    }

    fn remote_proxy_command(&self) -> String {
        let binary = remote_server::setup::remote_server_binary();
        let identity_key = self.auth_context.remote_server_identity_key();
        let quoted_identity_key = shell_words::quote(&identity_key);
        format!("{binary} remote-server-proxy --identity-key {quoted_identity_key}")
    }
}

#[derive(Debug)]
enum InstallError {
    ScriptFailed { exit_code: i32, stderr: String },
    Other(anyhow::Error),
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ScriptFailed { exit_code, stderr } => {
                write!(f, "install script failed (exit {exit_code}): {stderr}")
            }
            Self::Other(error) => write!(f, "{error:#}"),
        }
    }
}

impl From<anyhow::Error> for InstallError {
    fn from(error: anyhow::Error) -> Self {
        Self::Other(error)
    }
}

async fn detect_remote_platform(socket_path: &Path) -> Result<RemotePlatform> {
    let output = remote_server::ssh::run_ssh_command(
        socket_path,
        "uname -sm",
        remote_server::setup::CHECK_TIMEOUT,
    )
    .await?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        return parse_uname_output(&stdout);
    }

    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!("uname -sm exited with code {code}: {stderr}"))
}

async fn verify_installed_binary(socket_path: &Path) -> Result<()> {
    let output = remote_server::ssh::run_ssh_command(
        socket_path,
        &remote_server::setup::binary_check_command(),
        remote_server::setup::CHECK_TIMEOUT,
    )
    .await?;

    if output.status.success() {
        return Ok(());
    }

    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "installed binary check failed with code {code}: {stderr}"
    ))
}

async fn run_install_script(
    socket_path: &Path,
    staging_tarball_path: Option<&str>,
    timeout: std::time::Duration,
) -> core::result::Result<(), InstallError> {
    let script = remote_server::setup::install_script(staging_tarball_path);
    match remote_server::ssh::run_ssh_script(socket_path, &script, timeout).await {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => {
            let exit_code = output.status.code().unwrap_or(-1);
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Err(InstallError::ScriptFailed { exit_code, stderr })
        }
        Err(error) => Err(InstallError::Other(error)),
    }
}

fn should_skip_scp_fallback(error: &InstallError) -> bool {
    matches!(error, InstallError::ScriptFailed { exit_code: 2, .. })
}

// ===========================================================================
// Zaplex fork: dev-mode remote-server installation path
//
// Upstream/release builds have the remote install script download pre-built
// remote-server binaries from GitHub releases. However, during local source
// builds (`cargo run`), this would download the latest released version—a
// stale binary—rather than the developer's freshly modified code, making
// debugging remote-server changes impossible.
//
// Therefore, for DEBUG source builds without a release tag (see
// `remote_server::setup::is_dev_source_build()`), `install_binary()` instead:
//   1. Locally cross-compile the `warp` binary to x86_64 musl (profile/features
//      identical to `script/deploy_remote_server`);
//   2. Using the existing SSH ControlMaster socket, upload the result via
//      `scp_upload` to the remote path resolved by
//      `remote_server::setup::remote_server_binary()`;
//   3. Completely skip the GitHub download and install script.
//
// If cross-compile prerequisites are missing (musl target not installed, no
// musl linker), the build does not hard-fail. Instead, it prints a clear
// warning and falls back to the original download-install flow, keeping dev
// builds usable.
// ===========================================================================

/// Candidate musl linkers for dev-mode cross-compilation (in priority order).
/// On macOS typically `x86_64-linux-musl-gcc` (filosofittile/musl-cross),
/// on Linux commonly `musl-gcc`.
const DEV_MUSL_LINKER_CANDIDATES: &[&str] = &["x86_64-linux-musl-gcc", "musl-gcc"];

/// Returns the workspace root directory.
///
/// `ssh_transport.rs` belongs to the `app` crate, where `CARGO_MANIFEST_DIR`
/// points to `<workspace>/app`. Its parent is the workspace root.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        // In theory `app` always has a parent; if not, fall back to the manifest dir itself.
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

/// Returns PATH with `~/.cargo/bin` (and `$CARGO_HOME/bin`) prepended.
///
/// The warp process is often launched by the desktop environment or system
/// `cargo`, whose PATH may contain only `/usr/bin` without `~/.cargo/bin`.
/// This causes:
///   - `cargo zigbuild` cannot find the `cargo-zigbuild` subcommand → falls back to musl-gcc;
///   - cargo-zigbuild itself cannot find `cargo` / `rustc`.
/// Cross-compilation subprocesses uniformly use the PATH returned here to ensure
/// both can resolve. If no adjustment is needed (no HOME / cannot concatenate),
/// returns `None` and the caller uses the inherited PATH.
fn dev_build_path_env() -> Option<std::ffi::OsString> {
    let mut extra: Vec<PathBuf> = Vec::new();
    if let Some(cargo_home) = std::env::var_os("CARGO_HOME") {
        extra.push(PathBuf::from(cargo_home).join("bin"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        extra.push(PathBuf::from(home).join(".cargo").join("bin"));
    }
    if extra.is_empty() {
        return None;
    }
    let current = std::env::var_os("PATH").unwrap_or_default();
    extra.extend(std::env::split_paths(&current));
    std::env::join_paths(extra).ok()
}

/// Finds the first available musl linker in `PATH`, returning `None` if not found.
fn find_musl_linker() -> Option<&'static str> {
    DEV_MUSL_LINKER_CANDIDATES.iter().copied().find(|linker| {
        command::blocking::Command::new(linker)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    })
}

/// Build backend for dev cross-compilation.
enum DevBuildBackend {
    /// `cargo zigbuild`: zig serves as a complete C/C++ musl cross-toolchain,
    /// eliminating the need to separately install `*-musl-gcc` / `*-musl-g++`.
    /// Correctly compiles dependencies with C/C++ source like `freetype-sys`.
    /// This is the preferred backend.
    Zigbuild,
    /// Native `cargo build` + musl linker. Only reliable when the system has
    /// a complete musl C/C++ cross-toolchain. With only `*-musl-gcc` and no
    /// `*-musl-g++`, C++ dependencies like `freetype-sys` fail to compile.
    MuslGcc(&'static str),
}

/// Detects whether `cargo-zigbuild` is available.
///
/// Probes `cargo-zigbuild --version` (the binary itself), not
/// `cargo zigbuild --version`—the latter fails as the `zigbuild` subcommand
/// parses it as an unknown parameter. The probe PATH matches the actual build
/// (with `~/.cargo/bin` injected).
fn cargo_zigbuild_available() -> bool {
    let mut cmd = command::blocking::Command::new("cargo-zigbuild");
    cmd.arg("--version");
    if let Some(path) = dev_build_path_env() {
        cmd.env("PATH", path);
    }
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Selects the dev cross-compile backend: prefers `cargo zigbuild`, falls back to
/// native `cargo build` + musl linker. Returns `None` if both unavailable,
/// allowing the caller to fall back to download-install.
fn select_dev_build_backend() -> Option<DevBuildBackend> {
    if cargo_zigbuild_available() {
        return Some(DevBuildBackend::Zigbuild);
    }
    find_musl_linker().map(DevBuildBackend::MuslGcc)
}

/// Checks whether the `x86_64-unknown-linux-musl` target is installed via rustup.
async fn musl_target_installed() -> bool {
    let output = command::r#async::Command::new("rustup")
        .arg("target")
        .arg("list")
        .arg("--installed")
        .kill_on_drop(true)
        .output()
        .await;
    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.trim() == remote_server::setup::DEV_MUSL_TARGET),
        // When unable to get rustup output, conservatively assume uninstalled to trigger fallback.
        _ => false,
    }
}

/// Cross-compiles local `warp` binary to musl, returning the output path.
///
/// Profile/features align with `script/deploy_remote_server`.
async fn cross_compile_remote_server(backend: &DevBuildBackend) -> Result<PathBuf> {
    let root = workspace_root();
    // Name of the `[[bin]]` for the current channel — OSS fork is `warp-oss` (see app/Cargo.toml).
    // Cannot hardcode `warp`: that binary uses `load_config!("local")` and needs the private
    // `warp-channel-config` to generate `local_config.json`. OSS fork lacks it and fails to compile.
    // `warp-oss` (src/bin/oss.rs) inlines `ChannelConfig`, no such dependency.
    let bin_name = remote_server::setup::binary_name();
    let backend_desc = match backend {
        DevBuildBackend::Zigbuild => "cargo-zigbuild".to_string(),
        DevBuildBackend::MuslGcc(linker) => format!("cargo-build/{linker}"),
    };
    log::info!(
        "dev remote-server: cross-compiling {bin_name} -> {} (profile={}, backend={backend_desc})",
        remote_server::setup::DEV_MUSL_TARGET,
        remote_server::setup::DEV_REMOTE_PROFILE,
    );
    // First-time compilation of the entire warp typically takes several minutes.
    // stdout/stderr are directly inherited to the terminal running Zaplex, so developers
    // can see cargo's real-time progress (otherwise completely silent, easy to think
    // it's stuck).
    log::info!(
        "dev remote-server: cross-compiling now, first time typically takes several minutes — \
         cargo progress will be printed to the terminal running Zaplex"
    );

    let status = async {
        let mut cmd = command::r#async::Command::new("cargo");
        cmd.current_dir(&root);
        // Inject `~/.cargo/bin` to ensure `cargo zigbuild` can resolve the `cargo-zigbuild`
        // subcommand, and cargo-zigbuild can find `cargo` / `rustc`.
        if let Some(path) = dev_build_path_env() {
            cmd.env("PATH", path);
        }
        match backend {
            // zigbuild is a cargo subcommand with its own zig linker and C/C++ cross-compiler.
            // No need to set LINKER env.
            DevBuildBackend::Zigbuild => {
                cmd.arg("zigbuild");
            }
            // Native cargo build: specify musl linker via env and override rustflags
            // to prevent macOS-specific flags in .cargo/config.toml from polluting cross-compilation.
            DevBuildBackend::MuslGcc(linker) => {
                cmd.arg("build")
                    .env("CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER", *linker)
                    .env(
                        "CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS",
                        "-C symbol-mangling-version=v0",
                    );
            }
        }
        cmd.arg("-p")
            .arg("warp")
            .arg("--bin")
            .arg(bin_name)
            .arg("--target")
            .arg(remote_server::setup::DEV_MUSL_TARGET)
            .arg("--profile")
            .arg(remote_server::setup::DEV_REMOTE_PROFILE)
            .arg("--features")
            .arg(remote_server::setup::DEV_REMOTE_FEATURES)
            // inherit: pass cargo's real-time progress to terminal instead of buffering silently.
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true)
            .status()
            .await
    }
    .with_timeout(remote_server::setup::DEV_CROSS_COMPILE_TIMEOUT)
    .await
    .map_err(|_| {
        anyhow!(
            "dev remote-server cross-compilation timeout (>{:?})",
            remote_server::setup::DEV_CROSS_COMPILE_TIMEOUT
        )
    })?
    .map_err(|e| anyhow!("Failed to start cargo build: {e}"))?;

    if !status.success() {
        let code = status.code().unwrap_or(-1);
        return Err(anyhow!(
            "cargo cross-compilation failed (exit {code}); see cargo output in the terminal running Zaplex"
        ));
    }

    // Output location: `<target_dir>/<triple>/<profile>/<bin_name>`.
    // Prefer `CARGO_TARGET_DIR`, otherwise fall back to `<workspace>/target`.
    // The repo does not set `[build] target-dir` in `.cargo/config.toml`, so only env matters.
    let target_root = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target"));
    let binary = target_root
        .join(remote_server::setup::DEV_MUSL_TARGET)
        .join(remote_server::setup::DEV_REMOTE_PROFILE)
        .join(bin_name);
    if !binary.is_file() {
        return Err(anyhow!(
            "cross-compilation completed but output not found at {} (if CARGO_TARGET_DIR is set, verify the path)",
            binary.display()
        ));
    }
    Ok(binary)
}

/// Dev-mode install: cross-compile local `warp` and upload to remote remote-server path.
///
/// Upload target exactly matches `remote_server_binary()`, ensuring that
/// subsequent `check_binary()` / proxy startup can find it.
async fn dev_install_local_binary(socket_path: &Path) -> Result<()> {
    // Prerequisite check: missing any item returns error, caller falls back to download-install.
    if !musl_target_installed().await {
        return Err(anyhow!(
            "Rust target {} not installed; run `rustup target add {}`",
            remote_server::setup::DEV_MUSL_TARGET,
            remote_server::setup::DEV_MUSL_TARGET,
        ));
    }
    // Select cross-compile backend: prefer `cargo zigbuild` (zig includes complete C/C++ musl
    // toolchain, can compile C++ dependencies like freetype-sys), else fall back to musl-gcc.
    // If neither available, error.
    let backend = select_dev_build_backend().ok_or_else(|| {
        anyhow!(
            "No available musl cross-compilation backend found. Recommended: install cargo-zigbuild + zig\
             (`cargo install cargo-zigbuild`, and install `zig` via package manager),\
             or install a complete musl C/C++ cross-toolchain ({})",
            DEV_MUSL_LINKER_CANDIDATES.join(" / ")
        )
    })?;

    let local_binary = cross_compile_remote_server(&backend).await?;

    // Upload to the exact path resolved by `remote_server_binary()`, creating parent dir first.
    let remote_binary = remote_server::setup::remote_server_binary();
    let remote_dir = remote_server::setup::remote_server_dir();
    let mkdir_output = remote_server::ssh::run_ssh_command(
        socket_path,
        &format!("mkdir -p {remote_dir}"),
        remote_server::setup::CHECK_TIMEOUT,
    )
    .await?;
    if !mkdir_output.status.success() {
        let code = mkdir_output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&mkdir_output.stderr);
        return Err(anyhow!(
            "remote-server directory creation failed (exit {code}): {stderr}"
        ));
    }

    log::info!("dev remote-server: uploading local cross-compile output to {remote_binary} (scp -C compressed, hundreds of MB may take several minutes)");
    // Dev output is hundreds of MB, use DEV_UPLOAD_TIMEOUT (much longer than SCP_INSTALL_TIMEOUT)
    // to avoid large file uploads being interrupted by 120s timeout, then falling back to stale release.
    remote_server::ssh::scp_upload(
        socket_path,
        &local_binary,
        &remote_binary,
        remote_server::setup::DEV_UPLOAD_TIMEOUT,
    )
    .await?;

    // Make executable.
    let chmod_output = remote_server::ssh::run_ssh_command(
        socket_path,
        &format!("chmod 755 {remote_binary}"),
        remote_server::setup::CHECK_TIMEOUT,
    )
    .await?;
    if !chmod_output.status.success() {
        let code = chmod_output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&chmod_output.stderr);
        return Err(anyhow!("remote chmod failed (exit {code}): {stderr}"));
    }

    // Reuse existing verification logic to confirm the uploaded binary is executable.
    verify_installed_binary(socket_path).await
}

async fn download_remote_server_tarball(download_url: &str, tarball_path: &Path) -> Result<()> {
    let output = async {
        command::r#async::Command::new("curl")
            .arg("-fSL")
            .arg("--connect-timeout")
            .arg("15")
            .arg(download_url)
            .arg("-o")
            .arg(tarball_path.as_os_str())
            .kill_on_drop(true)
            .output()
            .await
    }
    .with_timeout(remote_server::setup::SCP_INSTALL_TIMEOUT)
    .await
    .map_err(|_| {
        anyhow!(
            "local tarball download timed out after {:?}",
            remote_server::setup::SCP_INSTALL_TIMEOUT
        )
    })?
    .map_err(|e| anyhow!("local curl failed to execute: {e}"))?;

    if output.status.success() {
        return Ok(());
    }

    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(anyhow!(
        "local tarball download failed with code {code}: {stderr}"
    ))
}

async fn scp_install_fallback(socket_path: &Path) -> Result<()> {
    let platform = detect_remote_platform(socket_path).await?;
    let download_url = remote_server::setup::download_tarball_url(&platform);
    let remote_server_dir = remote_server::setup::remote_server_dir();
    let mkdir_cmd = format!("mkdir -p {remote_server_dir}");
    let mkdir_output = remote_server::ssh::run_ssh_command(
        socket_path,
        &mkdir_cmd,
        remote_server::setup::CHECK_TIMEOUT,
    )
    .await?;

    if !mkdir_output.status.success() {
        let code = mkdir_output.status.code().unwrap_or(-1);
        let stderr = String::from_utf8_lossy(&mkdir_output.stderr);
        return Err(anyhow!(
            "remote-server dir creation failed with code {code}: {stderr}"
        ));
    }

    let tempdir = tempfile::tempdir()?;
    let tarball_path = tempdir.path().join("zap.tar.gz");
    download_remote_server_tarball(&download_url, &tarball_path).await?;

    let remote_tarball_path = format!("{remote_server_dir}/zap-upload.tar.gz");
    remote_server::ssh::scp_upload(
        socket_path,
        &tarball_path,
        &remote_tarball_path,
        remote_server::setup::SCP_INSTALL_TIMEOUT,
    )
    .await?;

    run_install_script(
        socket_path,
        Some(&remote_tarball_path),
        remote_server::setup::SCP_INSTALL_TIMEOUT,
    )
    .await
    .map_err(|error| anyhow!("staged install failed: {error}"))?;

    verify_installed_binary(socket_path).await
}

impl RemoteTransport for SshTransport {
    fn detect_platform(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<RemotePlatform, String>> + Send>> {
        let socket_path = self.socket_path.clone();
        Box::pin(async move {
            detect_remote_platform(&socket_path)
                .await
                .map_err(|e| format!("{e:#}"))
        })
    }

    fn run_preinstall_check(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<PreinstallCheckResult, String>> + Send>> {
        let socket_path = self.socket_path.clone();
        Box::pin(async move {
            match remote_server::ssh::run_ssh_script(
                &socket_path,
                remote_server::setup::PREINSTALL_CHECK_SCRIPT,
                remote_server::setup::CHECK_TIMEOUT,
            )
            .await
            {
                Ok(output) if output.status.success() => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    Ok(PreinstallCheckResult::parse(&stdout))
                }
                Ok(output) => {
                    let code = output.status.code().unwrap_or(-1);
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(format!(
                        "Preinstall check exited with code {code}: {stderr}"
                    ))
                }
                Err(e) => Err(format!("{e:#}")),
            }
        })
    }

    fn check_binary(&self) -> Pin<Box<dyn Future<Output = Result<bool, String>> + Send>> {
        let socket_path = self.socket_path.clone();
        Box::pin(async move {
            let bin_path = remote_server::setup::remote_server_binary();
            log::info!("Checking for remote server binary at {bin_path}");
            match remote_server::ssh::run_ssh_command(
                &socket_path,
                &remote_server::setup::binary_check_command(),
                remote_server::setup::CHECK_TIMEOUT,
            )
            .await
            {
                // `{binary} --version` exit 0 means exists and is executable.
                // 126/127 means missing or not executable; other non-0 exit treated as real check failure.
                Ok(output) => match output.status.code() {
                    Some(0) => Ok(true),
                    Some(126) | Some(127) => Ok(false),
                    Some(code) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        Err(format!("binary check exited with code {code}: {stderr}"))
                    }
                    None => Err("binary check terminated by signal".into()),
                },
                Err(e) => Err(format!("{e:#}")),
            }
        })
    }

    fn check_has_old_binary(&self) -> Pin<Box<dyn Future<Output = anyhow::Result<bool>> + Send>> {
        let socket_path = self.socket_path.clone();
        Box::pin(async move {
            // Treat the existence of the remote-server install directory
            // itself as evidence of a prior install. If `~/.warp-XX/remote-server`
            // exists, something was installed there before, so any mismatch
            // with the client's expected binary path should be auto-updated
            // rather than surfaced as a first-time install prompt.
            let cmd = format!("test -d {}", remote_server::setup::remote_server_dir());
            let output = remote_server::ssh::run_ssh_command(
                &socket_path,
                &cmd,
                remote_server::setup::CHECK_TIMEOUT,
            )
            .await?;
            // `test -d` exits 0 when present, 1 when missing.
            // Anything else is treated as a check failure.
            match output.status.code() {
                Some(0) => Ok(true),
                Some(1) => Ok(false),
                Some(code) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    Err(anyhow::anyhow!(
                        "remote-server dir check exited with code {code}: {stderr}"
                    ))
                }
                None => Err(anyhow::anyhow!(
                    "remote-server dir check terminated by signal"
                )),
            }
        })
    }

    fn install_binary(&self) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
        let socket_path = self.socket_path.clone();
        Box::pin(async move {
            log::info!(
                "Installing remote server binary to {}",
                remote_server::setup::remote_server_binary()
            );

            // Zaplex fork: DEBUG source build (no release tag) uses dev mode,
            // cross-compiling local `warp` and uploading instead of downloading stale GitHub release.
            // On failure (cross-compile prerequisites missing, etc.), print warning and fall back
            // to download-install, preserving dev experience. Release builds skip this entire block.
            if remote_server::setup::is_dev_source_build() {
                log::info!("dev remote-server: DEBUG source build detected, switching to local cross-compile install");
                match dev_install_local_binary(&socket_path).await {
                    Ok(()) => return Ok(()),
                    Err(error) => {
                        log::warn!(
                            "dev remote-server: local cross-compile install unavailable, falling back to download-install: {error:#}"
                        );
                        // Fall through, continue to regular download-install flow below.
                    }
                }
            }

            match run_install_script(&socket_path, None, remote_server::setup::INSTALL_TIMEOUT)
                .await
            {
                Ok(()) => verify_installed_binary(&socket_path)
                    .await
                    .map_err(|error| format!("{error:#}")),
                Err(error) if should_skip_scp_fallback(&error) => Err(error.to_string()),
                Err(error) => {
                    log::warn!("remote-server install failed, trying SCP fallback: {error}");
                    match scp_install_fallback(&socket_path).await {
                        Ok(()) => Ok(()),
                        Err(fallback_error) => {
                            Err(format!("{error}; SCP fallback failed: {fallback_error:#}"))
                        }
                    }
                }
            }
        })
    }

    fn connect(
        &self,
        executor: Arc<executor::Background>,
    ) -> Pin<Box<dyn Future<Output = Result<Connection>> + Send>> {
        let socket_path = self.socket_path.clone();
        let remote_proxy_command = self.remote_proxy_command();
        Box::pin(async move {
            let mut args = ssh_args(&socket_path);
            args.push(remote_proxy_command);

            // `kill_on_drop(true)` pairs with ownership of the `Child` being
            // returned in the [`Connection`] below: the
            // [`RemoteServerManager`] holds the `Child` on its per-session
            // state, and dropping that state (on explicit teardown or
            // spontaneous disconnect) sends SIGKILL to this ssh process.
            let mut child = command::r#async::Command::new("ssh")
                .args(&args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()?;

            let stdin = child
                .stdin
                .take()
                .ok_or_else(|| anyhow::anyhow!("Failed to capture child stdin"))?;
            let stdout = child
                .stdout
                .take()
                .ok_or_else(|| anyhow::anyhow!("Failed to capture child stdout"))?;
            let stderr = child
                .stderr
                .take()
                .ok_or_else(|| anyhow::anyhow!("Failed to capture child stderr"))?;

            let (client, event_rx) =
                RemoteServerClient::from_child_streams(stdin, stdout, stderr, &executor);
            Ok(Connection {
                client,
                event_rx,
                child,
                control_path: Some(socket_path),
            })
        })
    }

    fn remove_remote_server_binary(
        &self,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>> {
        let socket_path = self.socket_path.clone();
        Box::pin(async move {
            let cmd = format!("rm -f {}", remote_server::setup::remote_server_binary());
            log::info!("Removing stale remote server binary: {cmd}");
            let output = remote_server::ssh::run_ssh_command(
                &socket_path,
                &cmd,
                remote_server::setup::CHECK_TIMEOUT,
            )
            .await?;
            if output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(anyhow::anyhow!("Failed to remove binary: {stderr}"))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use warpui::r#async::BoxFuture;
    fn static_auth_context() -> Arc<RemoteServerAuthContext> {
        Arc::new(RemoteServerAuthContext::new(
            || -> BoxFuture<'static, Option<String>> { Box::pin(async { None }) },
            || "user id/with spaces".to_string(),
        ))
    }

    #[test]
    fn remote_proxy_command_quotes_identity_key() {
        let transport = SshTransport::new(
            PathBuf::from("/tmp/control-master.sock"),
            static_auth_context(),
        );

        let command = transport.remote_proxy_command();

        assert!(command.contains("remote-server-proxy --identity-key"));
        assert!(command.contains("'user id/with spaces'"));
    }
}
