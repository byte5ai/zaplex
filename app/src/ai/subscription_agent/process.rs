use super::{InstallationIdentity, SubscriptionAgent, SubscriptionTarget};
use anyhow::{Context, Result};
use async_process::{Child, ChildStdin, ChildStdout};
use command::r#async::Command;
use futures_lite::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessLocation {
    Local,
    /// Prebuilt OpenSSH argv including the program at index zero and the destination.
    Remote {
        ssh_argv: Vec<String>,
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
                    .extend(["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]);
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
                    .extend(["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN"]);
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
        match &self.location {
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
                Ok(command)
            }
            ProcessLocation::Remote { ssh_argv } => {
                let (program, ssh_args) = ssh_argv
                    .split_first()
                    .context("remote launch requires an SSH program")?;
                let mut command = Command::new(program);
                command.args(ssh_args).arg(self.remote_command());
                Ok(command)
            }
        }
    }

    fn remote_command(&self) -> String {
        let mut env_args = vec!["env".to_string()];
        for name in &self.unset_environment {
            env_args.extend(["-u".to_string(), (*name).to_string()]);
        }
        for (name, value) in &self.environment {
            env_args.push(format!("{name}={value}"));
        }
        env_args.push(self.program.to_string_lossy().into_owned());
        env_args.extend(self.args.iter().cloned());

        let cwd = shell_words::quote(&self.working_directory.to_string_lossy());
        let process = env_args
            .iter()
            .map(|arg| shell_words::quote(arg))
            .collect::<Vec<_>>()
            .join(" ");
        format!("cd -- {cwd} && exec {process}")
    }
}

pub(crate) struct JsonLineProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl JsonLineProcess {
    pub(crate) fn spawn(launch: &ProcessLaunch) -> Result<Self> {
        let mut command = launch.command()?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
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
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    pub(crate) async fn send(&mut self, value: &Value) -> Result<()> {
        let mut line = serde_json::to_vec(value)?;
        line.push(b'\n');
        self.stdin
            .write_all(&line)
            .await
            .context("failed to write subscription-agent protocol frame")?;
        self.stdin.flush().await?;
        Ok(())
    }

    pub(crate) async fn receive(&mut self) -> Result<Option<Value>> {
        let mut line = String::new();
        let bytes = self
            .stdout
            .read_line(&mut line)
            .await
            .context("failed to read subscription-agent protocol frame")?;
        if bytes == 0 {
            return Ok(None);
        }
        serde_json::from_str(&line)
            .with_context(|| format!("subscription agent emitted malformed JSON: {}", line.trim()))
            .map(Some)
    }

    pub(crate) fn terminate(&mut self) -> Result<()> {
        self.child
            .kill()
            .context("failed to terminate subscription-agent process")
    }
}

pub(crate) async fn query_cli_version(
    installation: &InstallationIdentity,
    working_directory: PathBuf,
    location: ProcessLocation,
) -> Result<String> {
    let mut launch = ProcessLaunch::for_discovery(installation, working_directory, location);
    launch.args = vec!["--version".to_string()];
    let output = launch
        .command()?
        .output()
        .await
        .with_context(|| format!("failed to query {} version", launch.program.display()))?;
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "{} --version exited with {}",
            launch.program.display(),
            output.status
        ));
    }
    let version = String::from_utf8(output.stdout)
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
