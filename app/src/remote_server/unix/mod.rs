//! Unix-specific implementation of the remote server daemon and proxy.
//!
//! - `run_proxy()`: entry point for the `remote-server-proxy` subcommand.
//!   Uses a ControlMaster-like pattern (flock + fork + exec) to daemonize
//!   the server and bridge the SSH stdio channel to its Unix socket.
//!
//! - `run_daemon()`: entry point for the `remote-server-daemon` subcommand.
//!   Binds a Unix domain socket, accepts multiple concurrent proxy connections,
//!   and exits after a grace period with no connections.
//!
//! All platform-specific code is contained here so that the parent `mod.rs`
//! is a thin dispatcher with no Unix assumptions.

pub(super) mod proxy;

use super::server_model::{
    ConnectionId, ConnectionOutbox, ServerModel, CONNECTION_OUTBOX_BUDGET_BYTES,
};
use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use warpui::r#async::executor;

/// Run the `remote-server-daemon` subcommand.
///
/// Binds a Unix domain socket and writes a PID file, then delegates the
/// WarpUI app startup to [`super::run_daemon_app`] with the Unix-specific
/// `ServerModel` constructor.
pub fn run_daemon(identity_key: String) -> anyhow::Result<()> {
    // socket_path: ~/.warp[-channel]/remote-server/{identity_key}/server[-<tag>].sock
    //   The Unix domain socket the daemon binds on.  Proxy processes connect
    //   to it and bridge their SSH stdio channel through it. Zaplex (Oss)
    //   release builds carry the release tag in the name (versioned
    //   rendezvous, see `setup::daemon_runtime_filename`) so a proxy only
    //   ever reaches a daemon of its own release.
    //
    // pid_path:    ~/.warp[-channel]/remote-server/{identity_key}/server[-<tag>].pid
    //   Contains the daemon's PID.  Proxy processes read it and use
    //   kill(pid, 0) to detect whether the daemon is still alive before
    //   deciding whether to start a new one.
    let socket_path = proxy::socket_path(&identity_key);
    let pid_path = proxy::pid_path(&identity_key);

    if let Some(parent) = socket_path.parent() {
        proxy::ensure_private_daemon_dir(parent)?;
    }
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)?;
    }

    // Bind with std (no async runtime needed yet); converted to
    // async_io::Async inside the closure where the executor is active.
    let listener = std::os::unix::net::UnixListener::bind(&socket_path)?;
    std::fs::set_permissions(&socket_path, Permissions::from_mode(0o600))?;
    // async_io::Async::new() requires non-blocking mode.
    listener.set_nonblocking(true)?;
    log::info!("Daemon bound to {}", socket_path.display());

    std::fs::write(&pid_path, std::process::id().to_string())?;

    super::run_daemon_app(move |ctx| {
        // Spawn the Unix socket accept loop.  The listener and connection
        // handling are entirely Unix-specific; ServerModel itself is
        // platform-agnostic and only sees register_connection /
        // deregister_connection calls.
        let spawner = ctx.spawner();
        let exec = ctx.background_executor();
        let spawner_loop = spawner.clone();
        let background_executor = exec.clone();

        exec.spawn(async move {
            let listener = match async_io::Async::new(listener) {
                Ok(l) => l,
                Err(e) => {
                    log::error!("Daemon: async listener error: {e}");
                    return;
                }
            };
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        let conn_id = uuid::Uuid::new_v4();
                        log::info!("Daemon: accepted connection {conn_id}");
                        let spawner = spawner_loop.clone();
                        background_executor
                            .spawn(handle_daemon_connection(
                                conn_id,
                                stream,
                                spawner,
                                background_executor.clone(),
                            ))
                            .detach();
                    }
                    Err(e) => log::error!("Daemon: accept error: {e}"),
                }
            }
        })
        .detach();

        ServerModel::new(ctx)
    })?;

    let _ = std::fs::remove_file(&socket_path);
    let _ = std::fs::remove_file(&pid_path);
    log::info!("Daemon exiting");
    Ok(())
}

