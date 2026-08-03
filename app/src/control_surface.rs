//! Authenticated, per-user control surface for local terminal panes.
//!
//! The socket is never exposed over TCP or a remote PTY. Every local PTY gets
//! a random surface-bound token, and requests must present the exact
//! `(token, surface_id, tab_id)` tuple injected into that PTY.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use futures::channel::oneshot;
use ipc::ServiceCaller as _;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;
use warp_cli::control::{
    ControlAuth, ControlCommand, ControlFailureCode, ControlOrientation, ControlRequest,
    ControlResponse, ControlService, ControlSuccess, ControlVerb, FocusSessionTarget,
    CONTROL_PROTOCOL_VERSION, CONTROL_SOCKET_ENV, CONTROL_TOKEN_ENV, SURFACE_ID_ENV, TAB_ID_ENV,
};
use warpui::r#async::executor::Background;
use warpui::{Entity, ModelContext, SingletonEntity, ViewHandle};

use crate::pane_group::{tree::Direction, PaneGroup};
use crate::terminal::TerminalView;
use crate::workspace::{Workspace, WorkspaceRegistry};

static CONTROL_ADDRESS: OnceLock<String> = OnceLock::new();
const CODEWHALE_TOOL_AUDIT_LOG_ENV: &str = "CODEWHALE_TOOL_AUDIT_LOG";

struct CodeWhaleAuditLog {
    path: PathBuf,
}

impl Drop for CodeWhaleAuditLog {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                log::warn!(
                    "Failed to remove CodeWhale audit log {}: {error}",
                    self.path.display()
                );
            }
        }
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

/// Secret-bearing identity injected into exactly one local PTY.
///
/// Deliberately does not implement `Debug`, so diagnostics cannot accidentally
/// print the bearer token.
#[derive(Clone)]
pub(crate) struct ControlPtyContext {
    socket: String,
    token: String,
    surface_id: String,
    tab_id: String,
    codewhale_audit_log: Option<Arc<CodeWhaleAuditLog>>,
}

impl ControlPtyContext {
    pub(crate) fn new(tab_id: String) -> Self {
        let surface_id = Uuid::new_v4().to_string();
        Self {
            socket: control_address().to_string(),
            token: format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple()),
            codewhale_audit_log: prepare_codewhale_audit_log(&surface_id).map(Arc::new),
            surface_id,
            tab_id,
        }
    }

    pub(crate) fn inject_env(&mut self, env: &mut HashMap<OsString, OsString>) {
        for (name, value) in self.env_pairs() {
            env.insert(name.to_os_string(), OsString::from(value));
        }
        if env.contains_key(OsStr::new(CODEWHALE_TOOL_AUDIT_LOG_ENV)) {
            // An explicit user-owned audit destination remains authoritative.
            // Zaplex cannot safely tail an unrelated file, so disable only the
            // pane-local audit bridge while leaving native hooks operational.
            self.codewhale_audit_log = None;
        } else if let Some(audit_log) = self.codewhale_audit_log.as_ref() {
            env.insert(
                OsString::from(CODEWHALE_TOOL_AUDIT_LOG_ENV),
                audit_log.path.as_os_str().to_os_string(),
            );
        }
    }

    fn env_pairs(&self) -> [(&'static OsStr, &str); 4] {
        [
            (OsStr::new(CONTROL_SOCKET_ENV), self.socket.as_str()),
            (OsStr::new(CONTROL_TOKEN_ENV), self.token.as_str()),
            (OsStr::new(SURFACE_ID_ENV), self.surface_id.as_str()),
            (OsStr::new(TAB_ID_ENV), self.tab_id.as_str()),
        ]
    }

    pub(crate) fn surface_id(&self) -> &str {
        &self.surface_id
    }

    pub(crate) fn tab_id(&self) -> &str {
        &self.tab_id
    }

    pub(crate) fn codewhale_audit_log_path(&self) -> Option<&Path> {
        self.codewhale_audit_log
            .as_deref()
            .map(|audit_log| audit_log.path.as_path())
    }

    pub(crate) fn authenticates(&self, auth: &ControlAuth) -> bool {
        constant_time_eq(self.token.as_bytes(), auth.token.as_bytes())
            && self.surface_id == auth.caller_surface_id
            && self.tab_id == auth.caller_tab_id
    }
}

