//! Assemble an `ssh ...` command from `SshServerInfo` and spawn a child process to test the connection.
//!
//! When writing to the PTY, `build_ssh_command_line` is called; it shell-escapes every arg
//! to prevent spaces or single quotes in the username / host / key_path from breaking the command line.
//!
//! ## Password authentication security & cross-platform compatibility
//!
//! **Non-Windows**: `ssh` can read the password from stdin normally in pipe stdin mode, using a one-shot
//! stdin injection (`build_password_auth_stdin`). Throughout, the password is held only in memory as a
//! `Zeroizing<String>`, never enters argv, and never appears in `/proc/<pid>/cmdline`,
//! `ps`, or other on-host-readable process info (the fix for sshpass `-p` mode).
//!
//! **Windows**: Win32-OpenSSH, even when stdin is a pipe, refuses to read the password from stdin because of
//! `CREATE_NO_WINDOW` (no console); it prints
//! `GetConsoleMode on STD_INPUT_HANDLE failed` and then hangs, see
//! PowerShell/Win32-OpenSSH issue #1470. The workaround is `SSH_ASKPASS`:
//! write a temporary .cmd script, have ssh spawn it and read its stdout as the password, completely bypassing stdin
//! and the console. `SSH_ASKPASS_REQUIRE=force` forces the askpass path. The password itself
//! is passed to the askpass script via a temporary file (not written to an env var, reducing the leak surface), and its entire lifecycle
//! is guaranteed by the `AskpassSession` RAII guard, which cleans up immediately after ssh exits.

use crate::types::{AuthType, ConnectionStatus, SshServerInfo};
#[cfg(not(windows))]
use futures_lite::io::AsyncWriteExt as _;
use std::borrow::Cow;
use std::process::Stdio;
use std::time::Duration;
use zeroize::Zeroizing;

pub fn build_ssh_args(server: &SshServerInfo) -> Vec<String> {
    let mut args: Vec<String> = vec!["ssh".into()];
    if server.port != 22 {
        args.push("-p".into());
        args.push(server.port.to_string());
    }
    if matches!(server.auth_type, AuthType::Key | AuthType::OneKey) {
        if let Some(path) = server.key_path.as_deref() {
            if !path.is_empty() {
                args.push("-i".into());
                args.push(path.to_string());
            }
        }
    }
    let target = if server.username.is_empty() {
        server.host.clone()
    } else {
        format!("{}@{}", server.username, server.host)
    };
    args.push(target);
    args
}

pub fn build_ssh_command_line(server: &SshServerInfo) -> String {
    let args = build_ssh_args(server);
    args.iter()
        .map(|a| shell_escape::unix::escape(Cow::Borrowed(a.as_str())).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ConnectionTestResult {
    pub status: ConnectionStatus,
    pub latency_ms: Option<u64>,
    pub error_message: Option<String>,
}

pub async fn test_connection(
    server: &SshServerInfo,
    password: Option<Zeroizing<String>>,
) -> ConnectionTestResult {
    let start = instant::Instant::now();

    let result = match server.auth_type {
        AuthType::Key => test_key_auth(server).await,
        AuthType::Password | AuthType::OneKey => test_password_auth(server, password).await,
    };

    let latency = start.elapsed().as_millis() as u64;

    match result {
        Ok(()) => ConnectionTestResult {
            status: ConnectionStatus::Online,
            latency_ms: Some(latency),
            error_message: None,
        },
        Err(e) => ConnectionTestResult {
            status: ConnectionStatus::Offline,
            latency_ms: Some(latency),
            error_message: Some(e),
        },
    }
}

async fn test_key_auth(server: &SshServerInfo) -> Result<(), String> {
    let mut args = build_ssh_args(server);
    // build_ssh_args ends with the destination (user@host); -o options must be inserted
    // before the destination, otherwise SSH treats -o as part of the remote command rather than its own option.
    let target = args.pop().unwrap();
    args.extend([
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=5".into(),
        "-o".into(),
        "StrictHostKeyChecking=no".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
    ]);
    args.push(target);
    args.push("echo ok".into());
    let cmd_args = args;

    match tokio::time::timeout(TEST_TIMEOUT, run_ssh_test(&cmd_args)).await {
        Ok(Ok(output)) => {
            // Strictly match `echo ok`; don't let a banner/motd ending exactly in "ok" cause a false positive.
            if output.trim() == "ok" {
                Ok(())
            } else {
                Err(format!("Unexpected output: {}", output.trim()))
            }
        }
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("Connection timeout".into()),
    }
}

async fn test_password_auth(
    server: &SshServerInfo,
    password: Option<Zeroizing<String>>,
) -> Result<(), String> {
    let password = password.ok_or("Password not provided")?;

    // Build the ssh command args (note: -o options must be inserted before the destination, see that function's comment)
    let cmd_args = build_password_auth_cmd_args(server);

    // Platform branch: Windows uses SSH_ASKPASS, other platforms use stdin injection
    #[cfg(windows)]
    return test_password_auth_windows(cmd_args, &password).await;
    #[cfg(not(windows))]
    test_password_auth_unix(cmd_args, &password).await
}

/// Non-Windows platforms: `ssh` can read the password normally from pipe stdin.
#[cfg(not(windows))]
async fn test_password_auth_unix(
    cmd_args: Vec<String>,
    password: &Zeroizing<String>,
) -> Result<(), String> {
    let stdin_bytes = build_password_auth_stdin(password);

    let mut child = command::r#async::Command::new("ssh")
        .args(&cmd_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to spawn ssh: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&stdin_bytes)
            .await
            .map_err(|e| format!("Failed to write password: {e}"))?;
    }

    let output = match tokio::time::timeout(TEST_TIMEOUT, child.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("Failed to read ssh output: {e}")),
        Err(_) => return Err("Connection timeout".into()),
    };

    finalize_password_test_result(&output)
}

