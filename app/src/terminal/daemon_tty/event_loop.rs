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
    /// The `OpenSession` request, held until the transport is `Connected`. Taken
    /// (once) by `try_open`. `None` after the session has been opened.
    pending_open: Option<(OpenSessionParams, SizeInfo)>,
    /// Byte offset just past the last `SessionOutput` byte we've rendered. Sent
    /// as `last_seq` on re-attach so the daemon replays only what we missed.
    last_seq: u64,
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
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut event_loop = Self::new(model, channel_event_listener, connection_session_id);
        match adopt_pty_session_id {
            // Adopt an existing daemon session: attach + replay on connect.
            Some(id) => event_loop.pty_session_id = Some(id),
            // Open a fresh session once the transport is connected.
            None => event_loop.pending_open = Some((open_params, size_info)),
        }

        // Output path: live PTY bytes arrive as manager pushes. Filter to our
        // own daemon session and feed them through the ANSI processor. The
        // connect-state arms gate `OpenSession` on the transport being ready.
        let manager = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&manager, |me, event, ctx| match event {
            RemoteServerManagerEvent::SessionOutput {
                pty_session_id,
                seq,
                bytes,
                ..
            } if me.is_our_session(pty_session_id) => {
                me.process_pty_bytes(bytes);
                me.last_seq = *seq + bytes.len() as u64;
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
            pending_open: None,
            last_seq: 0,
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
                if !attached.replay.is_empty() {
                    me.process_pty_bytes(&attached.replay);
                }
                me.last_seq = attached.base_seq + attached.replay.len() as u64;
                // Transport is back and we're re-attached — flush input buffered
                // during the outage so keystrokes/resizes aren't lost (§9).
                me.flush_pending_input(ctx);
            }
            Err(err) => log::error!("Failed to re-attach daemon session: {err:?}"),
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
        } = open_params;
        let rows = size_info.rows as u32;
        let cols = size_info.columns as u32;
        log::info!("daemon_tty: issuing OpenSession (cwd={cwd:?}, shell={shell:?}, {rows}x{cols}, ring_ceiling={ring_ceiling_bytes:?})");
        let future =
            async move { client.open_session(cwd, shell, env, rows, cols, ring_ceiling_bytes).await };
        ctx.spawn(future, |me, result, ctx| match result {
            Ok(opened) => me.on_session_opened(opened.session_id, ctx),
            Err(err) => log::error!("daemon_tty: OpenSession failed: {err:?}"),
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
        self.pty_session_id = Some(pty_session_id);
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
            None => self.pending_input.push(message),
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
            self.pending_input.push(message);
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
        let notice = match exit_code {
            Some(code) => format!("session ended (exit code {code})"),
            None => "session ended".to_string(),
        };
        self.write_notice(&notice);
    }

    /// Writes a Zaplex notice line (e.g. a connection error or session-ended
    /// message) into the terminal via the normal ANSI path, so the user sees it
    /// in the tab rather than a blank/hung view. Rendered in bold red.
    fn write_notice(&mut self, text: &str) {
        let line = format!("\r\n\x1b[1;31m[zaplex] {text}\x1b[0m\r\n");
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
        let manager = app.add_singleton_model(RemoteServerManager::new);
        let (listener, wakeups_rx) = test_listener();
        let model = Arc::new(FairMutex::new(TerminalModel::mock(None, Some(listener.clone()))));
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
}
