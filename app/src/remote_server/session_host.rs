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

use std::collections::HashMap;
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

/// Upper bound on the bytes accumulated while capturing a session's bootstrap
/// preamble (T1.3). The Zaplexify handshake completes within the first few KiB,
/// so this is far more than any real bootstrap; if a session emits this much
/// output before its client reports the boundary (an unbootstrappable shell, or
/// a chatty pre-prompt), capture is abandoned rather than growing unbounded.
pub(super) const BOOTSTRAP_PREAMBLE_CAP_BYTES: usize = 512 * 1024;

/// Maximum retry-safe startup deliveries remembered for one PTY session.
/// Legitimate sessions normally use one; the small fixed ceiling prevents a
/// client from growing the deduplication ledger for the session lifetime.
pub(super) const MAX_ACCEPTED_STARTUP_COMMANDS: usize = 64;

/// Read chunk size for the per-session PTY reader.
const READ_CHUNK: usize = 64 * 1024;

/// A live daemon-hosted session: the PTY master, the shell child, the output
/// ring, and the channel feeding the ordered input writer.
pub(super) struct Session {
    /// Monotonic daemon-process generation for stale-id rejection.
    pub(super) generation: u64,
    /// PTY master, async-wrapped (non-blocking). Shared with the reader/writer
    /// tasks via `Arc`; keeping a clone here keeps the fd alive for resize.
    pub(super) leader: Arc<Async<File>>,
    /// The spawned login shell. Reaped on close / shell exit.
    pub(super) child: std::process::Child,
    /// Keeps a fish/PowerShell bootstrap body file alive until the daemon
    /// session ends. Their init hooks source this file exactly once.
    pub(super) _bootstrap_file: Option<crate::terminal::TempBootstrapFile>,
    /// Replay buffer of recent output.
    pub(super) ring: OutputRing,
    pub(super) rows: usize,
    pub(super) cols: usize,
    /// Connection currently receiving this session's live output.
    pub(super) attached: ConnectionId,
    /// Ordered keyboard/mouse input → the writer task → the PTY.
    pub(super) input_tx: async_channel::Sender<Vec<u8>>,
    /// Retry-safe startup commands already accepted by the ordered writer,
    /// keyed by their stable client-generated delivery id. The bytes are kept
    /// with the id so an accidental id reuse with different content is rejected
    /// rather than acknowledged as if it had executed.
    pub(super) accepted_startup_commands: HashMap<String, Vec<u8>>,
    /// Working directory the session was opened in (for `ListSessions`).
    pub(super) cwd: Option<String>,
    /// Login shell the session runs (for `ListSessions` titles).
    pub(super) shell: String,
    /// Unix epoch millis of the last attach (open counts as the first attach);
    /// `0` means never. Drives `ListSessions` and the detached-idle GC.
    pub(super) last_attached_ms: u64,
    /// The session's captured bootstrap handshake, served to a later adopt whose
    /// ring has evicted it so bootstrap can still be armed (T1.3).
    pub(super) preamble: BootstrapPreamble,
}

/// The bootstrap-handshake prefix of a daemon-hosted session (T1.3).
///
/// A session's Zaplexify handshake (`InitShell`…`Bootstrapped` DCS) is emitted
/// once at the very start. If the session runs long enough, the ring evicts it,
/// and a client that then *adopts* the session can never arm bootstrap from the
/// replay alone — history, autocomplete, block parsing and command execution all
/// stay dead. This keeps that prefix aside, immune to ring eviction:
///
/// 1. **Capture** — while `capturing`, every output byte from seq 0 is mirrored
///    here. If the handshake never completes within `cap` bytes (an
///    unbootstrappable shell, or an unusually chatty pre-prompt), capture is
///    abandoned to bound RAM.
/// 2. **Freeze** — the client that *opened* the session reports the byte offset
///    at which its terminal model became bootstrapped; the prefix is truncated
///    to there and frozen. Only the opener defines the boundary (idempotent
///    afterwards).
/// 3. **Serve** — [`Self::frozen`] yields the bytes for an adopt's
///    `SessionAttached`; the daemon starts that adopt's replay at the preamble's
///    end so the two never overlap.
pub(super) struct BootstrapPreamble {
    bytes: Vec<u8>,
    /// True until the boundary is reported (freeze) or the cap is hit (abandon).
    capturing: bool,
    /// Upper bound on captured bytes before abandoning (see the module constant).
    cap: usize,
}