/// Windows platforms: use the SSH_ASKPASS mechanism to hand the password to ssh, completely bypassing stdin/console.
#[cfg(windows)]
async fn test_password_auth_windows(
    cmd_args: Vec<String>,
    password: &Zeroizing<String>,
) -> Result<(), String> {
    let askpass = AskpassSession::new(password).map_err(|e| format!("Failed to prepare askpass: {e}"))?;

    let mut cmd = command::r#async::Command::new("ssh");
    cmd.args(&cmd_args)
        // ssh no longer needs to read the password from stdin; set to null to avoid ssh thinking a tty is present
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    askpass.apply_env(&mut cmd);

    let child = cmd.spawn().map_err(|e| format!("Failed to spawn ssh: {e}"))?;

    // When the timeout fires, child is dropped → kill_on_drop automatically kills ssh.
    // The askpass guard is dropped at the end of the function, cleaning up the temporary files.
    let output = match tokio::time::timeout(TEST_TIMEOUT, child.output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("Failed to read ssh output: {e}")),
        Err(_) => return Err("Connection timeout".into()),
    };
    drop(askpass);

    finalize_password_test_result(&output)
}

/// Parse the ssh child process output, unifying the success/failure decision logic (shared by both platforms).
fn finalize_password_test_result(output: &std::process::Output) -> Result<(), String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr_trimmed = String::from_utf8_lossy(&output.stderr).trim().to_string();

    // Always log ssh's real stderr, leaving a trace even on success, to make it easier to later
    // diagnose discrepancies like "why did the server accept the password but the UI reported success".
    if !stderr_trimmed.is_empty() {
        log::warn!("ssh test stderr: {stderr_trimmed}");
    }

    // Success decision: strictly match the output of `echo ok`. The previous `ends_with("ok")` fallback
    // would falsely report success when a banner / motd happened to end in "ok"; that is removed here.
    if output.status.success() && stdout.trim() == "ok" {
        Ok(())
    } else if stderr_trimmed.contains("Permission denied")
        || stderr_trimmed.contains("Authentication failed")
    {
        // Attach a condensed stderr (<= 200 chars) to the error message, to help the user tell whether the server
        // has password auth disabled, or is configured with kbd-only AuthenticationMethods, etc.
        let detail = if stderr_trimmed.is_empty() {
            String::new()
        } else {
            let snippet: String = stderr_trimmed.chars().take(200).collect();
            if stderr_trimmed.chars().count() > 200 {
                format!(" ({snippet}...)")
            } else {
                format!(" ({snippet})")
            }
        };
        Err(format!("Authentication failed: wrong password{detail}"))
    } else {
        Err(format!(
            "Unexpected output: stdout={} stderr={}",
            stdout.trim(),
            stderr_trimmed
        ))
    }
}

