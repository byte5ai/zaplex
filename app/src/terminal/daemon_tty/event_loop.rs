use crate::remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use crate::terminal::{
    cli_agent::CLIAgent,
    cli_agent_sessions::{CLIAgentSessionsModel, CLIAgentSessionsModelEvent},
    event_listener::ChannelEventListener,
    model::ansi::Processor,
    writeable_pty::Message as EventLoopMessage,
    SizeInfo, TerminalModel,
};
use async_channel::Receiver;
use parking_lot::FairMutex;
use remote_server::{
    client::{ClientError, RemoteServerClient},
    proto::{AgentPtyBindingStatus, AgentSessionIdentity, SessionAttached},
};
use std::io;
use std::sync::Arc;
use warp_core::SessionId;
use warpui::{Entity, EntityId, ModelContext, SingletonEntity};
use zaplex_remote_session::types::{FEATURE_AGENT_PTY_BINDING, FEATURE_STARTUP_COMMAND_ACK};

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
    /// Exact daemon generation paired with `pty_session_id`. Present for
    /// capability-aware sessions and required for inventory-driven adopts.
    pty_generation: Option<u64>,
    /// Optional foreground identity captured from the inventory row that
    /// initiated this adopt. Cleared after the first validated attach.
    expected_attach_agent_binding: Option<AgentSessionIdentity>,
    /// Adopt/reconnect output stays buffered until `SessionAttached` supplies
    /// the capability-checked authoritative binding snapshot.
    awaiting_attach_snapshot: bool,
    /// Attach/replay request token. A reconnect invalidates an older callback.
    attach_in_flight: Option<u64>,
    next_attach_attempt: u64,
    /// Terminal view whose CLI-agent lifecycle is mirrored to the daemon.
    terminal_view_id: Option<EntityId>,
    /// Binding desired from the latest CLI-agent/account model state.
    desired_agent_binding: Option<AgentSessionIdentity>,
    /// Whether `desired_agent_binding` came from an observed local lifecycle
    /// event. Only such a request may survive an authoritative attach snapshot
    /// and become an explicit handoff.
    desired_agent_binding_from_lifecycle: bool,
    /// Binding most recently acknowledged by the daemon as foreground.
    agent_binding: Option<AgentSessionIdentity>,
    /// Monotonic attempt currently awaiting a daemon response. A reconnect
    /// invalidates the attempt so a callback from the dead transport cannot
    /// overwrite a retry on the new transport.
    agent_binding_in_flight: Option<u64>,
    next_agent_binding_attempt: u64,
    /// Input/resize messages received before the session id is known. Flushed,
    /// in order, once `OpenSession` resolves.
    pending_input: Vec<EventLoopMessage>,
    /// Output `(pty_session_id, seq, bytes)` pushed for our connection before the
    /// `OpenSession` response arrives (the daemon auto-attaches and starts the
    /// shell immediately). Rendered, in order, in `on_session_opened`, so the
    /// initial shell/bootstrap output isn't lost on a fresh tab.
    pending_output: Vec<(String, u64, Vec<u8>)>,
    /// The bounded output buffer dropped at least one byte; another daemon
    /// replay is required before live delivery may reopen.
    pending_output_overflowed: bool,
    /// Exit observed while an attach snapshot was in flight. It is applied
    /// after the matching replay callback, never before it.
    pending_exit: Option<Option<i32>>,
    /// The `OpenSession` request, held until the transport is `Connected`. Taken
    /// (once) by `try_open`. `None` after the session has been opened.
    pending_open: Option<(OpenSessionParams, SizeInfo)>,
    /// The host's startup command, captured from `OpenSessionParams` and run once
    /// (taken) only after the terminal model confirms that shell bootstrap has
    /// completed — the daemon-path analog of the local-PTY SSH startup-command
    /// injector. `None` for adopted sessions.
    startup_command: Option<String>,
    /// Stable logical delivery id for `startup_command`. It survives transport
    /// retries and reconnects so the daemon can acknowledge a lost-Ack retry
    /// without executing the command again.
    startup_command_id: Option<String>,
    /// Monotonic local attempt token currently awaiting a daemon Ack. An attempt
    /// token prevents a late callback from an old transport from clearing the
    /// state of a newer reconnect attempt.
    startup_command_in_flight: Option<u64>,
    next_startup_command_attempt: u64,
    /// A negative or malformed Ack indicates that retrying on the same live
    /// transport would only create a tight loop. Reconnect clears this latch.
    startup_retry_requires_reconnect: bool,
    /// Avoids repeating the same actionable compatibility notice on every
    /// output chunk from an older daemon.
    startup_capability_notice_shown: bool,
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
        adopt_pty_generation: Option<u64>,
        expected_attach_agent_binding: Option<AgentSessionIdentity>,
        install_progress_rx: Option<Receiver<String>>,
        host_label: String,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut event_loop = Self::new(model, channel_event_listener, connection_session_id);
        // Every session this loop drives is daemon-hosted; for bash/zsh the
        // daemon delivers the root shell's complete bootstrap (bash/zsh through
        // ordered input; fish/PowerShell through guarded body files) server-side.
        // Mark the model before any byte is
        // parsed so the root-shell `InitShell` it emits — the fresh open's live
        // handshake as much as an adopt preamble — is stamped
        // `suppress_bootstrap_write`. Without this, the client re-types the
        // ~90 KB body into the already-bootstrapped shell, where it executes a
        // second time visibly as command blocks (echo restored by the
        // server-side pass's `stty sane`): the connect-time script dump on
        // every fresh connect (RC 2026-07-21). `init_shell` scopes the stamp to
        // what the daemon actually delivers — root shells only; subshells and
        // legacy-SSH sessions keep their client-side write.
        event_loop
            .terminal_model
            .lock()
            .mark_bootstrap_delivered_server_side();
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
        match (adopt_pty_session_id, adopt_pty_generation) {
            // Adopt an existing daemon session: attach + replay on connect.
            (Some(id), generation) if !id.is_empty() => {
                event_loop.pty_session_id = Some(id);
                // A legacy daemon predates PTY generations and reports zero.
                // Preserve its id-only attach path; capability-aware inventory
                // requires and supplies a nonzero generation.
                event_loop.pty_generation = generation.filter(|generation| *generation != 0);
                event_loop.expected_attach_agent_binding = expected_attach_agent_binding;
                event_loop.awaiting_attach_snapshot = true;
            }
            (Some(_), _) | (None, Some(_)) => {
                event_loop
                    .write_notice("could not re-attach session: a non-empty PTY id is required");
                event_loop.terminated = true;
            }
            // Open a fresh session once the transport is connected. Only a
            // fresh open witnesses the real bootstrap handshake from seq 0, so
            // only it reports the boundary the daemon freezes (T1.3).
            (None, None) => {
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
                if me.is_our_session(pty_session_id) && !me.awaiting_attach_snapshot {
                    me.process_pty_bytes(bytes);
                    me.last_seq = *seq + bytes.len() as u64;
                    me.maybe_report_bootstrap_boundary(ctx);
                    me.maybe_dispatch_startup_command(ctx);
                } else if (me.is_our_session(pty_session_id) && me.awaiting_attach_snapshot)
                    || (me.pty_session_id.is_none() && *session_id == me.connection_session_id)
                {
                    // Output for our connection before `OpenSession` resolved — the
                    // daemon auto-attaches and starts the shell/bootstrap before the
                    // response reaches us. Buffer it (drained in `on_session_opened`)
                    // so the initial output isn't lost; stop past the cap so a hung
                    // open can't grow this without bound.
                    me.buffer_pending_output(pty_session_id, *seq, bytes);
                }
            }
            RemoteServerManagerEvent::SessionExited {
                pty_session_id,
                exit_code,
                ..
            } if me.is_our_session(pty_session_id) => {
                if me.awaiting_attach_snapshot {
                    me.pending_exit = Some(*exit_code);
                } else {
                    me.on_session_exited(*exit_code);
                }
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
                // The old transport cannot complete its request anymore. Keep
                // the logical command and id, but let the reconnected client
                // issue a new correlated attempt after attach.
                me.begin_transport_reconnect();
                me.reattach(ctx);
            }
            RemoteServerManagerEvent::SessionConnectionFailed {
                session_id,
                phase,
                error,
            } if *session_id == me.connection_session_id => {
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
            pty_generation: None,
            expected_attach_agent_binding: None,
            awaiting_attach_snapshot: false,
            attach_in_flight: None,
            next_attach_attempt: 0,
            terminal_view_id: None,
            desired_agent_binding: None,
            desired_agent_binding_from_lifecycle: false,
            agent_binding: None,
            agent_binding_in_flight: None,
            next_agent_binding_attempt: 0,
            pending_input: Vec::new(),
            pending_output: Vec::new(),
            pending_output_overflowed: false,
            pending_exit: None,
            pending_open: None,
            startup_command: None,
            startup_command_id: None,
            startup_command_in_flight: None,
            next_startup_command_attempt: 0,
            startup_retry_requires_reconnect: false,
            startup_capability_notice_shown: false,
            last_seq: 0,
            host_label: String::new(),
            welcomed: false,
            terminated: false,
            report_bootstrap_boundary: false,
        }
    }

    /// Starts mirroring this terminal's CLI-agent lifecycle to the daemon PTY.
    pub(super) fn bind_terminal_view(
        &mut self,
        terminal_view_id: EntityId,
        ctx: &mut ModelContext<Self>,
    ) {
        self.terminal_view_id = Some(terminal_view_id);
        let sessions = CLIAgentSessionsModel::handle(ctx);
        ctx.subscribe_to_model(&sessions, |me, event, ctx| {
            if me.terminal_view_id != Some(event.terminal_view_id()) {
                return;
            }
            match event {
                CLIAgentSessionsModelEvent::Started { .. }
                | CLIAgentSessionsModelEvent::StatusChanged { .. }
                | CLIAgentSessionsModelEvent::SessionUpdated { .. } => {
                    me.refresh_desired_agent_binding(ctx);
                }
                CLIAgentSessionsModelEvent::Ended { .. } => {
                    me.desired_agent_binding_from_lifecycle = true;
                    me.desired_agent_binding = None;
                    if !me.awaiting_attach_snapshot {
                        me.drive_agent_binding(ctx);
                    }
                }
                CLIAgentSessionsModelEvent::InputSessionChanged { .. } => {}
            }
        });
    }

    fn apply_authoritative_agent_binding_state(
        &mut self,
        agent_binding: Option<AgentSessionIdentity>,
    ) {
        self.agent_binding = agent_binding.clone();
        if !self.desired_agent_binding_from_lifecycle {
            self.desired_agent_binding = agent_binding;
        }
    }

    fn apply_authoritative_agent_binding(
        &mut self,
        agent_binding: Option<AgentSessionIdentity>,
        ctx: &mut ModelContext<Self>,
    ) {
        if !self.desired_agent_binding_from_lifecycle {
            if let Some(terminal_view_id) = self.terminal_view_id {
                CLIAgentSessionsModel::handle(ctx).update(ctx, |sessions, _ctx| {
                    if let Some(identity) = agent_binding.as_ref() {
                        let agent = match identity.provider.as_str() {
                            "claude" => Some(CLIAgent::Claude),
                            "codex" => Some(CLIAgent::Codex),
                            _ => None,
                        };
                        if let Some(agent) = agent {
                            sessions.bind_account_identity(
                                terminal_view_id,
                                agent,
                                (!identity.config_dir.is_empty())
                                    .then(|| identity.config_dir.clone()),
                                (!identity.account_email.is_empty())
                                    .then(|| identity.account_email.clone()),
                            );
                        } else {
                            sessions.unbind_account_identity(terminal_view_id);
                        }
                    } else {
                        sessions.unbind_account_identity(terminal_view_id);
                    }
                });
            }
        }
        self.apply_authoritative_agent_binding_state(agent_binding);
    }

    fn refresh_desired_agent_binding(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(terminal_view_id) = self.terminal_view_id else {
            return;
        };
        let sessions = CLIAgentSessionsModel::handle(ctx);
        self.desired_agent_binding_from_lifecycle = true;
        self.desired_agent_binding = sessions.read(ctx, |sessions, _ctx| {
            let session = sessions.session(terminal_view_id)?;
            let provider = match session.agent {
                CLIAgent::Claude => "claude",
                CLIAgent::Codex => "codex",
                CLIAgent::Gemini
                | CLIAgent::Amp
                | CLIAgent::Droid
                | CLIAgent::OpenCode
                | CLIAgent::Copilot
                | CLIAgent::Pi
                | CLIAgent::Auggie
                | CLIAgent::CursorCli
                | CLIAgent::Goose
                | CLIAgent::DeepSeek
                | CLIAgent::Antigravity
                | CLIAgent::Unknown => return None,
            };
            let account = sessions.account_identity(terminal_view_id)?;
            if account.agent() != session.agent {
                return None;
            }
            Some(AgentSessionIdentity {
                session_id: session.session_context.session_id.clone()?,
                provider: provider.to_string(),
                account_email: account.account_email.clone().unwrap_or_default(),
                config_dir: account.config_dir.clone().unwrap_or_default(),
            })
        });
        if !self.awaiting_attach_snapshot {
            self.drive_agent_binding(ctx);
        }
    }

    fn agent_binding_client(
        &self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<(Arc<RemoteServerClient>, bool)> {
        let session_id = self.connection_session_id;
        let manager = RemoteServerManager::handle(ctx);
        manager.read(ctx, |manager, _ctx| {
            manager
                .client_for_session(session_id)
                .cloned()
                .map(|client| {
                    let supported =
                        manager.session_supports_feature(session_id, FEATURE_AGENT_PTY_BINDING);
                    (client, supported)
                })
        })
    }

    /// Serializes bind/unbind requests so rapid lifecycle changes cannot race a
    /// stale callback into becoming foreground.
    fn drive_agent_binding(&mut self, ctx: &mut ModelContext<Self>) {
        self.settle_agent_binding_if_converged();
        if self.agent_binding_in_flight.is_some() {
            return;
        }
        let (Some(pty_session_id), Some(pty_generation)) =
            (self.pty_session_id.clone(), self.pty_generation)
        else {
            return;
        };
        let Some((client, supported)) = self.agent_binding_client(ctx) else {
            return;
        };
        if !supported {
            return;
        }

        match (
            self.agent_binding.clone(),
            self.desired_agent_binding.clone(),
        ) {
            (None, None) => {}
            (Some(current), Some(desired)) if current == desired => {}
            (Some(current), None) => {
                let attempt = self.start_agent_binding_attempt();
                let sent = current.clone();
                let future = async move {
                    client
                        .unbind_agent_pty(current, pty_session_id, pty_generation)
                        .await
                };
                ctx.spawn(future, move |me, result, ctx| {
                    if !me.finish_agent_binding_attempt(attempt) {
                        return;
                    }
                    let retry_immediately = result
                        .as_ref()
                        .err()
                        .is_some_and(Self::agent_binding_error_retries_immediately);
                    let accepted =
                        result.as_ref().ok().and_then(|response| {
                            AgentPtyBindingStatus::try_from(response.status).ok()
                        }) == Some(AgentPtyBindingStatus::Unbound);
                    if accepted && me.agent_binding.as_ref() == Some(&sent) {
                        me.agent_binding = None;
                        me.settle_agent_binding_if_converged();
                    } else if !accepted {
                        log::warn!("daemon_tty: agent PTY unbind failed: {result:?}");
                    }
                    if accepted || retry_immediately || me.desired_agent_binding.is_some() {
                        me.drive_agent_binding(ctx);
                    }
                });
            }
            (current, Some(desired)) => {
                let attempt = self.start_agent_binding_attempt();
                let sent = desired.clone();
                let future = async move {
                    client
                        .bind_agent_pty(desired, pty_session_id, pty_generation, current)
                        .await
                };
                ctx.spawn(future, move |me, result, ctx| {
                    if !me.finish_agent_binding_attempt(attempt) {
                        return;
                    }
                    let retry_immediately = result
                        .as_ref()
                        .err()
                        .is_some_and(Self::agent_binding_error_retries_immediately);
                    let accepted =
                        result.as_ref().ok().and_then(|response| {
                            AgentPtyBindingStatus::try_from(response.status).ok()
                        }) == Some(AgentPtyBindingStatus::Bound);
                    if accepted {
                        me.agent_binding = Some(sent.clone());
                        me.settle_agent_binding_if_converged();
                    } else {
                        log::warn!("daemon_tty: agent PTY bind failed: {result:?}");
                    }
                    if accepted
                        || retry_immediately
                        || me.desired_agent_binding.as_ref() != Some(&sent)
                    {
                        me.drive_agent_binding(ctx);
                    }
                });
            }
        }
    }

    fn settle_agent_binding_if_converged(&mut self) {
        if self.agent_binding == self.desired_agent_binding {
            self.desired_agent_binding_from_lifecycle = false;
        }
    }

    fn start_agent_binding_attempt(&mut self) -> u64 {
        self.next_agent_binding_attempt = self.next_agent_binding_attempt.wrapping_add(1);
        let attempt = self.next_agent_binding_attempt;
        self.agent_binding_in_flight = Some(attempt);
        attempt
    }

    fn finish_agent_binding_attempt(&mut self, attempt: u64) -> bool {
        if self.agent_binding_in_flight != Some(attempt) {
            return false;
        }
        self.agent_binding_in_flight = None;
        true
    }

    fn allow_agent_binding_retry(&mut self) {
        self.agent_binding_in_flight = None;
    }

    fn agent_binding_error_retries_immediately(error: &ClientError) -> bool {
        matches!(error, ClientError::Timeout(_))
    }

    fn attach_generation_is_valid(
        expected_generation: Option<u64>,
        supports_agent_binding: bool,
    ) -> bool {
        expected_generation.is_some() || !supports_agent_binding
    }

    fn attach_error_waits_for_reconnect(error: &ClientError) -> bool {
        matches!(
            error,
            ClientError::Disconnected | ClientError::ResponseChannelClosed
        )
    }

    fn start_attach_attempt(&mut self) -> u64 {
        self.next_attach_attempt = self.next_attach_attempt.wrapping_add(1);
        let attempt = self.next_attach_attempt;
        self.attach_in_flight = Some(attempt);
        attempt
    }

    fn finish_attach_attempt(&mut self, attempt: u64) -> bool {
        if self.attach_in_flight != Some(attempt) {
            return false;
        }
        self.attach_in_flight = None;
        true
    }

    fn allow_attach_retry(&mut self) {
        self.attach_in_flight = None;
    }

    /// On transport reconnect: re-attach to the still-running daemon session and
    /// replay everything produced while we were gone, reconstructing the grid.
    /// Falls back to opening the session if it was never opened (reconnect raced
    /// the initial open).
    fn reattach(&mut self, ctx: &mut ModelContext<Self>) {
        if self.attach_in_flight.is_some() {
            return;
        }
        let Some(pty_session_id) = self.pty_session_id.clone() else {
            self.try_open(ctx);
            return;
        };
        let Some(client) = self.client(ctx) else {
            return; // The reconnected client isn't registered yet.
        };
        let last_seq = self.last_seq;
        let expected_generation = self.pty_generation;
        let supports_agent_binding = self
            .agent_binding_client(ctx)
            .is_some_and(|(_, supported)| supported);
        if !Self::attach_generation_is_valid(expected_generation, supports_agent_binding) {
            self.write_notice(
                "could not re-attach session: the daemon returned an invalid PTY generation",
            );
            self.terminated = true;
            return;
        }
        if self.expected_attach_agent_binding.is_some() && !supports_agent_binding {
            self.write_notice(
                "could not re-attach agent: the host does not support validated agent routing",
            );
            self.terminated = true;
            return;
        }
        log::info!("daemon_tty: re-attaching pty_session_id={pty_session_id} from seq {last_seq}");
        let expected_agent_binding = self.expected_attach_agent_binding.clone();
        let attempt = self.start_attach_attempt();
        let future = async move {
            match expected_generation {
                Some(generation) => {
                    client
                        .attach_session_generation_and_agent(
                            pty_session_id,
                            last_seq,
                            Some(generation),
                            expected_agent_binding,
                        )
                        .await
                }
                None => client.attach_session(pty_session_id, last_seq).await,
            }
        };
        ctx.spawn(future, move |me, result, ctx| match result {
            Ok(attached) => {
                if !me.finish_attach_attempt(attempt) || me.terminated {
                    return;
                }
                me.on_session_attached(attached, supports_agent_binding, ctx);
            }
            Err(err) => {
                if !me.finish_attach_attempt(attempt) {
                    return;
                }
                if Self::attach_error_waits_for_reconnect(&err) {
                    // A transport drop clears the old client's pending request
                    // before the manager finishes reconnecting. Keep this loop
                    // provisional; SessionReconnected starts a fresh attach.
                    log::warn!("Session attach interrupted; waiting to retry: {err:?}");
                } else {
                    // A live connection will not emit SessionReconnected for a
                    // timeout, malformed response, or authoritative rejection.
                    // Fail visibly and release the provisional dedupe route.
                    log::error!("Session attach failed: {err:?}");
                    me.write_notice(&format!("could not re-attach session: {err}"));
                    me.abandon_failed_attach(ctx);
                    if let Some(exit_code) = me.pending_exit.take() {
                        me.awaiting_attach_snapshot = false;
                        me.on_session_exited(exit_code);
                    }
                }
            }
        });
    }

    fn on_session_attached(
        &mut self,
        attached: SessionAttached,
        supports_agent_binding: bool,
        ctx: &mut ModelContext<Self>,
    ) {
        if self
            .pty_generation
            .is_some_and(|expected| attached.generation != expected)
        {
            log::error!(
                "daemon_tty: rejected attach response with generation {} (expected {:?})",
                attached.generation,
                self.pty_generation
            );
            self.write_notice("could not re-attach session: daemon generation mismatch");
            self.terminated = true;
            self.abandon_failed_attach(ctx);
            return;
        }
        let pending_exit = self.pending_exit.take();
        if pending_exit.is_none() && supports_agent_binding {
            self.apply_authoritative_agent_binding(attached.agent_binding.clone(), ctx);
        } else {
            self.apply_authoritative_agent_binding(None, ctx);
        }
        self.expected_attach_agent_binding = None;
        self.apply_attach(
            &attached.bootstrap_preamble,
            attached.base_seq,
            &attached.replay,
        );
        let replay_again = self.drain_pending_output();
        if let Some(exit_code) = pending_exit {
            self.awaiting_attach_snapshot = false;
            if replay_again {
                self.write_warning(
                    "some final session output was truncated before the exit notification",
                );
            }
            self.on_session_exited(exit_code);
            return;
        }
        if replay_again {
            self.awaiting_attach_snapshot = true;
            self.reattach(ctx);
            return;
        }
        self.awaiting_attach_snapshot = false;
        // If bootstrap only completed now — a fresh open that dropped
        // mid-handshake and finished it from this reconnect's replay — the
        // live-output path never saw the flip, so report the boundary here
        // too (a no-op for adopted sessions and once already reported).
        self.maybe_report_bootstrap_boundary(ctx);
        self.maybe_dispatch_startup_command(ctx);
        // Stage the payoff moment: an adopt's first attach welcomes the
        // user into their running session; a reconnect after a drop
        // states plainly that nothing was lost.
        if self.welcomed {
            self.write_zaplexify(&format!(
                "Reconnected to {} — session restored, nothing lost.",
                self.host_label
            ));
        } else {
            self.write_zaplexify(&format!(
                "Re-attached to your running session on {} — right where you left off.",
                self.host_label
            ));
            self.welcomed = true;
        }
        // Transport is back and we're re-attached — flush input buffered
        // during the outage so keystrokes/resizes aren't lost (§9).
        self.flush_pending_input(ctx);
        self.drive_agent_binding(ctx);
    }

    fn abandon_failed_attach(&mut self, ctx: &mut ModelContext<Self>) {
        let session_id = self.connection_session_id;
        RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.deregister_session(session_id, false, ctx);
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

    /// Resolves both the live client and the negotiated retry-safe startup
    /// capability from the same manager state snapshot.
    fn startup_client(
        &self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<(Arc<RemoteServerClient>, bool)> {
        let session_id = self.connection_session_id;
        let manager = RemoteServerManager::handle(ctx);
        manager.read(ctx, |manager, _ctx| {
            manager
                .client_for_session(session_id)
                .cloned()
                .map(|client| {
                    let supported =
                        manager.session_supports_feature(session_id, FEATURE_STARTUP_COMMAND_ACK);
                    (client, supported)
                })
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
        // Run once after this shell reaches the real bootstrap boundary (see
        // `maybe_dispatch_startup_command`).
        self.startup_command = startup_command.filter(|command| !command.is_empty());
        self.startup_command_id = self
            .startup_command
            .as_ref()
            .map(|_| uuid::Uuid::new_v4().to_string());
        let rows = size_info.rows as u32;
        let cols = size_info.columns as u32;
        log::info!("daemon_tty: issuing OpenSession (cwd={cwd:?}, shell={shell:?}, {rows}x{cols}, ring_ceiling={ring_ceiling_bytes:?})");
        let future =
            async move { client.open_session(cwd, shell, env, rows, cols, ring_ceiling_bytes).await };
        ctx.spawn(future, |me, result, ctx| match result {
            Ok(opened) => me.on_session_opened(opened.session_id, opened.generation, ctx),
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

    fn on_session_opened(
        &mut self,
        pty_session_id: String,
        generation: u64,
        ctx: &mut ModelContext<Self>,
    ) {
        log::info!("daemon_tty: session opened, pty_session_id={pty_session_id}");
        self.pty_session_id = Some(pty_session_id.clone());
        self.pty_generation = (generation != 0).then_some(generation);
        if self.drain_pending_output() {
            self.awaiting_attach_snapshot = true;
            self.reattach(ctx);
            return;
        }
        self.awaiting_attach_snapshot = false;
        self.drive_agent_binding(ctx);
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
        // The bootstrap handshake may already be complete in that pre-open burst
        // (the daemon auto-attaches and starts the shell before this response
        // lands), so report the boundary now if so (T1.3).
        self.maybe_report_bootstrap_boundary(ctx);
        // The pending burst can already contain the full shell handshake. If it
        // does, start now; otherwise live output will retry at the exact boundary.
        self.maybe_dispatch_startup_command(ctx);
        // Flush any input that arrived before the session was addressable.
        self.flush_pending_input(ctx);
    }

    fn buffer_pending_output(&mut self, pty_session_id: &str, seq: u64, bytes: &[u8]) {
        let buffered: usize = self
            .pending_output
            .iter()
            .map(|(_, _, bytes)| bytes.len())
            .sum();
        if buffered.saturating_add(bytes.len()) <= MAX_PENDING_OUTPUT_BYTES {
            self.pending_output
                .push((pty_session_id.to_string(), seq, bytes.to_vec()));
        } else {
            self.pending_output_overflowed = true;
        }
    }

    /// Drains only a contiguous sequence. `true` means overflow or a gap was
    /// observed and the caller must request another daemon replay before live
    /// output resumes.
    fn drain_pending_output(&mut self) -> bool {
        let Some(pty_session_id) = self.pty_session_id.clone() else {
            return false;
        };
        let mut replay_required = std::mem::take(&mut self.pending_output_overflowed);
        let mut pending = std::mem::take(&mut self.pending_output);
        pending.sort_by_key(|(_, seq, _)| *seq);
        for (pty, seq, bytes) in pending {
            if pty != pty_session_id {
                continue;
            }
            let end_seq = seq + bytes.len() as u64;
            if end_seq <= self.last_seq {
                continue;
            }
            if seq > self.last_seq {
                replay_required = true;
                break;
            }
            let offset = self.last_seq.saturating_sub(seq).min(bytes.len() as u64) as usize;
            self.process_pty_bytes(&bytes[offset..]);
            self.last_seq = end_seq;
        }
        replay_required
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
        if self.awaiting_attach_snapshot {
            self.buffer_pending(message);
            return;
        }
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
        if self.awaiting_attach_snapshot {
            self.buffer_pending(message);
            return;
        }
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

    /// Dispatches the host's startup command exactly once, but only after the
    /// shell has completed its real bootstrap boundary. `SessionOpened` merely
    /// means the PTY is addressable, and `InitShell` is still too early because
    /// the daemon-delivered body has not yet emitted `Bootstrapped`.
    fn maybe_dispatch_startup_command(&mut self, ctx: &mut ModelContext<Self>) {
        if self.startup_command.is_none()
            || self.startup_command_in_flight.is_some()
            || self.startup_retry_requires_reconnect
            || !self.is_bootstrapped()
        {
            return;
        }
        let Some((client, supports_retry_safe_startup)) = self.startup_client(ctx) else {
            return; // Transport down; reconnect will retry with the same id.
        };
        if !supports_retry_safe_startup {
            if !self.startup_capability_notice_shown {
                self.write_notice(
                    "this host needs a newer Zaplex helper before it can start the requested \
                     command safely. Update the host helper and reconnect.",
                );
                self.startup_capability_notice_shown = true;
            }
            return;
        }
        self.startup_capability_notice_shown = false;

        let Some((pty_session_id, command_id, bytes, attempt)) =
            self.prepare_startup_command_delivery()
        else {
            return;
        };
        let future = async move {
            client
                .send_startup_command(pty_session_id, command_id, bytes)
                .await
        };
        ctx.spawn(future, move |me, result, ctx| {
            if me.startup_command_in_flight != Some(attempt) {
                return; // A newer reconnect attempt owns the state now.
            }
            match result {
                Ok(ack)
                    if ack.accepted
                        && me.pty_session_id.as_deref() == Some(ack.session_id.as_str())
                        && me.startup_command_id.as_deref()
                            == Some(ack.startup_command_id.as_str()) =>
                {
                    me.acknowledge_startup_command(&ack.startup_command_id);
                }
                Ok(ack)
                    if me.pty_session_id.as_deref() == Some(ack.session_id.as_str())
                        && me.startup_command_id.as_deref()
                            == Some(ack.startup_command_id.as_str()) =>
                {
                    me.startup_command_in_flight = None;
                    me.startup_retry_requires_reconnect = true;
                    log::warn!(
                        "daemon_tty: daemon rejected startup command {} for session {}",
                        ack.startup_command_id,
                        ack.session_id
                    );
                    me.write_notice(
                        "the requested startup command was not accepted and remains pending. \
                         Reconnect after the session helper recovers to retry it safely.",
                    );
                }
                Ok(ack) => {
                    me.startup_command_in_flight = None;
                    me.startup_retry_requires_reconnect = true;
                    log::error!(
                        "daemon_tty: mismatched startup Ack: session={}, command_id={}",
                        ack.session_id,
                        ack.startup_command_id
                    );
                    me.write_notice(
                        "the host returned an invalid startup confirmation; the requested \
                         command remains pending.",
                    );
                }
                Err(err) => {
                    me.startup_command_in_flight = None;
                    log::warn!(
                        "daemon_tty: startup command delivery attempt failed; retaining it for \
                         retry: {err:?}"
                    );
                    // A timeout is exactly the lost-Ack case: retry immediately
                    // with the same logical id so the daemon can return its
                    // cached positive Ack. Disconnect errors wait for the
                    // manager's SessionReconnected event to avoid spinning on a
                    // dead client.
                    if matches!(err, ClientError::Timeout(_)) {
                        me.maybe_dispatch_startup_command(ctx);
                    }
                }
            }
        });
    }

    /// Claims one local delivery attempt while preserving the logical command
    /// and id until a matching positive daemon Ack arrives.
    fn prepare_startup_command_delivery(&mut self) -> Option<(String, String, Vec<u8>, u64)> {
        if self.startup_command_in_flight.is_some()
            || self.startup_retry_requires_reconnect
            || !self.is_bootstrapped()
        {
            return None;
        }
        let pty_session_id = self.pty_session_id.clone()?;
        let command = self.startup_command.as_ref()?;
        let command_id = self
            .startup_command_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        let mut bytes = command.as_bytes().to_vec();
        bytes.push(b'\n');
        self.next_startup_command_attempt = self
            .next_startup_command_attempt
            .checked_add(1)
            .unwrap_or(1);
        let attempt = self.next_startup_command_attempt;
        self.startup_command_in_flight = Some(attempt);
        Some((pty_session_id, command_id, bytes, attempt))
    }

    /// Testable synchronous transport seam. A local enqueue error releases only
    /// the attempt latch; the command and stable id stay pending.
    fn try_dispatch_startup_command_with<E>(
        &mut self,
        dispatch: impl FnOnce(&str, &str, &[u8]) -> Result<(), E>,
    ) {
        let Some((pty_session_id, command_id, bytes, attempt)) =
            self.prepare_startup_command_delivery()
        else {
            return;
        };
        if dispatch(&pty_session_id, &command_id, &bytes).is_err()
            && self.startup_command_in_flight == Some(attempt)
        {
            self.startup_command_in_flight = None;
        }
    }

    /// Completes the logical startup delivery only for its exact stable id.
    fn acknowledge_startup_command(&mut self, command_id: &str) {
        if self.startup_command_id.as_deref() != Some(command_id) {
            return;
        }
        self.startup_command = None;
        self.startup_command_id = None;
        self.startup_command_in_flight = None;
        self.startup_retry_requires_reconnect = false;
    }

    /// Releases an attempt tied to a dead transport without changing the
    /// logical command id. The next connected transport can safely retry it.
    fn allow_startup_command_retry(&mut self) {
        if self.startup_command.is_some() {
            self.startup_command_in_flight = None;
            self.startup_retry_requires_reconnect = false;
        }
    }

    /// Invalidates operations owned by the dead transport before re-attaching
    /// them through the replacement connection.
    fn begin_transport_reconnect(&mut self) {
        self.allow_startup_command_retry();
        self.allow_agent_binding_retry();
        self.allow_attach_retry();
        self.awaiting_attach_snapshot = true;
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
        start_adopted_loop_impl(app, conn, true, Some(7))
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
        start_adopted_loop_impl(app, conn, false, Some(7))
    }

    fn start_adopted_loop_impl(
        app: &mut App,
        conn: SessionId,
        bootstrapped: bool,
        generation: Option<u64>,
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
                generation,
                None,
                None,
                "test-host".to_string(),
                ctx,
            )
        });
        (manager, event_loop, model, wakeups_rx)
    }

    fn complete_adopted_attach(event_loop: &ModelHandle<EventLoop>, app: &mut App) {
        event_loop.update(app, |me, ctx| {
            me.on_session_attached(
                SessionAttached {
                    session_id: OUR_PTY.to_string(),
                    size: None,
                    base_seq: 0,
                    replay: Vec::new(),
                    bootstrap_preamble: Vec::new(),
                    generation: 7,
                    agent_binding: None,
                },
                true,
                ctx,
            );
        });
    }

    #[test]
    fn legacy_generation_zero_adopts_by_id_only() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(6u64);
            let (_manager, event_loop, _model, _wakeups_rx) =
                start_adopted_loop_impl(&mut app, conn, true, Some(0));

            event_loop.read(&app, |me, _| {
                assert_eq!(me.pty_session_id.as_deref(), Some(OUR_PTY));
                assert!(
                    me.pty_generation.is_none(),
                    "generation zero from a legacy daemon must select id-only attach"
                );
                assert!(!me.terminated);
            });
        });
    }

    #[test]
    fn capability_aware_generation_zero_fails_closed() {
        assert!(
            !EventLoop::attach_generation_is_valid(None, true),
            "a capable daemon must never downgrade a malformed zero generation to id-only attach"
        );
        assert!(
            EventLoop::attach_generation_is_valid(None, false),
            "only a legacy daemon retains id-only attach compatibility"
        );
        assert!(EventLoop::attach_generation_is_valid(Some(7), true));
    }

    #[test]
    fn only_disconnected_attach_waits_for_manager_reconnect() {
        assert!(EventLoop::attach_error_waits_for_reconnect(
            &ClientError::Disconnected
        ));
        assert!(EventLoop::attach_error_waits_for_reconnect(
            &ClientError::ResponseChannelClosed
        ));
        assert!(!EventLoop::attach_error_waits_for_reconnect(
            &ClientError::Timeout(std::time::Duration::from_secs(1))
        ));
        assert!(!EventLoop::attach_error_waits_for_reconnect(
            &ClientError::ServerError {
                code: remote_server::proto::ErrorCode::InvalidRequest,
                message: "foreground agent changed".to_string(),
            }
        ));
    }

    #[test]
    fn agent_binding_timeout_is_immediately_retryable() {
        assert!(EventLoop::agent_binding_error_retries_immediately(
            &ClientError::Timeout(std::time::Duration::from_secs(1))
        ));
        assert!(!EventLoop::agent_binding_error_retries_immediately(
            &ClientError::Disconnected
        ));
        assert!(!EventLoop::agent_binding_error_retries_immediately(
            &ClientError::UnexpectedResponse
        ));
    }

    #[test]
    fn adopt_output_waits_for_authoritative_attach_snapshot() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(26u64);
            let (manager, event_loop, _model, wakeups_rx) = start_adopted_loop(&mut app, conn);
            drain(&wakeups_rx);

            manager.update(&mut app, |_manager, ctx| {
                ctx.emit(output_event(conn, OUR_PTY, 0, b"before-attach"));
            });

            event_loop.read(&app, |me, _| {
                assert_eq!(me.last_seq, 0);
                assert_eq!(me.pending_output.len(), 1);
                assert!(
                    me.awaiting_attach_snapshot,
                    "an adopted PTY is provisional until SessionAttached arrives"
                );
            });

            event_loop.update(&mut app, |me, _ctx| {
                me.apply_authoritative_agent_binding_state(None);
                me.awaiting_attach_snapshot = false;
                me.drain_pending_output();
            });
            event_loop.read(&app, |me, _| {
                assert_eq!(me.last_seq, b"before-attach".len() as u64);
                assert!(me.pending_output.is_empty());
            });
        });
    }

    #[test]
    fn exit_waits_for_matching_attach_snapshot() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(28u64);
            let (manager, event_loop, _model, _wakeups_rx) = start_adopted_loop(&mut app, conn);

            manager.update(&mut app, |_manager, ctx| {
                ctx.emit(RemoteServerManagerEvent::SessionExited {
                    session_id: conn,
                    host_id: HostId::new(HOST.to_string()),
                    pty_session_id: OUR_PTY.to_string(),
                    exit_code: Some(0),
                });
            });
            event_loop.read(&app, |me, _ctx| {
                assert_eq!(me.pending_exit, Some(Some(0)));
                assert!(!me.terminated, "exit is ordered behind the attach replay");
            });

            event_loop.update(&mut app, |me, ctx| {
                me.on_session_attached(
                    SessionAttached {
                        session_id: OUR_PTY.to_string(),
                        size: None,
                        base_seq: 0,
                        replay: b"final-output".to_vec(),
                        bootstrap_preamble: Vec::new(),
                        generation: 7,
                        agent_binding: None,
                    },
                    true,
                    ctx,
                );
            });
            event_loop.read(&app, |me, _ctx| {
                assert!(me.terminated);
                assert!(me.pending_exit.is_none());
                assert!(!me.awaiting_attach_snapshot);
                assert!(
                    !me.welcomed,
                    "a completed session is never welcomed as live"
                );
                assert_eq!(me.last_seq, b"final-output".len() as u64);
            });
        });
    }

    #[test]
    fn attach_output_overflow_requests_replay_before_live_delivery() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(29u64);
            let (_manager, event_loop, _model, _wakeups_rx) = start_adopted_loop(&mut app, conn);

            event_loop.update(&mut app, |me, ctx| {
                me.buffer_pending_output(OUR_PTY, 4, &vec![b'x'; MAX_PENDING_OUTPUT_BYTES]);
                me.buffer_pending_output(OUR_PTY, 4 + MAX_PENDING_OUTPUT_BYTES as u64, b"dropped");
                me.on_session_attached(
                    SessionAttached {
                        session_id: OUR_PTY.to_string(),
                        size: None,
                        base_seq: 0,
                        replay: b"base".to_vec(),
                        bootstrap_preamble: Vec::new(),
                        generation: 7,
                        agent_binding: None,
                    },
                    true,
                    ctx,
                );
            });
            event_loop.read(&app, |me, _ctx| {
                assert!(
                    me.awaiting_attach_snapshot,
                    "overflow must keep live output closed until another replay fills the gap"
                );
                assert!(!me.welcomed);
                assert!(me.pending_output.is_empty());
                assert!(!me.pending_output_overflowed);
                assert_eq!(
                    me.last_seq,
                    4 + MAX_PENDING_OUTPUT_BYTES as u64,
                    "the retry cursor advances only through contiguous buffered bytes"
                );
            });
        });
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
            complete_adopted_attach(&event_loop, &mut app);
            drain(&wakeups_rx);

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
                me.on_session_opened("pty-late".to_string(), 7, ctx);
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

    /// A daemon session becoming addressable is not proof that its shell is
    /// ready for input. The startup command must stay queued through `InitShell`
    /// and run only after the real `Bootstrapped` boundary. With no live client
    /// it remains pending; the synchronous transport seam then verifies the
    /// exact request bytes and positive-Ack completion independently.
    #[test]
    fn startup_command_waits_for_bootstrap_and_runs_once() {
        App::test((), |mut app| async move {
            let manager = app.add_singleton_model(RemoteServerManager::new);
            let conn = SessionId::from(17u64);
            let (listener, _wakeups_rx) = test_listener();
            let model = Arc::new(FairMutex::new(TerminalModel::mock_not_bootstrapped(Some(
                listener.clone(),
            ))));
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
                    None,
                    None,
                    "test-host".to_string(),
                    ctx,
                )
            });

            event_loop.update(&mut app, |me, ctx| {
                me.startup_command = Some("tmux attach".to_string());
                me.on_session_opened("pty-x".to_string(), 7, ctx);
            });

            event_loop.read(&app, |me, _| {
                assert_eq!(
                    me.startup_command.as_deref(),
                    Some("tmux attach"),
                    "SessionOpened alone must not consume the startup command"
                );
                assert!(
                    me.pending_input.is_empty(),
                    "SessionOpened alone must not send input into a bootstrapping shell"
                );
            });

            let init_shell = init_shell_dcs();
            manager.update(&mut app, |_manager, ctx| {
                ctx.emit(output_event(conn, "pty-x", 0, &init_shell));
            });
            event_loop.read(&app, |me, _| {
                assert_eq!(
                    me.startup_command.as_deref(),
                    Some("tmux attach"),
                    "InitShell is not readiness: the daemon body has not completed"
                );
                assert!(me.pending_input.is_empty());
            });

            let bootstrapped = bootstrapped_dcs();
            manager.update(&mut app, |_manager, ctx| {
                ctx.emit(output_event(
                    conn,
                    "pty-x",
                    init_shell.len() as u64,
                    &bootstrapped,
                ));
            });
            event_loop.read(&app, |me, _| {
                assert_eq!(
                    me.startup_command.as_deref(),
                    Some("tmux attach"),
                    "without a connected retry-safe client the ready command remains pending"
                );
                assert!(me.pending_input.is_empty());
            });

            let mut dispatched = None;
            event_loop.update(&mut app, |me, _ctx| {
                me.try_dispatch_startup_command_with(
                    |pty_session_id, command_id, bytes| -> Result<(), ()> {
                        dispatched = Some((
                            pty_session_id.to_string(),
                            command_id.to_string(),
                            bytes.to_vec(),
                        ));
                        Ok(())
                    },
                );
            });
            let (pty_session_id, command_id, bytes) =
                dispatched.expect("bootstrapped startup command dispatched");
            assert_eq!(pty_session_id, "pty-x");
            assert_eq!(bytes, b"tmux attach\n");
            event_loop.update(&mut app, |me, _ctx| {
                me.acknowledge_startup_command(&command_id)
            });
            event_loop.read(&app, |me, _| {
                assert!(
                    me.startup_command.is_none(),
                    "matching positive Ack completes the startup command"
                );
            });

            manager.update(&mut app, |_manager, ctx| {
                ctx.emit(output_event(
                    conn,
                    "pty-x",
                    (init_shell.len() + bootstrapped.len()) as u64,
                    b"later output",
                ));
            });
            event_loop.read(&app, |me, _| {
                assert!(me.startup_command.is_none());
                assert!(me.startup_command_in_flight.is_none());
            });
        });
    }

    #[test]
    fn does_not_run_on_session_opened_or_init_shell() {
        let mut event_loop =
            unbootstrapped_event_loop_with_startup("codex resume not-ready-session");

        assert!(
            event_loop.prepare_startup_command_delivery().is_none(),
            "SessionOpened is only represented by the PTY id and is not readiness"
        );
        event_loop.process_pty_bytes(&init_shell_dcs());
        assert!(
            event_loop.prepare_startup_command_delivery().is_none(),
            "InitShell must not release startup before the body reaches Bootstrapped"
        );
        assert_eq!(
            event_loop.startup_command.as_deref(),
            Some("codex resume not-ready-session")
        );
        assert!(event_loop.startup_command_id.is_none());
    }

    #[test]
    fn survives_replay_then_live_bootstrap() {
        let mut event_loop =
            unbootstrapped_event_loop_with_startup("claude --resume replay-session");
        let replay = init_shell_dcs();

        event_loop.apply_attach(&[], 0, &replay);
        assert!(
            event_loop.prepare_startup_command_delivery().is_none(),
            "an InitShell recovered from replay is still not readiness"
        );
        event_loop.process_pty_bytes(&bootstrapped_dcs());

        let (pty_session_id, command_id, bytes, _attempt) = event_loop
            .prepare_startup_command_delivery()
            .expect("the live Bootstrapped boundary releases the retained startup");
        assert_eq!(pty_session_id, OUR_PTY);
        assert!(!command_id.is_empty());
        assert_eq!(bytes, b"claude --resume replay-session\n");
        event_loop.acknowledge_startup_command(&command_id);
        assert!(event_loop.startup_command.is_none());
    }

    /// A startup command is not ordinary terminal input: losing it leaves the
    /// newly opened tab at a shell prompt instead of starting the requested
    /// agent. A failed client enqueue must therefore keep the command pending
    /// under the same delivery id for a later reconnect retry.
    ///
    /// `try_dispatch_startup_command_with` is the transport seam required by
    /// this contract. Production dispatch uses the real daemon client; the test
    /// supplies the precise failure that was previously only logged.
    #[test]
    fn startup_command_is_retained_when_daemon_enqueue_fails() {
        let mut event_loop = ready_event_loop_with_startup("codex resume session-1");
        let mut attempted = None;

        event_loop.try_dispatch_startup_command_with(
            |pty_session_id, command_id, bytes| -> Result<(), ()> {
                attempted = Some((
                    pty_session_id.to_string(),
                    command_id.to_string(),
                    bytes.to_vec(),
                ));
                Err(())
            },
        );

        let (pty_session_id, command_id, bytes) = attempted.expect("dispatch was attempted");
        assert_eq!(pty_session_id, OUR_PTY);
        assert!(
            !command_id.is_empty(),
            "every startup delivery needs a stable id"
        );
        assert_eq!(bytes, b"codex resume session-1\n");
        assert_eq!(
            event_loop.startup_command.as_deref(),
            Some("codex resume session-1"),
            "an enqueue error must not consume the startup command"
        );
    }

    /// Successfully placing a frame on the client channel is not proof that the
    /// daemon received or executed it. The command remains pending until an ack
    /// carrying its exact delivery id arrives; a stale or foreign ack is ignored.
    #[test]
    fn startup_command_remains_pending_until_daemon_ack() {
        let mut event_loop = ready_event_loop_with_startup("claude --resume session-2");
        let mut command_id = None;

        event_loop.try_dispatch_startup_command_with(
            |_pty_session_id, id, _bytes| -> Result<(), ()> {
                command_id = Some(id.to_string());
                Ok(())
            },
        );

        let command_id = command_id.expect("successful enqueue exposes its delivery id");
        assert_eq!(
            event_loop.startup_command.as_deref(),
            Some("claude --resume session-2"),
            "local enqueue must not clear an unacknowledged startup command"
        );

        event_loop.acknowledge_startup_command("ack-for-another-command");
        assert!(
            event_loop.startup_command.is_some(),
            "a mismatched ack must not clear the pending startup command"
        );

        event_loop.acknowledge_startup_command(&command_id);
        assert!(
            event_loop.startup_command.is_none(),
            "only the matching daemon ack completes startup delivery"
        );
    }

    /// If the daemon executed a command but its ack was lost, reconnect retries
    /// the same logical delivery. Reusing the id is what lets the daemon return
    /// its cached ack instead of executing the command a second time.
    #[test]
    fn midflight_reconnect_reuses_command_id_and_runs_once() {
        let mut event_loop = ready_event_loop_with_startup("codex resume session-3");
        let mut attempts = Vec::new();

        event_loop.try_dispatch_startup_command_with(
            |_pty_session_id, command_id, bytes| -> Result<(), ()> {
                attempts.push((command_id.to_string(), bytes.to_vec()));
                Ok(())
            },
        );
        event_loop.begin_transport_reconnect();
        assert!(
            event_loop.awaiting_attach_snapshot,
            "the production reconnect transition must require a fresh attach snapshot"
        );
        event_loop.try_dispatch_startup_command_with(
            |_pty_session_id, command_id, bytes| -> Result<(), ()> {
                attempts.push((command_id.to_string(), bytes.to_vec()));
                Ok(())
            },
        );

        assert_eq!(attempts.len(), 2, "lost ack causes one retry");
        assert_eq!(
            attempts[0].0, attempts[1].0,
            "retry must reuse the original id so the daemon can deduplicate it"
        );
        assert_eq!(attempts[0].1, attempts[1].1);
        assert!(
            event_loop.startup_command.is_some(),
            "without an ack the command is still pending after retry"
        );
        event_loop.acknowledge_startup_command(&attempts[1].0);
        event_loop.try_dispatch_startup_command_with(
            |_pty_session_id, command_id, bytes| -> Result<(), ()> {
                attempts.push((command_id.to_string(), bytes.to_vec()));
                Ok(())
            },
        );
        assert_eq!(
            attempts.len(),
            2,
            "after the reconnect retry is acknowledged, no third execution is dispatched"
        );
    }

    #[test]
    fn startup_command_in_flight_suppresses_duplicate_local_attempts() {
        let mut event_loop = ready_event_loop_with_startup("codex resume session-in-flight");
        let mut attempts = 0;

        for _ in 0..2 {
            event_loop.try_dispatch_startup_command_with(
                |_pty_session_id, _command_id, _bytes| -> Result<(), ()> {
                    attempts += 1;
                    Ok(())
                },
            );
        }

        assert_eq!(
            attempts, 1,
            "output bursts must not create parallel startup requests"
        );
        assert!(event_loop.startup_command_in_flight.is_some());
    }

    /// User keystrokes may be dropped oldest-first after a prolonged outage, but
    /// an unacknowledged startup command is control state, not disposable input.
    /// Buffer pressure must neither remove it nor mint a different delivery id.
    #[test]
    fn pending_buffer_never_evicts_unacknowledged_startup() {
        let mut event_loop = ready_event_loop_with_startup("codex resume session-4");
        let mut command_ids = Vec::new();
        event_loop.try_dispatch_startup_command_with(
            |_pty_session_id, command_id, _bytes| -> Result<(), ()> {
                command_ids.push(command_id.to_string());
                Ok(())
            },
        );
        let original_command_id = event_loop
            .startup_command_id
            .clone()
            .expect("first attempt creates a stable id");

        for _ in 0..5 {
            event_loop.buffer_pending(EventLoopMessage::Input(Cow::Owned(vec![b'x'; 100 * 1024])));
        }

        assert_eq!(
            event_loop.startup_command.as_deref(),
            Some("codex resume session-4"),
            "ordinary input eviction must never remove pending startup control state"
        );

        event_loop.allow_startup_command_retry();
        event_loop.try_dispatch_startup_command_with(
            |_pty_session_id, command_id, _bytes| -> Result<(), ()> {
                command_ids.push(command_id.to_string());
                Ok(())
            },
        );
        assert_eq!(
            original_command_id, command_ids[1],
            "buffer pressure must not replace the startup delivery identity"
        );

        let buffered_input_bytes: usize = event_loop
            .pending_input
            .iter()
            .map(|message| match message {
                EventLoopMessage::Input(bytes) => bytes.len(),
                EventLoopMessage::Resize(_)
                | EventLoopMessage::Shutdown
                | EventLoopMessage::ChildExited => 0,
            })
            .sum();
        assert!(
            buffered_input_bytes <= MAX_PENDING_INPUT_BYTES,
            "normal input remains bounded independently of startup delivery"
        );
    }

    fn ready_event_loop_with_startup(command: &str) -> EventLoop {
        let conn = SessionId::from(18u64);
        let (listener, _wakeups_rx) = test_listener();
        let model = Arc::new(FairMutex::new(TerminalModel::mock(
            None,
            Some(listener.clone()),
        )));
        let mut event_loop = EventLoop::new(model, listener, conn);
        event_loop.pty_session_id = Some(OUR_PTY.to_string());
        event_loop.startup_command = Some(command.to_string());
        event_loop
    }

    fn unbootstrapped_event_loop_with_startup(command: &str) -> EventLoop {
        let conn = SessionId::from(19u64);
        let (listener, _wakeups_rx) = test_listener();
        let model = Arc::new(FairMutex::new(TerminalModel::mock_not_bootstrapped(Some(
            listener.clone(),
        ))));
        let mut event_loop = EventLoop::new(model, listener, conn);
        event_loop.pty_session_id = Some(OUR_PTY.to_string());
        event_loop.startup_command = Some(command.to_string());
        event_loop
    }

    #[test]
    fn midflight_agent_binding_reconnect_invalidates_stale_callback() {
        let mut event_loop = ready_event_loop_with_startup("codex resume session-binding");

        let dead_transport_attempt = event_loop.start_agent_binding_attempt();
        event_loop.allow_agent_binding_retry();
        let reconnected_attempt = event_loop.start_agent_binding_attempt();

        assert_ne!(dead_transport_attempt, reconnected_attempt);
        assert!(
            !event_loop.finish_agent_binding_attempt(dead_transport_attempt),
            "a callback from the dead transport must not complete the retry"
        );
        assert_eq!(
            event_loop.agent_binding_in_flight,
            Some(reconnected_attempt)
        );
        assert!(event_loop.finish_agent_binding_attempt(reconnected_attempt));
        assert!(event_loop.agent_binding_in_flight.is_none());
    }

    #[test]
    fn adopted_foreground_agent_hydrates_lifecycle_routing() {
        let mut event_loop = ready_event_loop_with_startup("codex resume adopted");
        let identity = AgentSessionIdentity {
            session_id: "agent-1".to_string(),
            provider: "codex".to_string(),
            account_email: "agent@example.com".to_string(),
            config_dir: "/home/agent/.codex".to_string(),
        };

        event_loop.apply_authoritative_agent_binding_state(Some(identity.clone()));

        assert_eq!(event_loop.agent_binding.as_ref(), Some(&identity));
        assert_eq!(
            event_loop.desired_agent_binding.as_ref(),
            Some(&identity),
            "the first lifecycle change must hand off or unbind the daemon's existing foreground"
        );
    }

    #[test]
    fn attach_hydration_preserves_a_pending_explicit_handoff() {
        let mut event_loop = ready_event_loop_with_startup("codex resume adopted");
        let current = AgentSessionIdentity {
            session_id: "agent-1".to_string(),
            provider: "codex".to_string(),
            account_email: "agent@example.com".to_string(),
            config_dir: "/home/agent/.codex".to_string(),
        };
        let desired = AgentSessionIdentity {
            session_id: "agent-2".to_string(),
            ..current.clone()
        };
        event_loop.desired_agent_binding = Some(desired.clone());
        event_loop.desired_agent_binding_from_lifecycle = true;

        event_loop.apply_authoritative_agent_binding_state(Some(current.clone()));

        assert_eq!(event_loop.agent_binding.as_ref(), Some(&current));
        assert_eq!(
            event_loop.desired_agent_binding.as_ref(),
            Some(&desired),
            "the daemon foreground becomes handoff_from without erasing the locally desired agent"
        );
    }

    #[test]
    fn lifecycle_handoff_during_attach_is_not_discarded() {
        App::test((), |mut app| async move {
            use crate::terminal::cli_agent_sessions::{
                CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext,
                CLIAgentSessionStatus,
            };

            let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
            let conn = SessionId::from(29u64);
            let (listener, _wakeups_rx) = test_listener();
            let model = Arc::new(FairMutex::new(TerminalModel::mock(
                None,
                Some(listener.clone()),
            )));
            let terminal_view_id = EntityId::new();
            let event_loop = app.add_model(|_ctx| EventLoop::new(model, listener, conn));
            event_loop.update(&mut app, |me, ctx| {
                me.awaiting_attach_snapshot = true;
                me.bind_terminal_view(terminal_view_id, ctx);
            });

            sessions.update(&mut app, |sessions, ctx| {
                sessions.bind_account_identity(
                    terminal_view_id,
                    CLIAgent::Codex,
                    Some("/home/agent/.codex-b".to_string()),
                    Some("b@example.com".to_string()),
                );
                sessions.set_session(
                    terminal_view_id,
                    CLIAgentSession {
                        agent: CLIAgent::Codex,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext {
                            session_id: Some("agent-b".to_string()),
                            ..Default::default()
                        },
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        plugin_version: None,
                        remote_host: None,
                        draft_text: None,
                        custom_command_prefix: None,
                    },
                    ctx,
                );
            });

            let agent_a = AgentSessionIdentity {
                session_id: "agent-a".to_string(),
                provider: "codex".to_string(),
                account_email: "a@example.com".to_string(),
                config_dir: "/home/agent/.codex-a".to_string(),
            };
            event_loop.update(&mut app, |me, _ctx| {
                me.apply_authoritative_agent_binding_state(Some(agent_a.clone()));
            });
            event_loop.read(&app, |me, _ctx| {
                assert_eq!(me.agent_binding.as_ref(), Some(&agent_a));
                assert_eq!(
                    me.desired_agent_binding.as_ref(),
                    Some(&AgentSessionIdentity {
                        session_id: "agent-b".to_string(),
                        provider: "codex".to_string(),
                        account_email: "b@example.com".to_string(),
                        config_dir: "/home/agent/.codex-b".to_string(),
                    }),
                    "the lifecycle change must remain pending as an explicit handoff"
                );
                assert!(me.desired_agent_binding_from_lifecycle);
                assert!(me.agent_binding_in_flight.is_none());
            });
        });
    }

    #[test]
    fn authoritative_unbound_attach_clears_stale_inventory_seed() {
        let mut event_loop = ready_event_loop_with_startup("codex resume adopted");
        let stale = AgentSessionIdentity {
            session_id: "agent-stale".to_string(),
            provider: "codex".to_string(),
            account_email: "agent@example.com".to_string(),
            config_dir: "/home/agent/.codex".to_string(),
        };
        event_loop.agent_binding = Some(stale.clone());
        event_loop.desired_agent_binding = Some(stale);

        event_loop.apply_authoritative_agent_binding_state(None);

        assert!(event_loop.agent_binding.is_none());
        assert!(event_loop.desired_agent_binding.is_none());
    }

    #[test]
    fn sidebar_attach_binds_authoritative_agent_account_identity() {
        App::test((), |mut app| async move {
            let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
            let conn = SessionId::from(25u64);
            let (listener, _wakeups_rx) = test_listener();
            let model = Arc::new(FairMutex::new(TerminalModel::mock(
                None,
                Some(listener.clone()),
            )));
            let terminal_view_id = EntityId::new();
            let identity = AgentSessionIdentity {
                session_id: "agent-sidebar".to_string(),
                provider: "codex".to_string(),
                account_email: "sidebar@example.com".to_string(),
                config_dir: "/home/agent/.codex-sidebar".to_string(),
            };
            let event_loop = app.add_model(|_ctx| EventLoop::new(model, listener, conn));

            event_loop.update(&mut app, |me, ctx| {
                me.terminal_view_id = Some(terminal_view_id);
                me.apply_authoritative_agent_binding(Some(identity.clone()), ctx);
            });

            sessions.read(&app, |sessions, _ctx| {
                let account = sessions
                    .account_identity(terminal_view_id)
                    .expect("authoritative attach must bind the sidebar account");
                assert_eq!(account.agent(), CLIAgent::Codex);
                assert_eq!(
                    account.account_email.as_deref(),
                    Some("sidebar@example.com")
                );
                assert_eq!(
                    account.config_dir.as_deref(),
                    Some("/home/agent/.codex-sidebar")
                );
            });
        });
    }

    #[test]
    fn authoritative_none_clears_stale_adopt_account_identity() {
        App::test((), |mut app| async move {
            let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
            let conn = SessionId::from(27u64);
            let (listener, _wakeups_rx) = test_listener();
            let model = Arc::new(FairMutex::new(TerminalModel::mock(
                None,
                Some(listener.clone()),
            )));
            let terminal_view_id = EntityId::new();
            sessions.update(&mut app, |sessions, _ctx| {
                sessions.bind_account_identity(
                    terminal_view_id,
                    CLIAgent::Codex,
                    Some("/stale/config".to_string()),
                    Some("stale@example.com".to_string()),
                );
            });
            let event_loop = app.add_model(|_ctx| EventLoop::new(model, listener, conn));

            event_loop.update(&mut app, |me, ctx| {
                me.terminal_view_id = Some(terminal_view_id);
                me.apply_authoritative_agent_binding(None, ctx);
            });

            assert!(
                sessions.read(&app, |sessions, _ctx| {
                    sessions.account_identity(terminal_view_id).is_none()
                }),
                "an unbound attach snapshot must remove a provisional stale account route"
            );
        });
    }

    #[test]
    fn settled_binding_does_not_override_a_later_reconnect_snapshot() {
        let mut event_loop = ready_event_loop_with_startup("codex resume settled");
        let settled = AgentSessionIdentity {
            session_id: "agent-settled".to_string(),
            provider: "codex".to_string(),
            account_email: "agent@example.com".to_string(),
            config_dir: "/home/agent/.codex".to_string(),
        };
        event_loop.agent_binding = Some(settled.clone());
        event_loop.desired_agent_binding = Some(settled);
        event_loop.desired_agent_binding_from_lifecycle = true;

        // The no-op convergence path represents a bind that is fully settled.
        // A later authoritative reconnect snapshot must therefore replace it.
        event_loop.settle_agent_binding_if_converged();
        event_loop.apply_authoritative_agent_binding_state(None);

        assert!(event_loop.agent_binding.is_none());
        assert!(event_loop.desired_agent_binding.is_none());
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
                me.on_session_opened("pty-late".to_string(), 7, ctx)
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
            complete_adopted_attach(&event_loop, &mut app);
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

    /// The real completion boundary emitted after the shell has finished the
    /// bootstrap body. `InitShell` alone is deliberately insufficient.
    fn bootstrapped_dcs() -> Vec<u8> {
        // Fields with custom deserializers must be present even when empty.
        let json = r#"{
            "hook":"Bootstrapped",
            "value":{
                "histfile":"",
                "shell":"zsh",
                "home_dir":"",
                "path":"",
                "editor":"",
                "aliases":"",
                "abbreviations":"",
                "function_names":"",
                "env_var_names":"",
                "builtins":"",
                "keywords":"",
                "shell_version":"",
                "shell_options":"",
                "rcfiles_start_time":"",
                "rcfiles_end_time":"",
                "shell_plugins":"",
                "vi_mode_enabled":"",
                "os_category":"",
                "linux_distribution":"",
                "wsl_name":"",
                "shell_path":""
            }
        }"#;
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

    /// Like [`test_listener`] but also keeps the terminal-events receiver, so a
    /// test can observe the `HandlerEvent`s the model emits while parsing.
    fn test_listener_with_events() -> (
        ChannelEventListener,
        async_channel::Receiver<()>,
        async_channel::Receiver<crate::terminal::event::Event>,
    ) {
        let (wakeups_tx, wakeups_rx) = async_channel::unbounded();
        let (events_tx, events_rx) = async_channel::unbounded();
        let (pty_reads_tx, _pty_reads_rx) = async_broadcast::broadcast(1);
        (
            ChannelEventListener::new(wakeups_tx, events_tx, pty_reads_tx),
            wakeups_rx,
            events_rx,
        )
    }

    /// Drains the events receiver and returns the stamp of the first emitted
    /// `HandlerEvent::InitShell`, if any.
    fn drained_initshell_stamp(
        events_rx: &async_channel::Receiver<crate::terminal::event::Event>,
    ) -> Option<bool> {
        use crate::terminal::event::Event as TermEvent;
        use crate::terminal::model::terminal_model::HandlerEvent;
        while let Ok(event) = events_rx.try_recv() {
            if let TermEvent::Handler(HandlerEvent::InitShell {
                suppress_bootstrap_write,
                ..
            }) = event
            {
                return Some(suppress_bootstrap_write);
            }
        }
        None
    }

    fn daemon_root_initshell_stamp(app: &mut App, conn: u64, shell: &str) -> Option<bool> {
        let conn = SessionId::from(conn);
        let _manager = app.add_singleton_model(RemoteServerManager::new);
        let (listener, _wakeups_rx, events_rx) = test_listener_with_events();
        let model = Arc::new(FairMutex::new(TerminalModel::mock_not_bootstrapped(Some(
            listener.clone(),
        ))));
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
                Some(7),
                None,
                None,
                "test-host".to_string(),
                ctx,
            )
        });
        drain(&events_rx);

        let json = format!(
            r#"{{"hook":"InitShell","value":{{"session_id":167303092612203,"shell":"{shell}"}}}}"#
        );
        let mut dcs = vec![0x1b, 0x50, 0x24, 0x64]; // ESC P $ d
        dcs.extend_from_slice(hex::encode(json).as_bytes());
        dcs.push(0x9c); // ST
        event_loop.update(app, |me, _| me.process_pty_bytes(&dcs));

        drained_initshell_stamp(&events_rx)
    }

    /// The regression this guards (RC acceptance 2026-07-21): a daemon
    /// session's *live* `InitShell` handshake arrived unstamped — only the
    /// adopt-preamble re-feed was covered (T1.3) — so the client typed the
    /// ~90 KB bootstrap body into the shell the daemon had already bootstrapped
    /// server-side. It executed a second time, visibly, as command blocks, on
    /// every connect. A daemon-backed model must stamp its root-shell
    /// `InitShell` regardless of how the bytes arrive (the stamp source is the
    /// persistent mark set in `start`, identical for fresh opens and adopts —
    /// this test drives the live-stream parse path).
    #[test]
    fn live_initshell_of_a_daemon_session_is_stamped_suppressed() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(61u64);
            let _manager = app.add_singleton_model(RemoteServerManager::new);
            let (listener, _wakeups_rx, events_rx) = test_listener_with_events();
            let model = Arc::new(FairMutex::new(TerminalModel::mock_not_bootstrapped(Some(
                listener.clone(),
            ))));
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
                    Some(7),
                    None,
                    None,
                    "test-host".to_string(),
                    ctx,
                )
            });
            drain(&events_rx);

            // The live handshake: the InitShell DCS arrives in the normal output
            // stream of the daemon session — no adopt preamble, no armed latch.
            let dcs = init_shell_dcs();
            event_loop.update(&mut app, |me, _| me.process_pty_bytes(&dcs));

            assert_eq!(
                drained_initshell_stamp(&events_rx),
                Some(true),
                "a daemon-backed session's live InitShell must be stamped \
                 suppress_bootstrap_write — the daemon already delivered the \
                 bootstrap server-side; an unstamped event makes the client type \
                 the body into the live shell (the connect-time script dump)"
            );
        });
    }

    /// The subshell boundary: a nested shell Zaplexified INSIDE a daemon tab
    /// (`is_subshell` on the wire) is never bootstrapped by the daemon — the
    /// client-side write is its only mechanism — so its `InitShell` must stay
    /// unstamped even on a daemon-marked model.
    #[test]
    fn subshell_initshell_inside_a_daemon_tab_stays_unstamped() {
        App::test((), |mut app| async move {
            let conn = SessionId::from(67u64);
            let _manager = app.add_singleton_model(RemoteServerManager::new);
            let (listener, _wakeups_rx, events_rx) = test_listener_with_events();
            let model = Arc::new(FairMutex::new(TerminalModel::mock_not_bootstrapped(Some(
                listener.clone(),
            ))));
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
                    Some(7),
                    None,
                    None,
                    "test-host".to_string(),
                    ctx,
                )
            });
            drain(&events_rx);

            let json = r#"{"hook":"InitShell","value":{"session_id":167303092612202,"shell":"zsh","is_subshell":true}}"#;
            let mut dcs = vec![0x1b, 0x50, 0x24, 0x64]; // ESC P $ d
            dcs.extend_from_slice(hex::encode(json).as_bytes());
            dcs.push(0x9c); // ST
            event_loop.update(&mut app, |me, _| me.process_pty_bytes(&dcs));

            assert_eq!(
                drained_initshell_stamp(&events_rx),
                Some(false),
                "a subshell InitShell inside a daemon tab must stay unstamped — \
                 the daemon never bootstraps nested shells, so suppressing the \
                 client-side write would leave them without integration"
            );
        });
    }

    /// Fish is bootstrapped from the daemon-owned guarded body file, so the
    /// client must suppress its duplicate root-body write.
    #[test]
    fn fish_root_initshell_in_a_daemon_tab_is_stamped_suppressed() {
        App::test((), |mut app| async move {
            assert_eq!(
                daemon_root_initshell_stamp(&mut app, 71, "fish"),
                Some(true),
                "a daemon-backed fish root must not receive a second body from the client"
            );
        });
    }

    /// PowerShell follows the same guarded daemon-body contract as fish.
    #[test]
    fn pwsh_root_initshell_in_a_daemon_tab_is_stamped_suppressed() {
        App::test((), |mut app| async move {
            assert_eq!(
                daemon_root_initshell_stamp(&mut app, 72, "pwsh"),
                Some(true),
                "a daemon-backed PowerShell root must not receive a second body from the client"
            );
        });
    }

    /// The counterpart boundary: a model NOT driven by a daemon event loop (a
    /// local or legacy-SSH pane) must keep emitting unstamped `InitShell`s —
    /// those panes rely on the client-side bootstrap write.
    #[test]
    fn initshell_of_a_non_daemon_model_stays_unstamped() {
        App::test((), |mut app_| async move {
            let (listener, _wakeups_rx, events_rx) = test_listener_with_events();
            let model = Arc::new(FairMutex::new(TerminalModel::mock_not_bootstrapped(Some(
                listener,
            ))));
            drain(&events_rx);

            // Feed the same wire-form InitShell DCS straight through a parser —
            // no daemon event loop ever touched this model.
            let dcs = init_shell_dcs();
            let mut parser = Processor::default();
            parser.parse_bytes(&mut *model.lock(), &dcs, &mut io::sink());

            assert_eq!(
                drained_initshell_stamp(&events_rx),
                Some(false),
                "without a daemon backing, InitShell must stay unstamped so the \
                 client-side bootstrap write still initializes local/legacy panes"
            );
            let _ = &mut app_;
        });
    }
}