fn prepare_codewhale_audit_log(surface_id: &str) -> Option<CodeWhaleAuditLog> {
    let directory = crate::warp_managed_paths_watcher::warp_data_dir().join("codewhale-tool-audit");
    if let Err(error) = std::fs::create_dir_all(&directory) {
        log::warn!(
            "Failed to create CodeWhale audit directory {}: {error}",
            directory.display()
        );
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;

        if let Err(error) =
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
        {
            log::warn!(
                "Failed to secure CodeWhale audit directory {}: {error}",
                directory.display()
            );
            return None;
        }
    }

    let path = directory.join(format!("{surface_id}.jsonl"));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(_) => Some(CodeWhaleAuditLog { path }),
        Err(error) => {
            log::warn!(
                "Failed to create CodeWhale audit log {}: {error}",
                path.display()
            );
            None
        }
    }
}

fn constant_time_eq(expected: &[u8], candidate: &[u8]) -> bool {
    if expected.len() != candidate.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (expected, candidate) in expected.iter().zip(candidate) {
        difference |= expected ^ candidate;
    }
    difference == 0
}

fn control_address() -> &'static str {
    CONTROL_ADDRESS.get_or_init(|| {
        let nonce = Uuid::new_v4().simple().to_string();
        #[cfg(unix)]
        {
            // The IPC listener creates this parent as 0700 before binding, so
            // the socket is never reachable during its later chmod to 0600.
            // Keep the full path below the conservative Unix-domain limit.
            std::env::temp_dir()
                .join(format!("zplx-ctl-{}-{}", std::process::id(), &nonce[..12]))
                .join("control.sock")
                .to_string_lossy()
                .into_owned()
        }
        #[cfg(windows)]
        {
            // `interprocess` maps this to a named pipe and rejects remote
            // clients by default. The surface-bound bearer token is the
            // per-user authorization boundary for local clients.
            format!("Zaplex_Control_{}_{}", std::process::id(), &nonce[..12])
        }
        #[cfg(not(any(unix, windows)))]
        {
            format!("/tmp/zplx-ctl-{}-{}.sock", std::process::id(), &nonce[..12])
        }
    })
}

struct ControlEnvelope {
    request: ControlRequest,
    response_tx: oneshot::Sender<ControlResponse>,
}

#[derive(Clone)]
struct ControlServiceImpl {
    request_tx: async_channel::Sender<ControlEnvelope>,
}

#[async_trait]
impl ipc::ServiceImpl for ControlServiceImpl {
    type Service = ControlService;

    async fn handle_request(&self, request: ControlRequest) -> ControlResponse {
        let (response_tx, response_rx) = oneshot::channel();
        if self
            .request_tx
            .send(ControlEnvelope {
                request,
                response_tx,
            })
            .await
            .is_err()
        {
            return internal_failure("control service is unavailable");
        }
        response_rx
            .await
            .unwrap_or_else(|_| internal_failure("control request was cancelled"))
    }
}

pub(crate) struct ControlSurfaceServer {
    _server: Option<ipc::Server>,
}

impl ControlSurfaceServer {
    pub(crate) fn new(ctx: &mut ModelContext<Self>) -> Self {
        let (request_tx, request_rx) = async_channel::unbounded();
        let server = match ipc::ServerBuilder::default()
            .with_fixed_address(control_address().to_string())
            .with_service(ControlServiceImpl { request_tx })
            .build_and_run(ctx.background_executor())
        {
            Ok((server, _)) => server,
            Err(error) => {
                log::error!("Failed to initialize the local control surface: {error:?}");
                return Self { _server: None };
            }
        };

        ctx.spawn_stream_local(
            request_rx,
            |server, envelopes, ctx| {
                for envelope in envelopes {
                    server.handle_request(envelope, ctx);
                }
            },
            |_, _| {},
        );
        Self {
            _server: Some(server),
        }
    }

