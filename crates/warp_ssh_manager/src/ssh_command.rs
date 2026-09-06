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
use std::borrow::Cow;
use std::process::Stdio;
use std::time::Duration;
use zeroize::Zeroizing;

pub trait WorkspaceCommandFactory: Send + Sync {
    fn async_command(&self, program: &str) -> command::r#async::Command;
    fn blocking_command(&self, program: &str) -> command::blocking::Command;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultWorkspaceCommandFactory;

impl WorkspaceCommandFactory for DefaultWorkspaceCommandFactory {
    fn async_command(&self, program: &str) -> command::r#async::Command {
        command::r#async::Command::new(program)
    }

    fn blocking_command(&self, program: &str) -> command::blocking::Command {
        command::blocking::Command::new(program)
    }
}

pub fn build_ssh_args(server: &SshServerInfo) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "ssh".into(),
        "-o".into(),
        "StrictHostKeyChecking=ask".into(),
    ];
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
    args.push("--".into());
    args.push(target);
    args
}

/// The only multiplexer attach modes Zaplex may issue over classic SSH.
///
/// Screen's attached and detached modes are intentionally separate: `-x`
/// preserves an existing display, while `-r` resumes a detached session. The
/// destructive `-d`/`-D` and session-creating `-R`/`-RR` variants are not
/// representable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MultiplexerAttachMode {
    Tmux,
    ScreenAttached,
    ScreenDetached,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InvalidMultiplexerTarget;

impl std::fmt::Display for InvalidMultiplexerTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "multiplexer target must be non-empty, must not start with '-', and must contain no control characters",
        )
    }
}

impl std::error::Error for InvalidMultiplexerTarget {}

fn build_multiplexer_ssh_args(
    server: &SshServerInfo,
    mode: MultiplexerAttachMode,
    target: &str,
) -> Result<Vec<String>, InvalidMultiplexerTarget> {
    if target.is_empty() || target.starts_with('-') || target.chars().any(char::is_control) {
        return Err(InvalidMultiplexerTarget);
    }

    let (program, attach_flag) = match mode {
        MultiplexerAttachMode::Tmux => ("tmux", "attach-session"),
        MultiplexerAttachMode::ScreenAttached => ("screen", "-x"),
        MultiplexerAttachMode::ScreenDetached => ("screen", "-r"),
    };
    let remote_argv = [
        "env",
        "ZAPLEX_SESSION=1",
        "BYOBU_DISABLE=1",
        "LC_BYOBU=0",
        program,
        attach_flag,
        if mode == MultiplexerAttachMode::Tmux {
            "-t"
        } else {
            target
        },
        target,
    ];
    let remote_argv = if mode == MultiplexerAttachMode::Tmux {
        remote_argv.as_slice()
    } else {
        &remote_argv[..remote_argv.len() - 1]
    };
    let remote_command = remote_argv
        .iter()
        .map(|arg| shell_escape::unix::escape(Cow::Borrowed(*arg)).to_string())
        .collect::<Vec<_>>()
        .join(" ");

    let mut args = build_ssh_args(server);
    let destination_delimiter = args
        .iter()
        .position(|arg| arg == "--")
        .expect("SSH args always contain a destination delimiter");
    args.splice(
        destination_delimiter..destination_delimiter,
        [
            "-o".to_string(),
            "RequestTTY=force".to_string(),
            "-o".to_string(),
            "SetEnv=ZAPLEX_SESSION=1 BYOBU_DISABLE=1 LC_BYOBU=0".to_string(),
        ],
    );
    args.push(remote_command);
    Ok(args)
}

