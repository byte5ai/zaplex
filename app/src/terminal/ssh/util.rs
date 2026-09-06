use std::{io::Write as _, path::Path};

use lazy_static::lazy_static;
use regex::Regex;
use tempfile::NamedTempFile;
use thiserror::Error;
use warp_util::path::ShellFamily;

/// Converts a multiline bash or zsh script to one line by turning newlines into semicolons or
/// deleting them, as appropriate.
///
/// Extra semicolons are a syntax error in bash, so this is careful to avoid adding them except
/// where necessary.
///
/// This function exists because there's a strange macOS ssh server bug where sending a lot of data
/// containing newlines to a shell results in data corruption.
pub fn convert_script_to_one_line(script: &str) -> String {
    lazy_static! {
        static ref EXTRA_SPACES_REGEX: Regex = Regex::new(r"\n+\s*").expect("invalid regex");
        static ref NO_SEMICOLON_REGEX: Regex =
            Regex::new("(; ?|\\{|do|then|else|in)\n").expect("invalid regex");
        static ref REMOVE_COMMENTS_REGEX: Regex = Regex::new(r"(?m)^ *#.*").expect("invalid regex");
        static ref REMOVE_LEADING_NEWLINES: Regex = Regex::new(r"^\n*").expect("invalid regex");
    };
    let script = REMOVE_COMMENTS_REGEX.replace_all(script, "");
    let script = REMOVE_LEADING_NEWLINES.replace_all(&script, "");
    let script = EXTRA_SPACES_REGEX.replace_all(&script, "\n");
    let script = NO_SEMICOLON_REGEX.replace_all(&script, "$1 ");
    let mut script = script.replace('\n', ";");
    script.push('\n');
    script
}

pub enum SshLoginState {
    LastLogin,
    NonSshOutput,
    Authenticating,
    PromptDetected,
}

/// Reads the contents of the output grid to determine SSH login state. Returns [SshLoginState::LastLogin] if
/// "Last login:" is detected in the output. Returns [SshLoginState::NonSshOutput] if certain keywords
/// known to be a part of ssh login prompts are found in the current last line of command output. The
/// "password" and "Password" are for password authentication. "passphrase" is intended to cover authentication
/// by public key. And "yes/no" relates to trust-on-first-use prompts for host-based authentication.
pub fn check_ssh_login_state(block_output: &str) -> SshLoginState {
    lazy_static! {
        // Common final prompt characters followed by a space.
        static ref PROMPT_REGEX: Regex = Regex::new(r"[$#%>❯│⟫»▶λ→] $").expect("invalid regex");
    };

    let mut last_line = None;

    for line in block_output.lines() {
        if line.starts_with("Last login:") {
            return SshLoginState::LastLogin;
        }
        // With an iterator, there's no way to know if it's the last element so
        // we overwrite last_line at each iteration.
        last_line = Some(line);
    }

    last_line.map_or(SshLoginState::Authenticating, |line| {
        if line.contains("password")
            || line.contains("Password")
            || line.contains("passphrase")
            || line.contains("yes/no")
            || line.contains("Please type")
            || line.contains("'yes'")
            || line.contains("Confirm user presence")
            || line.starts_with("Enter ")
            || line.starts_with("Allow ")
        {
            SshLoginState::Authenticating
        } else if PROMPT_REGEX.is_match(line) {
            SshLoginState::PromptDetected
        } else {
            SshLoginState::NonSshOutput
        }
    })
}

/// Represents the parsed components of an interactive SSH command.
/// For some [`SshZaplexifyCommand`]s, we do not support parsing
/// a host or port In these cases, we can still parse to a valid
/// empty `InteractiveSshCommand` to indicate that we did
/// successfully detect an interactive SSH command.
#[derive(Clone, Debug, Default)]
pub struct InteractiveSshCommand {
    pub host: Option<String>,
    pub port: Option<String>,
}

