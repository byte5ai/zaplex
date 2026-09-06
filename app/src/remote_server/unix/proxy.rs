//! Remote server proxy — runs over SSH stdio and bridges to the long-lived
//! daemon process via a Unix domain socket.
//!
//! Responsibilities:
//! 1. Acquire an exclusive `flock` on the PID file to serialise concurrent
//!    proxy starts (e.g. two tabs SSH-ing to the same host at the same time).
//! 2. Connect to the matching daemon socket (versioned for Zaplex release builds, see
//!    [`setup::daemon_runtime_filename`]).
//! 3. If not: spawn the daemon subcommand in a new session and wait for its
//!    socket to accept connections.
//! 4. Bridge stdin/stdout over the connected socket using the existing
//!    4-byte length-prefixed frame format.
//!
//! For Zaplex (Oss) release builds the socket/PID names carry the release
//! tag: reuse-by-liveness alone once bridged new clients to a running daemon
//! from a PREVIOUS release (RC finding 2026-07-19) — versioned rendezvous
//! makes that structurally impossible. An old daemon stays reachable only
//! under its old rendezvous name (its own release's clients) and retires via
//! its idle grace timer once its last session ends. Source builds and
//! non-Oss channels keep the legacy unversioned names.

use std::fs::Permissions;
use std::io::ErrorKind;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use super::super::setup;

/// Path to the daemon Unix domain socket (versioned on Zaplex release
/// builds — see [`setup::daemon_runtime_filename`]).
pub(super) fn socket_path(identity_key: &str) -> PathBuf {
    let dir = setup::remote_server_daemon_dir(identity_key);
    let expanded = shellexpand::tilde(&dir).into_owned();
    PathBuf::from(expanded).join(setup::daemon_runtime_filename("sock"))
}

/// Path to the daemon PID file, also used as the flock target (versioned on
/// Zaplex release builds — see [`setup::daemon_runtime_filename`]).
pub(super) fn pid_path(identity_key: &str) -> PathBuf {
    let dir = setup::remote_server_daemon_dir(identity_key);
    let expanded = shellexpand::tilde(&dir).into_owned();
    PathBuf::from(expanded).join(setup::daemon_runtime_filename("pid"))
}

/// Ensures the daemon directory exists with owner-only permissions.
pub(super) fn ensure_private_daemon_dir(path: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;
    std::fs::set_permissions(path, Permissions::from_mode(0o700))?;
    Ok(())
}

/// Entry point for `remote-server-proxy`.
///
/// Ensures the daemon is running, then bridges stdin/stdout to the daemon's
/// Unix socket for the lifetime of this SSH session.
pub fn run(identity_key: &str) -> anyhow::Result<()> {
    let socket_path = socket_path(identity_key);
    let pid_path = pid_path(identity_key);

    // Ensure the parent directory exists.
    if let Some(parent) = socket_path.parent() {
        ensure_private_daemon_dir(parent)?;
    }

    // ---- Acquire exclusive flock on the PID file --------------------------------
    //
    // This serialises concurrent proxy starts.  If two tabs SSH in at the
    // same time and both see "no daemon running", only one will succeed in
    // forking a daemon; the other will block here, then connect to the one
    // the first proxy started.
    //
    // The lock is released automatically when the File is dropped.
    let pid_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&pid_path)?;
    let pid_fd = pid_file.as_raw_fd();
    flock_wait(pid_fd, libc::LOCK_EX)?;

    // ---- Connect to an existing daemon or start a new one ------------------------
    let stream = match select_daemon(&socket_path)? {
        DaemonSelection::Connected(stream) => {
            log::info!("Proxy: reusing existing daemon");
            stream
        }
        DaemonSelection::StartDaemon => {
            log::info!("Proxy: no reachable daemon, will start one");
            // Remove any stale socket from a previous crash.
            if socket_path.exists() {
                match std::fs::remove_file(&socket_path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }

            // Spawn the daemon in a new Unix session so it is detached from
            // the SSH session.  When SSH exits the OS sends SIGHUP to every
            // process in the session's foreground process group.  `setsid()`
            // creates a new session for the child, so the daemon is not in
            // SSH's process group and will not receive that signal.
            let exe = std::env::current_exe()?;
            let mut cmd = command::blocking::Command::new(&exe);
            cmd.arg("remote-server-daemon")
                .arg("--identity-key")
                .arg(identity_key)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            // SAFETY: setsid(2) is async-signal-safe and has no side effects
            // other than creating a new session.  pre_exec closures run between
            // fork and exec in the child process.
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
            cmd.spawn()
                .map_err(|e| anyhow::anyhow!("failed to spawn daemon: {e}"))?;

            // Wait for the daemon's socket to accept a connection before releasing the flock.
            // Holding the lock here prevents a concurrent proxy from acquiring it and
            // racing to spawn a second daemon.
            wait_for_socket_connection(&socket_path)?
        }
    };

    flock_wait(pid_fd, libc::LOCK_UN)?;
    drop(pid_file);

    // ---- Bridge stdin/stdout to the daemon socket --------------------------------
    bridge_stdio_to_stream(stream)
}

