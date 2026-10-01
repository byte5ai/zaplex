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
use std::io::Write as _;
use std::os::fd::AsRawFd;
use std::path::Path;
use std::process::{Output, Stdio};
use std::sync::Arc;
use std::time::Duration;

use crate::terminal::model::ansi::{DProtoHook, PendingHook};
use crate::terminal::shell::ShellType;
use async_io::Async;
use futures::io::{AsyncReadExt, AsyncWriteExt};
use nix::sys::termios::{self, LocalFlags, SetArg};
use vte::{Params, Parser, Perform};
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
/// output before its handshake completes (an unbootstrappable shell, or
/// a chatty pre-prompt), capture is abandoned rather than growing unbounded.
pub(super) const BOOTSTRAP_PREAMBLE_CAP_BYTES: usize = 512 * 1024;

/// Maximum retained output delivered in one attach response. Retention may be
/// much larger, but transport frames and foreground parsing must stay bounded.
pub(super) const ATTACH_REPLAY_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Maximum retry-safe startup deliveries remembered for one PTY session.
/// Legitimate sessions normally use one; the small fixed ceiling prevents a
/// client from growing the deduplication ledger for the session lifetime.
pub(super) const MAX_ACCEPTED_STARTUP_COMMANDS: usize = 64;

/// Read chunk size for the per-session PTY reader.
const READ_CHUNK: usize = 64 * 1024;

/// The multiplexer probe is advisory and must never hold a Tokio worker while
/// waiting for a slow or wedged `ps` process.
const MULTIPLEXER_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// One ordered write to a daemon-hosted PTY.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum PtyInput {
    /// Ordinary user keyboard/mouse input. The PTY's current echo mode applies.
    Visible(Vec<u8>),
    /// Shell bootstrap bytes. The PTY echo race is closed before the session is
    /// registered; the bootstrap itself restores the interactive terminal mode.
    Bootstrap(Vec<u8>),
    /// A client-requested startup command. Its text must not become observable
    /// terminal output even though the bootstrapped shell has restored ECHO.
    Startup(Vec<u8>),
}

impl PtyInput {
    #[cfg(test)]
    pub(super) fn into_bytes(self) -> Vec<u8> {
        match self {
            Self::Visible(bytes) | Self::Bootstrap(bytes) | Self::Startup(bytes) => bytes,
        }
    }
}

pub(super) fn is_valid_startup_input(bytes: &[u8]) -> bool {
    let Some(command) = bytes.strip_suffix(b"\n") else {
        return false;
    };
    !command.is_empty() && !command.contains(&b'\r') && !command.contains(&b'\n')
}

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
    pub(super) input_tx: async_channel::Sender<PtyInput>,
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
    /// Present only for daemon-owned agent fleet entries. Managed sessions are
    /// lifecycle-mutated explicitly and are exempt from detached-session GC.
    pub(super) managed: Option<super::managed_fleet::ManagedSessionMetadata>,
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
/// 2. **Freeze** — the daemon recognizes a complete root-shell handshake and
///    freezes the prefix through its final terminator without a client round trip.
///    Older clients may still explicitly report the boundary; either path is
///    idempotent after freezing.
/// 3. **Serve** — [`Self::frozen`] yields the bytes for an adopt's
///    `SessionAttached`; the daemon starts that adopt's replay at the preamble's
///    end so the two never overlap.
pub(super) struct BootstrapPreamble {
    bytes: Vec<u8>,
    /// True until the boundary is reported (freeze) or the cap is hit (abandon).
    capturing: bool,
    /// Upper bound on captured bytes before abandoning (see the module constant).
    cap: usize,
    parser: Parser,
    handshake: BootstrapHandshake,
}

impl BootstrapPreamble {
    pub(super) fn new(cap: usize) -> Self {
        Self {
            bytes: Vec::new(),
            capturing: true,
            cap,
            parser: Parser::new(),
            handshake: BootstrapHandshake::default(),
        }
    }

