use crate::remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use crate::terminal::{
    event_listener::ChannelEventListener, model::ansi::Processor,
    writeable_pty::Message as EventLoopMessage, SizeInfo, TerminalModel,
};
use async_channel::Receiver;
use parking_lot::FairMutex;
use remote_server::client::RemoteServerClient;
use std::io;
use std::sync::Arc;
use warp_core::SessionId;
use warpui::{Entity, ModelContext, SingletonEntity};

use super::terminal_manager::OpenSessionParams;

/// Cap on input buffered while the transport is down. A reconnect window is
/// normally seconds, but a laptop can sleep for hours — without a bound, typed
/// input / pastes would grow this `Vec` unboundedly. 256 KiB is far more than any
/// realistic burst of keystrokes; past it we drop the oldest input (a terminal
/// tolerates lost keystrokes far better than unbounded memory).
const MAX_PENDING_INPUT_BYTES: usize = 256 * 1024;

/// Safety valve for output buffered before `OpenSession` resolves (the daemon
/// auto-attaches and starts the shell/bootstrap before the response reaches us).
/// The open window is normally sub-second, so this is far more than any bootstrap
/// burst; past it we stop buffering (the early bootstrap prefix is preserved)
/// rather than grow without bound if an open hangs on a chatty session.
const MAX_PENDING_OUTPUT_BYTES: usize = 1024 * 1024;

/// Drives a terminal backed by a *daemon-hosted* PTY session.
///
/// Unlike [`crate::terminal::remote_tty`]'s event loop, which speaks the
/// websocket SSH-proxy protocol, this one is transport-agnostic: live PTY
/// output arrives as [`RemoteServerManagerEvent::SessionOutput`] pushes from the
/// remote-server protocol, and input/resize are routed back through the live
/// [`RemoteServerClient`]. This is what lets a session survive a transport drop
/// — the daemon owns the PTY and the replay buffer; the client is just an
/// attached view.
///
/// The daemon is responsible for bootstrapping the shell (Zaplexify init) when it
/// spawns the PTY, so — unlike the websocket path — this event loop never writes
/// a bootstrap script itself. Keeping bootstrap server-side is what makes a
/// later reattach clean: it must happen exactly once, not on every client
/// connection.
pub(super) struct EventLoop {
    terminal_model: Arc<FairMutex<TerminalModel>>,
    parser: Processor,
    channel_event_listener: ChannelEventListener,
    /// The manager/connection session used to resolve the live client.
    connection_session_id: SessionId,
    /// The daemon's PTY session id (from `OpenSession`). `None` until the open
    /// request resolves; until then input is buffered in `pending_input`.
    pty_session_id: Option<String>,
    /// Input/resize messages received before the session id is known. Flushed,
    /// in order, once `OpenSession` resolves.
    pending_input: Vec<EventLoopMessage>,
    /// Output `(pty_session_id, seq, bytes)` pushed for our connection before the
    /// `OpenSession` response arrives (the daemon auto-attaches and starts the
    /// shell immediately). Rendered, in order, in `on_session_opened`, so the
    /// initial shell/bootstrap output isn't lost on a fresh tab.
    pending_output: Vec<(String, u64, Vec<u8>)>,
    /// The `OpenSession` request, held until the transport is `Connected`. Taken
    /// (once) by `try_open`. `None` after the session has been opened.
    pending_open: Option<(OpenSessionParams, SizeInfo)>,
    /// The host's startup command, captured from `OpenSessionParams` and run once
    /// (taken) when the session opens — the daemon-path analog of the local-PTY
    /// SSH startup-command injector. `None` for adopted sessions.
    startup_command: Option<String>,
    /// Byte offset just past the last `SessionOutput` byte we've rendered. Sent
    /// as `last_seq` on re-attach so the daemon replays only what we missed.
    last_seq: u64,
    /// Human-readable host label for in-tab status lines ("… on <host>").
    host_label: String,
    /// Whether the one-time "Zaplexify active" welcome has been shown, so an
    /// adopt's first attach welcomes while later re-attaches announce the
    /// reconnect instead.
    welcomed: bool,
    /// Whether a *terminal* end-state notice has already been surfaced — a clean
    /// `session ended` (`SessionExited`) or a `connection lost`
    /// (`SessionDisconnected` with no reconnect left). Guards against a second,
    /// contradictory notice when both terminal signals reach this loop: e.g. the
    /// shell exits (`SessionExited`) and then the transport drops afterwards
    /// (`SessionDisconnected`) before the tab is closed. Whichever lands first
    /// wins; the latch swallows the other so we never tell the user the
    /// connection was lost right after telling them the session ended.
    terminated: bool,
    /// Whether this loop should still report its session's bootstrap boundary to
    /// the daemon (T1.3). True only for a session this loop *opened* (the client
    /// that witnesses the real handshake from seq 0); set false once reported, and
    /// false from the start for an *adopted* session, which did not see the
    /// handshake from seq 0 and so cannot define the boundary (by convention only
    /// the opener does). Lets the daemon freeze an eviction-proof preamble so a
    /// future adopt can arm bootstrap.
    report_bootstrap_boundary: bool,
}

