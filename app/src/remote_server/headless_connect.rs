//! Headless SSH ControlMaster setup for daemon-hosted sessions (Stage 2, Option B).
//!
//! A resilient SSH host (`session_resilience.is_enabled()`) opens directly as a
//! daemon-hosted session — there is no interactive `ssh` PTY whose zaplexify
//! bootstrap would establish the ControlMaster. So we establish it ourselves
//! (`ssh -f -N -o ControlMaster=auto -o ControlPath=<socket> …`) and hand the
//! socket to [`SshTransport`](super::ssh_transport::SshTransport) +
//! `RemoteServerManager::connect_session`.
//!
//! v1 supports **key/agent auth** only (clean headless, `BatchMode=yes`).
//! Password-auth hosts fall back to the normal (non-daemon) SSH path — see the
//! caller in `app/src/workspace/view.rs`. See
//! `docs/superpowers/specs/2026-06-27-stage2-increment3c-daemon-trigger-design.md`.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures::lock::Mutex as AsyncMutex;
use remote_server::auth::RemoteServerAuthContext;
use remote_server::proto::{
    AgentSessionInfo, AgentSessionList, InitializeResponse, MultiplexerSessionList, SessionList,
};
use remote_server::transport::{Connection, RemoteTransport};
use warp_core::{channel::ChannelState, SessionId};
use warp_ssh_manager::{
    build_ssh_args, preflight_host_key_with_factory, validate_ssh_endpoint, AuthType,
    DefaultWorkspaceCommandFactory, EndpointUse, HostKeyPreflight, ResolvedSshConnection,
    SshServerInfo, WorkspaceCommandFactory,
};
use warpui::r#async::{executor::Background, FutureExt as _};
use zaplex_remote_session::types::{
    has_feature, FEATURE_AGENT_INVENTORY, FEATURE_MULTIPLEXER_INVENTORY_V1, FEATURE_SESSION_HOST,
};

use super::session_inventory::{HostSessionInventory, RoutedDaemonSession};
use super::ssh_transport::{DaemonRuntimeRoute, SshTransport};

/// Daemon sessions are allocated `SessionId`s in the **top half** of the u64
/// space so they cannot collide with shell-bootstrap-minted ids (which are
/// PID/timestamp-derived and stay well below `2^63`). The manager keys all
/// sessions — interactive and daemon — by `SessionId`, so uniqueness matters.
const DAEMON_SESSION_ID_BASE: u64 = 1 << 63;
const SESSION_INVENTORY_TIMEOUT: Duration = Duration::from_secs(30);
static NEXT_DAEMON_SESSION_ID: AtomicU64 = AtomicU64::new(1);
static CONTROL_MASTER_LOCKS: LazyLock<Mutex<HashMap<PathBuf, Weak<AsyncMutex<()>>>>> =
    LazyLock::new(Mutex::default);

fn supports_multiplexer_inventory(response: &InitializeResponse) -> bool {
    has_feature(&response.features, FEATURE_MULTIPLEXER_INVENTORY_V1)
}

fn supports_agent_inventory(response: &InitializeResponse) -> bool {
    has_feature(&response.features, FEATURE_AGENT_INVENTORY)
}

fn supports_session_host(response: &InitializeResponse) -> bool {
    has_feature(&response.features, FEATURE_SESSION_HOST)
}

fn parse_release_daemon_version(version: &str) -> Option<semver::Version> {
    let version = version.strip_prefix('v').unwrap_or(version);
    let mut parsed = semver::Version::parse(version).ok().or_else(|| {
        let parts = version.split('.').collect::<Vec<_>>();
        match parts.as_slice() {
            [major, minor] => format!("{major}.{minor}.0").parse().ok(),
            [major, minor, suffix]
                if suffix
                    .strip_prefix("dev")
                    .or_else(|| suffix.strip_prefix("rc"))
                    .is_some_and(|number| {
                        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
                    }) =>
            {
                format!("{major}.{minor}.0-{suffix}").parse().ok()
            }
            _ => None,
        }
    })?;
    // Release tags also use rcN/devN (and alphaN/betaN). Split the numeric
    // component so rc10 follows rc2, and dotted legacy tags compare equally.
    for label in ["alpha", "beta", "dev", "rc"] {
        if let Some(number) = parsed.pre.as_str().strip_prefix(label) {
            if !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()) {
                parsed.pre = format!("{label}.{number}").parse().ok()?;
                break;
            }
        }
    }
    Some(parsed)
}