impl BootstrapPreamble {
    pub(super) fn new(cap: usize) -> Self {
        Self {
            bytes: Vec::new(),
            capturing: true,
            cap,
        }
    }

    /// Mirrors a chunk of session output while still capturing. Abandons capture
    /// (dropping what was collected) if the accumulated bytes exceed the cap, so
    /// a session that never reports a boundary can't grow this without bound.
    pub(super) fn capture(&mut self, bytes: &[u8]) {
        if !self.capturing {
            return;
        }
        self.bytes.extend_from_slice(bytes);
        if self.bytes.len() > self.cap {
            self.bytes = Vec::new();
            self.capturing = false;
        }
    }

    /// Freezes the preamble at `end_seq` bytes from session start. Idempotent:
    /// a preamble that is already frozen or was abandoned ignores this — by
    /// convention only the first (opening) client reports the boundary.
    ///
    /// If `end_seq` is past what we captured, the prefix we hold would be an
    /// *incomplete* handshake (it may not even contain `InitShell`), so capture is
    /// abandoned rather than frozen — a served-but-partial preamble could arm the
    /// client's write-suppression without a corresponding `InitShell`. In practice
    /// `end_seq` equals the captured length (same output byte-space), so this
    /// guard only fires on an out-of-range report.
    pub(super) fn freeze(&mut self, end_seq: u64) {
        if !self.capturing {
            return;
        }
        let end_seq = end_seq as usize;
        if end_seq > self.bytes.len() {
            self.bytes = Vec::new();
            self.capturing = false;
            return;
        }
        self.bytes.truncate(end_seq);
        self.bytes.shrink_to_fit();
        self.capturing = false;
    }

    /// The frozen preamble bytes, or `None` while still capturing or if capture
    /// was abandoned / produced nothing. `Some` means the handshake was captured
    /// in full and is safe to replay to an adopting client.
    pub(super) fn frozen(&self) -> Option<&[u8]> {
        if !self.capturing && !self.bytes.is_empty() {
            Some(&self.bytes)
        } else {
            None
        }
    }
}

