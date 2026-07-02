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

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use remote_server::auth::RemoteServerAuthContext;
use remote_server::proto::SessionInfo;
use remote_server::transport::{Connection, RemoteTransport};
use warp_core::SessionId;
use warp_ssh_manager::{AuthType, SshServerInfo};
use warpui::r#async::executor::Background;

use super::ssh_transport::SshTransport;

/// Daemon sessions are allocated `SessionId`s in the **top half** of the u64
/// space so they cannot collide with shell-bootstrap-minted ids (which are
/// PID/timestamp-derived and stay well below `2^63`). The manager keys all
/// sessions — interactive and daemon — by `SessionId`, so uniqueness matters.
const DAEMON_SESSION_ID_BASE: u64 = 1 << 63;
static NEXT_DAEMON_SESSION_ID: AtomicU64 = AtomicU64::new(1);

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

/// Whether a live ControlMaster is serving `socket_path` — `ssh -O check`
/// returns success only when the master process is actually alive (a stale
/// socket file fails the check). Runs entirely over the local Unix socket, so
/// it returns quickly; bounded by a short timeout regardless.
async fn control_master_alive(socket_path: &Path) -> bool {
    let mut cmd = command::r#async::Command::new("ssh");
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

/// Ensures a ControlMaster is up at `socket_path` (idempotent via
/// `ControlMaster=auto` + `ControlPersist`). Spawns `ssh -f -N …`, which
/// authenticates and then backgrounds itself; the master socket exists by the
/// time the foreground process exits. Key/agent auth only (`BatchMode=yes`).
pub async fn ensure_control_master(server: &SshServerInfo, socket_path: &Path) -> Result<()> {
    if socket_path.exists() {
        // A socket file is present, but the master may have died on an SSH drop,
        // leaving a stale socket. Verify it's actually serving: reuse a live
        // master, otherwise remove the stale socket and spawn a fresh one. This
        // is what lets a daemon session's transport be re-established after a
        // connection loss (the session itself kept running daemon-side).
        if control_master_alive(socket_path).await {
            return Ok(());
        }
        log::info!(
            "ControlMaster socket {} is stale; re-establishing",
            socket_path.display()
        );
        let _ = std::fs::remove_file(socket_path);
    }

    let mut args: Vec<String> = Vec::new();
    if server.port != 22 {
        args.push("-p".into());
        args.push(server.port.to_string());
    }
    if let Some(key) = server.key_path.as_deref().filter(|p| !p.is_empty()) {
        args.push("-i".into());
        args.push(key.to_string());
    }
    args.extend([
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
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
    ]);
    let target = if server.username.is_empty() {
        server.host.clone()
    } else {
        format!("{}@{}", server.username, server.host)
    };
    args.push(target);

    let output = tokio::time::timeout(
        Duration::from_secs(20),
        command::r#async::Command::new("ssh")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow!("ControlMaster setup timed out"))?
    .map_err(|e| anyhow!("failed to spawn ssh: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("ControlMaster setup failed: {}", stderr.trim()));
    }

    // `-f` returns once the master is backgrounded; the socket should exist now.
    // Poll briefly to absorb any small filesystem-visibility lag.
    for _ in 0..20 {
        if socket_path.exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(anyhow!(
        "ControlMaster socket did not appear at {}",
        socket_path.display()
    ))
}

/// Sentinel error returned by [`prepare_daemon_transport`] when the remote-server
/// binary is not present on the host. The caller uses this to fall back to a classic
/// SSH session (with a warning) instead of hanging on an install-on-connect.
pub const DAEMON_BINARY_MISSING: &str = "daemon-binary-missing";

/// Brings up the headless ControlMaster for `server` at `socket_path` and verifies
/// the remote-server binary is present (no install-on-connect). Shared by the
/// daemon-terminal connect (`Workspace::spawn_daemon_session_connect`) and the
/// adopt-sidebar's connect-to-list. On success the caller builds a fresh
/// [`SshTransport`] over `socket_path` and calls `connect_session`.
pub async fn prepare_daemon_transport(
    server: SshServerInfo,
    socket_path: PathBuf,
    auth_context: Arc<RemoteServerAuthContext>,
) -> std::result::Result<(), String> {
    let host = server.host.clone();
    log::info!("daemon connect [{host}]: establishing ControlMaster");
    ensure_control_master(&server, &socket_path)
        .await
        .map_err(|e| format!("ControlMaster setup failed: {e:#}"))?;
    let transport = SshTransport::new(socket_path, auth_context);
    log::info!("daemon connect [{host}]: checking remote-server binary");
    match transport.check_binary().await {
        Ok(true) => log::info!("daemon connect [{host}]: binary present"),
        // Binary missing: fail FAST instead of an unbounded install-on-connect
        // (GitHub download for a non-release tag + a 194 MB scp), which made the
        // connect appear to hang at "Starting…". The caller falls back to a classic
        // SSH session (with a warning). Installing the daemon is a separate,
        // explicit action — never a silent multi-minute stall on every connect.
        Ok(false) => {
            return Err(DAEMON_BINARY_MISSING.to_string());
        }
        Err(e) => return Err(format!("remote-server binary check failed: {e}")),
    }
    Ok(())
}

/// Connects to `server`'s daemon (a transient connection) and returns the
/// sessions it currently owns — including ones that survived an app restart or
/// transport drop, which is the whole point of the adopt-sidebar. Self-contained:
/// brings up the ControlMaster + binary, connects, runs the initialize handshake,
/// calls `list_sessions`, then tears the transient connection down again (the
/// daemon and its sessions persist independently of this connection).
///
/// Request/response works without draining the client event channel — responses
/// are routed to per-request oneshots; the event channel is unbounded so the
/// reader never blocks on our ignoring it.
pub async fn list_daemon_sessions(
    server: SshServerInfo,
    socket_path: PathBuf,
    auth_context: Arc<RemoteServerAuthContext>,
    executor: Arc<Background>,
) -> std::result::Result<Vec<SessionInfo>, String> {
    prepare_daemon_transport(server, socket_path.clone(), auth_context.clone()).await?;
    let transport = SshTransport::new(socket_path, auth_context.clone());
    let Connection { client, child, .. } = transport
        .connect(executor)
        .await
        .map_err(|e| format!("daemon connect failed: {e:#}"))?;
    // Keep the proxy/ssh child alive for the duration of the requests; it is torn
    // down when this returns. The daemon itself keeps running.
    let _child = child;
    let auth_token = auth_context.get_auth_token().await;
    client
        .initialize(auth_token.as_deref())
        .await
        .map_err(|e| format!("daemon handshake failed: {e:#}"))?;
    let list = client
        .list_sessions()
        .await
        .map_err(|e| format!("list_sessions failed: {e:#}"))?;
    Ok(list.sessions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use warp_ssh_manager::{AuthType, SshServerInfo};

    fn server(auth: AuthType) -> SshServerInfo {
        let mut s = SshServerInfo::new_default("node-1".to_string());
        s.host = "example.com".to_string();
        s.username = "me".to_string();
        s.port = 22;
        s.auth_type = auth;
        s
    }

    #[test]
    fn headless_capable_only_for_key_auth() {
        assert!(is_headless_capable(&server(AuthType::Key)));
        assert!(!is_headless_capable(&server(AuthType::Password)));
        // OneKey is resolved to Key/Password upstream (resolve_server_auth); the
        // bare OneKey marker is not headless-capable on its own.
        assert!(!is_headless_capable(&server(AuthType::OneKey)));
    }

    #[test]
    fn control_socket_path_is_stable_and_per_host() {
        let a1 = control_socket_path(&server(AuthType::Key));
        let a2 = control_socket_path(&server(AuthType::Key));
        assert_eq!(a1, a2, "same host → same socket path (run-to-run stable)");

        let mut other = server(AuthType::Key);
        other.host = "other.example.com".to_string();
        assert_ne!(
            a1,
            control_socket_path(&other),
            "different host → different socket"
        );
        assert!(a1.to_string_lossy().contains(".ssh/zaplex-daemon-"));
    }

    #[test]
    fn daemon_session_ids_are_unique_and_in_top_half() {
        let a = alloc_daemon_session_id();
        let b = alloc_daemon_session_id();
        assert_ne!(a, b, "each allocation is unique");
        assert!(a.as_u64() >= DAEMON_SESSION_ID_BASE, "top-half id (no collision with shell ids)");
        assert!(b.as_u64() >= DAEMON_SESSION_ID_BASE);
    }
}
