use std::env;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

pub const CONTROL_SOCKET_ENV: &str = "ZAPLEX_CONTROL_SOCKET";
pub const CONTROL_TOKEN_ENV: &str = "ZAPLEX_CONTROL_TOKEN";
pub const SURFACE_ID_ENV: &str = "ZAPLEX_SURFACE_ID";
pub const TAB_ID_ENV: &str = "ZAPLEX_TAB_ID";
pub const CONTROL_PROTOCOL_VERSION: u32 = 1;

const MAX_CONTROL_TOKEN_BYTES: usize = 256;
const MAX_SURFACE_ID_BYTES: usize = 128;
const MAX_TAB_ID_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_HOOK_EVENT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Subcommand)]
pub enum ControlCommand {
    /// Open a terminal split relative to the calling pane.
    SplitPane(SplitPaneArgs),

    /// Create or attach a git worktree and open it in an agent-ready split.
    OpenWorktreeInPane(OpenWorktreeInPaneArgs),

    /// Focus an existing terminal surface or fleet session.
    FocusSession(FocusSessionArgs),

    /// Place text in a terminal input and optionally submit it.
    SendText(SendTextArgs),
}

#[derive(Debug, Clone, Args)]
pub struct SplitPaneArgs {
    /// Side on which to create the split.
    #[arg(long, value_enum, default_value = "right")]
    pub orientation: ControlOrientation,

    /// Initial working directory for the new pane.
    #[arg(long, value_name = "PATH")]
    pub dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct OpenWorktreeInPaneArgs {
    /// Repository in which to create or attach the worktree.
    #[arg(long, value_name = "PATH")]
    pub repo: PathBuf,

    /// Branch to create or attach.
    #[arg(long)]
    pub branch: String,
}

#[derive(Debug, Clone, Args)]
pub struct FocusSessionArgs {
    /// Exact Zaplex terminal surface to focus.
    #[arg(long, conflicts_with_all = ["host", "session_id"])]
    pub surface_id: Option<String>,

    /// Fleet host id or label. Must be paired with --session-id.
    #[arg(long, requires = "session_id")]
    pub host: Option<String>,

    /// Agent session id. Must be paired with --host.
    #[arg(long, requires = "host")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct SendTextArgs {
    /// Target Zaplex terminal surface.
    #[arg(long)]
    pub surface_id: String,

    /// Text to place in the target terminal input.
    #[arg(long)]
    pub text: String,