/// Encode the password into the byte stream to write to ssh stdin: password UTF-8 + newline.
/// Kept as a standalone pure function to make it easy for unit tests to assert "stdin contains the password literal + newline".
/// Only the unix branch actually calls it (Windows uses SSH_ASKPASS), but the function itself compiles cross-platform,
/// so the `build_password_auth_stdin_*` unit tests can also run on Windows CI.
// On Windows only tests call this function; the production path uses SSH_ASKPASS, hence the dead_code suppression
#[cfg_attr(windows, allow(dead_code))]
fn build_password_auth_stdin(password: &Zeroizing<String>) -> Zeroizing<Vec<u8>> {
    let mut v = Zeroizing::new(Vec::with_capacity(password.len() + 1));
    v.extend_from_slice(password.as_bytes());
    v.push(b'\n');
    v
}

/// Assemble the full argv passed to the ssh child process during password authentication testing.
///
/// Unlike `build_ssh_args`: here we skip the first item `"ssh"` (we spawn explicitly via
/// `Command::new("ssh")`) and append the test `-o` options and the `echo ok` remote command.
///
/// Meaning of the key options:
/// - `BatchMode=no`: allow ssh to read the password from stdin / askpass (stdin is needed when not using askpass)
/// - `PreferredAuthentications=password`: declare we want to try **only** password, without
///   `keyboard-interactive`. Otherwise, server-side PAM triggers a kbd-interactive fallback after password;
///   the kbd-int sub-prompts get no response, so it retries each one
///   and triggers `pam_faildelay` (~2s each), accumulating ~8-10s and hitting `TEST_TIMEOUT`.
/// - `KbdInteractiveAuthentication=no`: a client capability switch that disables the entire kbd-int
///   protocol outright. `PreferredAuthentications` alone is not enough — it only constrains the prompt count
///   of the password sub-method; kbd-int can still proceed. Setting both switches is defense in depth.
/// - `NumberOfPasswordPrompts=1`: the password sub-method is allowed only 1 retry.
/// - `ConnectTimeout=5`: timeout for a single TCP connection.
/// - `StrictHostKeyChecking=no`: don't block on known_hosts (in test scenarios this avoids false errors from host key
///   changes; real terminal connections take a different path).
/// - `LogLevel=ERROR`: suppress noise like host key prompts / banners.
///
/// `echo ok` is the remote command; success is decided by strictly matching stdout (avoiding the false positive
/// of a banner / motd happening to end in "ok").
///
/// author: logic
/// date: 2026-06-01
fn build_password_auth_cmd_args(server: &SshServerInfo) -> Vec<String> {
    // skip(1) drops "ssh" itself (already specified by Command::new), leaving
    // ["-p","2222","user@host"]. -o options must be inserted before the destination,
    // otherwise SSH treats -o as part of the remote command rather than its own option.
    let mut args: Vec<String> = build_ssh_args(server).into_iter().skip(1).collect();
    let target = args.pop().unwrap();
    args.extend([
        "-o".into(),
        "BatchMode=no".into(),
        "-o".into(),
        "PreferredAuthentications=password".into(),
        "-o".into(),
        "KbdInteractiveAuthentication=no".into(),
        "-o".into(),
        "NumberOfPasswordPrompts=1".into(),
        "-o".into(),
        "ConnectTimeout=5".into(),
        "-o".into(),
        "StrictHostKeyChecking=no".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
    ]);
    args.push(target);
    args.push("echo ok".into());
    args
}

async fn run_ssh_test(args: &[String]) -> Result<String, std::io::Error> {
    // Uniformly spawn the child process via command::r#async, which adds CREATE_NO_WINDOW on Windows,
    // avoiding a flashing console window (see the ban on tokio::process::Command in .clippy.toml).
    let output = command::r#async::Command::new(&args[0])
        .args(&args[1..])
        .output()
        .await?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    // Success decision: process exit code is 0, or the remote `echo ok` output has been returned (some sshpass
    // warnings make the exit code non-zero, but stdout still contains "ok").
    if output.status.success() || stdout.contains("ok") {
        Ok(stdout)
    } else {
        Err(std::io::Error::other(stderr))
    }
}