impl EventLoop {
    /// Starts the event loop: subscribes to live output, begins draining
    /// input, and opens the daemon-hosted session.
    pub(super) fn start(
        model: Arc<FairMutex<TerminalModel>>,
        event_loop_rx: Receiver<EventLoopMessage>,
        channel_event_listener: ChannelEventListener,
        size_info: SizeInfo,
        connection_session_id: SessionId,
        open_params: OpenSessionParams,
        adopt_pty_session_id: Option<String>,
        install_progress_rx: Option<Receiver<String>>,
        host_label: String,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut event_loop = Self::new(model, channel_event_listener, connection_session_id);
        event_loop.host_label = host_label;
        // Name the tab from second 0: an OSC-0 title through the normal ANSI
        // path, so a connecting tab reads its host instead of sitting nameless
        // until the session opens (polish audit P0.5). Deliberately NOT the
        // sticky pane `custom_title` — the session's own title updates (the
        // bootstrap's precmd metadata drives title events once the shell is
        // up) must keep replacing this naturally. Control
        // characters are stripped: a label containing ESC/BEL would otherwise
        // terminate the sequence and inject terminal control through the
        // parser.
        let safe_label: String = event_loop
            .host_label
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        let title_seq = format!("\x1b]0;{safe_label}\x07");
        event_loop.process_pty_bytes(title_seq.as_bytes());
        match adopt_pty_session_id {
            // Adopt an existing daemon session: attach + replay on connect.
            Some(id) => event_loop.pty_session_id = Some(id),
            // Open a fresh session once the transport is connected. Only a
            // fresh open witnesses the real bootstrap handshake from seq 0, so
            // only it reports the boundary the daemon freezes (T1.3).
            None => {
                event_loop.pending_open = Some((open_params, size_info));
                event_loop.report_bootstrap_boundary = true;
            }
        }

        // First-connect auto-install: render the install ladder's phase messages
        // in this tab while the remote-server binary is being set up. The channel
        // closes when the install finishes (sender dropped), ending the stream.
        if let Some(progress_rx) = install_progress_rx {
            ctx.spawn_stream_local(
                progress_rx,
                |me, message, _ctx| me.write_progress(&message),
                |_, _| (),
            );
        }

        // Output path: live PTY bytes arrive as manager pushes. Filter to our
        // own daemon session and feed them through the ANSI processor. The
        // connect-state arms gate `OpenSession` on the transport being ready.
        let manager = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&manager, |me, event, ctx| match event {
            RemoteServerManagerEvent::SessionOutput {
                session_id,
                pty_session_id,
                seq,
                bytes,
                ..
            } => {
                if me.is_our_session(pty_session_id) {
                    me.process_pty_bytes(bytes);
                    me.last_seq = *seq + bytes.len() as u64;
                    me.maybe_report_bootstrap_boundary(ctx);
                } else if me.pty_session_id.is_none() && *session_id == me.connection_session_id {
                    // Output for our connection before `OpenSession` resolved — the
                    // daemon auto-attaches and starts the shell/bootstrap before the
                    // response reaches us. Buffer it (drained in `on_session_opened`)
                    // so the initial output isn't lost; stop past the cap so a hung
                    // open can't grow this without bound.
                    let buffered: usize = me.pending_output.iter().map(|(_, _, b)| b.len()).sum();
                    if buffered < MAX_PENDING_OUTPUT_BYTES {
                        me.pending_output
                            .push((pty_session_id.clone(), *seq, bytes.clone()));
                    }
                }
            }
            RemoteServerManagerEvent::SessionExited {
                pty_session_id,
                exit_code,
                ..
            } if me.is_our_session(pty_session_id) => {
                me.on_session_exited(*exit_code);
            }
            RemoteServerManagerEvent::SessionConnected { session_id, .. }
                if *session_id == me.connection_session_id =>
            {
                me.on_transport_connected(ctx);
            }
            // Transport reconnected (SSH blip): the daemon session kept running —
            // re-attach and replay what we missed (§9).
            RemoteServerManagerEvent::SessionReconnected { session_id, .. }
                if *session_id == me.connection_session_id =>
            {
                me.reattach(ctx);
            }
            RemoteServerManagerEvent::SessionConnectionFailed {
                session_id,
                phase,
                error,
            } if *session_id == me.connection_session_id =>
            {
                me.on_connect_failed(&format!("{phase:?}"), error);
            }
            // Advisory from the daemon: this session landed inside a terminal
            // multiplexer (hand-rolled auto-attach). zaplex owns persistence
            // natively, so surface the nesting in the tab; the workspace shows
            // the actionable warning toast.
            RemoteServerManagerEvent::SessionNotice {
                pty_session_id,
                kind,
                detail,
                ..
            } if me.is_our_session(pty_session_id) && kind == "multiplexer-detected" => {
                me.write_warning(&format!(
                    "this session is running inside {detail} (auto-attached by the host's \
                     login profile). zaplex already keeps this session alive natively — \
                     two persistence layers are nested."
                ));
            }
            // The transport went away for good: a spontaneous drop with no
            // reconnect possible, or reconnect attempts exhausted (§9). A mere
            // blip never reaches here — it arrives as `SessionReconnected` and is
            // handled above. Nothing will bring this view back on its own, so
            // surface it instead of freezing the grid on its last frame and
            // silently swallowing everything the user types.
            RemoteServerManagerEvent::SessionDisconnected { session_id, .. }
                if *session_id == me.connection_session_id =>
            {
                me.on_transport_lost();
            }
            _ => {}
        });

        // Input path: drain the channel with `ctx` access so resizes and
        // keystrokes can be routed to the live client.
        ctx.spawn_stream_local(event_loop_rx, Self::on_event_loop_message, |_, _| ());

        // If the transport is already connected, act now (open or adopt);
        // otherwise the `SessionConnected` arm above does it once it connects.
        event_loop.on_transport_connected(ctx);