impl InteractiveSshCommand {
    /// Parses ssh commands of the form `ssh ...`.
    /// Only returns an `InteractiveSshCommand` if we determine the command is interactive.
    fn parse_ssh_command(command: &str) -> Option<InteractiveSshCommand> {
        let command = if let Some(suffix) = command.strip_prefix("command ") {
            suffix
        } else {
            command
        };
        let tokens = parse_ssh_command_tokens(command)?;
        let mut host: Option<String> = None;
        let mut port: Option<String> = None;

        let mut i = 1;
        while i < tokens.len() {
            match tokens[i].as_str() {
                // -T or -W imply a non-interactive session.
                "-T" | "-W" => return None,

                "-p" => {
                    i += 1;
                    if i < tokens.len() {
                        port = Some(tokens[i].clone());
                    } else {
                        return None;
                    }
                }

                // SSH option that doesn't change interactivity and require an argument: Skip the next item.
                "-B" | "-b" | "-c" | "-D" | "-E" | "-e" | "-F" | "-I" | "-i" | "-J" | "-L"
                | "-l" | "-m" | "-O" | "-o" | "-P" | "-Q" | "-R" | "-S" | "-w" => {
                    i += 1;
                }

                // SSH option(s) that don't change interactivity.
                arg if arg.starts_with('-') => {}

                // Otherwise, it's a positional argument (e.g., hostname, command to run)
                pos_arg => {
                    // If we detect multiple positional args, there's some type of unknown command formulation.
                    if host.is_some() {
                        return None;
                    }
                    host = Some(pos_arg.to_string());
                }
            }
            i += 1;
        }

        Some(InteractiveSshCommand { host, port })
    }
}

pub enum SshLikeCommand {
    Gcloud,
    ElasticBeanstalk,
    DigitalOceanDroplet,
}

/// TMUX SSH Zaplexification can be triggered by any command that
/// we determine to be an interactive SSH command. This enum
/// represents the different types of SSH commands we support
/// for TMUX Zaplexification. `Ssh` means a literal `ssh` command,
/// where all other commands are categorized as SSH-like commands.
pub enum SshZaplexifyCommand {
    Ssh,
    SshLike(SshLikeCommand),
}

impl SshZaplexifyCommand {
    /// Not a literal `ssh` command, but another command that starts an interactive SSH
    /// session that we can Zaplexify with TMUX.
    pub fn is_ssh_like_command(&self) -> bool {
        matches!(self, SshZaplexifyCommand::SshLike(_))
    }
}

lazy_static! {
    static ref INTERACTIVE_SSH: Regex = Regex::new(r"^ssh\s+").expect("interactive SSH regex invalid");

    /// Matches "gcloud compute ssh" for connecting to GCP VMs.
    static ref GCLOUD_REGEX: Regex = Regex::new(r"^gcloud\s+compute\s+ssh\s.+").expect("gcloud SSH regex invalid");

    /// Matches "eb ssh" for connecting to AWS Elastic Beanstalk VMs.
    static ref ELASTIC_BEANSTALK_REGEX: Regex = Regex::new(r"^eb\s+ssh\s.+").expect("elastic beanstalk SSH regex invalid");

    /// Matches "doctl compute ssh" for connecting to a digital ocean droplet.
    static ref DIGITAL_OCEAN_DROPLET_REGEX: Regex = Regex::new(r"^doctl\s+compute\s+ssh\s.+").expect("digital ocean SSH regex invalid");
}

impl SshZaplexifyCommand {
    pub fn matches(command: &str) -> Option<SshZaplexifyCommand> {
        let command = if let Some(suffix) = command.strip_prefix("command ") {
            suffix
        } else {
            command
        };
        if INTERACTIVE_SSH.is_match(command) {
            Some(SshZaplexifyCommand::Ssh)
        } else if GCLOUD_REGEX.is_match(command) {
            Some(SshZaplexifyCommand::SshLike(SshLikeCommand::Gcloud))
        } else if ELASTIC_BEANSTALK_REGEX.is_match(command) {
            Some(SshZaplexifyCommand::SshLike(
                SshLikeCommand::ElasticBeanstalk,
            ))
        } else if DIGITAL_OCEAN_DROPLET_REGEX.is_match(command) {
            Some(SshZaplexifyCommand::SshLike(
                SshLikeCommand::DigitalOceanDroplet,
            ))
        } else {
            None
        }
    }
}

