//! Daemon-side session host (Stage 1).
//!
//! The daemon owns a PTY + login shell per session, buffers the shell's output
//! in a per-session [`OutputRing`], and streams it to the attached connection as
//! `SessionOutput` pushes. Because the daemon owns the PTY (not the SSH
//! channel), the session survives SSH drops.
//!
//! This module holds the per-session state and the two async tasks (reader and
//! writer); the message handlers that mutate [`ServerModel`] live in
//! `server_model.rs` (where the model internals are in scope). See
//! `docs/superpowers/specs/2026-06-24-stage1-session-host-design.md`.

use std::fs::File;
use std::sync::Arc;

use async_io::Async;
use futures::io::{AsyncReadExt, AsyncWriteExt};
use warpui::ModelSpawner;
use zaplex_remote_session::server::output_ring::OutputRing;

use super::server_model::{ConnectionId, ServerModel};

/// Per-session output ring ceiling (Stage 1 constant; a configurable setting in
/// Stage 4). Bounds host RAM per session while keeping enough scrollback for a
/// reconnect replay.
pub(super) const RING_CEILING_BYTES: usize = 4 * 1024 * 1024;

/// Read chunk size for the per-session PTY reader.
const READ_CHUNK: usize = 64 * 1024;

/// A live daemon-hosted session: the PTY master, the shell child, the output
/// ring, and the channel feeding the ordered input writer.
pub(super) struct Session {
    /// PTY master, async-wrapped (non-blocking). Shared with the reader/writer
    /// tasks via `Arc`; keeping a clone here keeps the fd alive for resize.
    pub(super) leader: Arc<Async<File>>,
    /// The spawned login shell. Reaped on close / shell exit.
    pub(super) child: std::process::Child,
    /// Replay buffer of recent output.
    pub(super) ring: OutputRing,
    pub(super) rows: usize,
    pub(super) cols: usize,
    /// Connection currently receiving this session's live output.
    pub(super) attached: ConnectionId,
    /// Ordered keyboard/mouse input → the writer task → the PTY.
    pub(super) input_tx: async_channel::Sender<Vec<u8>>,
    /// Working directory the session was opened in (for `ListSessions`).
    pub(super) cwd: Option<String>,
    /// Login shell the session runs (for `ListSessions` titles).
    pub(super) shell: String,
    /// Unix epoch millis of the last attach (open counts as the first attach);
    /// `0` means never. Drives `ListSessions` and the detached-idle GC.
    pub(super) last_attached_ms: u64,
}

/// Per-session reader task: pumps PTY output into the model (which appends it to
/// the ring and pushes `SessionOutput`). On EOF (shell exit / PTY close) it
/// notifies the model so it can reap the child and emit `SessionExited`.
pub(super) async fn run_session_reader(
    session_id: String,
    leader: Arc<Async<File>>,
    spawner: ModelSpawner<ServerModel>,
) {
    let mut reader: &Async<File> = &leader;
    let mut buf = vec![0u8; READ_CHUNK];
    loop {
        let n = match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let chunk = buf[..n].to_vec();
        let id = session_id.clone();
        // Re-enter the model to append + push; bail out if the model is gone.
        if spawner
            .spawn(move |me, _ctx| me.on_session_output(&id, chunk))
            .await
            .is_err()
        {
            return;
        }
    }
    let id = session_id.clone();
    let _ = spawner.spawn(move |me, _ctx| me.on_session_reader_eof(&id)).await;
}

/// Per-session writer task: drains the ordered input channel and writes each
/// chunk to the PTY in full, preserving keystroke order. Ends when the session
/// is dropped (its `input_tx` is dropped, closing the channel).
pub(super) async fn run_session_writer(
    leader: Arc<Async<File>>,
    input_rx: async_channel::Receiver<Vec<u8>>,
) {
    let mut writer: &Async<File> = &leader;
    while let Ok(bytes) = input_rx.recv().await {
        let mut rest: &[u8] = &bytes;
        while !rest.is_empty() {
            match writer.write(rest).await {
                Ok(0) => return,
                Ok(n) => rest = &rest[n..],
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return,
            }
        }
    }
}
