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
                            "grok" => Some(CLIAgent::Grok),
                            "antigravity" => Some(CLIAgent::Antigravity),
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
                CLIAgent::Grok => "grok",
                CLIAgent::Antigravity => "antigravity",
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
        let future = async move {
            client
                .open_session(cwd, shell, env, rows, cols, ring_ceiling_bytes)
                .await
        };
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
            self.terminal_model
                .lock()
                .take_suppress_next_bootstrap_write();
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
#[path = "event_loop_tests.rs"]
mod tests;