fn is_older_release_daemon(current: Option<&str>, observed: &str) -> bool {
    match (
        current.and_then(parse_release_daemon_version),
        parse_release_daemon_version(observed),
    ) {
        (Some(current), Some(observed)) => observed.cmp_precedence(&current).is_lt(),
        (None, _) | (Some(_), None) => false,
    }
}

fn agent_provider_display_name(provider: &str) -> &str {
    if provider.eq_ignore_ascii_case("codex") {
        "Codex"
    } else if provider.eq_ignore_ascii_case("claude") {
        "Claude"
    } else {
        provider
    }
}

fn agent_session_display_title(session: &AgentSessionInfo) -> String {
    let provider = agent_provider_display_name(&session.provider);
    let identity = [&session.name, &session.project_name]
        .into_iter()
        .find(|value| !value.is_empty() && !value.eq_ignore_ascii_case(provider));
    identity
        .map(|identity| format!("{provider} · {identity}"))
        .unwrap_or_else(|| provider.to_string())
}

fn enrich_daemon_session_titles(daemon: &mut SessionList, agents: &AgentSessionList) {
    for session in &mut daemon.sessions {
        let agent = agents.sessions.iter().find(|agent| {
            agent.pty_foreground
                && agent.pty_session_id == session.session_id
                && agent.pty_session_generation == session.generation
        });
        session.title = agent.map(agent_session_display_title).unwrap_or_else(|| {
            std::path::Path::new(&session.cwd)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    }
}

fn neutralize_legacy_session_titles(daemon: &mut SessionList) {
    for session in &mut daemon.sessions {
        session.title.clear();
        session.managed = None;
    }
}

/// Allocates a fresh, collision-safe `SessionId` for a daemon-hosted session.
pub fn alloc_daemon_session_id() -> SessionId {
    let n = NEXT_DAEMON_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    SessionId::from(DAEMON_SESSION_ID_BASE | n)
}

/// Whether this (already auth-resolved) host can be connected headlessly.
///
/// v1: key auth only — it runs non-interactively under `BatchMode=yes` (with an
/// ssh-agent or an unencrypted key). Password auth needs an interactive prompt
/// we don't have here, so those hosts use the normal SSH path instead.
pub fn is_headless_capable(server: &SshServerInfo) -> bool {
    matches!(server.auth_type, AuthType::Key)
}

/// Headless predicate for an agent route after repository authentication has
/// been resolved exactly once.
pub fn is_agent_route_headless_capable(connection: &ResolvedSshConnection) -> bool {
    is_headless_capable(&connection.server)
}

/// FNV-1a, used only to derive a short, stable, run-to-run-consistent socket
/// name per host (so multiple tabs to the same host share one master).
fn stable_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Local path for the ControlMaster socket. Uses a real (`$HOME`-expanded) path
/// so both `ssh -o ControlPath=` and our existence check agree (ssh would expand
/// `~` itself, but `Path::exists` would not).
pub fn control_socket_path(server: &SshServerInfo) -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let key = format!("{}@{}:{}", server.username, server.host, server.port);
    home.join(".ssh")
        .join(format!("zaplex-daemon-{:016x}", stable_hash(&key)))
}

fn managed_known_hosts_path(server: &SshServerInfo) -> Result<PathBuf> {
    let endpoint =
        validate_ssh_endpoint(EndpointUse::Connect, &server.host, &server.port.to_string())
            .map_err(|error| anyhow!(error))?;
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("HOME is unavailable for managed SSH host keys"))?;
    let key = format!("{}:{}", endpoint.host, endpoint.port);
    Ok(home
        .join(".ssh")
        .join(format!("zaplex-known-host-{:016x}", stable_hash(&key))))
}