/// Windows-specific askpass session: creates a password file and askpass helper script
/// in a temporary directory, exposes them to `ssh` via the `SSH_ASKPASS` environment
/// variable, and automatically cleans up both files on drop.
///
/// `ssh.exe` on Windows refuses to read the password from stdin even when stdin is a pipe
/// because there is no console (prints `GetConsoleMode on STD_INPUT_HANDLE failed` and then
/// hangs). See PowerShell/Win32-OpenSSH issue #1470. The workaround is `SSH_ASKPASS`:
/// when ssh sees this environment variable, it spawns the specified program and treats its
/// stdout as the password, completely bypassing stdin and the console. `SSH_ASKPASS_REQUIRE=force`
/// forces ssh to use the askpass path even if it detects a TTY.
///
/// The password is passed to the askpass script via a temporary file (not via an env var,
/// to reduce the leak surface): env vars are visible to the `ssh` child process and all its
/// descendants. The askpass process lifetime is very short (ssh forks then immediately execs,
/// reads the password and exits), keeping the disk exposure window to milliseconds.
///
/// **Security trade-off**: the two temporary files do not set `FILE_ATTRIBUTE_HIDDEN` and
/// do not modify ACLs; they rely on Windows `%TEMP%` default isolation
/// (`C:\Users\<user>\AppData\Local\Temp`, per-user). Earlier versions tried setting
/// the hidden attribute and tightening ACLs to `(R)`, but `FILE_ATTRIBUTE_HIDDEN` causes
/// `posix_spawnp` to return `ERROR_ACCESS_DENIED` (error 5) during the `CreateProcessW` phase,
/// preventing askpass from starting at all and incorrectly sending the password to the server's
/// password prompt (users see "wrong password" but no credentials actually left the machine).
/// The per-user isolation of Windows temp dir is sufficient; here simplicity and reliability
/// take precedence over "defense in depth".
///
/// author: logic
/// date: 2026-06-01
#[cfg(windows)]
struct AskpassSession {
    password_path: std::path::PathBuf,
    script_path: std::path::PathBuf,
}

#[cfg(windows)]
impl AskpassSession {
    fn new(password: &Zeroizing<String>) -> std::io::Result<Self> {
        use std::io::Write as _;

        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let suffix = format!("{pid}-{nanos}");

        let password_path = dir.join(format!("warp-ssh-askpass-{suffix}.txt"));
        let script_path = dir.join(format!("warp-ssh-askpass-{suffix}.cmd"));

        // Write password to temporary file (no hidden attribute, no ACL changes; see
        // type doc for security trade-off).
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&password_path)?;
            f.write_all(password.as_bytes())?;
            f.sync_all()?;
        }

        // Write askpass helper script: read the first line from the file pointed to by
        // %ZAPLEX_SSH_ASKPASS_FILE% and echo it to stdout. `set /p` reads the first line
        // (stripping newline), `echo !PW!` outputs it. Use `setlocal enabledelayedexpansion`
        // + `!PW!` for delayed expansion to avoid password containing cmd special characters
        // (&, |, <, >, ^) being truncated by immediate expansion of %PW%.
        let body = "@echo off\r\nsetlocal enabledelayedexpansion\r\nset /p PW=<\"%ZAPLEX_SSH_ASKPASS_FILE%\"\r\necho !PW!\r\nendlocal\r\n";
        {
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&script_path)?;
            f.write_all(body.as_bytes())?;
            f.sync_all()?;
        }

        Ok(Self {
            password_path,
            script_path,
        })
    }

    /// Attach the SSH_ASKPASS environment variables to the ssh child process.
    fn apply_env(&self, cmd: &mut command::r#async::Command) {
        cmd.env("SSH_ASKPASS", &self.script_path)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("ZAPLEX_SSH_ASKPASS_FILE", &self.password_path)
            .env_remove("DISPLAY");
    }
}

#[cfg(windows)]
impl Drop for AskpassSession {
    fn drop(&mut self) {
        // Immediately delete both temporary files after ssh exits, minimizing the time
        // the password spends on disk. Errors are silently ignored: cleanup failures
        // should not affect the main flow return value.
        let _ = std::fs::remove_file(&self.password_path);
        let _ = std::fs::remove_file(&self.script_path);
    }
}

#[cfg(test)]
#[path = "ssh_command_tests.rs"]
mod tests;