/// Handles a single Unix socket connection from a proxy process.
///
/// Spawns a dedicated **reader task** that owns the read half of the socket
/// and runs a tight `read_client_message` loop, forwarding each decoded
/// message to `ServerModel` via the spawner.  The reader is never cancelled
/// mid-read, which avoids the framing desynchronisation that would occur if
/// `read_client_message` were polled inside a `select!` branch.
///
/// The calling task becomes the **writer loop**: it drains the per-connection
/// outbound queue and writes each `ServerMessage` to the socket.
/// When the reader exits (EOF / error) it calls `deregister_connection`, which
/// closes the outbox and cancels the writer loop.
pub(super) async fn handle_daemon_connection(
    conn_id: ConnectionId,
    stream: async_io::Async<std::os::unix::net::UnixStream>,
    spawner: warpui::ModelSpawner<ServerModel>,
    exec: std::sync::Arc<executor::Background>,
) {
    use futures::future::{select, Either};
    use futures::io::{AsyncWriteExt, BufReader, BufWriter};
    use futures::AsyncReadExt as _;

    let shutdown_stream = match stream.get_ref().try_clone() {
        Ok(stream) => stream,
        Err(error) => {
            log::error!("Daemon: failed to clone socket for conn {conn_id}: {error}");
            return;
        }
    };
    let shutdown = std::sync::Arc::new(move || {
        let _ = shutdown_stream.shutdown(std::net::Shutdown::Both);
    });
    let (conn_outbox, conn_drain) = ConnectionOutbox::new(CONNECTION_OUTBOX_BUDGET_BYTES, shutdown);

    // Register with ServerModel (cancels grace timer if running).
    let _ = spawner
        .spawn(move |me, ctx| {
            me.register_connection(conn_id, conn_outbox, ctx);
        })
        .await;

    let (read_half, write_half) = stream.split();
    let mut writer = BufWriter::new(write_half);

    // ---- Reader task -------------------------------------------------------
    // Owns the read half; dispatches decoded messages to ServerModel.
    // On exit it calls deregister_connection, which closes the outbox and
    // cancels the writer loop below.
    let spawner_reader = spawner.clone();
    exec.spawn(async move {
        let mut reader = BufReader::new(read_half);
        loop {
            match remote_server::protocol::read_client_message(&mut reader).await {
                Ok(msg) => {
                    let result = spawner_reader
                        .spawn(move |me, ctx| {
                            me.handle_message(conn_id, msg, ctx);
                        })
                        .await;
                    if result.is_err() {
                        log::warn!("Daemon: ServerModel dropped, closing conn {conn_id}");
                        break;
                    }
                }
                Err(remote_server::protocol::ProtocolError::UnexpectedEof) => {
                    log::info!("Daemon: proxy {conn_id} disconnected (EOF)");
                    break;
                }
                Err(e) if e.is_read_recoverable() => {
                    log::warn!("Daemon: skipping malformed message from conn {conn_id}: {e}");
                }
                Err(e) => {
                    log::error!("Daemon: fatal read error from conn {conn_id}: {e}");
                    break;
                }
            }
        }
        // Deregistering closes the outbox, cancels the writer, and shuts down
        // the shared socket handle.
        let _ = spawner_reader
            .spawn(move |me, ctx| {
                me.deregister_connection(conn_id, ctx);
            })
            .await;
    })
    .detach();

    // ---- Writer loop -------------------------------------------------------
    // Drains outbound messages until the reader deregisters this connection,
    // the byte budget is exceeded, or a write fails. Cancellation is selected
    // against both dequeue and socket I/O so a blocked slow consumer exits.
    loop {
        let receive = conn_drain.recv();
        let cancelled = conn_drain.cancelled();
        futures::pin_mut!(receive, cancelled);
        let msg = match select(receive, cancelled).await {
            Either::Left((Ok(message), _)) => message,
            Either::Left((Err(_), _)) | Either::Right(_) => break,
        };

        let write_and_flush = async {
            remote_server::protocol::write_server_message(&mut writer, &msg)
                .await
                .map_err(|error| error.to_string())?;
            writer.flush().await.map_err(|error| error.to_string())?;
            Ok::<(), String>(())
        };
        let cancelled = conn_drain.cancelled();
        futures::pin_mut!(write_and_flush, cancelled);
        match select(write_and_flush, cancelled).await {
            Either::Left((Ok(()), _)) => {}
            Either::Left((Err(error), _)) => {
                log::error!("Daemon: write error on conn {conn_id}: {error}");
                break;
            }
            Either::Right(_) => break,
        }
    }

    // Deregister in case the writer exited due to a write error before the
    // reader task called deregister. This is a no-op if already deregistered.
    let _ = spawner
        .spawn(move |me, ctx| {
            me.deregister_connection(conn_id, ctx);
        })
        .await;
}