    fn handle_request(&mut self, envelope: ControlEnvelope, ctx: &mut ModelContext<Self>) {
        let ControlEnvelope {
            request,
            response_tx,
        } = envelope;
        if request.version != CONTROL_PROTOCOL_VERSION {
            let _ = response_tx.send(ControlResponse::failure(
                ControlFailureCode::UnsupportedVersion,
                "unsupported control protocol version",
            ));
            return;
        }
        if request.auth.validate().is_err() {
            let _ = response_tx.send(unauthorized_failure());
            return;
        }

        let mut callers = WorkspaceRegistry::as_ref(ctx)
            .all_workspaces(ctx)
            .into_iter()
            .flat_map(|(_, workspace)| {
                workspace
                    .as_ref(ctx)
                    .control_surface_matches(workspace.clone(), &request.auth, ctx)
            })
            .collect::<Vec<_>>();
        if callers.len() != 1 {
            let _ = response_tx.send(unauthorized_failure());
            return;
        }
        let caller = callers.pop().expect("length checked");
        if request.validate().is_err() {
            let _ = response_tx.send(ControlResponse::failure(
                ControlFailureCode::InvalidRequest,
                "invalid control request",
            ));
            return;
        }

        match request.verb {
            ControlVerb::SplitPane { dir, orientation } => {
                if let Some(dir) = dir.as_deref() {
                    if !dir.is_dir() {
                        let _ = response_tx.send(ControlResponse::failure(
                            ControlFailureCode::InvalidRequest,
                            "split directory does not exist",
                        ));
                        return;
                    }
                }
                let result = caller.pane_group.update(ctx, |pane_group, ctx| {
                    pane_group.add_control_terminal_pane(orientation.into(), dir, false, ctx)
                });
                let _ = response_tx.send(surface_result(result));
            }
            ControlVerb::OpenWorktreeInPane { repo, branch } => {
                self.open_worktree(caller.pane_group, repo, branch, response_tx, ctx);
            }
            ControlVerb::FocusSession { target } => match target {
                FocusSessionTarget::Surface { surface_id } => {
                    let mut targets = WorkspaceRegistry::as_ref(ctx)
                        .all_workspaces(ctx)
                        .into_iter()
                        .flat_map(|(_, workspace)| {
                            workspace.as_ref(ctx).control_surface_id_matches(
                                workspace.clone(),
                                &surface_id,
                                ctx,
                            )
                        })
                        .collect::<Vec<_>>();
                    let response = if targets.len() == 1 {
                        let target = targets.pop().expect("length checked");
                        let focused = caller.workspace.update(ctx, |workspace, ctx| {
                            workspace.focus_control_terminal(target.terminal.id(), ctx)
                        });
                        if focused {
                            ControlResponse::success(ControlSuccess {
                                surface_id: Some(surface_id),
                                tab_id: Some(target.context.tab_id().to_string()),
                            })
                        } else {
                            not_found_failure("terminal surface is no longer available")
                        }
                    } else {
                        not_found_failure("terminal surface was not found uniquely")
                    };
                    let _ = response_tx.send(response);
                }
                FocusSessionTarget::Fleet { host, session_id } => {
                    let focused = caller.workspace.update(ctx, |workspace, ctx| {
                        workspace.focus_control_fleet_session(&host, &session_id, ctx)
                    });
                    let response = if focused {
                        ControlResponse::success(ControlSuccess::empty())
                    } else {
                        not_found_failure("fleet session was not found uniquely")
                    };
                    let _ = response_tx.send(response);
                }
            },
            ControlVerb::SendText {
                surface_id,
                text,
                submit,
            } => {
                let mut targets = WorkspaceRegistry::as_ref(ctx)
                    .all_workspaces(ctx)
                    .into_iter()
                    .flat_map(|(_, workspace)| {
                        workspace.as_ref(ctx).control_surface_id_matches(
                            workspace.clone(),
                            &surface_id,
                            ctx,
                        )
                    })
                    .collect::<Vec<_>>();
                let response = if targets.len() == 1 {
                    let target = targets.pop().expect("length checked");
                    let accepted = target.terminal.update(ctx, |terminal, ctx| {
                        terminal.input().update(ctx, |input, ctx| {
                            input.replace_buffer_content(&text, ctx);
                            input.focus_input_box(ctx);
                            if submit {
                                input.try_execute_command(&text, ctx)
                            } else {
                                true
                            }
                        })
                    });
                    if accepted {
                        ControlResponse::success(ControlSuccess {
                            surface_id: Some(surface_id),
                            tab_id: Some(target.context.tab_id().to_string()),
                        })
                    } else {
                        ControlResponse::failure(
                            ControlFailureCode::Conflict,
                            "terminal input did not accept the submitted text",
                        )
                    }
                } else {
                    not_found_failure("terminal surface was not found uniquely")
                };
                let _ = response_tx.send(response);
            }
            ControlVerb::HookEvent { body } => {
                caller.terminal.update(ctx, |terminal, ctx| {
                    terminal.handle_control_hook_event(&body, ctx);
                });
                let _ = response_tx.send(ControlResponse::success(ControlSuccess {
                    surface_id: Some(caller.context.surface_id().to_string()),
                    tab_id: Some(caller.context.tab_id().to_string()),
                }));
            }
        }
    }