pub fn confirm_host_key(
    server: &SshServerInfo,
    host_key: &warp_ssh_manager::UnknownHostKey,
) -> std::result::Result<PathBuf, String> {
    let path = managed_known_hosts_path(server)
        .map_err(|error| format!("Failed to resolve managed SSH host-key path: {error:#}"))?;
    confirm_host_key_at(server, host_key, &path)?;
    Ok(path)
}

fn confirm_host_key_at(
    server: &SshServerInfo,
    host_key: &warp_ssh_manager::UnknownHostKey,
    path: &Path,
) -> std::result::Result<(), String> {
    warp_ssh_manager::persist_confirmed_host_key(server, host_key, path)
}

/// Whether a live ControlMaster is serving `socket_path` — `ssh -O check`
/// returns success only when the master process is actually alive (a stale
/// socket file fails the check). Runs entirely over the local Unix socket, so
/// it returns quickly; bounded by a short timeout regardless.
async fn control_master_alive(
    socket_path: &Path,
    command_factory: &dyn WorkspaceCommandFactory,
) -> bool {
    let mut cmd = command_factory.async_command("ssh");
    cmd.arg("-O")
        .arg("check")
        .arg("-o")
        .arg(format!("ControlPath={}", socket_path.display()))
        .arg("placeholder@placeholder")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    matches!(
        tokio::time::timeout(Duration::from_secs(5), cmd.output()).await,
        Ok(Ok(output)) if output.status.success()
    )
}

fn normalized_socket_path(socket_path: &Path) -> PathBuf {
    let Some(file_name) = socket_path.file_name() else {
        return socket_path.to_path_buf();
    };
    socket_path
        .parent()
        .and_then(|parent| parent.canonicalize().ok())
        .map(|parent| parent.join(file_name))
        .unwrap_or_else(|| socket_path.to_path_buf())
}