/// Build a classic SSH command that attaches one exact existing multiplexer
/// session. The target is shell-quoted once for the remote login shell, then the
/// complete SSH argv is independently quoted for Zaplex's local shell.
pub fn build_multiplexer_ssh_command_line(
    server: &SshServerInfo,
    mode: MultiplexerAttachMode,
    target: &str,
) -> Result<String, InvalidMultiplexerTarget> {
    Ok(build_multiplexer_ssh_args(server, mode, target)?
        .iter()
        .map(|arg| shell_escape::unix::escape(Cow::Borrowed(arg.as_str())).to_string())
        .collect::<Vec<_>>()
        .join(" "))
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
    pub unknown_host_key: Option<UnknownHostKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownHostKey {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostKeyPreflight {
    Verified,
    ConfirmationRequired(UnknownHostKey),
    Changed,
}

pub async fn preflight_host_key(server: &SshServerInfo) -> Result<HostKeyPreflight, String> {
    preflight_host_key_with_factory(server, &DefaultWorkspaceCommandFactory).await
}

pub async fn preflight_host_key_with_factory(
    server: &SshServerInfo,
    command_factory: &dyn WorkspaceCommandFactory,
) -> Result<HostKeyPreflight, String> {
    match probe_host_key(server, command_factory).await {
        Ok(HostKeyProbeOutcome::Verified) => Ok(HostKeyPreflight::Verified),
        Ok(HostKeyProbeOutcome::Unknown(host_key)) => {
            Ok(HostKeyPreflight::ConfirmationRequired(host_key))
        }
        Err(error) if error == "SSH host key changed; connection blocked" => {
            Ok(HostKeyPreflight::Changed)
        }
        Err(error) => Err(error),
    }
}

pub async fn test_connection(
    server: &SshServerInfo,
    password: Option<Zeroizing<String>>,
) -> ConnectionTestResult {
    test_connection_with_factory_policy(server, password, None, &DefaultWorkspaceCommandFactory)
        .await
}

pub async fn test_connection_confirm_host_key(
    server: &SshServerInfo,
    password: Option<Zeroizing<String>>,
    expected_host_key: &UnknownHostKey,
) -> ConnectionTestResult {
    test_connection_with_factory_policy(
        server,
        password,
        Some(expected_host_key),
        &DefaultWorkspaceCommandFactory,
    )
    .await
}

pub async fn test_connection_with_factory(
    server: &SshServerInfo,
    password: Option<Zeroizing<String>>,
    command_factory: &dyn WorkspaceCommandFactory,
) -> ConnectionTestResult {
    test_connection_with_factory_policy(server, password, None, command_factory).await
}

async fn test_connection_with_factory_policy(
    server: &SshServerInfo,
    password: Option<Zeroizing<String>>,
    expected_host_key: Option<&UnknownHostKey>,
    command_factory: &dyn WorkspaceCommandFactory,
) -> ConnectionTestResult {
    let start = instant::Instant::now();
    let pinned_host_key = if let Some(expected_host_key) = expected_host_key {
        match pin_confirmed_host_key(server, expected_host_key, command_factory).await {
            Ok(pinned_host_key) => Some(pinned_host_key),
            Err(error) => {
                return ConnectionTestResult {
                    status: ConnectionStatus::Offline,
                    latency_ms: Some(start.elapsed().as_millis() as u64),
                    error_message: Some(error),
                    unknown_host_key: None,
                };
            }
        }
    } else {
        match probe_host_key(server, command_factory).await {
            Ok(HostKeyProbeOutcome::Verified) => {}
            Ok(HostKeyProbeOutcome::Unknown(unknown_host_key)) => {
                return ConnectionTestResult {
                    status: ConnectionStatus::Unknown,
                    latency_ms: Some(start.elapsed().as_millis() as u64),
                    error_message: None,
                    unknown_host_key: Some(unknown_host_key),
                };
            }
            Err(error) => {
                return ConnectionTestResult {
                    status: ConnectionStatus::Offline,
                    latency_ms: Some(start.elapsed().as_millis() as u64),
                    error_message: Some(error),
                    unknown_host_key: None,
                };
            }
        }
        None
    };

    let result = match server.auth_type {
        // Key auth also carries a secret: the *passphrase* of an encrypted
        // private key (resolved by the caller into the same `password` slot). It
        // must reach ssh, or testing an encrypted key always fails.
        AuthType::Key => {
            test_key_auth(server, password, pinned_host_key.as_ref(), command_factory).await
        }
        AuthType::Password | AuthType::OneKey => {
            test_password_auth(server, password, pinned_host_key.as_ref(), command_factory).await
        }
    };

    let latency = start.elapsed().as_millis() as u64;

    match result {
        Ok(()) => ConnectionTestResult {
            status: ConnectionStatus::Online,
            latency_ms: Some(latency),
            error_message: None,
            unknown_host_key: None,
        },
        Err(e) => ConnectionTestResult {
            status: ConnectionStatus::Offline,
            latency_ms: Some(latency),
            error_message: Some(e),
            unknown_host_key: None,
        },
    }
}

#[derive(Debug)]
enum HostKeyProbeOutcome {
    Verified,
    Unknown(UnknownHostKey),
}

async fn probe_host_key(
    server: &SshServerInfo,
    command_factory: &dyn WorkspaceCommandFactory,
) -> Result<HostKeyProbeOutcome, String> {
    let args = build_host_key_probe_args(server);
    let output = tokio::time::timeout(
        TEST_TIMEOUT,
        run_ssh_test_capture(&args, None, command_factory),
    )
    .await
    .map_err(|_| "Connection timeout".to_string())??;
    match classify_host_key_probe(server, &output)? {
        HostKeyProbeOutcome::Verified => Ok(HostKeyProbeOutcome::Verified),
        HostKeyProbeOutcome::Unknown(_) => {
            let (pinned_host_key, fingerprint) =
                capture_host_key(server, "yes", command_factory).await?;
            let key = std::fs::read_to_string(&pinned_host_key.path)
                .map_err(|error| format!("Failed to read captured SSH host key: {error}"))?
                .trim()
                .to_string();
            Ok(HostKeyProbeOutcome::Unknown(UnknownHostKey {
                host: server.host.clone(),
                port: server.port,
                fingerprint,
                key,
            }))
        }
    }
}

fn build_host_key_probe_args(server: &SshServerInfo) -> Vec<String> {
    let mut args: Vec<String> = build_ssh_args(server).into_iter().skip(1).collect();
    if let Some(strict_host_key_checking) = args
        .iter_mut()
        .find(|arg| arg.starts_with("StrictHostKeyChecking="))
    {
        *strict_host_key_checking = "StrictHostKeyChecking=yes".to_string();
    }
    let target = args.pop().expect("SSH args always contain a destination");
    assert_eq!(args.pop().as_deref(), Some("--"));
    args.extend([
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "PreferredAuthentications=none".into(),
        "-o".into(),
        "ConnectTimeout=5".into(),
        "-o".into(),
        "LogLevel=VERBOSE".into(),
    ]);
    args.push("--".into());
    args.push(target);
    args.push("true".into());
    args
}

fn classify_host_key_probe(
    server: &SshServerInfo,
    output: &std::process::Output,
) -> Result<HostKeyProbeOutcome, String> {
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("REMOTE HOST IDENTIFICATION HAS CHANGED") {
        return Err("SSH host key changed; connection blocked".to_string());
    }
    if stderr.contains("Host key verification failed")
        || stderr.contains("authenticity of host")
        || stderr.contains("can't be established")
    {
        let fingerprint = regex::Regex::new(r"(?i)fingerprint is ([^\s.]+)")
            .expect("valid host-key fingerprint regex")
            .captures(&stderr)
            .and_then(|captures| captures.get(1))
            .map(|fingerprint| fingerprint.as_str().to_string())
            .unwrap_or_else(|| "unavailable".to_string());
        return Ok(HostKeyProbeOutcome::Unknown(UnknownHostKey {
            host: server.host.clone(),
            port: server.port,
            fingerprint,
            key: String::new(),
        }));
    }
    Ok(HostKeyProbeOutcome::Verified)
}

#[cfg(unix)]
pub fn persist_confirmed_host_key(
    server: &SshServerInfo,
    expected: &UnknownHostKey,
    destination: &std::path::Path,
) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let endpoint = crate::validation::validate_ssh_endpoint(
        crate::validation::EndpointUse::Connect,
        &server.host,
        &server.port.to_string(),
    )
    .map_err(|error| format!("Invalid SSH endpoint for host-key confirmation: {error}"))?;
    if expected.host != endpoint.host || expected.port != endpoint.port {
        return Err("SSH endpoint changed before host-key confirmation".to_string());
    }
    if expected.key.is_empty() || expected.key.lines().count() != 1 {
        return Err("Captured SSH host key is invalid; connection blocked".to_string());
    }

    let parent = destination
        .parent()
        .ok_or_else(|| "Managed SSH known-hosts path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("Failed to prepare managed SSH host-key directory: {error}"))?;
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Managed SSH known-hosts path has no valid file name".to_string())?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temporary = parent.join(format!(".{file_name}.{}-{nonce}.tmp", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("Failed to create managed SSH host-key file: {error}"))?;
    let write_result = file
        .write_all(expected.key.as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all());
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!(
            "Failed to write managed SSH host-key file: {error}"
        ));
    }
    drop(file);
    if let Err(error) = std::fs::rename(&temporary, destination) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!(
            "Failed to commit managed SSH host-key file: {error}"
        ));
    }
    Ok(())
}