    fn open_worktree(
        &mut self,
        pane_group: ViewHandle<PaneGroup>,
        repo: PathBuf,
        branch: String,
        response_tx: oneshot::Sender<ControlResponse>,
        ctx: &mut ModelContext<Self>,
    ) {
        let future = create_or_attach_worktree(repo, branch);
        ctx.spawn(future, move |_, result, ctx| {
            let response = match result {
                Ok(worktree) => {
                    let surface = pane_group.update(ctx, |pane_group, ctx| {
                        pane_group.add_control_terminal_pane(
                            Direction::Right,
                            Some(worktree),
                            true,
                            ctx,
                        )
                    });
                    surface_result(surface)
                }
                Err(error) => {
                    ControlResponse::failure(ControlFailureCode::InvalidRequest, error.to_string())
                }
            };
            let _ = response_tx.send(response);
        });
    }
}

impl Entity for ControlSurfaceServer {
    type Event = ();
}

impl SingletonEntity for ControlSurfaceServer {}

pub(crate) struct ControlSurfaceMatch {
    pub(crate) workspace: ViewHandle<Workspace>,
    pub(crate) pane_group: ViewHandle<PaneGroup>,
    pub(crate) terminal: ViewHandle<TerminalView>,
    pub(crate) context: ControlPtyContext,
}

pub(crate) struct PaneControlSurfaceMatch {
    pub(crate) terminal: ViewHandle<TerminalView>,
    pub(crate) context: ControlPtyContext,
}

impl From<ControlOrientation> for Direction {
    fn from(value: ControlOrientation) -> Self {
        match value {
            ControlOrientation::Left => Self::Left,
            ControlOrientation::Right => Self::Right,
            ControlOrientation::Up => Self::Up,
            ControlOrientation::Down => Self::Down,
        }
    }
}