enum DaemonSelection {
    Connected(UnixStream),
    StartDaemon,
}

fn select_daemon(socket_path: &std::path::Path) -> std::io::Result<DaemonSelection> {
    match UnixStream::connect(socket_path) {
        Ok(stream) => Ok(DaemonSelection::Connected(stream)),
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::NotFound | ErrorKind::ConnectionRefused
            ) =>
        {
            Ok(DaemonSelection::StartDaemon)
        }
        Err(error) => Err(error),
    }
}

/// Poll until the daemon's socket accepts a connection or the timeout elapses.
///
/// After we spawn the daemon there is a race: the daemon needs time to bind
/// and listen on the socket before the proxy can connect to it.  We poll
/// the endpoint itself so a stale socket file cannot be mistaken for readiness.
fn wait_for_socket_connection(socket_path: &std::path::Path) -> anyhow::Result<UnixStream> {
    const TIMEOUT: Duration = Duration::from_secs(10);
    const POLL_INTERVAL: Duration = Duration::from_millis(20);
    let start = instant::Instant::now();
    loop {
        match UnixStream::connect(socket_path) {
            Ok(stream) => {
                log::info!("Proxy: daemon socket ready after {:?}", start.elapsed());
                return Ok(stream);
            }
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::ConnectionRefused
                ) =>
            {
                if start.elapsed() >= TIMEOUT {
                    anyhow::bail!(
                        "timed out waiting for daemon socket at {}: {error}",
                        socket_path.display()
                    );
                }
            }
            Err(error) => return Err(error.into()),
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Calls `flock(2)` with the given operation, retrying on `EINTR`.
///
/// Blocking `flock(LOCK_EX)` can be interrupted by a signal before acquiring
/// the lock; ignoring the return value would cause the proxy to proceed
/// without actually holding the lock.
fn flock_wait(fd: std::os::unix::io::RawFd, operation: libc::c_int) -> anyhow::Result<()> {
    loop {
        // SAFETY: flock(2) is safe to call with a valid fd and a valid operation.
        let ret = unsafe { libc::flock(fd, operation) };
        if ret == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EINTR) {
            continue; // Interrupted by signal — retry.
        }
        return Err(anyhow::anyhow!("flock failed: {err}"));
    }
}

/// Connect to the daemon's Unix socket and copy bytes bidirectionally between
/// stdin/stdout and the socket.
///
/// The proxy is protocol-agnostic — it forwards raw bytes without parsing the
/// length-prefixed framing.  The framing is handled at the endpoints (Zaplex
/// client and daemon).
///
/// **Important**: the stdout direction uses a manual read→write→flush loop
/// instead of `io::copy` because `std::io::stdout()` wraps the fd in a
/// `LineWriter` that only flushes up to the last `\n` byte in each write.
/// For a binary protocol the trailing bytes after the last `0x0a` get stuck
/// in the internal `BufWriter` and are never flushed, causing the client to
/// hang forever waiting for complete messages.
///
/// **Shutdown coordination**: each direction explicitly
/// [`shutdown(Both)`s][Shutdown] the Unix socket when its copy loop
/// returns, which unblocks the other thread's read/write on the same
/// underlying socket. Without this, when the client SIGKILLs the local
/// `ssh ... remote-server-proxy` slave (e.g. on `ExitShell`), sshd
/// closes our stdin but the daemon has no reason to close its end of
/// the Unix socket, so the stdout thread sits forever in a blocking
/// read. That keeps the proxy alive with stdout still open, which
/// keeps the SSH channel half-closed on the server side, which in
/// turn keeps the client's `ssh` ControlMaster from exiting until
/// sshd's session cleanup eventually fires. Shutting the Unix socket
/// here makes teardown deterministic and independent of whatever the
/// daemon is doing.
///
/// [Shutdown]: std::net::Shutdown
fn bridge_stdio_to_stream(stream: UnixStream) -> anyhow::Result<()> {
    use std::io::{Read, Write};
    use std::net::Shutdown;

    log::info!("Proxy: connected, bridging stdio");

    // Each thread holds two clones: one it actively reads/writes, and
    // one used solely to `shutdown(Both)` on exit so the peer thread's
    // blocking call returns. `UnixStream::try_clone` shares the
    // underlying socket, so `shutdown` on any clone tears down both
    // directions for every clone.
    let stream_for_t1 = stream.try_clone()?;
    let stream_shutdown_for_t1 = stream.try_clone()?;
    let stream_for_t2 = stream.try_clone()?;
    let stream_shutdown_for_t2 = stream.try_clone()?;
    drop(stream);

    let t1 = std::thread::Builder::new()
        .name("proxy-stdin-fwd".into())
        .spawn(move || {
            let result = std::io::copy(&mut std::io::stdin(), &mut &stream_for_t1);
            match &result {
                Ok(total) => log::info!(
                    "Proxy: stdin->socket copy ended ({total} bytes); \
                     shutting down socket to unblock peer"
                ),
                Err(e) => log::info!(
                    "Proxy: stdin->socket copy errored ({e}); \
                     shutting down socket to unblock peer"
                ),
            }
            let _ = stream_shutdown_for_t1.shutdown(Shutdown::Both);
            result
        })?;

    // Socket → stdout: flush after every write so that complete protocol
    // frames reach the SSH tunnel without waiting for the `LineWriter`
    // buffer to fill.
    let t2 = std::thread::Builder::new()
        .name("proxy-stdout-fwd".into())
        .spawn(move || -> std::io::Result<u64> {
            let mut stdout = std::io::stdout().lock();
            let mut buf = [0u8; 8192];
            let mut total = 0u64;
            let result = loop {
                let n = match (&stream_for_t2).read(&mut buf) {
                    Ok(0) => break Ok(total),
                    Ok(n) => n,
                    Err(e) => break Err(e),
                };
                if let Err(e) = stdout.write_all(&buf[..n]) {
                    break Err(e);
                }
                if let Err(e) = stdout.flush() {
                    break Err(e);
                }
                total += n as u64;
            };
            match &result {
                Ok(total) => log::info!(
                    "Proxy: socket->stdout copy ended ({total} bytes); \
                     shutting down socket to unblock peer"
                ),
                Err(e) => log::info!(
                    "Proxy: socket->stdout copy errored ({e}); \
                     shutting down socket to unblock peer"
                ),
            }
            let _ = stream_shutdown_for_t2.shutdown(Shutdown::Both);
            result
        })?;

    let _ = t1.join();
    let _ = t2.join();

    log::info!("Proxy: bridge closed, exiting");
    Ok(())
}

#[cfg(test)]
#[path = "proxy_tests.rs"]
mod tests;