/// Plans an `AttachSession` reply's replay window and bootstrap preamble (T1.3).
///
/// Returns `(base_seq, replay, preamble)`:
/// - On a **fresh adopt** (`last_seq == 0`) whose ring has evicted seq 0
///   (`base_seq() > 0`) *and* has a frozen preamble: the preamble is served and
///   the replay starts at the preamble's end (`replay_from(preamble.len())`), so
///   preamble `[0, P)` and replay `[≥P, end)` never overlap. When the ring's
///   oldest byte is past `P` the client sees a genuine gap after the preamble and
///   resets its screen; when it is at `P` the two are contiguous.
/// - Otherwise (the client did not opt in, a reconnect with `last_seq > 0`, a
///   session that never evicted its handshake, or one with no frozen preamble):
///   no preamble, and a normal `replay_from(last_seq)`.
///
/// `client_supports_preamble` gates the whole preamble path: an old client that
/// does not understand `bootstrap_preamble` (the field decodes as `false`) gets
/// the exact pre-T1.3 behaviour — a plain `replay_from(last_seq)`, never shifted
/// past a preamble it could not consume.
pub(super) fn plan_attach(
    ring: &OutputRing,
    preamble: &BootstrapPreamble,
    last_seq: u64,
    client_supports_preamble: bool,
) -> (u64, Vec<u8>, Vec<u8>) {
    if client_supports_preamble && last_seq == 0 && ring.base_seq() > 0 {
        if let Some(preamble) = preamble.frozen() {
            let (base_seq, replay) = ring.replay_from(preamble.len() as u64);
            return (base_seq, replay, preamble.to_vec());
        }
    }
    let (base_seq, replay) = ring.replay_from(last_seq);
    (base_seq, replay, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frozen_at(bytes: &[u8], end_seq: u64) -> BootstrapPreamble {
        let mut p = BootstrapPreamble::new(1024);
        p.capture(bytes);
        p.freeze(end_seq);
        p
    }

    #[test]
    fn capture_then_freeze_keeps_the_prefix_up_to_the_boundary() {
        let mut p = BootstrapPreamble::new(1024);
        assert_eq!(p.frozen(), None, "nothing frozen while still capturing");
        p.capture(b"INIT-SHELL...BOOTSTRAPPED...prompt$ ");
        assert_eq!(p.frozen(), None, "still capturing until the boundary is set");
        // Boundary lands right after "...BOOTSTRAPPED..." (28 bytes).
        p.freeze(28);
        assert_eq!(p.frozen(), Some(&b"INIT-SHELL...BOOTSTRAPPED..."[..]));
    }

    #[test]
    fn capture_across_multiple_chunks_concatenates_in_order() {
        let mut p = BootstrapPreamble::new(1024);
        p.capture(b"AAAA");
        p.capture(b"BBBB");
        p.capture(b"CCCC");
        p.freeze(10);
        assert_eq!(p.frozen(), Some(&b"AAAABBBBCC"[..]));
    }

    #[test]
    fn freeze_beyond_captured_length_abandons() {
        // A boundary past what we captured would leave a partial (possibly
        // InitShell-less) handshake, so it is abandoned rather than frozen.
        let p = frozen_at(b"short", 9999);
        assert_eq!(p.frozen(), None, "an out-of-range boundary abandons capture");
    }

    #[test]
    fn exceeding_the_cap_abandons_capture() {
        let mut p = BootstrapPreamble::new(8);
        p.capture(b"0123456789ABCDEF"); // 16 > cap 8
        assert_eq!(p.frozen(), None, "over-cap capture is abandoned");
        // A later freeze can't resurrect it, and capture stays off.
        p.freeze(4);
        assert_eq!(p.frozen(), None);
        p.capture(b"more");
        assert_eq!(p.frozen(), None, "capture stays abandoned");
    }

    #[test]
    fn freeze_is_idempotent_only_the_first_boundary_counts() {
        let mut p = frozen_at(b"HELLO-WORLD", 5);
        assert_eq!(p.frozen(), Some(&b"HELLO"[..]));
        p.freeze(11); // a second (later) boundary must be ignored
        assert_eq!(p.frozen(), Some(&b"HELLO"[..]), "second freeze is a no-op");
    }

    #[test]
    fn capture_after_freeze_is_ignored() {
        let mut p = frozen_at(b"DONE", 4);
        p.capture(b"-late-output");
        assert_eq!(p.frozen(), Some(&b"DONE"[..]), "post-freeze output is not captured");
    }

    #[test]
    fn empty_frozen_preamble_is_none() {
        let mut p = BootstrapPreamble::new(1024);
        p.freeze(0); // frozen with nothing captured
        assert_eq!(p.frozen(), None, "an empty frozen preamble serves nothing");
    }

    /// A fresh adopt (`last_seq == 0`) whose ring evicted seq 0 gets the frozen
    /// preamble, and the replay starts at the preamble's end so the two never
    /// overlap. This is the T1.3 fix: without the preamble the adopting client
    /// would never see the bootstrap handshake and could not arm bootstrap.
    #[test]
    fn plan_attach_serves_preamble_and_non_overlapping_replay_on_evicted_adopt() {
        // Ring holds only the most recent 10 bytes; the session has produced 30,
        // so seq 0 (and the whole handshake) is long evicted.
        let mut ring = OutputRing::new(10);
        ring.append(&vec![b'x'; 30]);
        assert!(ring.base_seq() > 0, "precondition: the ring evicted its start");

        let preamble = frozen_at(b"HANDSHAKE!!", 6); // frozen preamble = "HANDSH" (6 bytes)
        let (base_seq, replay, sent) = plan_attach(&ring, &preamble, 0, true);

        assert_eq!(sent, b"HANDSH", "the frozen preamble is served");
        assert!(
            base_seq >= 6,
            "replay starts at or after the preamble end ({base_seq}), never overlapping [0,6)"
        );
        assert_eq!(
            base_seq,
            ring.base_seq(),
            "with the whole preamble range evicted, replay is the ring's live window"
        );
        assert_eq!(replay.len(), ring.len(), "replay is the current ring contents");
    }

    /// Backward compatibility: a client that did NOT opt in (`false`) gets the
    /// exact pre-T1.3 behaviour even on an evicted adopt — no preamble, and a
    /// plain `replay_from(0)` that is NOT shifted past a preamble it can't consume.
    #[test]
    fn plan_attach_without_client_opt_in_serves_plain_replay() {
        let mut ring = OutputRing::new(10);
        ring.append(&vec![b'x'; 30]);
        let preamble = frozen_at(b"HANDSHAKE!!", 6);

        let (base_seq, replay, sent) = plan_attach(&ring, &preamble, 0, false);
        assert!(sent.is_empty(), "an opted-out client must never receive the preamble");
        assert_eq!(base_seq, ring.base_seq(), "plain replay_from(0): the ring's live window");
        assert_eq!(replay.len(), ring.len());
    }

    /// A reconnect (`last_seq > 0`) is already bootstrapped: no preamble, just the
    /// bytes it missed. Serving a preamble here would double-arm bootstrap.
    #[test]
    fn plan_attach_sends_no_preamble_on_reconnect() {
        let mut ring = OutputRing::new(1024);
        ring.append(&vec![b'y'; 100]);
        let preamble = frozen_at(b"HANDSHAKE", 9);

        let (base_seq, _replay, sent) = plan_attach(&ring, &preamble, 40, true);
        assert!(sent.is_empty(), "a reconnect must not receive the preamble");
        assert_eq!(base_seq, 40, "a reconnect replays from its own cursor");
    }

    /// A fresh adopt of a session whose ring never evicted anything already has
    /// the handshake in its replay, so no preamble is sent (avoids double-arm).
    #[test]
    fn plan_attach_sends_no_preamble_when_nothing_evicted() {
        let mut ring = OutputRing::new(1024);
        ring.append(b"INIT...prompt$ ");
        assert_eq!(ring.base_seq(), 0, "precondition: nothing evicted");
        let preamble = frozen_at(b"INIT", 4);

        let (base_seq, replay, sent) = plan_attach(&ring, &preamble, 0, true);
        assert!(sent.is_empty(), "replay still contains the handshake; no preamble");
        assert_eq!(base_seq, 0);
        assert_eq!(replay, b"INIT...prompt$ ");
    }

    /// The middle window: the ring evicted *part* of the pre-handshake output but
    /// its base is still within the preamble range. Replay must still start at the
    /// preamble's end, giving a clean contiguous `preamble ++ replay` with no
    /// overlap and no gap.
    #[test]
    fn plan_attach_replay_starts_at_preamble_end_in_the_middle_window() {
        // 20 bytes produced, ring keeps the last 16 → base_seq == 4.
        let mut ring = OutputRing::new(16);
        ring.append(&vec![b'z'; 20]);
        assert_eq!(ring.base_seq(), 4);

        let preamble = frozen_at(&vec![b'z'; 20], 8); // preamble covers [0,8)
        let (base_seq, replay, sent) = plan_attach(&ring, &preamble, 0, true);

        assert_eq!(sent.len(), 8, "preamble [0,8) served");
        assert_eq!(base_seq, 8, "replay starts exactly at the preamble end — contiguous, no gap");
        assert_eq!(replay.len(), 12, "replay is [8,20)");
    }

    /// Without a frozen preamble (capture abandoned, or never bootstrapped) an
    /// evicted adopt falls back to the plain replay — the pre-fix behaviour.
    #[test]
    fn plan_attach_without_frozen_preamble_falls_back_to_plain_replay() {
        let mut ring = OutputRing::new(10);
        ring.append(&vec![b'x'; 30]);
        let mut preamble = BootstrapPreamble::new(8);
        preamble.capture(&vec![b'x'; 16]); // over cap → abandoned

        let (base_seq, replay, sent) = plan_attach(&ring, &preamble, 0, true);
        assert!(sent.is_empty(), "no preamble to serve");
        assert_eq!(base_seq, ring.base_seq());
        assert_eq!(replay.len(), ring.len());
    }
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
    let _ = spawner
        .spawn(move |me, ctx| me.on_session_reader_eof(&id, ctx))
        .await;
}

/// One-shot multiplexer probe: shortly after a session opens, check whether the
/// user's login profile auto-attached a terminal multiplexer despite the
/// spawn-env opt-outs (`BYOBU_DISABLE`/`LC_BYOBU` cover byobu, but hand-rolled
/// `[ -z "$TMUX" ] && tmux attach` snippets have no universal off-switch). The
/// daemon owns persistence natively, so nesting a second persistence layer is
/// worth an advisory (`SessionNotice`, kind "multiplexer-detected") — the client
/// renders a tab notice + warning toast.
///
/// Two probes (post-profile settle, then a late retry for slow profiles); stops
/// after the first hit. Deliberate *later* `tmux` use never fires — only
/// auto-attach-timed nesting is flagged, which is exactly the target.
pub(super) async fn run_multiplexer_probe(
    session_id: String,
    child_pid: u32,
    spawner: ModelSpawner<ServerModel>,
) {
    for delay_secs in [4u64, 8] {
        async_io::Timer::after(std::time::Duration::from_secs(delay_secs)).await;
        if let Some(mux) = multiplexer_on_session_tty(child_pid) {
            let id = session_id.clone();
            let _ = spawner
                .spawn(move |me, _ctx| me.on_session_multiplexer_detected(&id, &mux))
                .await;
            return;
        }
    }
}

/// Returns the multiplexer name if one is running on the session shell's TTY.
/// Portable (Linux/macOS): resolve the child's TTY via `ps -o tty=`, then list
/// the commands on that TTY — a `tmux`/`screen` client there means the session
/// landed inside a multiplexer.
fn multiplexer_on_session_tty(child_pid: u32) -> Option<String> {
    let tty_out = std::process::Command::new("ps")
        .args(["-o", "tty=", "-p", &child_pid.to_string()])
        .output()
        .ok()?;
    let tty = String::from_utf8_lossy(&tty_out.stdout).trim().to_string();
    if tty.is_empty() || tty == "?" || tty == "??" {
        return None;
    }
    let comm_out = std::process::Command::new("ps")
        .args(["-o", "comm=", "-t", &tty])
        .output()
        .ok()?;
    let comms = String::from_utf8_lossy(&comm_out.stdout);
    for line in comms.lines() {
        let comm = line.trim();
        // Linux reports the tmux client as "tmux: client"; screen's client is
        // "screen" (the detached server, "SCREEN", lives on another TTY).
        if comm.contains("tmux") {
            return Some("tmux".to_string());
        }
        if comm.eq_ignore_ascii_case("screen") {
            return Some("screen".to_string());
        }
    }
    None
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