        event_loop
    }

    /// On (initial) transport connect: open a fresh session if one is pending,
    /// otherwise attach to the adopted session id.
    fn on_transport_connected(&mut self, ctx: &mut ModelContext<Self>) {
        if self.pending_open.is_some() {
            self.try_open(ctx);
        } else if self.pty_session_id.is_some() {
            self.reattach(ctx);
        }
    }

    fn new(
        terminal_model: Arc<FairMutex<TerminalModel>>,
        channel_event_listener: ChannelEventListener,
        connection_session_id: SessionId,
    ) -> Self {
        Self {
            terminal_model,
            parser: Processor::default(),
            channel_event_listener,
            connection_session_id,
            pty_session_id: None,
            pending_input: Vec::new(),
            pending_output: Vec::new(),
            pending_open: None,
            startup_command: None,
            last_seq: 0,
            host_label: String::new(),
            welcomed: false,
            terminated: false,
            report_bootstrap_boundary: false,
        }
    }

    /// On transport reconnect: re-attach to the still-running daemon session and
    /// replay everything produced while we were gone, reconstructing the grid.
    /// Falls back to opening the session if it was never opened (reconnect raced
    /// the initial open).
    fn reattach(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(pty_session_id) = self.pty_session_id.clone() else {
            self.try_open(ctx);
            return;
        };
        let Some(client) = self.client(ctx) else {
            return; // The reconnected client isn't registered yet.
        };
        let last_seq = self.last_seq;
        log::info!("daemon_tty: re-attaching pty_session_id={pty_session_id} from seq {last_seq}");
        let future = async move { client.attach_session(pty_session_id, last_seq).await };
        ctx.spawn(future, |me, result, ctx| match result {
            Ok(attached) => {
                me.apply_attach(
                    &attached.bootstrap_preamble,
                    attached.base_seq,
                    &attached.replay,
                );
                // If bootstrap only completed now — a fresh open that dropped
                // mid-handshake and finished it from this reconnect's replay — the
                // live-output path never saw the flip, so report the boundary here
                // too (a no-op for adopted sessions and once already reported).
                me.maybe_report_bootstrap_boundary(ctx);
                // Stage the payoff moment: an adopt's first attach welcomes the
                // user into their running session; a reconnect after a drop
                // states plainly that nothing was lost.
                if me.welcomed {
                    me.write_zaplexify(&format!(
                        "Reconnected to {} — session restored, nothing lost.",
                        me.host_label
                    ));
                } else {
                    me.write_zaplexify(&format!(
                        "Re-attached to your running session on {} — right where you left off.",
                        me.host_label
                    ));
                    me.welcomed = true;
                }
                // Transport is back and we're re-attached — flush input buffered
                // during the outage so keystrokes/resizes aren't lost (§9).
                me.flush_pending_input(ctx);
            }
            Err(err) => {
                // Surface attach failure (e.g. the session exited in the race
                // between listing and adopting) instead of a blank tab.
                log::error!("Failed to re-attach daemon session: {err:?}");
                me.write_notice(&format!("could not re-attach session: {err}"));
            }
        });
    }

    fn is_our_session(&self, pty_session_id: &str) -> bool {
        self.pty_session_id.as_deref() == Some(pty_session_id)
    }

    /// Resolves the live client for this session from the manager, if any.
    fn client(&self, ctx: &mut ModelContext<Self>) -> Option<Arc<RemoteServerClient>> {
        let session_id = self.connection_session_id;
        let manager = RemoteServerManager::handle(ctx);
        manager.read(ctx, |manager, _ctx| {
            manager.client_for_session(session_id).cloned()
        })
    }

    /// Opens the daemon session if the transport is connected and a pending
    /// request is still outstanding. Idempotent: a no-op once opened, and a
    /// no-op (leaving the request pending) while the transport is not yet
    /// connected — the `SessionConnected` arm calls this again when it is.
    fn try_open(&mut self, ctx: &mut ModelContext<Self>) {
        if self.pty_session_id.is_some() || self.pending_open.is_none() {
            return;
        }
        let Some(client) = self.client(ctx) else {
            return; // Not connected yet; wait for `SessionConnected`.
        };
        let (open_params, size_info) = self
            .pending_open
            .take()
            .expect("pending_open is Some (checked above)");
        self.open_session(client, open_params, size_info, ctx);
    }

    /// Issues the `OpenSession` request over a connected client. The initial
    /// size is taken from the terminal model so the daemon-side PTY matches
    /// what the user sees.
    fn open_session(
        &mut self,
        client: Arc<RemoteServerClient>,
        open_params: OpenSessionParams,
        size_info: SizeInfo,
        ctx: &mut ModelContext<Self>,
    ) {
        let OpenSessionParams {
            cwd,
            shell,
            env,
            ring_ceiling_bytes,
            startup_command,
        } = open_params;
        // Run once when the session opens (see `on_session_opened`).
        self.startup_command = startup_command;
        let rows = size_info.rows as u32;
        let cols = size_info.columns as u32;
        log::info!("daemon_tty: issuing OpenSession (cwd={cwd:?}, shell={shell:?}, {rows}x{cols}, ring_ceiling={ring_ceiling_bytes:?})");
        let future =
            async move { client.open_session(cwd, shell, env, rows, cols, ring_ceiling_bytes).await };
        ctx.spawn(future, |me, result, ctx| match result {
            Ok(opened) => me.on_session_opened(opened.session_id, ctx),
            Err(err) => {
                // The transport is up (so the connect-failure path never fired),
                // but the daemon refused to open the session (bad cwd, unspawnable
                // shell, fd exhaustion, …). Surface it instead of leaving a blank,
                // hung tab; drop the pending open so a later event can't reopen it.
                log::error!("daemon_tty: OpenSession failed: {err:?}");
                me.write_notice(&format!("could not start session: {err}"));
                me.pending_open = None;
            }
        });
    }

    fn on_connect_failed(&mut self, phase: &str, error: &str) {
        log::error!(
            "daemon connect failed for {:?} at {phase}: {error}",
            self.connection_session_id
        );
        // Surface the failure in the tab so the user sees *why* instead of a
        // blank/hung view (the connection never produced any PTY output).
        self.write_notice(&format!("connection failed ({phase}): {error}"));
        // Drop the pending open so a later spurious event can't reopen it.
        self.pending_open = None;
    }

    fn on_session_opened(&mut self, pty_session_id: String, ctx: &mut ModelContext<Self>) {
        log::info!("daemon_tty: session opened, pty_session_id={pty_session_id}");
        self.pty_session_id = Some(pty_session_id.clone());
        // Stage the moment: the user should SEE they're in a persistent session
        // (the whole point of zaplex), not have to infer it. One line, once.
        self.write_zaplexify(&format!(
            "Zaplexify active — persistent session on {}. Disconnects won't lose your work.",
            self.host_label
        ));
        self.welcomed = true;
        // Render output the daemon produced before this response arrived (it
        // auto-attaches and starts the shell immediately), so the initial
        // shell/bootstrap output isn't missing from a fresh tab. In seq order.
        let pending = std::mem::take(&mut self.pending_output);
        for (pty, seq, bytes) in pending {
            if pty == pty_session_id {
                self.process_pty_bytes(&bytes);
                self.last_seq = seq + bytes.len() as u64;
            }
        }
        // The bootstrap handshake may already be complete in that pre-open burst
        // (the daemon auto-attaches and starts the shell before this response
        // lands), so report the boundary now if so (T1.3).
        self.maybe_report_bootstrap_boundary(ctx);
        // Run the host's startup command once, after the session is open — the
        // daemon-path analog of the local-PTY SSH startup-command injector. Sent
        // as input + newline (bash/zsh execute byte), the same way the daemon
        // injects its own bootstrap. `take()` ensures it never re-runs on reattach.
        if let Some(cmd) = self.startup_command.take() {
            if !cmd.is_empty() {
                let mut bytes = cmd.into_bytes();
                bytes.push(b'\n');
                self.dispatch_message(
                    &pty_session_id,
                    EventLoopMessage::Input(std::borrow::Cow::Owned(bytes)),
                    ctx,
                );
            }
        }
        // Flush any input that arrived before the session was addressable.
        self.flush_pending_input(ctx);
    }

    /// Flush input buffered while the session wasn't addressable — either before
    /// the first open (pre-`pty_session_id`) or while the transport was down
    /// mid-session (the reconnect window). A no-op when nothing is pending or no
    /// session id exists yet. Any message whose client is *still* unavailable is
    /// re-buffered by `dispatch_message`, so it survives until the next flush.
    fn flush_pending_input(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(pty_session_id) = self.pty_session_id.clone() else {
            return;
        };
        let pending = std::mem::take(&mut self.pending_input);
        for message in pending {
            self.dispatch_message(&pty_session_id, message, ctx);
        }
    }

    fn on_event_loop_message(&mut self, message: EventLoopMessage, ctx: &mut ModelContext<Self>) {
        match self.pty_session_id.clone() {
            Some(pty_session_id) => self.dispatch_message(&pty_session_id, message, ctx),
            None => self.buffer_pending(message),
        }
    }

    /// Buffer an input/resize while the session isn't addressable (pre-open or
    /// transport down). Coalesces resizes (only the latest matters) and bounds the
    /// buffered input bytes, dropping the oldest input past the cap so a long
    /// outage can't grow `pending_input` without limit.
    fn buffer_pending(&mut self, message: EventLoopMessage) {
        if matches!(message, EventLoopMessage::Resize(_)) {
            // Intermediate window sizes are irrelevant — keep only the latest.
            self.pending_input
                .retain(|m| !matches!(m, EventLoopMessage::Resize(_)));
        }
        self.pending_input.push(message);

        let mut total: usize = self
            .pending_input
            .iter()
            .map(|m| match m {
                EventLoopMessage::Input(b) => b.len(),
                _ => 0,
            })
            .sum();
        if total > MAX_PENDING_INPUT_BYTES {
            log::warn!(
                "daemon_tty: buffered input exceeded {MAX_PENDING_INPUT_BYTES} bytes during an \
                 outage — dropping oldest input"
            );
            let mut i = 0;
            while total > MAX_PENDING_INPUT_BYTES && i < self.pending_input.len() {
                if let EventLoopMessage::Input(b) = &self.pending_input[i] {
                    total -= b.len();
                    self.pending_input.remove(i);
                } else {
                    i += 1;
                }
            }
        }
    }

    fn dispatch_message(
        &mut self,
        pty_session_id: &str,
        message: EventLoopMessage,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(client) = self.client(ctx) else {
            // Transport is down (e.g. an SSH blip mid-session). Buffer instead of
            // dropping so keystrokes/resizes survive the outage — `reattach`
            // flushes them once the transport reconnects (§9 resilience). This is
            // the whole point of the native session layer: a drop must not lose
            // input that was typed during the gap.
            log::debug!(
                "daemon_tty: buffering {message:?} for {pty_session_id} (transport down, will flush on reattach)"
            );
            self.buffer_pending(message);
            return;
        };
        let result = match message {
            EventLoopMessage::Input(bytes) => {
                client.send_session_input(pty_session_id.to_string(), bytes.into_owned())
            }
            EventLoopMessage::Resize(size_info) => client.send_resize_session(
                pty_session_id.to_string(),
                size_info.rows as u32,
                size_info.columns as u32,
            ),
            // The daemon owns the PTY lifecycle; a client-side shutdown simply
            // detaches this view — the session keeps running for reattachment.
            EventLoopMessage::Shutdown | EventLoopMessage::ChildExited => {
                client.send_detach_session(pty_session_id.to_string())
            }
        };
        if let Err(err) = result {
            log::error!("Failed to send message to daemon session {pty_session_id}: {err:?}");
        }
    }

    fn on_session_exited(&mut self, exit_code: Option<i32>) {
        log::info!(
            "Daemon session {:?} exited (code {exit_code:?})",
            self.pty_session_id
        );
        // A clean exit is a terminal state: latch it so that if the transport
        // later drops (a `SessionDisconnected` reaching this still-open tab) we
        // don't append a contradictory "connection lost" line under this one.
        self.terminated = true;
        let notice = match exit_code {
            Some(code) => format!("session ended (exit code {code})"),
            None => "session ended".to_string(),
        };
        self.write_notice(&notice);
    }

    /// A terminal transport loss with no auto-reconnect left (spontaneous drop
    /// or reconnect exhausted). Tell the user once instead of leaving a frozen
    /// grid that quietly eats keystrokes — and be honest about the payoff: the
    /// daemon owns the PTY, so a persistent session is very likely still running
    /// on the host and reopening it reattaches. No-op if a terminal state was
    /// already surfaced (a clean `SessionExited`, or a prior disconnect).
    fn on_transport_lost(&mut self) {
        if self.terminated {
            return;
        }
        self.terminated = true;
        self.write_notice(&format!(
            "connection to {} lost. Your persistent session may still be running there — \
             reopen the host to reattach.",
            self.host_label
        ));
    }

    /// Writes a Zaplex notice line (e.g. a connection error or session-ended
    /// message) into the terminal via the normal ANSI path, so the user sees it
    /// in the tab rather than a blank/hung view. Rendered in bold red.
    fn write_notice(&mut self, text: &str) {
        let line = format!("\r\n\x1b[1;31m[zaplex] {text}\x1b[0m\r\n");
        self.process_pty_bytes(line.as_bytes());
    }

    /// Neutral (non-error) status line, used for install/setup progress. Dim
    /// cyan instead of the red error styling of [`Self::write_notice`].
    fn write_progress(&mut self, text: &str) {
        let line = format!("\r\n\x1b[2;36m[zaplex] {text}\x1b[0m\r\n");
        self.process_pty_bytes(line.as_bytes());
    }

    /// Advisory (non-fatal) warning line — yellow, between the dim-cyan
    /// progress and the red error notices.
    fn write_warning(&mut self, text: &str) {
        let line = format!("\r\n\x1b[1;33m[zaplex] {text}\x1b[0m\r\n");
        self.process_pty_bytes(line.as_bytes());
    }

    /// The Zaplexify signature line — bold cyan. Used for the persistent-session
    /// welcome and the reconnect/re-attach payoff moments.
    fn write_zaplexify(&mut self, text: &str) {
        let line = format!("\r\n\x1b[1;36m{text}\x1b[0m\r\n");
        self.process_pty_bytes(line.as_bytes());
    }

    /// Processes a byte slice through the [`Processor`], identical to the
    /// local- and remote-PTY paths.
    fn process_pty_bytes(&mut self, bytes: &[u8]) {
        let mut terminal_model = self.terminal_model.lock();
        self.parser
            .parse_bytes(&mut *terminal_model, bytes, &mut io::sink());
        self.channel_event_listener.send_wakeup_event();
    }

    /// Applies an attach reply's bootstrap preamble and replay to the terminal,
    /// advancing the `last_seq` replay cursor. Split out of `reattach` so the
    /// preamble/gap/replay bookkeeping (the T1.3-sensitive part) is unit-testable
    /// without a live client.
    ///
    /// - **Preamble** (T1.3): on an adopt whose ring already evicted the bootstrap
    ///   handshake, the daemon ships it as `bootstrap_preamble`. Feed it through
    ///   the normal parser path first — arming bootstrap exactly as a fresh
    ///   session would — but only if we aren't already bootstrapped (a reconnect
    ///   is). `base_seq` already points past the preamble range, so preamble and
    ///   replay never overlap.
    /// - **Gap**: if the ring evicted output we never saw (`base_seq > last_seq`),
    ///   applying post-gap bytes onto the stale grid would corrupt it, so reset
    ///   the screen — and, if a preamble was just fed, the parser too, so a
    ///   preamble that ended mid-sequence can't bleed into the replay — then note
    ///   the truncation.
    fn apply_attach(&mut self, bootstrap_preamble: &[u8], base_seq: u64, replay: &[u8]) {
        let fed_preamble = !bootstrap_preamble.is_empty() && !self.is_bootstrapped();
        if fed_preamble {
            // The daemon already bootstrapped this shell server-side, so arming
            // bootstrap from the re-fed preamble must NOT make the client write
            // the bootstrap body back into the running shell (T1.3). Arm the
            // one-shot latch: the preamble's `InitShell` — parsed synchronously by
            // the line below — consumes it and stamps *its* event so only that
            // event skips the write.
            self.terminal_model.lock().suppress_next_bootstrap_write();
            self.process_pty_bytes(bootstrap_preamble);
            // Belt-and-suspenders: if the preamble somehow carried no `InitShell`
            // the latch is still armed; clear it so it can never leak onto a later
            // genuine `InitShell`. (A frozen preamble always contains the
            // handshake, so this is normally a no-op.)
            self.terminal_model.lock().take_suppress_next_bootstrap_write();
            self.last_seq = bootstrap_preamble.len() as u64;
        }
        if base_seq > self.last_seq {
            if fed_preamble {
                self.reset_parser();
            }
            self.process_pty_bytes(b"\x1b[H\x1b[2J\x1b[3J");
            self.write_notice("scrollback truncated during a long disconnect");
        }
        if !replay.is_empty() {
            self.process_pty_bytes(replay);
        }
        self.last_seq = base_seq + replay.len() as u64;
    }

    /// Whether the terminal model has completed the Zaplexify bootstrap.
    fn is_bootstrapped(&self) -> bool {
        self.terminal_model.lock().block_list().is_bootstrapped()
    }

    /// Resets the ANSI parser to its ground state without touching the terminal
    /// model. Used between a bootstrap preamble and a post-gap replay so a
    /// preamble that ended mid-sequence can't corrupt the replay (T1.3); the
    /// model's bootstrap arming lives in the block list, not the parser, so it
    /// survives this reset.
    fn reset_parser(&mut self) {
        self.parser = Processor::default();
    }

    /// Reports this session's bootstrap boundary to the daemon exactly once —
    /// only for a session this loop opened, only after the model is bootstrapped,
    /// and only with a live client (T1.3). The daemon freezes the output up to
    /// `last_seq` as an eviction-proof preamble for future adopts. A no-op
    /// afterwards, for adopted sessions, and while not yet bootstrapped; if the
    /// transport is momentarily down it stays pending and retries on the next
    /// output chunk.
    fn maybe_report_bootstrap_boundary(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.report_bootstrap_boundary {
            return;
        }
        let Some(pty_session_id) = self.pty_session_id.clone() else {
            return;
        };
        if !self.is_bootstrapped() {
            return;
        }
        let Some(client) = self.client(ctx) else {
            return; // Transport down; retry on the next output chunk.
        };
        // Only latch off once the report is actually enqueued; a lost send (closed
        // or full channel) leaves the flag set so a later output chunk retries —
        // otherwise the daemon never freezes the preamble and a future adopt hits
        // T1.3 again.
        if client.set_bootstrap_preamble(pty_session_id, self.last_seq) {
            self.report_bootstrap_boundary = false;
        }
    }
}