#[cfg(test)]
fn host_key_fingerprint_matches(expected: &str, ssh_keygen_output: &[u8]) -> bool {
    String::from_utf8_lossy(ssh_keygen_output)
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .any(|fingerprint| fingerprint == expected)
}

async fn pin_confirmed_host_key(
    server: &SshServerInfo,
    expected: &UnknownHostKey,
    command_factory: &dyn WorkspaceCommandFactory,
) -> Result<KnownHostsSession, String> {
    if expected.host != server.host || expected.port != server.port {
        return Err("SSH endpoint changed before host-key confirmation".to_string());
    }

    let (pinned_host_key, fingerprint) =
        capture_host_key(server, &expected.fingerprint, command_factory).await?;
    if fingerprint == expected.fingerprint {
        return Ok(pinned_host_key);
    }

    Err("SSH host key changed before confirmation; connection blocked".to_string())
}

async fn capture_host_key(
    server: &SshServerInfo,
    response: &str,
    command_factory: &dyn WorkspaceCommandFactory,
) -> Result<(KnownHostsSession, String), String> {
    let pinned_host_key = KnownHostsSession::new("")
        .map_err(|error| format!("Failed to prepare SSH host-key pin: {error}"))?;
    let response = Zeroizing::new(response.to_string());
    let askpass = AskpassSession::new(&response)
        .map_err(|error| format!("Failed to prepare host-key response: {error}"))?;
    let mut args = build_host_key_probe_args(server);
    if let Some(strict_host_key_checking) = args
        .iter_mut()
        .find(|arg| arg.starts_with("StrictHostKeyChecking="))
    {
        *strict_host_key_checking = "StrictHostKeyChecking=ask".to_string();
    }
    if let Some(batch_mode) = args.iter_mut().find(|arg| arg.starts_with("BatchMode=")) {
        *batch_mode = "BatchMode=no".to_string();
    }
    apply_host_key_file(&mut args, &pinned_host_key, false);
    let output = tokio::time::timeout(
        TEST_TIMEOUT,
        run_ssh_test_capture(&args, Some(&askpass), command_factory),
    )
    .await
    .map_err(|_| "SSH host-key capture timed out".to_string())??;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("REMOTE HOST IDENTIFICATION HAS CHANGED") {
        return Err("SSH host key changed; connection blocked".to_string());
    }

    let fingerprint_args = vec![
        "-lf".to_string(),
        pinned_host_key.path.display().to_string(),
        "-E".to_string(),
        "sha256".to_string(),
    ];
    let fingerprint_output = tokio::time::timeout(
        TEST_TIMEOUT,
        run_command_capture("ssh-keygen", &fingerprint_args, command_factory),
    )
    .await
    .map_err(|_| "SSH host-key fingerprint timed out".to_string())??;
    let fingerprint = String::from_utf8_lossy(&fingerprint_output.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .next()
        .filter(|fingerprint| fingerprint.starts_with("SHA256:"))
        .map(str::to_string)
        .ok_or_else(|| "SSH host key was not captured; connection blocked".to_string())?;
    Ok((pinned_host_key, fingerprint))
}

async fn run_command_capture(
    program: &str,
    args: &[String],
    command_factory: &dyn WorkspaceCommandFactory,
) -> Result<std::process::Output, String> {
    let mut command = command_factory.async_command(program);
    let child = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("Failed to spawn {program}: {error}"))?;
    child
        .output()
        .await
        .map_err(|error| format!("Failed to read {program} output: {error}"))
}