    /// Mirrors output until the complete root handshake is observed. Process
    /// byte-by-byte so a large chunk containing a short handshake plus normal
    /// output cannot discard the handshake merely because the chunk exceeds cap.
    pub(super) fn capture(&mut self, bytes: &[u8]) {
        if !self.capturing {
            return;
        }
        for &byte in bytes {
            if self.bytes.len() == self.cap {
                self.bytes = Vec::new();
                self.capturing = false;
                self.handshake = BootstrapHandshake::default();
                return;
            }
            self.bytes.push(byte);
            self.handshake.byte = byte;
            // VTE ends DCS/OSC at ESC, before seeing the second byte of ST.
            // Commit only a real ESC-backslash, never an interrupted sequence.
            if let Some(action) = self.handshake.pending.take() {
                if byte == b'\\' {
                    self.handshake.commit(action);
                } else {
                    self.handshake.ambiguous = true;
                }
            }
            self.parser.advance(&mut self.handshake, byte);
            if self.handshake.complete {
                self.bytes.shrink_to_fit();
                self.capturing = false;
                self.handshake = BootstrapHandshake::default();
                return;
            }
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
        let Ok(end_seq) = usize::try_from(end_seq) else {
            self.bytes = Vec::new();
            self.capturing = false;
            self.handshake = BootstrapHandshake::default();
            return;
        };
        if end_seq > self.bytes.len() {
            self.bytes = Vec::new();
            self.capturing = false;
            self.handshake = BootstrapHandshake::default();
            return;
        }
        self.bytes.truncate(end_seq);
        self.bytes.shrink_to_fit();
        self.capturing = false;
        self.handshake = BootstrapHandshake::default();
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

/// Only observes lifecycle hooks; it never executes terminal commands or writes
/// to the PTY. Framing and payload decoding are shared with the terminal parser.
#[derive(Default)]
struct BootstrapHandshake {
    byte: u8,
    dcs_marker: Option<char>,
    dcs_data: Vec<u8>,
    pending: Option<BootstrapHookAction>,
    kv_hook: Option<PendingHook>,
    root_shell: Option<ShellType>,
    ambiguous: bool,
    complete: bool,
}

enum BootstrapHookAction {
    Hook(DProtoHook),
    Start(String),
    Update(String, String),
    End,
}

impl BootstrapHandshake {
    fn receive(&mut self, action: BootstrapHookAction) {
        match self.byte {
            0x1b => self.pending = Some(action),
            0x07 | 0x9c => self.commit(action),
            // CAN/SUB abort the frame; VTE still calls its end callback.
            _ => self.ambiguous = true,
        }
    }

    fn decode(&mut self, marker: char, bytes: &[u8]) {
        let decoded;
        let data = if marker == 'd' {
            let Ok(value) = hex::decode(bytes) else {
                return;
            };
            decoded = value;
            decoded.as_slice()
        } else {
            bytes
        };
        if let Ok(hook) = serde_json::from_slice::<DProtoHook>(data) {
            // Like the terminal, accept unencoded InitShell but never an
            // unencoded Bootstrapped payload (its fields can contain escapes).
            if marker == 'd' || matches!(hook, DProtoHook::InitShell { .. }) {
                self.receive(BootstrapHookAction::Hook(hook));
            }
        }
    }

    fn commit(&mut self, action: BootstrapHookAction) {
        if self.ambiguous {
            return;
        }
        match action {
            BootstrapHookAction::Hook(hook) => {
                if self.kv_hook.is_some() {
                    self.ambiguous = true;
                } else {
                    self.observe(hook);
                }
            }
            BootstrapHookAction::Start(name) => {
                if self.kv_hook.is_some() {
                    self.ambiguous = true;
                } else if matches!(name.as_str(), "InitShell" | "Bootstrapped") {
                    self.kv_hook = PendingHook::create(&name);
                }
            }
            BootstrapHookAction::Update(key, value) => {
                if let Some(hook) = self.kv_hook.as_mut() {
                    hook.update(key, value);
                }
            }
            BootstrapHookAction::End => {
                if let Some(hook) = self.kv_hook.take() {
                    self.observe(hook.finish());
                }
            }
        }
    }

    fn observe(&mut self, hook: DProtoHook) {
        if let DProtoHook::InitShell { value } = hook {
            if self.root_shell.is_some() || value.is_subshell || value.session_id.as_u64() == 0 {
                // Bootstrapped carries no portable session id. Never pair a
                // nested shell's completion with the root shell's InitShell.
                self.ambiguous = true;
            } else {
                self.root_shell = ShellType::from_name(&value.shell);
                self.ambiguous = self.root_shell.is_none();
            }
        } else if let DProtoHook::Bootstrapped { value } = hook {
            self.complete =
                self.root_shell.is_some() && self.root_shell == ShellType::from_name(&value.shell);
        }
    }
}

impl Perform for BootstrapHandshake {
    fn hook(&mut self, params: &Params, intermediates: &[u8], ignore: bool, marker: char) {
        self.dcs_data.clear();
        self.dcs_marker = (!ignore
            && intermediates == b"$"
            && params.iter().all(|param| param == [0])
            && matches!(marker, 'd' | 'f'))
        .then_some(marker);
    }

    fn put(&mut self, byte: u8) {
        if self.dcs_marker.is_some() {
            self.dcs_data.push(byte);
        }
    }

    fn unhook(&mut self) {
        if let Some(marker) = self.dcs_marker.take() {
            let data = std::mem::take(&mut self.dcs_data);
            self.decode(marker, &data);
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _: bool) {
        if params.first() != Some(&b"9278".as_slice()) {
            return;
        }
        match params.get(1).copied() {
            Some(b"d") | Some(b"f") if params.len() == 3 => {
                self.decode(params[1][0] as char, params[2]);
            }
            Some(b"k") => match params.get(2).copied() {
                Some(b"A") if params.len() == 4 => {
                    if let Ok(name) = std::str::from_utf8(params[3]) {
                        self.receive(BootstrapHookAction::Start(name.to_owned()));
                    }
                }
                Some(b"B") if params.len() >= 5 => {
                    if let Ok(key) = std::str::from_utf8(params[3]) {
                        let value = params[4..]
                            .iter()
                            .map(|part| String::from_utf8_lossy(part))
                            .collect::<Vec<_>>()
                            .join(";");
                        self.receive(BootstrapHookAction::Update(key.to_owned(), value));
                    }
                }
                Some(b"C") if params.len() == 3 => self.receive(BootstrapHookAction::End),
                _ => (),
            },
            _ => (),
        }
    }
}

/// Plans an `AttachSession` reply's replay window and bootstrap preamble (T1.3).
///
/// Returns `(base_seq, replay, preamble)`:
/// - On a **fresh adopt** (`last_seq == 0`) whose replay omits seq 0 through
///   ring eviction or the attach-size limit *and* has a frozen preamble: it is served and
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
    let tail_start = ring
        .end_seq()
        .saturating_sub(ATTACH_REPLAY_MAX_BYTES as u64);
    if client_supports_preamble && last_seq == 0 && (ring.base_seq() > 0 || tail_start > 0) {
        if let Some(preamble) = preamble.frozen() {
            let replay_start = (preamble.len() as u64).max(tail_start);
            let (base_seq, replay) = ring.replay_from(replay_start);
            return (base_seq, replay, preamble.to_vec());
        }
    }
    let (base_seq, replay) = ring.replay_from(last_seq.max(tail_start));
    (base_seq, replay, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    use super::super::proto::{server_message, ServerMessage, SessionAttached};
    use ::remote_server::protocol::MAX_MESSAGE_SIZE;

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
        assert_eq!(
            p.frozen(),
            None,
            "still capturing until the boundary is set"
        );
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
        assert_eq!(
            p.frozen(),
            None,
            "an out-of-range boundary abandons capture"
        );
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
        assert_eq!(
            p.frozen(),
            Some(&b"DONE"[..]),
            "post-freeze output is not captured"
        );
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
        assert!(
            ring.base_seq() > 0,
            "precondition: the ring evicted its start"
        );

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
        assert_eq!(
            replay.len(),
            ring.len(),
            "replay is the current ring contents"
        );
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
        assert!(
            sent.is_empty(),
            "an opted-out client must never receive the preamble"
        );
        assert_eq!(
            base_seq,
            ring.base_seq(),
            "plain replay_from(0): the ring's live window"
        );
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
        assert!(
            sent.is_empty(),
            "replay still contains the handshake; no preamble"
        );
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
        assert_eq!(
            base_seq, 8,
            "replay starts exactly at the preamble end — contiguous, no gap"
        );
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

    #[test]
    fn maximum_ring_attach_response_stays_within_transport_limit() {
        const HOST_RING_CAP_BYTES: usize = 256 * 1024 * 1024;
        let extra = 4096;
        let mut ring = OutputRing::new(HOST_RING_CAP_BYTES);
        let output: Vec<u8> = (0..ATTACH_REPLAY_MAX_BYTES + extra)
            .map(|index| (index % 251) as u8)
            .collect();
        ring.append(&output);

        let (base_seq, replay, preamble) = plan_attach(&ring, &BootstrapPreamble::new(1), 0, false);
        let response = ServerMessage {
            request_id: "attach-request".to_string(),
            message: Some(server_message::Message::SessionAttached(SessionAttached {
                session_id: "session-at-maximum-ring-capacity".to_string(),
                size: None,
                base_seq,
                replay: replay.clone(),
                bootstrap_preamble: preamble.clone(),
                generation: u64::MAX,
                agent_binding: None,
            })),
        };

        assert_eq!(ring.capacity(), HOST_RING_CAP_BYTES);
        assert!(preamble.is_empty());
        assert_eq!(base_seq, extra as u64);
        assert_eq!(replay, output[extra..]);
        assert!(response.encoded_len() <= MAX_MESSAGE_SIZE);
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
        async_io::Timer::after(Duration::from_secs(delay_secs)).await;
        if let Some(mux) = multiplexer_on_session_tty(child_pid).await {
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
async fn multiplexer_probe_output(args: &[&str]) -> Option<Output> {
    let mut command = command::r#async::Command::new("ps");
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    tokio::time::timeout(MULTIPLEXER_PROBE_TIMEOUT, command.output())
        .await
        .ok()?
        .ok()
}

async fn multiplexer_on_session_tty(child_pid: u32) -> Option<String> {
    let child_pid = child_pid.to_string();
    let tty_out = multiplexer_probe_output(&["-o", "tty=", "-p", &child_pid]).await?;
    let tty = String::from_utf8_lossy(&tty_out.stdout).trim().to_string();
    if tty.is_empty() || tty == "?" || tty == "??" {
        return None;
    }
    let comm_out = multiplexer_probe_output(&["-o", "comm=", "-t", &tty]).await?;
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
    input_rx: async_channel::Receiver<PtyInput>,
    shell_type: Option<ShellType>,
) {
    let mut writer: &Async<File> = &leader;
    // Keep the cleanup guards alive until the session writer ends. The sourced
    // file normally unlinks itself immediately; an aborted shell or write still
    // gets deterministic cleanup when its input channel closes.
    let mut staged_startup_files = Vec::new();
    while let Ok(input) = input_rx.recv().await {
        match input {
            PtyInput::Visible(bytes) | PtyInput::Bootstrap(bytes) => {
                if write_all(&mut writer, &bytes).await.is_err() {
                    return;
                }
            }
            PtyInput::Startup(bytes) => {
                match write_startup_via_private_file(&mut writer, &bytes, shell_type).await {
                    Ok(staged_file) => staged_startup_files.push(staged_file),
                    Err(_) => return,
                }
            }
        }
    }
}

async fn write_all(writer: &mut &Async<File>, mut bytes: &[u8]) -> std::io::Result<()> {
    while !bytes.is_empty() {
        match writer.write(bytes).await {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "daemon PTY writer returned zero bytes",
                ));
            }
            Ok(n) => bytes = &bytes[n..],
            Err(ref error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

async fn write_startup_via_private_file(
    writer: &mut &Async<File>,
    bytes: &[u8],
    shell_type: Option<ShellType>,
) -> std::io::Result<tempfile::TempPath> {
    let Some(text) = bytes.strip_suffix(b"\n").filter(|text| !text.is_empty()) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "startup input must be one non-empty line terminated by LF",
        ));
    };
    if text.contains(&b'\r') || text.contains(&b'\n') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "startup input must not contain embedded line breaks",
        ));
    }

    // Readline can render injected characters itself after bootstrap, even
    // while the kernel ECHO flag is disabled. Keep the source bytes out of the
    // PTY entirely; the private file unlinks itself before running the command.
    let mut staged = tempfile::Builder::new()
        .prefix("zaplex-startup-")
        .tempfile()?;
    let quoted_path = quote_startup_path(staged.path(), shell_type);
    let cleanup = match shell_type {
        Some(ShellType::PowerShell) => {
            format!("Remove-Item -LiteralPath {quoted_path} -Force\n")
        }
        Some(ShellType::Bash | ShellType::Zsh | ShellType::Fish) | None => {
            format!("command rm -f -- {quoted_path}\n")
        }
    };
    staged.write_all(cleanup.as_bytes())?;
    staged.write_all(text)?;
    staged.write_all(b"\n")?;
    staged.flush()?;
    let staged_path = staged.into_temp_path();

    let source_command = match shell_type {
        Some(ShellType::Fish) => format!("source {quoted_path}\n"),
        Some(ShellType::Bash | ShellType::Zsh | ShellType::PowerShell) | None => {
            format!(". {quoted_path}\n")
        }
    };
    write_startup_without_echo(writer, source_command.as_bytes()).await?;
    Ok(staged_path)
}

fn quote_startup_path(path: &Path, shell_type: Option<ShellType>) -> String {
    let path = path.to_string_lossy();
    let escaped = match shell_type {
        Some(ShellType::Fish) => path.replace('\'', r"\'"),
        Some(ShellType::PowerShell) => path.replace('\'', "''"),
        Some(ShellType::Bash | ShellType::Zsh) | None => path.replace('\'', r#"'"'"'"#),
    };
    format!("'{escaped}'")
}

/// Writes one PTY command with ECHO disabled, restores the prior echo mode, then
/// sends the final execution newline. The shell cannot execute the command
/// before that newline, so it cannot race the restoration by switching its own
/// terminal mode (for example when `codex` immediately enters raw mode).
async fn write_startup_without_echo(
    writer: &mut &Async<File>,
    bytes: &[u8],
) -> std::io::Result<()> {
    let Some(text) = bytes.strip_suffix(b"\n").filter(|text| !text.is_empty()) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "startup input must be one non-empty line terminated by LF",
        ));
    };
    if text.contains(&b'\r') || text.contains(&b'\n') {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "startup input must not contain embedded line breaks",
        ));
    }

    let fd = writer.as_raw_fd();
    let original = termios::tcgetattr(fd).map_err(std::io::Error::other)?;
    let restore_echo = original.local_flags.contains(LocalFlags::ECHO);
    let mut no_echo = original.clone();
    no_echo.local_flags.remove(LocalFlags::ECHO);
    termios::tcsetattr(fd, SetArg::TCSANOW, &no_echo).map_err(std::io::Error::other)?;

    if let Err(error) = write_all(writer, text).await {
        let _ = restore_echo_flag(fd, restore_echo);
        return Err(error);
    }

    restore_echo_flag(fd, restore_echo)?;
    write_all(writer, b"\n").await
}

fn restore_echo_flag(fd: i32, echo_was_enabled: bool) -> std::io::Result<()> {
    let mut current = termios::tcgetattr(fd).map_err(std::io::Error::other)?;
    if echo_was_enabled {
        current.local_flags.insert(LocalFlags::ECHO);
    } else {
        current.local_flags.remove(LocalFlags::ECHO);
    }
    termios::tcsetattr(fd, SetArg::TCSANOW, &current).map_err(std::io::Error::other)
}

#[cfg(test)]
#[path = "session_host_bootstrap_tests.rs"]
mod bootstrap_tests;