impl Entity for EventLoop {
    type Event = ();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;
    use warp_core::HostId;
    use warpui::{App, ModelHandle};

    const OUR_PTY: &str = "pty-ours";
    const HOST: &str = "test-host";

    /// A [`ChannelEventListener`] whose wakeup channel we keep. `process_pty_bytes`
    /// fires a wakeup *after* feeding the bytes through the ANSI processor into the
    /// terminal model, so an observed wakeup proves the output reached the parser
    /// and model for our session (the shared parser's rendering itself is covered
    /// by the terminal-model / ANSI tests — here we test daemon-session routing).
    fn test_listener() -> (ChannelEventListener, async_channel::Receiver<()>) {
        let (wakeups_tx, wakeups_rx) = async_channel::unbounded();
        let (events_tx, _events_rx) = async_channel::unbounded();
        let (pty_reads_tx, _pty_reads_rx) = async_broadcast::broadcast(1);
        (
            ChannelEventListener::new(wakeups_tx, events_tx, pty_reads_tx),
            wakeups_rx,
        )
    }

    fn output_event(
        conn: SessionId,
        pty: &str,
        seq: u64,
        bytes: &[u8],
    ) -> RemoteServerManagerEvent {
        RemoteServerManagerEvent::SessionOutput {
            session_id: conn,
            host_id: HostId::new(HOST.to_string()),
            pty_session_id: pty.to_string(),
            seq,
            bytes: bytes.to_vec(),
        }
    }