fn apply_host_key_file(
    args: &mut Vec<String>,
    pinned_host_key: &KnownHostsSession,
    require_exact_match: bool,
) {
    if require_exact_match {
        if let Some(setting) = args
            .iter_mut()
            .find(|arg| arg.starts_with("StrictHostKeyChecking="))
        {
            *setting = "StrictHostKeyChecking=yes".to_string();
        }
    }
    let delimiter_index = args
        .iter()
        .position(|arg| arg == "--")
        .expect("SSH test args delimit the destination");
    args.splice(
        delimiter_index..delimiter_index,
        [
            "-o".to_string(),
            format!("UserKnownHostsFile={}", pinned_host_key.path.display()),
        ],
    );
}

async fn test_key_auth(
    server: &SshServerInfo,
    passphrase: Option<Zeroizing<String>>,
    pinned_host_key: Option<&KnownHostsSession>,
    command_factory: &dyn WorkspaceCommandFactory,
) -> Result<(), String> {
    // A non-empty passphrase means the private key is encrypted. ssh reads a key
    // passphrase the same way it reads a login password — from the controlling
    // tty or SSH_ASKPASS, never from stdin — so we hand it over through the same
    // askpass helper as password auth, and drop `BatchMode` (which would suppress
    // the passphrase prompt entirely). With no passphrase we keep `BatchMode=yes`:
    // the key must be usable non-interactively (ssh-agent or unencrypted) and no
    // secret is needed. Same auth double-path the file manager had — the test path
    // now pulls even.
    let passphrase = passphrase.filter(|secret| !secret.is_empty());
    let mut cmd_args = build_key_auth_cmd_args(server, passphrase.is_some());
    if let Some(pinned_host_key) = pinned_host_key {
        apply_host_key_file(&mut cmd_args, pinned_host_key, true);
    }

    let askpass = match &passphrase {
        Some(secret) => Some(
            AskpassSession::new(secret).map_err(|e| format!("Failed to prepare askpass: {e}"))?,
        ),
        None => None,
    };

    let output = match tokio::time::timeout(
        TEST_TIMEOUT,
        run_ssh_test_capture(&cmd_args, askpass.as_ref(), command_factory),
    )
    .await
    {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("Connection timeout".into()),
    };
    drop(askpass);

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !stderr.is_empty() {
        log::warn!("ssh key-auth test stderr: {stderr}");
    }
    // Strictly match `echo ok`; don't let a banner/motd ending in "ok" pass.
    if output.status.success() && stdout.trim() == "ok" {
        Ok(())
    } else if !stderr.is_empty() {
        let snippet: String = stderr.chars().take(200).collect();
        Err(format!("Key authentication failed ({snippet})"))
    } else {
        Err(format!("Unexpected output: {}", stdout.trim()))
    }
}