pub fn parse_interactive_ssh_command(command: &str) -> Option<InteractiveSshCommand> {
    match SshZaplexifyCommand::matches(command) {
        Some(SshZaplexifyCommand::Ssh) => InteractiveSshCommand::parse_ssh_command(command),
        Some(SshZaplexifyCommand::SshLike(SshLikeCommand::Gcloud)) => {
            Some(InteractiveSshCommand::default())
        }
        Some(SshZaplexifyCommand::SshLike(SshLikeCommand::ElasticBeanstalk)) => {
            Some(InteractiveSshCommand::default())
        }
        Some(SshZaplexifyCommand::SshLike(SshLikeCommand::DigitalOceanDroplet)) => {
            Some(InteractiveSshCommand::default())
        }
        None => None,
    }
}

fn parse_ssh_command_tokens(command: &str) -> Option<Vec<String>> {
    let Ok(tokens) = shell_words::split(command) else {
        return None;
    };

    // Cases: "", "ls", "ssh-add-key"
    if tokens.is_empty() || tokens[0] != "ssh" {
        return None;
    }
    Some(tokens)
}

#[derive(Debug, Error)]
pub enum SftpUploadError {
    #[error("the SFTP upload requires at least one local path")]
    NoLocalPaths,
    #[error("the {field} contains a forbidden NUL, carriage return, or newline")]
    InvalidBatchPath { field: &'static str },
    #[error("the SSH host is not a valid destination argument")]
    InvalidHost,
    #[error("the SSH port is not a valid non-zero TCP port")]
    InvalidPort,
    #[error("the temporary SFTP batch path is not valid UTF-8")]
    InvalidBatchFilePath,
    #[error("failed to prepare the private SFTP batch file: {0}")]
    BatchFile(#[from] std::io::Error),
}

/// Separates data interpreted by SFTP from arguments interpreted by the local
/// shell. Local and remote paths only occur in `batch`, never in `argv`.
#[derive(Debug, PartialEq, Eq)]
pub struct SftpUploadPlan {
    argv: Vec<String>,
    batch: Vec<u8>,
}

impl SftpUploadPlan {
    pub fn new(
        local_file_paths: &[String],
        ssh_host: &str,
        ssh_port: Option<&str>,
        remote_dest_path: Option<&str>,
    ) -> Result<Self, SftpUploadError> {
        if local_file_paths.is_empty() {
            return Err(SftpUploadError::NoLocalPaths);
        }
        if ssh_host.is_empty()
            || ssh_host.starts_with('-')
            || contains_forbidden_record_character(ssh_host)
        {
            return Err(SftpUploadError::InvalidHost);
        }

        let mut argv = vec!["sftp".to_string()];
        if let Some(port) = ssh_port {
            if port.parse::<u16>().ok().filter(|port| *port != 0).is_none() {
                return Err(SftpUploadError::InvalidPort);
            }
            argv.extend(["-P".to_string(), port.to_string()]);
        }
        argv.push(ssh_host.to_string());

        let remote_dest_path = remote_dest_path
            .map(|path| quote_sftp_batch_path(path, "remote destination"))
            .transpose()?;
        let mut commands = Vec::with_capacity(local_file_paths.len());
        for local_file_path in local_file_paths {
            let recursive = Path::new(local_file_path)
                .metadata()
                .is_ok_and(|metadata| metadata.is_dir());
            let local_file_path = quote_sftp_batch_path(local_file_path, "local path")?;
            let mut command = if recursive {
                format!("put -r {local_file_path}")
            } else {
                format!("put {local_file_path}")
            };
            if let Some(remote_dest_path) = &remote_dest_path {
                command.push(' ');
                command.push_str(remote_dest_path);
            }
            commands.push(command);
        }

        let mut batch = commands.join("\n").into_bytes();
        batch.push(b'\n');
        Ok(Self { argv, batch })
    }

    pub fn materialize(
        &self,
        shell_family: ShellFamily,
    ) -> Result<MaterializedSftpUpload, SftpUploadError> {
        let mut batch_file = NamedTempFile::new()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            batch_file
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        batch_file.write_all(&self.batch)?;
        batch_file.flush()?;

        let batch_path = batch_file
            .path()
            .to_str()
            .ok_or(SftpUploadError::InvalidBatchFilePath)?;
        let argv = self
            .argv
            .iter()
            .map(|arg| shell_family.escape(arg).into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let batch_path = shell_family.escape(batch_path);
        let command = match shell_family {
            ShellFamily::Posix => format!("{argv} < {batch_path}"),
            ShellFamily::PowerShell => {
                format!("Get-Content -Raw -LiteralPath {batch_path} | & {argv}")
            }
        };

        Ok(MaterializedSftpUpload {
            command,
            _batch_file: batch_file,
        })
    }

    #[cfg(test)]
    pub(crate) fn batch(&self) -> &[u8] {
        &self.batch
    }
}

#[derive(Debug)]
pub struct MaterializedSftpUpload {
    command: String,
    _batch_file: NamedTempFile,
}

impl MaterializedSftpUpload {
    pub fn command(&self) -> &str {
        &self.command
    }

    #[cfg(test)]
    pub(crate) fn batch_path(&self) -> std::path::PathBuf {
        self._batch_file.path().to_path_buf()
    }
}

fn contains_forbidden_record_character(value: &str) -> bool {
    value
        .chars()
        .any(|character| matches!(character, '\0' | '\r' | '\n'))
}

fn quote_sftp_batch_path(path: &str, field: &'static str) -> Result<String, SftpUploadError> {
    if contains_forbidden_record_character(path) {
        return Err(SftpUploadError::InvalidBatchPath { field });
    }
    Ok(format!(
        "\"{}\"",
        path.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

/// Creates the shared two-stage SFTP upload plan for one local file.
pub fn transfer_file_sftp_command(
    local_file_path: String,
    ssh_host: String,
    ssh_port: Option<String>,
    pwd: Option<String>,
) -> Result<SftpUploadPlan, SftpUploadError> {
    SftpUploadPlan::new(
        &[local_file_path],
        &ssh_host,
        ssh_port.as_deref(),
        pwd.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_gcloud_ssh_parsing() {
        assert!(parse_interactive_ssh_command("gcloud").is_none());
        assert!(parse_interactive_ssh_command("gcloud compute").is_none());
        assert!(parse_interactive_ssh_command("gcloud compute ss").is_none());
        assert!(parse_interactive_ssh_command("gcloud compute ssh").is_none());
        assert!(parse_interactive_ssh_command("command gcloud compute ssh").is_none());

        assert!(
            parse_interactive_ssh_command("command gcloud compute ssh --zone us-west1-a").is_some()
        );
        assert!(parse_interactive_ssh_command("gcloud compute ssh --zone us-west1-a").is_some());
        assert!(
            parse_interactive_ssh_command("gcloud compute ssh --zone us-west1-a my-instance")
                .is_some()
        );
        assert!(parse_interactive_ssh_command(
            "gcloud compute ssh --zone us-west1-a my-instance --project my-project"
        )
        .is_some());
    }

    #[test]
    fn ssh_elastic_beanstalk_parsing() {
        assert!(parse_interactive_ssh_command("eb").is_none());
        assert!(parse_interactive_ssh_command("eb ss").is_none());
        assert!(parse_interactive_ssh_command("eb ssh").is_none());
        assert!(parse_interactive_ssh_command("command eb ssh").is_none());

        assert!(parse_interactive_ssh_command("command eb ssh --profile my-profile").is_some());
        assert!(parse_interactive_ssh_command("eb ssh --profile my-profile").is_some());
        assert!(parse_interactive_ssh_command("eb ssh --profile my-profile my-env").is_some());
    }

    #[test]
    fn ssh_digital_ocean_droplet_parsing() {
        assert!(parse_interactive_ssh_command("doctl").is_none());
        assert!(parse_interactive_ssh_command("doctl compute").is_none());
        assert!(parse_interactive_ssh_command("doctl compute ss").is_none());
        assert!(parse_interactive_ssh_command("doctl compute ssh").is_none());
        assert!(parse_interactive_ssh_command("command doctl compute ssh").is_none());

        assert!(parse_interactive_ssh_command("command doctl compute ssh --region nyc1").is_some());
        assert!(parse_interactive_ssh_command("doctl compute ssh --region nyc1").is_some());
        assert!(
            parse_interactive_ssh_command("doctl compute ssh --region nyc1 my-droplet").is_some()
        );
    }

    /// Verifies that commands resulting from shell alias expansion are correctly
    /// detected as interactive SSH commands. When a user types an alias (e.g.
    /// `myssh`), the terminal view expands it to the alias value before passing
    /// it to `parse_interactive_ssh_command`. These tests cover representative
    /// expanded forms.
    #[test]
    fn ssh_alias_expanded_commands() {
        // Simple alias: alias myssh='ssh user@host'
        assert_eq!(
            parse_interactive_ssh_command("ssh user@host").unwrap().host,
            Some("user@host".to_string())
        );

        // Alias with key and user: alias company1='ssh -i /path/to/key user@server'
        assert_eq!(
            parse_interactive_ssh_command("ssh -i /path/to/key user@server")
                .unwrap()
                .host,
            Some("user@server".to_string())
        );

        // Alias with extra args appended by the user: alias myssh='ssh -i key'
        // then the user types `myssh user@host` which expands to `ssh -i key user@host`
        assert_eq!(
            parse_interactive_ssh_command("ssh -i key user@host")
                .unwrap()
                .host,
            Some("user@host".to_string())
        );

        // Alias that isn't SSH should not match
        assert!(parse_interactive_ssh_command("ls -la").is_none());
    }

    #[test]
    fn ssh_interactive_shell_parsing() {
        assert!(parse_interactive_ssh_command("").is_none());
        assert!(parse_interactive_ssh_command("ls").is_none());
        assert!(parse_interactive_ssh_command("ssh-add-key").is_none());

        // Basic interactive command
        assert!(
            parse_interactive_ssh_command("ssh localhost").unwrap().host
                == Some("localhost".to_string())
        );
        assert!(
            parse_interactive_ssh_command("command ssh localhost")
                .unwrap()
                .host
                == Some("localhost".to_string())
        );
        assert!(
            parse_interactive_ssh_command("ssh root@127.14.80.1 -p 2222")
                .unwrap()
                .host
                == Some("root@127.14.80.1".to_string())
        );
        assert!(
            parse_interactive_ssh_command("ssh -4vw root@127.14.80.1 -p 2222")
                .unwrap()
                .host
                == Some("root@127.14.80.1".to_string())
        );

        // Commands with -T or -W, which are non-interactive
        assert!(parse_interactive_ssh_command("ssh -T user@host").is_none());
        assert!(parse_interactive_ssh_command("ssh -v user@host -W localhost:22").is_none());
        assert!(
            parse_interactive_ssh_command("ssh -o IdentityFile=/etc/file -T user@host").is_none()
        );

        // Commands with multiple positional arguments, implying non-interactive
        assert!(parse_interactive_ssh_command("ssh user@host ls").is_none());
        assert!(parse_interactive_ssh_command("ssh user@host echo 'Hello, World!'").is_none());

        // Weird spacing and shell characters shouldn't matter
        assert!(
            parse_interactive_ssh_command("ssh     user@host")
                .unwrap()
                .host
                == Some("user@host".to_string())
        );
        assert!(
            parse_interactive_ssh_command("ssh -4 -- localhost")
                .unwrap()
                .host
                == Some("localhost".to_string())
        );
    }
}