    fn drain<T>(rx: &async_channel::Receiver<T>) {
        while rx.try_recv().is_ok() {}
    }

    /// Starts an EventLoop that has *adopted* `OUR_PTY` (so it is immediately
    /// addressable without a connected client to open) on a real
    /// `RemoteServerManager` singleton. The manager is what the loop subscribes
    /// to for live `SessionOutput`, so emitting from it drives the real path.
    fn start_adopted_loop(
        app: &mut App,
        conn: SessionId,
    ) -> (
        ModelHandle<RemoteServerManager>,
        ModelHandle<EventLoop>,
        Arc<FairMutex<TerminalModel>>,
        async_channel::Receiver<()>,
    ) {
        start_adopted_loop_impl(app, conn, true)
    }

    /// Like [`start_adopted_loop`] but the terminal model is NOT bootstrapped,
    /// matching a real adopt tab — so `apply_attach` actually feeds the preamble
    /// (`fed_preamble` is true) instead of short-circuiting on an already-
    /// bootstrapped model (T1.3).
    fn start_adopted_loop_unbootstrapped(
        app: &mut App,
        conn: SessionId,
    ) -> (
        ModelHandle<RemoteServerManager>,
        ModelHandle<EventLoop>,
        Arc<FairMutex<TerminalModel>>,
        async_channel::Receiver<()>,
    ) {
        start_adopted_loop_impl(app, conn, false)
    }

    fn start_adopted_loop_impl(
        app: &mut App,
        conn: SessionId,
        bootstrapped: bool,
    ) -> (
        ModelHandle<RemoteServerManager>,
        ModelHandle<EventLoop>,
        Arc<FairMutex<TerminalModel>>,
        async_channel::Receiver<()>,
    ) {
        let manager = app.add_singleton_model(RemoteServerManager::new);
        let (listener, wakeups_rx) = test_listener();
        let model = Arc::new(FairMutex::new(if bootstrapped {
            TerminalModel::mock(None, Some(listener.clone()))
        } else {
            TerminalModel::mock_not_bootstrapped(Some(listener.clone()))
        }));
        // The input stream isn't exercised here; dropping the sender just closes it.
        let (_event_loop_tx, event_loop_rx) = async_channel::unbounded::<EventLoopMessage>();
        let size = SizeInfo::new_without_font_metrics(24, 80);
        let model_for_loop = model.clone();
        let event_loop = app.add_model(|ctx| {
            EventLoop::start(
                model_for_loop,
                event_loop_rx,
                listener,
                size,
                conn,
                OpenSessionParams::default(),
                Some(OUR_PTY.to_string()),
                None,
                "test-host".to_string(),
                ctx,
            )
        });
        (manager, event_loop, model, wakeups_rx)
    }

    /// The core client-side output path: a live `SessionOutput` push for our
    /// daemon session is fed to the terminal (proven by the repaint wakeup) and
    /// advances `last_seq` (= seq + len, the replay cursor) — while a push for a
    /// *different* `pty_session_id` on the same connection is ignored.
    #[test]
    fn session_output_routes_to_terminal_and_filters_by_pty() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(7u64);
            let (manager, event_loop, _model, wakeups_rx) = start_adopted_loop(&mut app, conn);

            // Delivery is synchronous: `ctx.emit` queues an effect that
            // `flush_effects` dispatches to subscribers before `update` returns.
            manager.update(&mut app, |_m, ctx| {
                ctx.emit(output_event(conn, OUR_PTY, 0, b"hello-daemon"));
            });