async fn test_password_auth(
    server: &SshServerInfo,
    password: Option<Zeroizing<String>>,
    pinned_host_key: Option<&KnownHostsSession>,
    command_factory: &dyn WorkspaceCommandFactory,
) -> Result<(), String> {
    let password = password.ok_or("Password not provided")?;

    // Build the ssh command args (note: -o options must be inserted before the destination, see that function's comment)
    let mut cmd_args = build_password_auth_cmd_args(server);
    if let Some(pinned_host_key) = pinned_host_key {
        apply_host_key_file(&mut cmd_args, pinned_host_key, true);
    }

    // ssh never reads the password from the pipe stdin for the `password` auth
    // method — it uses the controlling tty or SSH_ASKPASS. A GUI app launched from
    // Finder/Dock has no controlling tty, so ssh can't read the prompt at all and
    // the piped password is ignored; SSH_ASKPASS (OpenSSH >= 8.4, forced via
    // SSH_ASKPASS_REQUIRE) is the only channel that works on *every* platform. The
    // secret is handed over out-of-band through a temp file, never on argv or
    // stdin. (This replaces the old stdin injection, which failed that way.)
    let askpass =
        AskpassSession::new(&password).map_err(|e| format!("Failed to prepare askpass: {e}"))?;

    let output = match tokio::time::timeout(
        TEST_TIMEOUT,
        run_ssh_test_capture(&cmd_args, Some(&askpass), command_factory),
    )
    .await
    {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err("Connection timeout".into()),
    };
    drop(askpass);

    finalize_password_test_result(&output)
}

