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
/// The daemon is responsible for bootstrapping the shell (Warpify init) when it
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
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut event_loop = Self::new(model, channel_event_listener, connection_session_id);

        // Output path: live PTY bytes arrive as manager pushes. Filter to our
        // own daemon session and feed them through the ANSI processor.
        let manager = RemoteServerManager::handle(ctx);
        ctx.subscribe_to_model(&manager, |me, event, _ctx| match event {
            RemoteServerManagerEvent::SessionOutput {
                pty_session_id,
                bytes,
                ..
            } if me.is_our_session(pty_session_id) => {
                me.process_pty_bytes(bytes);
            }
            RemoteServerManagerEvent::SessionExited {
                pty_session_id,
                exit_code,
                ..
            } if me.is_our_session(pty_session_id) => {
                me.on_session_exited(*exit_code);
            }
            _ => {}
        });

        // Input path: drain the channel with `ctx` access so resizes and
        // keystrokes can be routed to the live client.
        ctx.spawn_stream_local(event_loop_rx, Self::on_event_loop_message, |_, _| ());

        // Open the daemon-hosted session; `pty_session_id` is filled in when it
        // resolves, at which point any buffered input is flushed.
        event_loop.open_session(open_params, size_info, ctx);

        event_loop
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
        }
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

    /// Issues the `OpenSession` request. The initial size is taken from the
    /// terminal model so the daemon-side PTY matches what the user sees.
    fn open_session(
        &mut self,
        open_params: OpenSessionParams,
        size_info: SizeInfo,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(client) = self.client(ctx) else {
            log::error!(
                "Cannot open daemon session: no live client for {:?}",
                self.connection_session_id
            );
            return;
        };
        let OpenSessionParams { cwd, shell, env } = open_params;
        let rows = size_info.rows as u32;
        let cols = size_info.columns as u32;
        let future = async move { client.open_session(cwd, shell, env, rows, cols).await };
        ctx.spawn(future, |me, result, ctx| match result {
            Ok(opened) => me.on_session_opened(opened.session_id, ctx),
            Err(err) => log::error!("Failed to open daemon session: {err:?}"),
        });
    }

    fn on_session_opened(&mut self, pty_session_id: String, ctx: &mut ModelContext<Self>) {
        self.pty_session_id = Some(pty_session_id.clone());
        // Flush any input that arrived before the session was addressable.
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
            log::warn!("Dropping {message:?} for daemon session {pty_session_id}: no live client");
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
        // Stage 3+: surface the exit to the terminal model / close the block.
        // For now, request a repaint so the final output is shown.
        self.channel_event_listener.send_wakeup_event();
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
