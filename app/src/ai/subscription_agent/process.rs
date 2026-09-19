use super::{
    InstallationIdentity, SubscriptionAgent, SubscriptionTarget, CLAUDE_PROVIDER_MANAGED_BY_HOST,
    CLAUDE_SUBSCRIPTION_PROVIDER_ENVIRONMENT_VARIABLES,
};
use anyhow::{anyhow, bail, Context, Result};
use async_process::{Child, ChildStderr, ChildStdin, ChildStdout};
use command::r#async::Command;
use futures_lite::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use serde_json::Value;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;
use warpui::r#async::FutureExt as _;

const PROCESS_GRACEFUL_EXIT_TIMEOUT: Duration = Duration::from_secs(2);
const PROCESS_FORCED_EXIT_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_PROBE_OUTPUT_BYTES: usize = 64 * 1024;
const MAX_DIAGNOSTIC_BYTES: usize = 8 * 1024;
const STDERR_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProcessTermination {
    Graceful,
    Forced,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessLocation {
    Local,
    /// Prebuilt OpenSSH argv including the program at index zero and the destination.
    Remote {
        ssh_argv: Vec<String>,
        /// The PATH observed while resolving the CLI in the remote login shell.
        environment_path: Option<String>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProcessLaunch {
    pub(crate) program: PathBuf,
    pub(crate) args: Vec<String>,
    pub(crate) environment: Vec<(&'static str, String)>,
    pub(crate) unset_environment: Vec<&'static str>,
    pub(crate) working_directory: PathBuf,
    pub(crate) location: ProcessLocation,
}

impl ProcessLaunch {
    pub(crate) fn for_discovery(
        installation: &InstallationIdentity,
        working_directory: PathBuf,
        location: ProcessLocation,
    ) -> Self {
        let mut launch = Self {
            program: installation.executable.clone(),
            args: Vec::new(),
            environment: Vec::new(),
            unset_environment: Vec::new(),
            working_directory,
            location,
        };
        match installation.agent {
            SubscriptionAgent::ClaudeCode => {
                launch.args.extend([
                    "-p".to_string(),
                    "--input-format".to_string(),
                    "stream-json".to_string(),
                    "--output-format".to_string(),
                    "stream-json".to_string(),
                    "--verbose".to_string(),
                    "--permission-mode".to_string(),
                    "default".to_string(),
                ]);
                launch
                    .unset_environment
                    .extend(CLAUDE_SUBSCRIPTION_PROVIDER_ENVIRONMENT_VARIABLES);
                launch.environment.push((
                    CLAUDE_PROVIDER_MANAGED_BY_HOST.0,
                    CLAUDE_PROVIDER_MANAGED_BY_HOST.1.to_string(),
                ));
                if let Some(config_dir) = installation.account.config_dir.as_ref() {
                    launch.environment.push((
                        "CLAUDE_CONFIG_DIR",
                        config_dir.to_string_lossy().into_owned(),
                    ));
                }
            }
            SubscriptionAgent::Codex => {
                launch.args.extend([
                    "app-server".to_string(),
                    "--listen".to_string(),
                    "stdio://".to_string(),
                ]);
                launch.unset_environment.push("OPENAI_API_KEY");
                if let Some(config_dir) = installation.account.config_dir.as_ref() {
                    launch
                        .environment
                        .push(("CODEX_HOME", config_dir.to_string_lossy().into_owned()));
                }
            }
        }
        launch
    }

    pub(crate) fn for_session(
        target: &SubscriptionTarget,
        session: Option<&str>,
        location: ProcessLocation,
    ) -> Self {
        let mut launch = Self {
            program: target.installation.executable.clone(),
            args: Vec::new(),
            environment: Vec::new(),
            unset_environment: Vec::new(),
            working_directory: target.working_directory.clone(),
            location,
        };
        match target.installation.agent {
            SubscriptionAgent::ClaudeCode => {
                launch.args.extend([
                    "-p".to_string(),
                    "--input-format".to_string(),
                    "stream-json".to_string(),
                    "--output-format".to_string(),
                    "stream-json".to_string(),
                    "--verbose".to_string(),
                    "--permission-mode".to_string(),
                    "default".to_string(),
                    "--model".to_string(),
                    target.model.id.clone(),
                ]);
                if let Some(effort) = target.effort.as_deref() {
                    launch
                        .args
                        .extend(["--effort".to_string(), effort.to_string()]);
                }
                if let Some(session) = session {
                    launch
                        .args
                        .extend(["--resume".to_string(), session.to_string()]);
                }
                launch
                    .unset_environment
                    .extend(CLAUDE_SUBSCRIPTION_PROVIDER_ENVIRONMENT_VARIABLES);
                launch.environment.push((
                    CLAUDE_PROVIDER_MANAGED_BY_HOST.0,
                    CLAUDE_PROVIDER_MANAGED_BY_HOST.1.to_string(),
                ));
                if let Some(config_dir) = target.installation.account.config_dir.as_ref() {
                    launch.environment.push((
                        "CLAUDE_CONFIG_DIR",
                        config_dir.to_string_lossy().into_owned(),
                    ));
                }
            }
            SubscriptionAgent::Codex => {
                launch.args.extend([
                    "app-server".to_string(),
                    "--listen".to_string(),
                    "stdio://".to_string(),
                ]);
                launch.unset_environment.push("OPENAI_API_KEY");
                if let Some(config_dir) = target.installation.account.config_dir.as_ref() {
                    launch
                        .environment
                        .push(("CODEX_HOME", config_dir.to_string_lossy().into_owned()));
                }
            }
        }
        launch
    }

    fn command(&self) -> Result<Command> {
        let mut command = match &self.location {
            ProcessLocation::Local => {
                let mut command = Command::new(&self.program);
                command
                    .args(&self.args)
                    .current_dir(&self.working_directory);
                for (name, value) in &self.environment {
                    command.env(name, value);
                }
                for name in &self.unset_environment {
                    command.env_remove(name);
                }
                command
            }
            ProcessLocation::Remote { ssh_argv, .. } => {
                remote_ssh_command(ssh_argv, &self.remote_command())?
            }
        };
        command.kill_on_drop(true);
        Ok(command)
    }

    fn remote_command(&self) -> String {
        let mut env_args = vec!["env".to_string()];
        for name in &self.unset_environment {
            env_args.extend(["-u".to_string(), (*name).to_string()]);
        }
        for (name, value) in &self.environment {
            env_args.push(format!("{name}={value}"));
        }
        if let ProcessLocation::Remote {
            environment_path: Some(path),
            ..
        } = &self.location
        {
            env_args.push(format!("PATH={path}"));
        }
        env_args.push(self.program.to_string_lossy().into_owned());
        env_args.extend(self.args.iter().cloned());

        let working_directory = self.working_directory.to_string_lossy();
        let cwd = shell_words::quote(&working_directory);
        let process = env_args
            .iter()
            .map(|arg| shell_words::quote(arg))
            .collect::<Vec<_>>()
            .join(" ");
        format!("cd -- {cwd} && exec {process}")
    }
}

fn remote_ssh_command(ssh_argv: &[String], remote_command: &str) -> Result<Command> {
    let (program, ssh_args) = ssh_argv
        .split_first()
        .context("remote launch requires an SSH program")?;
    let destination_delimiter = ssh_args
        .iter()
        .position(|arg| arg == "--")
        .context("remote launch requires an SSH destination delimiter")?;
    let mut ssh_args = ssh_args.to_vec();
    ssh_args.splice(
        destination_delimiter..destination_delimiter,
        [
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            "-o".to_string(),
            "ConnectTimeout=10".to_string(),
            "-o".to_string(),
            "ConnectionAttempts=1".to_string(),
        ],
    );
    let mut command = Command::new(program);
    command
        .args(ssh_args)
        .arg(remote_command)
        .kill_on_drop(true);
    Ok(command)
}

/// Resolves a real executable and its interpreter PATH without running a CLI or
/// mixing login-shell startup messages into its structured protocol stream.
pub(crate) async fn resolve_remote_executable(
    installation: &mut InstallationIdentity,
    location: &mut ProcessLocation,
) -> Result<()> {
    let ProcessLocation::Remote {
        ssh_argv,
        environment_path,
    } = location
    else {
        return Ok(());
    };
    let command = remote_ssh_command(
        ssh_argv,
        &remote_executable_probe(&installation.executable)?,
    )?;
    let (status, stdout, stderr) = bounded_probe_output(command).await?;
    if !status.success() {
        bail!(
            "Could not locate {} on the remote host: {}",
            installation.agent.display_name(),
            process_diagnostic(&stderr).unwrap_or(
                "the login shell could not report an executable; check its startup files and CLI installation"
            )
        );
    }
    let (executable, path) = parse_remote_executable(&stdout)?;
    installation.executable = executable;
    *environment_path = Some(path);
    Ok(())
}

fn remote_executable_probe(program: &Path) -> Result<String> {
    let program = program
        .to_str()
        .filter(|program| !program.is_empty() && !program.chars().any(char::is_control))
        .context("remote CLI name is invalid")?;
    // A separate POSIX shell ignores aliases/functions and resolves an executable
    // using the PATH exported by bash/zsh/fish login + interactive startup files.
    // fd 3 carries only the probe result; startup output is drained as stderr.
    let resolver = r#"program=$(command -v "$1") || { printf '%s\n' ZAPLEX_CLI_NOT_FOUND >&2; exit 127; }
case "$program" in /*) ;; *) printf '%s\n' ZAPLEX_CLI_NOT_EXECUTABLE >&2; exit 126;; esac
[ -f "$program" ] && [ -x "$program" ] || { printf '%s\n' ZAPLEX_CLI_NOT_EXECUTABLE >&2; exit 126; }
printf '%s\0%s\0' "$program" "$PATH" >&3"#;
    let login_command = format!(
        "exec /bin/sh -c {} zaplex-cli-probe {}",
        shell_words::quote(resolver),
        shell_words::quote(program)
    );
    let wrapper = format!(
        "exec 3>&1; BYOBU_DISABLE=1 LC_BYOBU=0 \"${{SHELL:-/bin/sh}}\" -lic {} 1>&2",
        shell_words::quote(&login_command)
    );
    Ok(format!("exec /bin/sh -c {}", shell_words::quote(&wrapper)))
}

fn parse_remote_executable(output: &[u8]) -> Result<(PathBuf, String)> {
    let fields = output.split(|byte| *byte == 0).collect::<Vec<_>>();
    let [executable, path, []] = fields.as_slice() else {
        bail!("Remote CLI lookup returned unexpected shell output; check the login shell startup files");
    };
    let executable = std::str::from_utf8(executable).context("remote CLI path is not UTF-8")?;
    let path = std::str::from_utf8(path).context("remote CLI search path is not UTF-8")?;
    if !executable.starts_with('/')
        || executable.chars().any(char::is_control)
        || path.is_empty()
        || path.chars().any(char::is_control)
    {
        bail!("Remote CLI lookup did not return a valid absolute executable and PATH");
    }
    Ok((PathBuf::from(executable), path.to_string()))
}

// Never surface raw stderr: authentication tools and shell startup files can
// print secrets. Retain only a bounded in-memory prefix and tail so noisy login
// banners cannot hide the final failure reason.
fn process_diagnostic(stderr: &[u8]) -> Option<&'static str> {
    let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
    if text.contains("host key verification failed")
        || text.contains("remote host identification has changed")
    {
        Some("SSH host verification failed; reconnect the host and verify its SSH key")
    } else if text.contains("permission denied (publickey") {
        Some("SSH authentication failed; reconnect the host with its configured SSH key")
    } else if text.contains("mux_client")
        || text.contains("control socket")
        || text.contains("kex_exchange_identification")
        || text.contains("connection closed by unknown port")
    {
        Some("The managed SSH connection is unavailable; reconnect the host and retry")
    } else if text.contains("zaplex_cli_not_found") || text.contains("zaplex_cli_not_executable") {
        Some("The CLI is not executable on the login shell PATH; check its installation on this host")
    } else if text.contains("node") && (text.contains("no such file") || text.contains("not found"))
    {
        Some("The CLI runtime is unavailable; check Node.js and the login shell PATH on this host")
    } else {
        None
    }
}

async fn drain_stderr(reader: &mut ChildStderr, captured: &mut Vec<u8>) -> std::io::Result<()> {
    let mut chunk = [0; 1024];
    loop {
        let length = reader.read(&mut chunk).await?;
        if length == 0 {
            return Ok(());
        }
        captured.extend_from_slice(&chunk[..length]);
        if captured.len() > MAX_DIAGNOSTIC_BYTES {
            let prefix_length = MAX_DIAGNOSTIC_BYTES / 2;
            let tail_length = MAX_DIAGNOSTIC_BYTES - prefix_length - 1;
            drop(captured.drain(prefix_length..captured.len() - tail_length));
            // Separate nonadjacent output rather than joining two partial words.
            captured.insert(prefix_length, b'\n');
        }
    }
}

async fn with_stderr<T>(
    reader: &mut ChildStderr,
    captured: &mut Vec<u8>,
    operation: impl Future<Output = T>,
) -> T {
    futures_lite::future::race(operation, async {
        let _ = drain_stderr(reader, captured).await;
        futures_lite::future::pending().await
    })
    .await
}

async fn read_probe_output(mut reader: impl AsyncRead + Unpin) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    let mut chunk = [0; 1024];
    loop {
        let length = reader.read(&mut chunk).await?;
        if length == 0 {
            return Ok(output);
        }
        if output.len() + length > MAX_PROBE_OUTPUT_BYTES {
            bail!("CLI lookup returned excessive output; check the login shell startup files");
        }
        output.extend_from_slice(&chunk[..length]);
    }
}

async fn bounded_probe_output(mut command: Command) -> Result<(ExitStatus, Vec<u8>, Vec<u8>)> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("failed to start CLI lookup")?;
    let stdout = child
        .stdout
        .take()
        .context("CLI lookup stdout is missing")?;
    let mut stderr = child
        .stderr
        .take()
        .context("CLI lookup stderr is missing")?;
    let mut captured = Vec::new();
    let (status, stdout, ()) = futures_util::try_join!(
        async {
            child
                .status()
                .await
                .context("failed to wait for CLI lookup")
        },
        read_probe_output(stdout),
        async {
            drain_stderr(&mut stderr, &mut captured)
                .await
                .context("failed to read CLI lookup diagnostics")
        },
    )?;
    Ok((status, stdout, captured))
}

pub(crate) struct JsonLineProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: ChildStderr,
    diagnostics: Vec<u8>,
}

impl JsonLineProcess {
    pub(crate) fn spawn(launch: &ProcessLaunch) -> Result<Self> {
        let mut command = launch.command()?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().with_context(|| {
            format!(
                "failed to start subscription agent {}",
                launch.program.display()
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .context("subscription agent stdin was not piped")?;
        let stdout = child
            .stdout
            .take()
            .context("subscription agent stdout was not piped")?;
        let stderr = child
            .stderr
            .take()
            .context("subscription agent stderr was not piped")?;
        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            stderr,
            diagnostics: Vec::new(),
        })
    }

    pub(crate) async fn send(&mut self, value: &Value) -> Result<()> {
        let mut line = serde_json::to_vec(value)?;
        line.push(b'\n');
        let stdin = self
            .stdin
            .as_mut()
            .context("subscription agent stdin is closed")?;
        let result = with_stderr(&mut self.stderr, &mut self.diagnostics, async {
            stdin
                .write_all(&line)
                .await
                .context("failed to write subscription-agent protocol frame")?;
            stdin
                .flush()
                .await
                .context("failed to flush subscription-agent protocol frame")
        })
        .await;
        if let Err(error) = result {
            let _ = drain_stderr(&mut self.stderr, &mut self.diagnostics)
                .with_timeout(STDERR_DRAIN_TIMEOUT)
                .await;
            return Err(match process_diagnostic(&self.diagnostics) {
                Some(message) => error.context(message),
                None => error,
            });
        }
        Ok(())
    }

    pub(crate) async fn receive(&mut self) -> Result<Option<Value>> {
        let mut line = String::new();
        let bytes = with_stderr(
            &mut self.stderr,
            &mut self.diagnostics,
            self.stdout.read_line(&mut line),
        )
        .await
        .context("failed to read subscription-agent protocol frame")?;
        if bytes == 0 {
            let _ = drain_stderr(&mut self.stderr, &mut self.diagnostics)
                .with_timeout(STDERR_DRAIN_TIMEOUT)
                .await;
            if let Some(message) = process_diagnostic(&self.diagnostics) {
                bail!("{message}");
            }
            // The session layer maps EOF to an error event carrying the native session ID.
            return Ok(None);
        }
        serde_json::from_str(&line)
            .with_context(|| format!("subscription agent emitted malformed JSON: {}", line.trim()))
            .map(Some)
    }

    pub(super) async fn terminate(&mut self) -> Result<ProcessTermination> {
        self.terminate_with_timeouts(PROCESS_GRACEFUL_EXIT_TIMEOUT, PROCESS_FORCED_EXIT_TIMEOUT)
            .await
    }

    async fn terminate_with_timeouts(
        &mut self,
        graceful_timeout: Duration,
        forced_timeout: Duration,
    ) -> Result<ProcessTermination> {
        if let Some(mut stdin) = self.stdin.take() {
            if let Err(error) = stdin.close().await {
                log::debug!("Failed to close subscription-agent stdin before exit: {error}");
            }
        }

        match with_stderr(
            &mut self.stderr,
            &mut self.diagnostics,
            wait_for_exit(&mut self.child, graceful_timeout),
        )
        .await
        {
            Ok(true) => return Ok(ProcessTermination::Graceful),
            Ok(false) => {}
            Err(error) => {
                log::warn!(
                    "Failed while waiting for subscription-agent exit; forcing termination: {error:#}"
                );
            }
        }

        match self.child.kill() {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {}
            Err(error) => {
                return Err(error).context("failed to force-stop subscription-agent process");
            }
        }

        if with_stderr(
            &mut self.stderr,
            &mut self.diagnostics,
            wait_for_exit(&mut self.child, forced_timeout),
        )
        .await?
        {
            Ok(ProcessTermination::Forced)
        } else {
            Err(anyhow::anyhow!(
                "subscription-agent process did not exit after forced termination"
            ))
        }
    }
}

async fn wait_for_exit(child: &mut Child, timeout: Duration) -> Result<bool> {
    match child.status().with_timeout(timeout).await {
        Ok(status) => {
            status.context("failed to wait for subscription-agent process")?;
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}

pub(crate) async fn query_cli_version(
    installation: &InstallationIdentity,
    working_directory: PathBuf,
    location: ProcessLocation,
) -> Result<String> {
    let mut launch = ProcessLaunch::for_discovery(installation, working_directory, location);
    launch.args = vec!["--version".to_string()];
    let (status, stdout, stderr) = bounded_probe_output(launch.command()?)
        .await
        .with_context(|| format!("failed to query {} version", launch.program.display()))?;
    if !status.success() {
        return Err(anyhow!(
            "{} --version exited with {status}: {}",
            launch.program.display(),
            process_diagnostic(&stderr)
                .unwrap_or("check the CLI installation on the selected host")
        ));
    }
    let version = String::from_utf8(stdout)
        .context("CLI version output was not UTF-8")?
        .trim()
        .to_string();
    if version.is_empty() {
        return Err(anyhow::anyhow!(
            "{} --version returned no version",
            launch.program.display()
        ));
    }
    Ok(version)
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "process_lifecycle_tests.rs"]
mod lifecycle_tests;