/// Spawn `ssh <cmd_args>` for a connection test and capture its completed output.
/// `cmd_args` never includes the leading `"ssh"` (spawned explicitly here). When an
/// [`AskpassSession`] is supplied, ssh obtains the password / key passphrase from it
/// via `SSH_ASKPASS`, never from stdin or argv; stdin is `/dev/null` either way so
/// ssh doesn't hang believing a tty is present. On timeout the caller drops this
/// future and `kill_on_drop` kills ssh.
async fn run_ssh_test_capture(
    cmd_args: &[String],
    askpass: Option<&AskpassSession>,
    command_factory: &dyn WorkspaceCommandFactory,
) -> Result<std::process::Output, String> {
    let mut cmd = command_factory.async_command("ssh");
    cmd.args(cmd_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    if let Some(askpass) = askpass {
        askpass.apply_env(&mut cmd);
    }
    let child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn ssh: {e}"))?;
    child
        .output()
        .await
        .map_err(|e| format!("Failed to read ssh output: {e}"))
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
    if stderr_trimmed.contains("REMOTE HOST IDENTIFICATION HAS CHANGED") {
        Err("SSH host key changed; connection blocked".to_string())
    } else if output.status.success() && stdout.trim() == "ok" {
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
/// - `StrictHostKeyChecking=ask`: unknown keys require explicit confirmation and changed keys are blocked.
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
    assert_eq!(args.pop().as_deref(), Some("--"));
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
        "LogLevel=ERROR".into(),
    ]);
    args.push("--".into());
    args.push(target);
    args.push("echo ok".into());
    args
}

/// Assemble the full argv (minus the leading `"ssh"`, spawned explicitly) for a
/// key-auth connection test. `has_passphrase` picks the mode:
///
/// - `false` (ssh-agent / unencrypted key): `BatchMode=yes` — no prompt at all;
///   the key must be usable non-interactively. This is the original behavior.
/// - `true` (encrypted key, passphrase supplied): `BatchMode=no` so ssh will call
///   `SSH_ASKPASS` for the key passphrase, plus `PreferredAuthentications=publickey`
///   so a wrong passphrase can't fall through to an interactive password prompt
///   that would hang the test. The passphrase is delivered out-of-band via
///   [`AskpassSession`], never on argv.
///
/// `-o` options precede the destination (else ssh treats them as the remote
/// command); the destination precedes `echo ok`, whose exact stdout decides success.
fn build_key_auth_cmd_args(server: &SshServerInfo, has_passphrase: bool) -> Vec<String> {
    let mut args: Vec<String> = build_ssh_args(server).into_iter().skip(1).collect();
    let target = args.pop().unwrap();
    assert_eq!(args.pop().as_deref(), Some("--"));
    if has_passphrase {
        args.extend([
            "-o".into(),
            "BatchMode=no".into(),
            "-o".into(),
            "PreferredAuthentications=publickey".into(),
            // A wrong key passphrase must never fall through to the server's
            // password prompt — that risks account lockout, or a false "success"
            // if the passphrase happens to equal the login password. Disabling
            // both interactive server methods outright is belt-and-suspenders on
            // top of the publickey-only preference.
            "-o".into(),
            "PasswordAuthentication=no".into(),
            "-o".into(),
            "KbdInteractiveAuthentication=no".into(),
        ]);
    } else {
        args.extend(["-o".into(), "BatchMode=yes".into()]);
    }
    args.extend([
        "-o".into(),
        "ConnectTimeout=5".into(),
        "-o".into(),
        "LogLevel=ERROR".into(),
    ]);
    args.push("--".into());
    args.push(target);
    args.push("echo ok".into());
    args
}