            assert!(
                !wakeups_rx.is_empty(),
                "our SessionOutput must reach the parser/model and request a repaint"
            );
            assert_eq!(
                event_loop.read(&app, |me, _| me.last_seq),
                b"hello-daemon".len() as u64,
                "last_seq must advance to seq + bytes.len() (the replay cursor)"
            );

            // A push for another session on the same connection is filtered out.
            drain(&wakeups_rx);
            manager.update(&mut app, |_m, ctx| {
                ctx.emit(output_event(conn, "pty-someone-else", 999, b"NOT-OURS"));
            });
            assert!(
                wakeups_rx.is_empty(),
                "output for a foreign pty_session_id must not reach our terminal"
            );
            assert_eq!(
                event_loop.read(&app, |me, _| me.last_seq),
                b"hello-daemon".len() as u64,
                "foreign output must not advance our last_seq"
            );

            // A contiguous follow-up chunk for our session advances the cursor by
            // its own length from the new seq.
            manager.update(&mut app, |_m, ctx| {
                ctx.emit(output_event(conn, OUR_PTY, 12, b"-more"));
            });
            assert_eq!(
                event_loop.read(&app, |me, _| me.last_seq),
                (b"hello-daemon".len() + b"-more".len()) as u64,
                "last_seq tracks the latest seq + len"
            );
        });
    }

    /// Keystrokes that arrive before `OpenSession` resolves are buffered in order
    /// so nothing typed during the connect window is lost. On open we attempt to
    /// flush; with no live client the input is *retained* (re-buffered), never
    /// dropped — it flushes for real once a client is available.
    #[test]
    fn input_before_session_open_is_buffered_and_not_lost() {
        App::test((), |mut app| async move {
            // Held for the duration so the singleton stays registered.
            let _manager = app.add_singleton_model(RemoteServerManager::new);
            let conn = SessionId::from(9u64);
            let (listener, _wakeups_rx) = test_listener();
            let model = Arc::new(FairMutex::new(TerminalModel::mock(None, Some(listener.clone()))));
            let (_event_loop_tx, event_loop_rx) = async_channel::unbounded::<EventLoopMessage>();
            let size = SizeInfo::new_without_font_metrics(24, 80);
            let model_for_loop = model.clone();
            // `None` = open a fresh session; with no connected client it never
            // resolves, so `pty_session_id` stays `None` and input must buffer.
            let event_loop = app.add_model(|ctx| {
                EventLoop::start(
                    model_for_loop,
                    event_loop_rx,
                    listener,
                    size,
                    conn,
                    OpenSessionParams::default(),
                    None,
                    None,
                    "test-host".to_string(),
                    ctx,
                )
            });

            event_loop.update(&mut app, |me, ctx| {
                me.on_event_loop_message(EventLoopMessage::Input(Cow::Owned(b"a".to_vec())), ctx);
                me.on_event_loop_message(EventLoopMessage::Input(Cow::Owned(b"b".to_vec())), ctx);
            });
            event_loop.read(&app, |me, _| {
                assert!(me.pty_session_id.is_none(), "session not opened yet");
                assert_eq!(me.pending_input.len(), 2, "input must be buffered before open");
            });

            // Opening records the id and attempts to flush. With no live client
            // the input can't be sent yet, so it must be *retained* (re-buffered),
            // not dropped — preserving the no-loss guarantee until a client exists.
            event_loop.update(&mut app, |me, ctx| {
                me.on_session_opened("pty-late".to_string(), ctx);
            });
            event_loop.read(&app, |me, _| {
                assert_eq!(me.pty_session_id.as_deref(), Some("pty-late"));
                assert_eq!(
                    me.pending_input.len(),
                    2,
                    "without a live client the flushed input must be retained, not lost"
                );
            });
        });
    }

    /// Regression (§9 resilience): once a session is open, input that arrives
    /// while the transport is down (the reconnect window) must be buffered, not
    /// dropped — otherwise keystrokes typed during an SSH blip are lost. The
    /// adopted loop has a `pty_session_id` but no registered client, which is
    /// exactly the "session open, transport down" state.
    #[test]
    fn input_during_transport_outage_is_buffered_not_dropped() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(13u64);
            let (_manager, event_loop, _model, _wakeups_rx) = start_adopted_loop(&mut app, conn);

            event_loop.read(&app, |me, _| {
                assert_eq!(
                    me.pty_session_id.as_deref(),
                    Some(OUR_PTY),
                    "adopted loop is open (has a pty id) but has no live client"
                );
            });

            // Session is open, transport is down (no client): input must buffer.
            event_loop.update(&mut app, |me, ctx| {
                me.on_event_loop_message(EventLoopMessage::Input(Cow::Owned(b"x".to_vec())), ctx);
                me.on_event_loop_message(
                    EventLoopMessage::Resize(SizeInfo::new_without_font_metrics(40, 100)),
                    ctx,
                );
            });
            event_loop.read(&app, |me, _| {
                assert_eq!(
                    me.pending_input.len(),
                    2,
                    "input during the outage must be buffered (flushed on reattach), not dropped"
                );
            });
        });
    }

    /// The host's startup command runs once when the session opens — the
    /// daemon-path analog of the local-PTY SSH startup-command injector. With no
    /// live client the queued command lands in `pending_input` (sent for real on
    /// reattach), so we assert it was queued as `command + "\n"`.
    #[test]
    fn startup_command_is_queued_as_input_on_open() {
        App::test((), |mut app| async move {
            let _manager = app.add_singleton_model(RemoteServerManager::new);
            let conn = SessionId::from(17u64);
            let (listener, _wakeups_rx) = test_listener();
            let model = Arc::new(FairMutex::new(TerminalModel::mock(None, Some(listener.clone()))));
            let (_event_loop_tx, event_loop_rx) = async_channel::unbounded::<EventLoopMessage>();
            let size = SizeInfo::new_without_font_metrics(24, 80);
            let model_for_loop = model.clone();
            let event_loop = app.add_model(|ctx| {
                EventLoop::start(
                    model_for_loop,
                    event_loop_rx,
                    listener,
                    size,
                    conn,
                    OpenSessionParams::default(),
                    None,
                    None,
                    "test-host".to_string(),
                    ctx,
                )
            });

            event_loop.update(&mut app, |me, ctx| {
                me.startup_command = Some("tmux attach".to_string());
                me.on_session_opened("pty-x".to_string(), ctx);
            });

            event_loop.read(&app, |me, _| {
                assert!(me.startup_command.is_none(), "startup command must be taken (run once)");
                assert_eq!(me.pending_input.len(), 1, "startup command queued as input");
                match &me.pending_input[0] {
                    EventLoopMessage::Input(bytes) => {
                        assert_eq!(&**bytes, b"tmux attach\n", "command + execute newline");
                    }
                    other => panic!("expected Input, got {other:?}"),
                }
            });
        });
    }

    /// During a long outage the buffered input must stay bounded: consecutive
    /// resizes coalesce to the latest, and input past the byte cap drops oldest-
    /// first — so a sleeping laptop can't grow `pending_input` without limit.
    #[test]
    fn buffered_input_is_capped_and_resizes_coalesce() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(19u64);
            // Adopted loop: pty id set, no live client → everything buffers.
            let (_manager, event_loop, _model, _wakeups_rx) = start_adopted_loop(&mut app, conn);

            event_loop.update(&mut app, |me, ctx| {
                me.on_event_loop_message(
                    EventLoopMessage::Resize(SizeInfo::new_without_font_metrics(20, 60)),
                    ctx,
                );
                me.on_event_loop_message(
                    EventLoopMessage::Resize(SizeInfo::new_without_font_metrics(30, 90)),
                    ctx,
                );
                // 5 x 100 KiB = 500 KiB of input, over the 256 KiB cap.
                for _ in 0..5 {
                    me.on_event_loop_message(
                        EventLoopMessage::Input(Cow::Owned(vec![b'x'; 100 * 1024])),
                        ctx,
                    );
                }
            });

            event_loop.read(&app, |me, _| {
                let resizes = me
                    .pending_input
                    .iter()
                    .filter(|m| matches!(m, EventLoopMessage::Resize(_)))
                    .count();
                assert_eq!(resizes, 1, "consecutive resizes coalesce to the latest");
                let input_bytes: usize = me
                    .pending_input
                    .iter()
                    .map(|m| match m {
                        EventLoopMessage::Input(b) => b.len(),
                        _ => 0,
                    })
                    .sum();
                assert!(
                    input_bytes <= MAX_PENDING_INPUT_BYTES,
                    "buffered input must be capped (was {input_bytes})"
                );
            });
        });
    }

    /// Output the daemon pushes before `OpenSession` resolves (it auto-attaches
    /// and starts the shell immediately) must not be lost: it is buffered while
    /// the pty id is unknown, then rendered when `on_session_opened` records the id.
    #[test]
    fn output_before_open_is_buffered_then_rendered() {
        App::test((), |mut app| async move {
            let manager = app.add_singleton_model(RemoteServerManager::new);
            let conn = SessionId::from(23u64);
            let (listener, wakeups_rx) = test_listener();
            let model = Arc::new(FairMutex::new(TerminalModel::mock(None, Some(listener.clone()))));
            let (_event_loop_tx, event_loop_rx) = async_channel::unbounded::<EventLoopMessage>();
            let size = SizeInfo::new_without_font_metrics(24, 80);
            let model_for_loop = model.clone();
            // Fresh open (adopt = None): with no live client the open never
            // resolves, so pty_session_id stays None and output must buffer.
            let event_loop = app.add_model(|ctx| {
                EventLoop::start(
                    model_for_loop,
                    event_loop_rx,
                    listener,
                    size,
                    conn,
                    OpenSessionParams::default(),
                    None,
                    None,
                    "test-host".to_string(),
                    ctx,
                )
            });

            // Daemon pushes output for our connection before OpenSession resolves.
            manager.update(&mut app, |_m, ctx| {
                ctx.emit(output_event(conn, "pty-late", 0, b"BOOT"));
            });
            event_loop.read(&app, |me, _| {
                assert!(me.pty_session_id.is_none(), "not opened yet");
                assert_eq!(
                    me.pending_output.len(),
                    1,
                    "pre-open output must be buffered, not dropped"
                );
            });

            // Opening renders the buffered output (proven by the repaint wakeup),
            // advances last_seq, and clears the buffer.
            drain(&wakeups_rx);
            event_loop.update(&mut app, |me, ctx| {
                me.on_session_opened("pty-late".to_string(), ctx)
            });
            assert!(
                !wakeups_rx.is_empty(),
                "buffered pre-open output must be rendered on open"
            );
            event_loop.read(&app, |me, _| {
                assert!(me.pending_output.is_empty(), "buffer drained on open");
                assert_eq!(
                    me.last_seq,
                    b"BOOT".len() as u64,
                    "last_seq advances past the replayed pre-open output"
                );
            });
        });
    }

    /// A connect failure must surface in the tab — `on_connect_failed` renders a
    /// notice through the terminal (so the user sees *why* instead of a blank /
    /// hung view), which requests a repaint.
    #[test]
    fn connect_failure_writes_a_visible_notice() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(11u64);
            let (_manager, event_loop, _model, wakeups_rx) = start_adopted_loop(&mut app, conn);
            drain(&wakeups_rx);
            event_loop.update(&mut app, |me, _| {
                me.on_connect_failed("Connect", "ssh: connect timed out")
            });
            assert!(
                !wakeups_rx.is_empty(),
                "a connect failure must render a notice and request a repaint"
            );
        });
    }

    /// Regression (T1.2): a *terminal* transport loss for our connection — a
    /// spontaneous drop with no reconnect, or reconnect exhausted (§9) — must
    /// surface a notice, not freeze the grid on its last frame while silently
    /// swallowing every keystroke. (A mere blip never reaches this arm; it
    /// arrives as `SessionReconnected`.) Proven by the repaint wakeup the notice
    /// fires and the `terminated` latch it sets. Without the fix the event falls
    /// into `_ => {}`: no wakeup, no latch — the frozen tab the user reported.
    #[test]
    fn terminal_disconnect_is_surfaced_not_frozen() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(23u64);
            let (manager, event_loop, _model, wakeups_rx) = start_adopted_loop(&mut app, conn);
            drain(&wakeups_rx);

            manager.update(&mut app, |_m, ctx| {
                ctx.emit(RemoteServerManagerEvent::SessionDisconnected {
                    session_id: conn,
                    host_id: HostId::new(HOST.to_string()),
                    exit_status: None,
                });
            });

            assert!(
                !wakeups_rx.is_empty(),
                "a terminal disconnect must write a notice (repaint wakeup), not freeze the grid"
            );
            assert!(
                event_loop.read(&app, |me, _| me.terminated),
                "a terminal disconnect must latch `terminated`"
            );
        });
    }

    /// If a terminal `SessionDisconnected` ever reaches this loop *after* a clean
    /// shell exit — e.g. the transport drops post-exit while the tab is still
    /// open — it must not append a contradictory "connection lost" line under the
    /// "session ended" one: the `terminated` latch set by `SessionExited` swallows
    /// the later disconnect (no second wakeup). (Both events are emitted here by
    /// the test to exercise the latch directly.)
    #[test]
    fn clean_exit_suppresses_the_trailing_disconnect_notice() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(29u64);
            let (manager, event_loop, _model, wakeups_rx) = start_adopted_loop(&mut app, conn);
            drain(&wakeups_rx);

            // Clean exit first — one notice ("session ended").
            manager.update(&mut app, |_m, ctx| {
                ctx.emit(RemoteServerManagerEvent::SessionExited {
                    session_id: conn,
                    host_id: HostId::new(HOST.to_string()),
                    pty_session_id: OUR_PTY.to_string(),
                    exit_code: Some(0),
                });
            });
            assert!(
                !wakeups_rx.is_empty(),
                "a clean exit must write the session-ended notice"
            );
            assert!(
                event_loop.read(&app, |me, _| me.terminated),
                "a clean exit latches `terminated`"
            );
            drain(&wakeups_rx);

            // The teardown disconnect that follows must be swallowed.
            manager.update(&mut app, |_m, ctx| {
                ctx.emit(RemoteServerManagerEvent::SessionDisconnected {
                    session_id: conn,
                    host_id: HostId::new(HOST.to_string()),
                    exit_status: None,
                });
            });
            assert!(
                wakeups_rx.is_empty(),
                "after a clean exit, the trailing disconnect must not add a second, \
                 contradictory notice"
            );
        });
    }

    /// T1.3: an adopt whose ring evicted the handshake receives a
    /// `bootstrap_preamble`; `apply_attach` feeds it (arming bootstrap via the
    /// normal parser path) and then the contiguous replay, tracking the cursor as
    /// `preamble_end + replay_len`. A repaint wakeup proves both reached the
    /// parser. (The mock model is never bootstrapped, so the preamble is always
    /// fed here — exactly the evicted-adopt case.)
    #[test]
    fn apply_attach_feeds_preamble_then_replay_contiguously() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(31u64);
            let (_manager, event_loop, _model, wakeups_rx) =
                start_adopted_loop_unbootstrapped(&mut app, conn);
            drain(&wakeups_rx);

            // Preamble "PRE" (3 bytes); replay starts exactly at seq 3 (contiguous).
            event_loop.update(&mut app, |me, _| me.apply_attach(b"PRE", 3, b"replay"));

            assert!(
                !wakeups_rx.is_empty(),
                "the preamble and replay must reach the parser/model (repaint wakeup)"
            );
            assert_eq!(
                event_loop.read(&app, |me, _| me.last_seq),
                (3 + 6) as u64,
                "last_seq = preamble end (3) advanced by the replay length (6)"
            );
        });
    }

    /// T1.3: when the replay starts past the preamble's end (`base_seq > preamble
    /// end`), the evicted bytes are a genuine hole: the screen is reset and the
    /// user is told scrollback was truncated, then the replay is applied and the
    /// cursor lands at `base_seq + replay_len`.
    #[test]
    fn apply_attach_preamble_then_gap_truncates_and_advances_cursor() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(37u64);
            let (_manager, event_loop, _model, wakeups_rx) =
                start_adopted_loop_unbootstrapped(&mut app, conn);
            drain(&wakeups_rx);

            // Preamble is 3 bytes; replay starts at seq 10 → a gap of [3,10).
            event_loop.update(&mut app, |me, _| me.apply_attach(b"PRE", 10, b"tail"));

            assert!(
                !wakeups_rx.is_empty(),
                "the gap path still renders (reset + truncation notice + replay)"
            );
            assert_eq!(
                event_loop.read(&app, |me, _| me.last_seq),
                (10 + 4) as u64,
                "after a gap, last_seq = base_seq (10) + replay length (4)"
            );
        });
    }

    /// A fresh short adopt (or any attach without a preamble) replays plainly from
    /// `base_seq`; the handshake is still in the replay, so no preamble is fed.
    #[test]
    fn apply_attach_without_preamble_replays_plainly() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(41u64);
            let (_manager, event_loop, _model, _wakeups_rx) = start_adopted_loop(&mut app, conn);

            event_loop.update(&mut app, |me, _| me.apply_attach(b"", 0, b"hello"));

            assert_eq!(
                event_loop.read(&app, |me, _| me.last_seq),
                b"hello".len() as u64,
                "no preamble: last_seq is the replay length from base_seq 0"
            );
        });
    }

    /// A reconnect (already-advanced cursor, no preamble, no gap) replays only
    /// what was missed and advances from its own cursor — the daemon never ships a
    /// preamble here, and `apply_attach` must not fabricate a gap.
    #[test]
    fn apply_attach_reconnect_replays_from_existing_cursor() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(43u64);
            let (_manager, event_loop, _model, _wakeups_rx) = start_adopted_loop(&mut app, conn);

            event_loop.update(&mut app, |me, _| {
                me.last_seq = 100; // already consumed 100 bytes before the blip
                me.apply_attach(b"", 100, b"more");
            });
            assert_eq!(
                event_loop.read(&app, |me, _| me.last_seq),
                (100 + 4) as u64,
                "a reconnect advances from its own cursor (base_seq 100 + 4)"
            );
        });
    }

    /// An attach with no preamble (a short adopt or a reconnect) must NOT arm the
    /// suppression latch — a genuine later `InitShell` must still write bootstrap.
    #[test]
    fn apply_attach_without_preamble_does_not_arm_suppression() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(53u64);
            let (_manager, event_loop, model, _wakeups_rx) =
                start_adopted_loop_unbootstrapped(&mut app, conn);

            event_loop.update(&mut app, |me, _| me.apply_attach(b"", 0, b"hello"));

            assert!(
                !model.lock().take_suppress_next_bootstrap_write(),
                "no preamble → the suppression latch must stay disarmed"
            );
        });
    }

    /// A real serialized `InitShell` DCS (`ESC P $ d <hex-json> ST`) — the same
    /// wire form a bootstrapping shell emits — so feeding it drives the real
    /// `TerminalModel::init_shell` (not just plain-text rendering).
    fn init_shell_dcs() -> Vec<u8> {
        let json = r#"{"hook":"InitShell","value":{"session_id":167303092612201,"shell":"zsh"}}"#;
        let mut out = vec![0x1b, 0x50, 0x24, 0x64]; // ESC P $ d
        out.extend_from_slice(hex::encode(json).as_bytes());
        out.push(0x9c); // ST
        out
    }

    /// T1.3 correlation (the crux of the write-suppression design): the latch must
    /// be consumed by a *real* `InitShell` driven from the preamble — not by
    /// unrelated output — so the suppression stamps exactly the preamble's
    /// `InitShell` event and can never leak onto a later genuine one. Plain output
    /// leaves the armed latch untouched; a real `InitShell` DCS consumes it
    /// synchronously while parsing.
    #[test]
    fn suppression_latch_is_consumed_only_by_a_real_initshell() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(59u64);
            let (_manager, event_loop, model, _wakeups_rx) =
                start_adopted_loop_unbootstrapped(&mut app, conn);

            // Plain output emits no InitShell, so the armed latch is left intact.
            event_loop.update(&mut app, |me, _| {
                me.terminal_model.lock().suppress_next_bootstrap_write();
                me.process_pty_bytes(b"just some output\r\n");
            });
            assert!(
                model.lock().take_suppress_next_bootstrap_write(),
                "plain output must not consume the latch — only an InitShell does"
            );

            // A real InitShell DCS drives init_shell, which consumes the armed
            // latch synchronously while parsing (and stamps its own event).
            let dcs = init_shell_dcs();
            event_loop.update(&mut app, |me, _| {
                me.terminal_model.lock().suppress_next_bootstrap_write();
                me.process_pty_bytes(&dcs);
            });
            assert!(
                !model.lock().take_suppress_next_bootstrap_write(),
                "a real InitShell must consume the armed latch (correlation)"
            );
        });
    }
}