fn control_master_lock(socket_path: &Path) -> Arc<AsyncMutex<()>> {
    let key = normalized_socket_path(socket_path);
    let mut locks = CONTROL_MASTER_LOCKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(AsyncMutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

fn control_master_args(
    server: &SshServerInfo,
    socket_path: &Path,
    known_hosts_path: Option<&Path>,
) -> Result<Vec<String>> {
    let endpoint =
        validate_ssh_endpoint(EndpointUse::Connect, &server.host, &server.port.to_string())
            .map_err(|error| anyhow!(error))?;
    let mut validated_server = server.clone();
    validated_server.host = endpoint.host;
    validated_server.port = endpoint.port;

    let mut args: Vec<String> = build_ssh_args(&validated_server)
        .into_iter()
        .skip(1)
        .collect();
    let destination_delimiter = args
        .iter()
        .position(|arg| arg == "--")
        .ok_or_else(|| anyhow!("SSH destination delimiter is missing"))?;
    let mut master_args = Vec::new();
    if let Some(known_hosts_path) = known_hosts_path {
        let strict_host_key_checking = args
            .iter_mut()
            .find(|arg| arg.starts_with("StrictHostKeyChecking="))
            .ok_or_else(|| anyhow!("SSH host-key policy is missing"))?;
        *strict_host_key_checking = "StrictHostKeyChecking=yes".to_string();
        master_args.extend([
            "-o".to_string(),
            format!("UserKnownHostsFile={}", known_hosts_path.display()),
        ]);
    }
    master_args.extend([
        "-f".into(), // background after authentication
        "-N".into(), // no remote command — pure multiplexing master
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        // Idle timeout, NOT `yes`: `yes` keeps the backgrounded master alive
        // forever (it even survives app exit, since `-f` detaches it), and daemon
        // sessions no longer stop it on tab close (it's a shared per-host master).
        // A timeout lets it self-retire after the last client goes idle, while
        // still being reused for reconnects / new tabs within the window. The
        // remote daemon session is independent of the master and survives either way.
        "ControlPersist=600".into(),
        "-o".into(),
        format!("ControlPath={}", socket_path.display()),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
    ]);
    args.splice(destination_delimiter..destination_delimiter, master_args);
    Ok(args)
}

/// Ensures a ControlMaster is up at `socket_path` (idempotent via
/// `ControlMaster=auto` + `ControlPersist`). Spawns `ssh -f -N …`, which
/// authenticates and then backgrounds itself; the master socket exists by the
/// time the foreground process exits. Key/agent auth only (`BatchMode=yes`).
pub async fn ensure_control_master(server: &SshServerInfo, socket_path: &Path) -> Result<()> {
    let managed_known_hosts = managed_known_hosts_path(server)?;
    let managed_known_hosts = managed_known_hosts.exists().then_some(managed_known_hosts);
    ensure_control_master_with_factory(
        server,
        socket_path,
        managed_known_hosts.as_deref(),
        &DefaultWorkspaceCommandFactory,
    )
    .await
}

async fn ensure_control_master_with_factory(
    server: &SshServerInfo,
    socket_path: &Path,
    known_hosts_path: Option<&Path>,
    command_factory: &dyn WorkspaceCommandFactory,
) -> Result<()> {
    let args = control_master_args(server, socket_path, known_hosts_path)?;
    let lock = control_master_lock(socket_path);
    let _guard = lock.lock().await;

    if control_master_alive(socket_path, command_factory).await {
        return Ok(());
    }
    if socket_path.exists() {
        // A socket file is present, but the master may have died on an SSH drop,
        // leaving a stale socket. Verify it's actually serving: reuse a live
        // master, otherwise remove the stale socket and spawn a fresh one. This
        // is what lets a daemon session's transport be re-established after a
        // connection loss (the session itself kept running daemon-side).
        log::info!(
            "ControlMaster socket {} is stale; re-establishing",
            socket_path.display()
        );
        let _ = std::fs::remove_file(socket_path);
    }

    let mut command = command_factory.async_command("ssh");
    command
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(20), command.output())
        .await
        .map_err(|_| anyhow!("ControlMaster setup timed out"))?
        .map_err(|error| anyhow!("failed to spawn ssh: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if known_hosts_path.is_some()
            && (stderr.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
                || stderr.contains("Host key verification failed"))
        {
            return Err(anyhow!(HOST_KEY_CHANGED));
        }
        return Err(anyhow!("ControlMaster setup failed: {}", stderr.trim()));
    }

    // `-f` returns once the master is backgrounded. Verify the new master instead
    // of trusting socket visibility, which can also describe a stale endpoint.
    for _ in 0..20 {
        if control_master_alive(socket_path, command_factory).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(anyhow!(
        "ControlMaster did not become responsive at {}",
        socket_path.display()
    ))
}

/// Sentinel error returned by [`prepare_daemon_transport`] when the remote-server
/// binary is not present on the host. The caller uses this to fall back to a classic
/// SSH session (with a warning) instead of hanging on an install-on-connect.
pub const DAEMON_BINARY_MISSING: &str = "daemon-binary-missing";
pub const HOST_KEY_CHANGED: &str = "SSH host key changed; connection blocked";

/// Outcome of [`preflight_daemon_transport`]: the daemon is ready to connect,
/// absent but installable, or waiting for explicit host-key confirmation.
pub enum DaemonPreflight {
    /// The remote-server binary is present — connect right away.
    Ready,
    /// Binary missing, but an install source exists (dev cross-compile, bundled
    /// tarball, or reachable release asset). An explicit connection may install
    /// it; inventory requests must ask the user to connect first.
    NeedsInstall,
    /// The endpoint presented an unknown key. The ControlMaster has not been
    /// started; the caller must obtain explicit confirmation first.
    HostKeyConfirmationRequired(warp_ssh_manager::UnknownHostKey),
}

async fn preflight_control_master_host_key_with_factory(
    server: &SshServerInfo,
    known_hosts_path: &Path,
    command_factory: &dyn WorkspaceCommandFactory,
) -> std::result::Result<HostKeyPreflight, String> {
    if known_hosts_path.exists() {
        return Ok(HostKeyPreflight::Verified);
    }
    preflight_host_key_with_factory(server, command_factory).await
}

/// Fast, bounded preflight for a daemon connect: verifies the endpoint identity,
/// brings up the headless ControlMaster only after that succeeds, and classifies
/// the host (binary present / installable / unavailable). No install work happens
/// here, so the caller can create the daemon tab *first* and stream install
/// progress into it.
pub async fn preflight_daemon_transport(
    server: SshServerInfo,
    socket_path: PathBuf,
    auth_context: Arc<RemoteServerAuthContext>,
) -> std::result::Result<DaemonPreflight, String> {
    let host = server.host.clone();
    let known_hosts_path = managed_known_hosts_path(&server)
        .map_err(|error| format!("SSH host-key preflight failed: {error:#}"))?;
    match preflight_control_master_host_key_with_factory(
        &server,
        &known_hosts_path,
        &DefaultWorkspaceCommandFactory,
    )
    .await
    .map_err(|error| format!("SSH host-key preflight failed: {error}"))?
    {
        HostKeyPreflight::Verified => {}
        HostKeyPreflight::ConfirmationRequired(host_key) => {
            return Ok(DaemonPreflight::HostKeyConfirmationRequired(host_key));
        }
        HostKeyPreflight::Changed => {
            return Err(HOST_KEY_CHANGED.to_string());
        }
    }
    log::info!("daemon connect [{host}]: establishing ControlMaster");
    ensure_control_master(&server, &socket_path)
        .await
        .map_err(format_control_master_setup_error)?;
    let transport = SshTransport::new(socket_path, auth_context);
    log::info!("daemon connect [{host}]: checking remote-server binary");
    match transport.check_binary().await {
        Ok(true) => {
            log::info!("daemon connect [{host}]: binary present");
            Ok(DaemonPreflight::Ready)
        }
        // Binary missing → auto-install on first connect (like Warp), IF it can
        // actually be sourced (dev cross-compile, bundled tarball, or a reachable
        // version-matched release asset). Otherwise fail fast with the
        // DAEMON_BINARY_MISSING sentinel so the caller opens a classic SSH session
        // with a warning — never a multi-minute stall on a doomed download.
        Ok(false) => {
            if transport.install_source_available().await {
                log::info!("daemon connect [{host}]: binary missing — install source available");
                Ok(DaemonPreflight::NeedsInstall)
            } else {
                log::info!(
                    "daemon connect [{host}]: remote-server binary missing and no install \
                     source available — falling back to classic SSH"
                );
                Err(DAEMON_BINARY_MISSING.to_string())
            }
        }
        Err(e) => Err(format!("remote-server binary check failed: {e}")),
    }
}

fn format_control_master_setup_error(error: anyhow::Error) -> String {
    if error.to_string().contains(HOST_KEY_CHANGED) {
        HOST_KEY_CHANGED.to_string()
    } else {
        format!("{error:#}")
    }
}

/// Brings up the headless ControlMaster for `server` at `socket_path` and ensures
/// the remote-server binary is present, auto-installing (without progress UI) if
/// needed. Used for explicit headless connections; session inventory only calls
/// [`preflight_daemon_transport`] and never installs. The interactive first-connect
/// path uses that preflight + `install_binary_with_progress` so the tab can show
/// progress.
pub async fn prepare_daemon_transport(
    server: SshServerInfo,
    socket_path: PathBuf,
    auth_context: Arc<RemoteServerAuthContext>,
) -> std::result::Result<(), String> {
    let host = server.host.clone();
    match preflight_daemon_transport(server, socket_path.clone(), auth_context.clone()).await? {
        DaemonPreflight::Ready => Ok(()),
        DaemonPreflight::NeedsInstall => {
            log::info!("daemon connect [{host}]: binary missing — installing (first connect)");
            let transport = SshTransport::new(socket_path, auth_context);
            transport
                .install_binary()
                .await
                .map_err(|e| format!("remote-server install failed: {e}"))?;
            log::info!("daemon connect [{host}]: install complete");
            Ok(())
        }
        DaemonPreflight::HostKeyConfirmationRequired(host_key) => Err(format!(
            "SSH host-key confirmation required for {}:{} ({})",
            host_key.host, host_key.port, host_key.fingerprint
        )),
    }
}

/// Connects transiently to one prepared daemon runtime and returns its sessions
/// plus any existing tmux/byobu sessions. The caller must establish the
/// ControlMaster and verify that the binary is present; this query never installs.
/// After the initialize handshake and inventory requests, the transient connection
/// is torn down. The daemon and its sessions persist independently.
///
/// Request/response works without draining the client event channel — responses
/// are routed to per-request oneshots; the event channel is unbounded so the
/// reader never blocks on our ignoring it.
async fn query_daemon_inventory(
    transport: &SshTransport,
    auth_context: &RemoteServerAuthContext,
    executor: Arc<Background>,
) -> std::result::Result<(InitializeResponse, SessionList, MultiplexerSessionList), String> {
    let Connection { client, child, .. } = transport
        .connect(executor)
        .await
        .map_err(|e| format!("daemon connect failed: {e:#}"))?;
    // Keep the proxy/ssh child alive for the duration of the requests; it is torn
    // down when this returns. The daemon itself keeps running.
    let _child = child;
    let auth_token = auth_context.get_auth_token().await;
    let initialize = client
        .initialize(auth_token.as_deref())
        .await
        .map_err(|e| format!("daemon handshake failed: {e:#}"))?;
    let mut daemon = client
        .list_sessions()
        .await
        .map_err(|e| format!("list_sessions failed: {e:#}"))?;
    if supports_agent_inventory(&initialize) {
        match client.list_agent_sessions().await {
            Ok(agents) => enrich_daemon_session_titles(&mut daemon, &agents),
            Err(error) => {
                log::warn!(
                    "list_agent_sessions failed while enriching persistent session names: {error:#}"
                );
                enrich_daemon_session_titles(&mut daemon, &AgentSessionList::default());
            }
        }
    } else {
        neutralize_legacy_session_titles(&mut daemon);
    }
    let multiplexers = if supports_multiplexer_inventory(&initialize) {
        client
            .list_multiplexer_sessions()
            .await
            .map_err(|e| format!("list_multiplexer_sessions failed: {e:#}"))?
    } else {
        MultiplexerSessionList::default()
    };
    Ok((initialize, daemon, multiplexers))
}

fn merge_daemon_inventory(
    inventory: &mut HostSessionInventory,
    daemon: SessionList,
    multiplexers: MultiplexerSessionList,
    route: Option<DaemonRuntimeRoute>,
    observed_daemons: usize,
) {
    let routed_sessions: Vec<RoutedDaemonSession> = daemon
        .sessions
        .iter()
        .cloned()
        .map(|session| RoutedDaemonSession {
            session,
            route: route.clone(),
        })
        .collect();
    if observed_daemons == 0 {
        inventory.daemon = daemon;
        inventory.multiplexers = multiplexers;
    } else {
        // Every daemon enforces its own ring cap. A single SessionList cannot
        // represent several independent caps honestly, so suppress the host
        // aggregate while retaining exact per-session usage.
        inventory.daemon.host_ring_cap_bytes = 0;
        inventory.daemon.sessions.extend(daemon.sessions);
        if inventory.multiplexers.sessions.is_empty() {
            inventory.multiplexers = multiplexers;
        }
    }
    inventory.sessions.extend(routed_sessions);
}

fn single_daemon_inventory(
    daemon: SessionList,
    multiplexers: MultiplexerSessionList,
    route: Option<DaemonRuntimeRoute>,
) -> HostSessionInventory {
    let sessions = daemon
        .sessions
        .iter()
        .cloned()
        .map(|session| RoutedDaemonSession {
            session,
            route: route.clone(),
        })
        .collect();
    HostSessionInventory {
        daemon,
        sessions,
        multiplexers,
    }
}

/// Lists sessions across discoverable runtimes without installing a daemon.
/// One deadline covers preflight, the ControlMaster queue, and every query.
pub async fn list_daemon_sessions(
    server: SshServerInfo,
    socket_path: PathBuf,
    auth_context: Arc<RemoteServerAuthContext>,
    executor: Arc<Background>,
) -> std::result::Result<HostSessionInventory, String> {
    inventory_with_timeout(
        list_daemon_sessions_inner(server, socket_path, auth_context, executor),
        SESSION_INVENTORY_TIMEOUT,
    )
    .await
}

async fn inventory_with_timeout(
    inventory: impl Future<Output = std::result::Result<HostSessionInventory, String>>,
    timeout: Duration,
) -> std::result::Result<HostSessionInventory, String> {
    // Bound the whole scan, including the ControlMaster queue and all daemon
    // runtimes. Dropping the owned future releases its lock guards and SSH child.
    inventory.with_timeout(timeout).await.map_err(|_| {
        crate::t!(
            "workspace-left-panel-ssh-manager-sessions-timeout",
            seconds = timeout.as_secs()
        )
    })?
}

fn require_daemon_inventory_ready(preflight: DaemonPreflight) -> std::result::Result<(), String> {
    match preflight {
        DaemonPreflight::Ready => Ok(()),
        DaemonPreflight::NeedsInstall => Err(crate::t!(
            "workspace-left-panel-ssh-manager-sessions-needs-install"
        )),
        DaemonPreflight::HostKeyConfirmationRequired(host_key) => Err(format!(
            "SSH host-key confirmation required for {}:{} ({})",
            host_key.host, host_key.port, host_key.fingerprint
        )),
    }
}

async fn list_daemon_sessions_inner(
    server: SshServerInfo,
    socket_path: PathBuf,
    auth_context: Arc<RemoteServerAuthContext>,
    executor: Arc<Background>,
) -> std::result::Result<HostSessionInventory, String> {
    let preflight =
        preflight_daemon_transport(server, socket_path.clone(), auth_context.clone()).await?;
    require_daemon_inventory_ready(preflight)?;
    let base_transport = SshTransport::new(socket_path, auth_context.clone());
    let runtimes = match base_transport.list_daemon_runtime_filenames().await {
        Ok(runtimes) => runtimes,
        Err(error) => {
            log::warn!("{error}; falling back to the current daemon runtime");
            let (_, daemon, multiplexers) =
                query_daemon_inventory(&base_transport, &auth_context, executor).await?;
            return Ok(single_daemon_inventory(daemon, multiplexers, None));
        }
    };

    if runtimes.is_empty() {
        let (_, daemon, multiplexers) =
            query_daemon_inventory(&base_transport, &auth_context, executor).await?;
        return Ok(single_daemon_inventory(daemon, multiplexers, None));
    }

    let mut inventory = HostSessionInventory::default();
    let mut observed_daemons = 0;
    for runtime_filename in runtimes {
        let current_runtime =
            runtime_filename == remote_server::setup::daemon_runtime_filename("sock");
        if !current_runtime && ChannelState::app_version().is_none() {
            log::warn!(
                "skipping daemon runtime {runtime_filename} from an unversioned source build"
            );
            continue;
        }
        let transport = if current_runtime {
            base_transport.clone()
        } else {
            let probe_route = DaemonRuntimeRoute::new(runtime_filename.clone(), String::new())?;
            base_transport.clone().with_daemon_runtime(probe_route)
        };
        let (initialize, daemon, multiplexers) =
            match query_daemon_inventory(&transport, &auth_context, executor.clone()).await {
                Ok(result) => result,
                Err(error) => {
                    log::warn!("skipping unreachable daemon runtime {runtime_filename}: {error}");
                    continue;
                }
            };
        if !supports_session_host(&initialize) {
            log::warn!("skipping daemon runtime {runtime_filename} without session-host support");
            continue;
        }
        if !current_runtime
            && !is_older_release_daemon(
                ChannelState::app_version(),
                initialize.server_version.as_str(),
            )
        {
            log::warn!(
                "skipping non-older daemon runtime {runtime_filename} reporting version {:?}",
                initialize.server_version
            );
            continue;
        }
        let route = if current_runtime {
            None
        } else {
            Some(DaemonRuntimeRoute::new(
                runtime_filename,
                initialize.server_version,
            )?)
        };
        merge_daemon_inventory(
            &mut inventory,
            daemon,
            multiplexers,
            route,
            observed_daemons,
        );
        observed_daemons += 1;
    }

    if observed_daemons == 0 {
        let (_, daemon, multiplexers) =
            query_daemon_inventory(&base_transport, &auth_context, executor).await?;
        return Ok(single_daemon_inventory(daemon, multiplexers, None));
    }
    Ok(inventory)
}

#[cfg(test)]
#[path = "headless_connect_tests.rs"]
mod tests;