struct KnownHostsSession {
    path: std::path::PathBuf,
}

impl KnownHostsSession {
    fn new(host_key: &str) -> std::io::Result<Self> {
        use std::io::Write as _;
        #[cfg(unix)]
        use std::os::unix::fs::OpenOptionsExt as _;

        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let path = dir.join(format!("zaplex-ssh-known-host-{pid}-{nanos}"));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&path)?;
        if let Err(error) = file
            .write_all(host_key.as_bytes())
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all())
        {
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
        drop(file);
        Ok(Self { path })
    }
}

impl Drop for KnownHostsSession {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A cross-platform askpass session: writes the secret (login password or the
/// passphrase of an encrypted key) to a temporary file plus a tiny helper script
/// that echoes it, exposes both to `ssh` via `SSH_ASKPASS`, and removes them on
/// drop.
///
/// Why askpass on *every* platform, not just Windows: `ssh` never reads the
/// password/passphrase from the pipe stdin for its interactive prompts — it uses
/// the controlling tty or `SSH_ASKPASS`. A GUI app launched from Finder/Dock has
/// no controlling tty, so stdin injection fails on macOS (ssh can't read the
/// prompt), and on Windows Win32-OpenSSH prints `GetConsoleMode on
/// STD_INPUT_HANDLE failed` and hangs (Win32-OpenSSH #1470).
/// `SSH_ASKPASS_REQUIRE=force` (OpenSSH >= 8.4) forces the askpass path even when
/// a tty is present.
///
/// The secret travels through a temp file, not an env var, to keep it out of the
/// child's environment (visible to `ssh` and every descendant). The askpass
/// process lifetime is milliseconds (ssh forks, execs it, reads, exits), and the
/// [`Drop`] impl deletes both files immediately after ssh finishes.
///
/// **Security trade-off**: on Windows the temp files rely on per-user `%TEMP%`
/// isolation — they intentionally do *not* set `FILE_ATTRIBUTE_HIDDEN` or tighten
/// ACLs, because the hidden attribute made `posix_spawnp` return
/// `ERROR_ACCESS_DENIED` (error 5) and askpass never ran. On Unix the secret file
/// is created mode `0600` and the helper script `0700` (owner-only) in `TMPDIR`.
///
/// author: logic (Windows path)
/// date: 2026-06-01
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

        // Create + write each file, cleaning up on failure — but only files THIS
        // call created. `Drop` can't run yet (Self isn't built), so a bare `?`
        // after a successful `create_new` would leak a partial plaintext password;
        // yet blindly removing both paths would delete a *colliding* file another
        // session owns. `create_new` failing means we did not create it.
        //
        // Password file: no hidden attribute / ACL changes (see the type doc for
        // the security trade-off). Askpass helper: `set /p PW=<file` reads the
        // first line, `echo !PW!` outputs it; `setlocal enabledelayedexpansion` +
        // `!PW!` avoids truncation on cmd metacharacters (& | < > ^).
        let body = "@echo off\r\nsetlocal enabledelayedexpansion\r\nset /p PW=<\"%ZAPLEX_SSH_ASKPASS_FILE%\"\r\necho !PW!\r\nendlocal\r\n";

        let mut pw = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&password_path)?;
        if let Err(e) = pw
            .write_all(password.as_bytes())
            .and_then(|()| pw.sync_all())
        {
            let _ = std::fs::remove_file(&password_path);
            return Err(e);
        }