async fn create_or_attach_worktree(repo: PathBuf, branch: String) -> Result<PathBuf> {
    let repo = repo
        .canonicalize()
        .with_context(|| format!("repository does not exist: {}", repo.display()))?;
    let root = crate::util::git::run_git_command(&repo, &["rev-parse", "--show-toplevel"])
        .await
        .context("path is not a git repository")?;
    let root = PathBuf::from(root.trim())
        .canonicalize()
        .context("git repository root is not accessible")?;
    crate::util::git::run_git_command(&root, &["check-ref-format", "--branch", &branch])
        .await
        .context("invalid branch name")?;

    let worktree = crate::tab_configs::tab_config::generated_worktree_path(
        &root,
        &worktree_directory_name(&root, &branch),
    );
    if worktree.exists() {
        let existing_branch =
            crate::util::git::run_git_command(&worktree, &["branch", "--show-current"]).await?;
        if existing_branch.trim() == branch {
            return Ok(worktree);
        }
        bail!("worktree destination exists for a different branch");
    }
    if let Some(parent) = worktree.parent() {
        std::fs::create_dir_all(parent).context("failed to create worktree parent directory")?;
    }

    let worktree_arg = worktree
        .to_str()
        .context("worktree path is not valid UTF-8")?;
    let branch_exists = crate::util::git::run_git_command(
        &root,
        &[
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .await
    .is_ok();
    if branch_exists {
        crate::util::git::run_git_command(&root, &["worktree", "add", worktree_arg, &branch])
            .await?;
    } else {
        let base = resolve_worktree_base(&root).await?;
        crate::util::git::run_git_command(
            &root,
            &["worktree", "add", "-b", &branch, worktree_arg, &base],
        )
        .await?;
    }
    Ok(worktree)
}

async fn resolve_worktree_base(repo: &Path) -> Result<String> {
    if let Ok(remote_head) =
        crate::util::git::run_git_command(repo, &["symbolic-ref", "refs/remotes/origin/HEAD"]).await
    {
        let remote_head = remote_head.trim();
        if !remote_head.is_empty()
            && crate::util::git::run_git_command(
                repo,
                &[
                    "rev-parse",
                    "--verify",
                    &format!("{remote_head}^{{commit}}"),
                ],
            )
            .await
            .is_ok()
        {
            return Ok(remote_head.to_string());
        }
    }

    for candidate in ["origin/main", "origin/master", "main", "master", "develop"] {
        if crate::util::git::run_git_command(
            repo,
            &["rev-parse", "--verify", &format!("{candidate}^{{commit}}")],
        )
        .await
        .is_ok()
        {
            return Ok(candidate.to_string());
        }
    }

    bail!(
        "cannot create branch: repository has no explicit default base (origin/HEAD, main, master, or develop)"
    )
}

fn worktree_directory_name(repo: &Path, branch: &str) -> String {
    let mut sanitized = branch
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    while sanitized.contains("..") {
        sanitized = sanitized.replace("..", ".");
    }
    sanitized = sanitized.trim_matches(['.', '-']).to_string();
    if sanitized.is_empty() {
        sanitized.push_str("branch");
    }
    sanitized.truncate(72);

    let mut hasher = Sha256::new();
    hasher.update(repo.to_string_lossy().as_bytes());
    hasher.update([0]);
    hasher.update(branch.as_bytes());
    let digest = hasher.finalize();
    format!(
        "{sanitized}-{:02x}{:02x}{:02x}{:02x}",
        digest[0], digest[1], digest[2], digest[3]
    )
}

fn surface_result(context: Option<ControlPtyContext>) -> ControlResponse {
    match context {
        Some(context) => ControlResponse::success(ControlSuccess {
            surface_id: Some(context.surface_id().to_string()),
            tab_id: Some(context.tab_id().to_string()),
        }),
        None => internal_failure("new terminal surface was not created"),
    }
}

fn unauthorized_failure() -> ControlResponse {
    ControlResponse::failure(
        ControlFailureCode::Unauthorized,
        "control authorization failed",
    )
}

fn not_found_failure(message: &str) -> ControlResponse {
    ControlResponse::failure(ControlFailureCode::NotFound, message)
}

fn internal_failure(message: &str) -> ControlResponse {
    ControlResponse::failure(ControlFailureCode::Internal, message)
}

pub(crate) fn run_control_command(command: ControlCommand) -> Result<()> {
    let auth = ControlAuth::from_env()?;
    let request = ControlRequest::from_cli(auth, command)?;
    let response = send_control_request(request)?;
    match response.result {
        Ok(success) => {
            if let Some(surface_id) = success.surface_id {
                println!("{surface_id}");
            }
            Ok(())
        }
        Err(failure) => bail!("{}: {}", failure_code_name(failure.code), failure.message),
    }
}

pub(crate) fn forward_hook_event(body: Vec<u8>) -> Result<()> {
    let body = String::from_utf8(body).context("normalized hook event is not UTF-8")?;
    let request = ControlRequest::hook_event(ControlAuth::from_env()?, body)?;
    let response = send_control_request(request)?;
    match response.result {
        Ok(_) => Ok(()),
        Err(failure) => bail!("{}: {}", failure_code_name(failure.code), failure.message),
    }
}

/// Forwards a hook event when the agent is running inside a Zaplex-managed PTY.
///
/// Hooks are installed in user-global agent configuration, so the same command
/// also runs in ordinary terminals. With no Zaplex control environment it must
/// be a no-op rather than breaking the agent's native hook or title pipeline.
pub(crate) fn forward_hook_event_if_available(body: Vec<u8>) -> Result<()> {
    let variables = [
        CONTROL_SOCKET_ENV,
        CONTROL_TOKEN_ENV,
        SURFACE_ID_ENV,
        TAB_ID_ENV,
    ];
    if variables
        .iter()
        .all(|name| std::env::var_os(name).is_none())
    {
        return Ok(());
    }
    forward_hook_event(body)
}

fn send_control_request(request: ControlRequest) -> Result<ControlResponse> {
    let address = warp_cli::control::socket_from_env()?;
    warpui::r#async::block_on(async move {
        let executor = Arc::new(Background::default());
        let client = Arc::new(
            ipc::Client::connect(address.into(), executor)
                .await
                .context("failed to connect to the local Zaplex control socket")?,
        );
        ipc::service_caller::<ControlService>(client)
            .call(request)
            .await
            .context("local Zaplex control request failed")
    })
}

fn failure_code_name(code: ControlFailureCode) -> &'static str {
    match code {
        ControlFailureCode::Unauthorized => "unauthorized",
        ControlFailureCode::UnsupportedVersion => "unsupported_version",
        ControlFailureCode::InvalidRequest => "invalid_request",
        ControlFailureCode::NotFound => "not_found",
        ControlFailureCode::Conflict => "conflict",
        ControlFailureCode::Internal => "internal",
    }
}

#[cfg(test)]
#[path = "control_surface_tests.rs"]
mod tests;