    /// Submit the text after placing it in the input.
    #[arg(long)]
    pub submit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum ControlOrientation {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlAuth {
    pub token: String,
    pub caller_surface_id: String,
    pub caller_tab_id: String,
}

impl ControlAuth {
    pub fn from_env() -> Result<Self> {
        let auth = Self {
            token: env::var(CONTROL_TOKEN_ENV)
                .with_context(|| format!("{CONTROL_TOKEN_ENV} is not set"))?,
            caller_surface_id: env::var(SURFACE_ID_ENV)
                .with_context(|| format!("{SURFACE_ID_ENV} is not set"))?,
            caller_tab_id: env::var(TAB_ID_ENV)
                .with_context(|| format!("{TAB_ID_ENV} is not set"))?,
        };
        auth.validate()?;
        Ok(auth)
    }

    pub fn validate(&self) -> Result<()> {
        validate_nonempty_bounded("control token", &self.token, MAX_CONTROL_TOKEN_BYTES)?;
        validate_nonempty_bounded(
            "caller surface id",
            &self.caller_surface_id,
            MAX_SURFACE_ID_BYTES,
        )?;
        validate_nonempty_bounded("caller tab id", &self.caller_tab_id, MAX_TAB_ID_BYTES)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlRequest {
    pub version: u32,
    pub auth: ControlAuth,
    pub verb: ControlVerb,
}

impl ControlRequest {
    pub fn from_cli(auth: ControlAuth, command: ControlCommand) -> Result<Self> {
        auth.validate()?;
        let verb = match command {
            ControlCommand::SplitPane(args) => ControlVerb::SplitPane {
                dir: args.dir,
                orientation: args.orientation,
            },
            ControlCommand::OpenWorktreeInPane(args) => {
                validate_nonempty_bounded("branch", &args.branch, MAX_SURFACE_ID_BYTES)?;
                ControlVerb::OpenWorktreeInPane {
                    repo: args.repo,
                    branch: args.branch,
                }
            }
            ControlCommand::FocusSession(args) => {
                let target = match (args.surface_id, args.host, args.session_id) {
                    (Some(surface_id), None, None) => {
                        validate_nonempty_bounded("surface id", &surface_id, MAX_SURFACE_ID_BYTES)?;
                        FocusSessionTarget::Surface { surface_id }
                    }
                    (None, Some(host), Some(session_id)) => {
                        validate_nonempty_bounded("host", &host, 512)?;
                        validate_nonempty_bounded("session id", &session_id, 512)?;
                        FocusSessionTarget::Fleet { host, session_id }
                    }
                    _ => bail!(
                        "focus-session requires either --surface-id or both --host and --session-id"
                    ),
                };
                ControlVerb::FocusSession { target }
            }
            ControlCommand::SendText(args) => {
                if args.text.len() > MAX_TEXT_BYTES {
                    bail!("text exceeds {MAX_TEXT_BYTES} bytes");
                }
                let surface_id = args.surface_id;
                validate_nonempty_bounded("surface id", &surface_id, MAX_SURFACE_ID_BYTES)?;
                ControlVerb::SendText {
                    surface_id,
                    text: args.text,
                    submit: args.submit,
                }
            }
        };
        Ok(Self {
            version: CONTROL_PROTOCOL_VERSION,
            auth,
            verb,
        })
    }

    pub fn hook_event(auth: ControlAuth, body: String) -> Result<Self> {
        auth.validate()?;
        if body.is_empty() {
            bail!("hook event must not be empty");
        }
        if body.len() > MAX_HOOK_EVENT_BYTES {
            bail!("hook event exceeds {MAX_HOOK_EVENT_BYTES} bytes");
        }
        Ok(Self {
            version: CONTROL_PROTOCOL_VERSION,
            auth,
            verb: ControlVerb::HookEvent { body },
        })
    }

    /// Revalidates a deserialized request at the server trust boundary.
    pub fn validate(&self) -> Result<()> {
        self.auth.validate()?;
        match &self.verb {
            ControlVerb::SplitPane { .. } => Ok(()),
            ControlVerb::OpenWorktreeInPane { branch, .. } => {
                validate_nonempty_bounded("branch", branch, MAX_SURFACE_ID_BYTES)
            }
            ControlVerb::FocusSession { target } => match target {
                FocusSessionTarget::Surface { surface_id } => {
                    validate_nonempty_bounded("surface id", surface_id, MAX_SURFACE_ID_BYTES)
                }
                FocusSessionTarget::Fleet { host, session_id } => {
                    validate_nonempty_bounded("host", host, 512)?;
                    validate_nonempty_bounded("session id", session_id, 512)
                }
            },
            ControlVerb::SendText {
                surface_id, text, ..
            } => {
                validate_nonempty_bounded("surface id", surface_id, MAX_SURFACE_ID_BYTES)?;
                if text.len() > MAX_TEXT_BYTES {
                    bail!("text exceeds {MAX_TEXT_BYTES} bytes");
                }
                Ok(())
            }
            ControlVerb::HookEvent { body } => {
                if body.is_empty() {
                    bail!("hook event must not be empty");
                }
                if body.len() > MAX_HOOK_EVENT_BYTES {
                    bail!("hook event exceeds {MAX_HOOK_EVENT_BYTES} bytes");
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlVerb {
    SplitPane {
        dir: Option<PathBuf>,
        orientation: ControlOrientation,
    },
    OpenWorktreeInPane {
        repo: PathBuf,
        branch: String,
    },
    FocusSession {
        target: FocusSessionTarget,
    },
    SendText {
        surface_id: String,
        text: String,
        submit: bool,
    },
    /// Internal typed transport for the self-managed CLI-agent hook bridge.
    HookEvent {
        body: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FocusSessionTarget {
    Surface { surface_id: String },
    Fleet { host: String, session_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlResponse {
    pub result: std::result::Result<ControlSuccess, ControlFailure>,
}

impl ControlResponse {
    pub fn success(success: ControlSuccess) -> Self {
        Self {
            result: Ok(success),
        }
    }

    pub fn failure(code: ControlFailureCode, message: impl Into<String>) -> Self {
        Self {
            result: Err(ControlFailure {
                code,
                message: message.into(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlSuccess {
    pub surface_id: Option<String>,
    pub tab_id: Option<String>,
}

impl ControlSuccess {
    pub fn empty() -> Self {
        Self {
            surface_id: None,
            tab_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlFailure {
    pub code: ControlFailureCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlFailureCode {
    Unauthorized,
    UnsupportedVersion,
    InvalidRequest,
    NotFound,
    Conflict,
    Internal,
}

pub struct ControlService;

#[async_trait::async_trait]
impl ipc::Service for ControlService {
    type Request = ControlRequest;
    type Response = ControlResponse;
}

pub fn socket_from_env() -> Result<String> {
    let socket =
        env::var(CONTROL_SOCKET_ENV).with_context(|| format!("{CONTROL_SOCKET_ENV} is not set"))?;
    validate_nonempty_bounded("control socket", &socket, 4096)?;
    Ok(socket)
}

fn validate_nonempty_bounded(name: &str, value: &str, max_bytes: usize) -> Result<()> {
    if value.is_empty() {
        bail!("{name} must not be empty");
    }
    if value.len() > max_bytes {
        bail!("{name} exceeds {max_bytes} bytes");
    }
    if value.contains('\0') {
        bail!("{name} contains a NUL byte");
    }
    Ok(())
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