        let mut script = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&script_path)
        {
            Ok(script) => script,
            Err(e) => {
                let _ = std::fs::remove_file(&password_path);
                return Err(e);
            }
        };
        if let Err(e) = script
            .write_all(body.as_bytes())
            .and_then(|()| script.sync_all())
        {
            let _ = std::fs::remove_file(&password_path);
            let _ = std::fs::remove_file(&script_path);
            return Err(e);
        }
        drop(pw);
        drop(script);

        Ok(Self {
            password_path,
            script_path,
        })
    }
}

#[cfg(not(windows))]
impl AskpassSession {
    fn new(password: &Zeroizing<String>) -> std::io::Result<Self> {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;

        let dir = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let suffix = format!("{pid}-{nanos}");

        let password_path = dir.join(format!("warp-ssh-askpass-{suffix}.secret"));
        let script_path = dir.join(format!("warp-ssh-askpass-{suffix}.sh"));

        // Create + write each file, cleaning up on failure — but only files THIS
        // call created. `Drop` can't run yet (Self isn't built), so a bare `?`
        // after a successful `create_new` would leak a partial plaintext secret;
        // yet blindly removing both paths would delete a *colliding* file another
        // session owns. `create_new` failing means we did not create it, so we
        // leave it alone.
        //
        // Secret file: owner-only (0600), exact bytes with no trailing newline.
        // Askpass helper: ssh execs it and reads its stdout as the secret; it cats
        // the secret file verbatim. OpenSSH takes the askpass output up to the
        // first `\r`/`\n`, so this delivers the secret exactly for any single-line
        // secret — all the UI's single-line field can produce. Owner-only exec
        // (0700); `$ZAPLEX_SSH_ASKPASS_FILE` is our own absolute temp path.
        let body = "#!/bin/sh\ncat -- \"$ZAPLEX_SSH_ASKPASS_FILE\"\n";

        let mut secret = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&password_path)?;
        if let Err(e) = secret
            .write_all(password.as_bytes())
            .and_then(|()| secret.sync_all())
        {
            let _ = std::fs::remove_file(&password_path);
            return Err(e);
        }

        let mut script = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&script_path)
        {
            Ok(script) => script,
            Err(e) => {
                let _ = std::fs::remove_file(&password_path);
                return Err(e);
            }
        };
        if let Err(e) = script
            .write_all(body.as_bytes())
            .and_then(|()| script.sync_all())
        {
            let _ = std::fs::remove_file(&password_path);
            let _ = std::fs::remove_file(&script_path);
            return Err(e);
        }
        drop(secret);
        drop(script);

        Ok(Self {
            password_path,
            script_path,
        })
    }
}

impl AskpassSession {
    /// Attach the SSH_ASKPASS environment variables to the ssh child process.
    fn apply_env(&self, cmd: &mut command::r#async::Command) {
        cmd.env("SSH_ASKPASS", &self.script_path)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env("ZAPLEX_SSH_ASKPASS_FILE", &self.password_path)
            .env_remove("DISPLAY");
    }
}

impl Drop for AskpassSession {
    fn drop(&mut self) {
        // Immediately delete both temporary files after ssh exits, minimizing the time
        // the secret spends on disk. Errors are silently ignored: cleanup failures
        // should not affect the main flow return value.
        let _ = std::fs::remove_file(&self.password_path);
        let _ = std::fs::remove_file(&self.script_path);
    }
}

#[cfg(test)]
#[path = "ssh_command_tests.rs"]
mod tests;
