use crate::remote_server::manager::{RemoteServerManager, RemoteServerManagerEvent};
use crate::terminal::{
    cli_agent::CLIAgent,
    cli_agent_sessions::{CLIAgentSessionsModel, CLIAgentSessionsModelEvent},
    event_listener::ChannelEventListener,
    model::{ansi::Processor, terminal_model::ExitReason},
    view::{RemoteInputPhase, TerminalView},
    writeable_pty::Message as EventLoopMessage,
    SizeInfo, TerminalModel,
};
use async_channel::Receiver;
use parking_lot::FairMutex;
use remote_server::{
    client::{ClientError, RemoteServerClient},
    proto::{AgentLaunchRoute, AgentPtyBindingStatus, AgentSessionIdentity, SessionAttached},
};
use std::borrow::Cow;
use std::io::{self, Write};
use std::sync::{Arc, Weak};
use std::time::Duration;
use warp_core::SessionId;
use warpui::{Entity, EntityId, ModelContext, SingletonEntity, ViewHandle, WeakViewHandle};
use zaplex_remote_session::types::{
    FEATURE_AGENT_ACCOUNT_ROUTING_V1, FEATURE_AGENT_PTY_BINDING_V2,
    FEATURE_LOGICAL_OPEN_ATTEMPT_V1, FEATURE_LOGICAL_OPEN_ID_V1, FEATURE_MANAGED_AGENT_FLEET_V1,
    FEATURE_MANAGED_OPEN_ATTACH_V1, FEATURE_STARTUP_COMMAND_ACK,
};

use super::terminal_manager::OpenSessionParams;

/// Cap on protocol replies buffered while the transport is down. Ordinary user
/// input is never placed in this buffer: executing text later would be more
/// surprising than rejecting it while the pane is not ready.
const MAX_PENDING_PROTOCOL_INPUT_BYTES: usize = 256 * 1024;
const MAX_STARTUP_COMMAND_DELIVERY_ATTEMPTS: u64 = 3;
const MAX_OPEN_SESSION_DELIVERY_ATTEMPTS: u64 = 2;

/// Safety valve for output buffered before `OpenSession` resolves (the daemon
/// auto-attaches and starts the shell/bootstrap before the response reaches us).
/// The open window is normally sub-second, so this is far more than any bootstrap
/// burst; past it we stop buffering (the early bootstrap prefix is preserved)
/// rather than grow without bound if an open hangs on a chatty session.
const MAX_PENDING_OUTPUT_BYTES: usize = 1024 * 1024;

/// Terminal query replies are normally a few bytes. Keep a generous hard cap
/// so malformed output cannot make response collection unbounded.
const MAX_TERMINAL_REPLY_BYTES: usize = 64 * 1024;

#[derive(Default)]
struct TerminalReplyBuffer {
    bytes: Vec<u8>,
    truncated: bool,
}

impl Write for TerminalReplyBuffer {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let remaining = MAX_TERMINAL_REPLY_BYTES.saturating_sub(self.bytes.len());
        let accepted = remaining.min(buffer.len());
        self.bytes.extend_from_slice(&buffer[..accepted]);
        self.truncated |= accepted < buffer.len();
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn account_route_is_compatible(
    route: Option<&AgentLaunchRoute>,
    supports_account_routing: bool,
) -> bool {
    route.is_none() || supports_account_routing
}

const ATTACH_PARSE_CHUNK_BYTES: usize = 64 * 1024;
const INITIAL_ATTACH_TIMEOUT: Duration = Duration::from_secs(60);

struct PendingAttachReplay {
    bootstrap_preamble: Vec<u8>,
    preamble_offset: usize,
    base_seq: u64,
    replay: Vec<u8>,
    replay_offset: usize,
    gap_applied: bool,
    fed_preamble: bool,
    pending_exit: Option<Option<i32>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReadyNoticeKind {
    PersistentSessionActive,
    Attached,
}

struct PendingReadyNotice {
    connection_session_id: SessionId,
    pty_session_id: String,
    generation: Option<u64>,
    kind: ReadyNoticeKind,
}

/// Deduplicates the two manager events that describe one connected transport.
///
/// A successful reconnect currently emits `SessionConnected` while installing
/// the replacement client and then `SessionReconnected` as the reconnect
/// notification. Both events carry the same client phase. Attach must run once
/// for that `(transport, PTY, generation)` tuple, while a later replacement
/// client must start a new attach. A `Weak` identity avoids retaining a dead
/// transport solely for deduplication.
struct AttachPhaseGuard<T> {
    phase: Option<(Weak<T>, String, Option<u64>)>,
}

impl<T> Default for AttachPhaseGuard<T> {
    fn default() -> Self {
        Self { phase: None }
    }
}

impl<T> AttachPhaseGuard<T> {
    fn begin(&mut self, client: &Arc<T>, pty_session_id: &str, generation: Option<u64>) -> bool {
        let candidate = Arc::downgrade(client);
        if self
            .phase
            .as_ref()
            .is_some_and(|(current_client, current_pty, current_generation)| {
                Weak::ptr_eq(current_client, &candidate)
                    && current_pty == pty_session_id
                    && *current_generation == generation
            })
        {
            return false;
        }
        self.phase = Some((candidate, pty_session_id.to_string(), generation));
        true
    }
}

struct PendingOpen {
    logical_open_id: String,
    open_params: OpenSessionParams,
    size_info: SizeInfo,
    in_flight: Option<u64>,
    next_attempt: u64,
}

struct OpenSessionClient {
    client: Arc<RemoteServerClient>,
    supports_account_routing: bool,
    supports_managed_fleet: bool,
    supports_logical_open_id: bool,
    supports_logical_open_attempt: bool,
}

impl PendingOpen {
    fn new(open_params: OpenSessionParams, size_info: SizeInfo) -> Self {
        Self {
            logical_open_id: uuid::Uuid::new_v4().to_string(),
            open_params,
            size_info,
            in_flight: None,
            next_attempt: 0,
        }
    }

    fn begin_attempt(&mut self) -> Option<(String, OpenSessionParams, SizeInfo, u64)> {
        if self.in_flight.is_some() || self.next_attempt >= MAX_OPEN_SESSION_DELIVERY_ATTEMPTS {
            return None;
        }
        self.next_attempt = self.next_attempt.saturating_add(1);
        self.in_flight = Some(self.next_attempt);
        Some((
            self.logical_open_id.clone(),
            self.open_params.clone(),
            self.size_info,
            self.next_attempt,
        ))
    }

    fn finish_attempt(&mut self, logical_open_id: &str, attempt: u64) -> bool {
        if self.logical_open_id != logical_open_id || self.in_flight != Some(attempt) {
            return false;
        }
        self.in_flight = None;
        true
    }

    fn can_retry(&self) -> bool {
        self.in_flight.is_none() && self.next_attempt < MAX_OPEN_SESSION_DELIVERY_ATTEMPTS
    }

    fn can_retry_ambiguous_open(&self, supports_attempt_aware_open: bool) -> bool {
        supports_attempt_aware_open && self.can_retry()
    }

    fn allow_retry(&mut self) {
        self.in_flight = None;
    }
}

fn open_ack_requires_authoritative_attach(
    attempt: u64,
    opened: &remote_server::proto::SessionOpened,
) -> bool {
    attempt > 1 || opened.requires_attach
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OpenedDaemonClaim {
    Owned,
    Conflict,
    Unavailable,
}

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
pub(crate) struct EventLoop {
    terminal_model: Arc<FairMutex<TerminalModel>>,
    parser: Processor,
    channel_event_listener: ChannelEventListener,
    /// The manager/connection session used to resolve the live client.
    connection_session_id: SessionId,
    /// Persisted daemon identity expected for a restored session. A registry
    /// node or host label is not sufficient because either may point at a
    /// replacement daemon with a different PTY namespace.
    expected_host_id: Option<String>,
    /// The daemon's PTY session id (from `OpenSession`). `None` until the open
    /// request resolves; until then user input is rejected and control traffic
    /// may be buffered in `pending_input`.
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
    initial_attach_pending: bool,
    pending_attach_replay: Option<PendingAttachReplay>,
    /// A transport replacement that arrives while a replay is being parsed is
    /// deferred until that snapshot is consumed, avoiding overlapping attaches.
    reattach_after_replay: bool,
    /// Attach/replay request token. A reconnect invalidates an older callback.
    attach_in_flight: Option<u64>,
    next_attach_attempt: u64,
    /// One attach per exact transport/PTY/generation phase. The manager emits
    /// both `SessionConnected` and `SessionReconnected` for a reconnect.
    attach_phase: AttachPhaseGuard<RemoteServerClient>,
    /// True only after the authoritative attach/open path and terminal model
    /// prove that user input can be delivered to this exact PTY generation.
    ///
    /// The event loop has no draft editor or renderable input-control state.
    /// The TerminalView must consume this readiness as visible UI gating and
    /// keep text in its draft; this event loop never queues rejected bytes for
    /// later execution.
    user_input_ready: bool,
    input_phase: RemoteInputPhase,
    /// Terminal view whose CLI-agent lifecycle is mirrored to the daemon.
    terminal_view_id: Option<EntityId>,
    terminal_view: Option<WeakViewHandle<TerminalView>>,
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
    /// Internal protocol replies and resize/lifecycle messages received while
    /// the exact session transport is unavailable. Ordinary user input is
    /// rejected instead of entering this later-flushed buffer.
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
    /// Stable logical `OpenSession`, retained until its exact acknowledgement.
    /// A replacement transport retries the same id and parameters; an attempt
    /// generation rejects callbacks from the dead transport.
    pending_open: Option<PendingOpen>,
    /// Stable correlation id for a managed OpenSession. It is consumed only
    /// after an authoritative daemon Ack or terminal failure.
    managed_launch_id: Option<String>,
    /// Managed PTY identity retained until shell input readiness is proven.
    /// Only then may the spawn card report success.
    managed_open_identity: Option<(String, u64)>,
    /// A lost-ack retry may need to bind the newly observed foreground agent
    /// before it can issue the required generation- and agent-checked attach.
    awaiting_managed_agent_binding: bool,
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
    /// The first Ready message distinguishes a newly opened PTY from an adopted
    /// session, even when its transport drops before the initial Ready event.
    first_ready_notice_kind: ReadyNoticeKind,
    /// Success copy staged by an exact open/attach and consumed only when the
    /// same daemon route and PTY generation have real shell-input readiness.
    pending_ready_notice: Option<PendingReadyNotice>,
    /// An attach replay dropped history since the last success notice, so the
    /// next notice must not claim that nothing was lost.
    replay_truncated: bool,
    /// Success notices handed to the terminal view, observable by tests.
    #[cfg(test)]
    published_ready_notices: Vec<String>,
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
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start(
        model: Arc<FairMutex<TerminalModel>>,
        event_loop_rx: Receiver<EventLoopMessage>,
        channel_event_listener: ChannelEventListener,
        size_info: SizeInfo,
        connection_session_id: SessionId,
        open_params: OpenSessionParams,
        adopt_pty_session_id: Option<String>,
        adopt_pty_generation: Option<u64>,
        expected_host_id: Option<String>,
        expected_attach_agent_binding: Option<AgentSessionIdentity>,
        install_progress_rx: Option<Receiver<String>>,
        host_label: String,
        ctx: &mut ModelContext<Self>,
    ) -> Self {
        let mut event_loop = Self::new(model, channel_event_listener, connection_session_id);
        event_loop.expected_host_id = expected_host_id;
        event_loop.managed_launch_id = open_params
            .managed_launch
            .as_ref()
            .map(|launch| launch.launch_id.clone());
        event_loop.startup_command = open_params
            .managed_launch
            .is_none()
            .then(|| open_params.startup_command.clone())
            .flatten()
            .filter(|command| !command.is_empty());
        event_loop.startup_command_id = event_loop
            .startup_command
            .as_ref()
            .map(|_| uuid::Uuid::new_v4().to_string());
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
        event_loop.process_historical_pty_bytes(title_seq.as_bytes());
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
                event_loop.arm_initial_attach_timeout(ctx);
            }
            (Some(_), _) | (None, Some(_)) => {
                event_loop.write_notice(&crate::t!("terminal-daemon-attach-identity-invalid"));
                event_loop.finish_failed_startup(ctx);
            }
            // Open a fresh session once the transport is connected. Only a
            // fresh open witnesses the real bootstrap handshake from seq 0, so
            // only it reports the boundary the daemon freezes (T1.3).
            (None, None) => {
                event_loop.pending_open = Some(PendingOpen::new(open_params, size_info));
                event_loop.report_bootstrap_boundary = true;
                event_loop.arm_initial_attach_timeout(ctx);
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
                if me.terminated {
                    return;
                }
                if me.is_our_session(pty_session_id)
                    && (!me.awaiting_attach_snapshot || me.awaiting_managed_agent_binding)
                {
                    let end_seq = seq.saturating_add(bytes.len() as u64);
                    if end_seq > me.last_seq {
                        if *seq > me.last_seq {
                            // Do not parse across a missing range merely to
                            // discover the managed agent lifecycle. Preserve
                            // the bytes for the authoritative replay instead;
                            // the initial-attach deadline fails closed if the
                            // binding cannot be learned safely before then.
                            me.buffer_pending_output(pty_session_id, *seq, bytes);
                            return;
                        }
                        let offset =
                            me.last_seq.saturating_sub(*seq).min(bytes.len() as u64) as usize;
                        me.process_live_pty_bytes(&bytes[offset..], ctx);
                        me.last_seq = end_seq;
                        me.maybe_report_bootstrap_boundary(ctx);
                        me.maybe_dispatch_startup_command(ctx);
                        me.complete_initial_attach_if_ready(ctx);
                    }
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
                    me.on_session_exited(*exit_code, ctx);
                }
            }
            RemoteServerManagerEvent::SessionConnected { session_id, .. }
                if *session_id == me.connection_session_id =>
            {
                me.on_transport_connected(ctx);
            }
            RemoteServerManagerEvent::SessionConnecting {
                session_id,
                reconnecting,
            } if *session_id == me.connection_session_id => {
                if *reconnecting {
                    me.begin_transport_reconnect(ctx);
                } else {
                    me.set_input_phase(RemoteInputPhase::Transport, ctx);
                }
            }
            // Transport reconnected (SSH blip): the daemon session kept running —
            // re-attach and replay what we missed (§9).
            RemoteServerManagerEvent::SessionReconnected {
                session_id, client, ..
            } if *session_id == me.connection_session_id => {
                // `mark_session_connected` emits `SessionConnected` immediately
                // before this event. `on_transport_connected` has already
                // started the one attach for that replacement-client phase.
                me.on_transport_reconnected(client, ctx);
            }
            RemoteServerManagerEvent::SessionConnectionFailed {
                session_id,
                phase,
                error,
            } if *session_id == me.connection_session_id => {
                me.on_connect_failed(&format!("{phase:?}"), error, ctx);
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
                me.write_warning(&crate::t!(
                    "terminal-daemon-multiplexer-nested",
                    detail = detail.clone()
                ));
            }
            // The transport went away for good: a spontaneous drop with no
            // reconnect possible, or reconnect attempts exhausted (§9). A mere
            // blip never reaches here — it arrives as `SessionReconnected` and is
            // handled above. Nothing will bring this view back on its own, so
            // surface it instead of freezing the grid on its last frame and
            // silently swallowing everything the user types.
            RemoteServerManagerEvent::SessionDisconnected { session_id, .. }
            | RemoteServerManagerEvent::SessionDeregistered { session_id }
                if *session_id == me.connection_session_id =>
            {
                me.on_transport_lost(ctx);
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
        if self.terminated {
            return;
        }
        let Some(client) = self.client(ctx) else {
            self.set_input_phase(RemoteInputPhase::Transport, ctx);
            return;
        };
        if !self.validate_expected_host(ctx) {
            return;
        }
        self.set_input_phase(RemoteInputPhase::Attach, ctx);
        if self.pending_open.is_some() {
            self.try_open(ctx);
            return;
        }
        self.on_attached_transport_ready(&client, ctx);
    }

    fn on_transport_reconnected(
        &mut self,
        event_client: &Arc<RemoteServerClient>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.terminated {
            return;
        }
        if !self.validate_expected_host(ctx) {
            return;
        }
        let Some(current_client) = self.client(ctx) else {
            return;
        };
        if !Arc::ptr_eq(event_client, &current_client) {
            log::error!(
                "daemon_tty: ignored reconnect event from a stale connection route for {:?}",
                self.connection_session_id
            );
            return;
        }
        self.on_attached_transport_ready(&current_client, ctx);
    }

    fn validate_expected_host(&mut self, ctx: &mut ModelContext<Self>) -> bool {
        let Some(expected_host_id) = self.expected_host_id.as_deref() else {
            return true;
        };
        let matches_expected_host = RemoteServerManager::handle(ctx).read(ctx, |manager, _| {
            manager
                .host_id_for_session(self.connection_session_id)
                .is_some_and(|host_id| host_id.as_str() == expected_host_id)
        });
        if matches_expected_host {
            return true;
        }
        self.write_notice(&crate::t!("terminal-daemon-restore-host-identity-mismatch"));
        self.abandon_failed_attach(ctx);
        false
    }

    fn on_attached_transport_ready(
        &mut self,
        client: &Arc<RemoteServerClient>,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(pty_session_id) = self.pty_session_id.clone() else {
            return;
        };
        if !self
            .attach_phase
            .begin(client, &pty_session_id, self.pty_generation)
        {
            return;
        }
        // The old transport cannot complete its requests anymore. Keep
        // retry-safe control state, close ordinary input, and attach once on
        // the replacement client before accepting more user bytes.
        self.prepare_transport_reconnect();
        self.set_input_phase(RemoteInputPhase::Attach, ctx);
        self.reattach(ctx);
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
            expected_host_id: None,
            pty_session_id: None,
            pty_generation: None,
            expected_attach_agent_binding: None,
            awaiting_attach_snapshot: false,
            initial_attach_pending: false,
            pending_attach_replay: None,
            reattach_after_replay: false,
            attach_in_flight: None,
            next_attach_attempt: 0,
            attach_phase: AttachPhaseGuard::default(),
            user_input_ready: false,
            input_phase: RemoteInputPhase::Transport,
            terminal_view_id: None,
            terminal_view: None,
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
            managed_launch_id: None,
            managed_open_identity: None,
            awaiting_managed_agent_binding: false,
            startup_command: None,
            startup_command_id: None,
            startup_command_in_flight: None,
            next_startup_command_attempt: 0,
            startup_retry_requires_reconnect: false,
            startup_capability_notice_shown: false,
            last_seq: 0,
            host_label: String::new(),
            welcomed: false,
            first_ready_notice_kind: ReadyNoticeKind::Attached,
            pending_ready_notice: None,
            replay_truncated: false,
            #[cfg(test)]
            published_ready_notices: Vec::new(),
            terminated: false,
            report_bootstrap_boundary: false,
        }
    }

    /// Readiness seam for the owning TerminalView's visible input gate. The
    /// EventLoop itself can reject unsafe writes, but only the view can retain
    /// and present an editable draft instead of a seemingly active cursor.
    pub(super) fn is_user_input_ready(&self) -> bool {
        self.user_input_ready && !self.awaiting_attach_snapshot && !self.terminated
    }

    pub(crate) fn input_phase(&self) -> RemoteInputPhase {
        self.input_phase
    }

    fn set_input_phase(&mut self, phase: RemoteInputPhase, ctx: &mut ModelContext<Self>) {
        if self.input_phase == phase {
            return;
        }
        self.input_phase = phase;
        self.user_input_ready = phase == RemoteInputPhase::Ready;
        if let Some(terminal_view) = self
            .terminal_view
            .as_ref()
            .and_then(|terminal_view| terminal_view.upgrade(ctx))
        {
            let connection_session_id = self.connection_session_id;
            terminal_view.update(ctx, |view, ctx| {
                view.set_remote_input_phase(phase, Some(connection_session_id), ctx);
            });
        }
        ctx.notify();
    }

    /// Starts mirroring this terminal's CLI-agent lifecycle to the daemon PTY.
    pub(super) fn bind_terminal_view(
        &mut self,
        terminal_view: &ViewHandle<TerminalView>,
        ctx: &mut ModelContext<Self>,
    ) {
        let terminal_view_id = terminal_view.id();
        self.terminal_view_id = Some(terminal_view_id);
        self.terminal_view = Some(terminal_view.downgrade());
        let input_phase = self.input_phase;
        let connection_session_id = self.connection_session_id;
        terminal_view.update(ctx, |view, ctx| {
            view.set_remote_input_phase(input_phase, Some(connection_session_id), ctx);
        });
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
                            sessions.bind_account_identity_with_id(
                                terminal_view_id,
                                agent,
                                (!identity.config_dir.is_empty())
                                    .then(|| identity.config_dir.clone()),
                                (!identity.account_email.is_empty())
                                    .then(|| identity.account_email.clone()),
                                (!identity.account_id.is_empty())
                                    .then(|| identity.account_id.clone()),
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
                unsupported @ (CLIAgent::Grok
                | CLIAgent::Antigravity
                | CLIAgent::Gemini
                | CLIAgent::Amp
                | CLIAgent::Droid
                | CLIAgent::OpenCode
                | CLIAgent::Copilot
                | CLIAgent::Pi
                | CLIAgent::Auggie
                | CLIAgent::CursorCli
                | CLIAgent::Goose
                | CLIAgent::DeepSeek
                | CLIAgent::Unknown) => {
                    log::debug!(
                        "daemon_tty: skipping PTY binding for unsupported live-verification \
                         provider {unsupported:?}"
                    );
                    return None;
                }
            };
            let account = sessions.account_identity(terminal_view_id)?;
            if account.agent() != session.agent {
                return None;
            }
            Some(AgentSessionIdentity {
                session_id: session.session_context.session_id.clone()?,
                provider: provider.to_string(),
                account_email: account.account_email.clone().unwrap_or_default(),
                config_dir: account
                    .account_id
                    .is_none()
                    .then(|| account.config_dir.clone())
                    .flatten()
                    .unwrap_or_default(),
                account_id: account.account_id.clone().unwrap_or_default(),
            })
        });
        if !self.awaiting_attach_snapshot || self.awaiting_managed_agent_binding {
            self.drive_agent_binding(ctx);
        }
    }

    fn agent_binding_client(
        &self,
        ctx: &mut ModelContext<Self>,
    ) -> Option<(Arc<RemoteServerClient>, bool, String)> {
        let session_id = self.connection_session_id;
        let manager = RemoteServerManager::handle(ctx);
        manager.read(ctx, |manager, _ctx| {
            manager
                .client_for_session(session_id)
                .cloned()
                .and_then(|client| {
                    manager.host_id_for_session(session_id).map(|host_id| {
                        let supported = manager
                            .session_supports_feature(session_id, FEATURE_AGENT_PTY_BINDING_V2);
                        (client, supported, host_id.as_str().to_string())
                    })
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
        let Some((client, supported, host_id)) = self.agent_binding_client(ctx) else {
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
                        .unbind_agent_pty(host_id, current, pty_session_id, pty_generation)
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
                        .bind_agent_pty(host_id, desired, pty_session_id, pty_generation, current)
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
                        if me.awaiting_managed_agent_binding {
                            me.expected_attach_agent_binding = Some(sent.clone());
                            me.awaiting_managed_agent_binding = false;
                            me.reattach(ctx);
                            return;
                        }
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
        if self.terminated {
            return;
        }
        if self.pending_attach_replay.is_some() {
            self.reattach_after_replay = true;
            return;
        }
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
            .is_some_and(|(_, supported, _)| supported);
        if !Self::attach_generation_is_valid(expected_generation, supports_agent_binding) {
            self.write_notice(&crate::t!("terminal-daemon-attach-generation-invalid"));
            self.abandon_failed_attach(ctx);
            return;
        }
        if self.expected_attach_agent_binding.is_some() && !supports_agent_binding {
            self.write_notice(&crate::t!(
                "terminal-daemon-attach-agent-routing-unsupported"
            ));
            self.abandon_failed_attach(ctx);
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
                    me.write_notice(&crate::t!(
                        "terminal-daemon-attach-failed",
                        detail = err.to_string()
                    ));
                    me.abandon_failed_attach(ctx);
                    if let Some(exit_code) = me.pending_exit.take() {
                        me.awaiting_attach_snapshot = false;
                        me.on_session_exited(exit_code, ctx);
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
        if self.pty_session_id.as_deref() != Some(attached.session_id.as_str()) {
            log::error!(
                "daemon_tty: rejected attach response for PTY {} (expected {:?})",
                attached.session_id,
                self.pty_session_id
            );
            self.write_notice(&crate::t!("terminal-daemon-attach-pty-identity-mismatch"));
            self.abandon_failed_attach(ctx);
            return;
        }
        if self
            .pty_generation
            .is_some_and(|expected| attached.generation != expected)
        {
            log::error!(
                "daemon_tty: rejected attach response with generation {} (expected {:?})",
                attached.generation,
                self.pty_generation
            );
            self.write_notice(&crate::t!("terminal-daemon-attach-generation-mismatch"));
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
        self.awaiting_managed_agent_binding = false;
        self.set_input_phase(RemoteInputPhase::Replay, ctx);
        let bootstrap_preamble = if self.is_bootstrapped() {
            Vec::new()
        } else {
            attached.bootstrap_preamble
        };
        let fed_preamble = !bootstrap_preamble.is_empty();
        if fed_preamble {
            self.terminal_model.lock().suppress_next_bootstrap_write();
        }
        self.pending_attach_replay = Some(PendingAttachReplay {
            bootstrap_preamble,
            preamble_offset: 0,
            base_seq: attached.base_seq,
            replay: attached.replay,
            replay_offset: 0,
            gap_applied: false,
            fed_preamble,
            pending_exit,
        });
        self.process_attach_replay_chunk(ctx);
    }

    fn process_attach_replay_chunk(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(mut pending) = self.pending_attach_replay.take() else {
            return;
        };

        if pending.preamble_offset < pending.bootstrap_preamble.len() {
            let end = (pending.preamble_offset + ATTACH_PARSE_CHUNK_BYTES)
                .min(pending.bootstrap_preamble.len());
            self.process_historical_pty_bytes(
                &pending.bootstrap_preamble[pending.preamble_offset..end],
            );
            pending.preamble_offset = end;
            self.last_seq = end as u64;
            self.pending_attach_replay = Some(pending);
            self.schedule_attach_replay_chunk(ctx);
            return;
        }

        if !pending.gap_applied {
            if pending.fed_preamble {
                self.terminal_model
                    .lock()
                    .take_suppress_next_bootstrap_write();
            }
            if pending.base_seq > self.last_seq {
                if pending.fed_preamble {
                    self.reset_parser();
                }
                self.process_historical_pty_bytes(b"\x1b[H\x1b[2J\x1b[3J");
                self.write_notice(&crate::t!("terminal-daemon-scrollback-truncated"));
                self.replay_truncated = true;
            }
            pending.gap_applied = true;
            pending.fed_preamble = false;
        }

        if pending.replay_offset < pending.replay.len() {
            let end = (pending.replay_offset + ATTACH_PARSE_CHUNK_BYTES).min(pending.replay.len());
            self.process_historical_pty_bytes(&pending.replay[pending.replay_offset..end]);
            pending.replay_offset = end;
            self.last_seq = pending.base_seq + end as u64;
            self.pending_attach_replay = Some(pending);
            self.schedule_attach_replay_chunk(ctx);
            return;
        }

        self.last_seq = pending.base_seq + pending.replay.len() as u64;
        self.finish_attach_replay(pending.pending_exit, ctx);
    }

    fn schedule_attach_replay_chunk(&mut self, ctx: &mut ModelContext<Self>) {
        ctx.spawn(
            async move { futures_lite::future::yield_now().await },
            |me, _, ctx| me.process_attach_replay_chunk(ctx),
        );
    }

    fn finish_attach_replay(
        &mut self,
        pending_exit: Option<Option<i32>>,
        ctx: &mut ModelContext<Self>,
    ) {
        let replay_again = self.drain_pending_output(ctx);
        if let Some(exit_code) = self.pending_exit.take().or(pending_exit) {
            self.awaiting_attach_snapshot = false;
            self.reattach_after_replay = false;
            if replay_again {
                self.write_warning(&crate::t!("terminal-daemon-final-output-truncated"));
            }
            self.on_session_exited(exit_code, ctx);
            return;
        }
        if self.terminated {
            self.awaiting_attach_snapshot = false;
            self.reattach_after_replay = false;
            return;
        }
        let reattach_after_replay = std::mem::take(&mut self.reattach_after_replay);
        if replay_again || reattach_after_replay {
            self.awaiting_attach_snapshot = true;
            self.reattach(ctx);
            return;
        }
        self.awaiting_attach_snapshot = false;
        let notice_kind = if self.welcomed {
            ReadyNoticeKind::Attached
        } else {
            self.first_ready_notice_kind
        };
        self.stage_ready_notice(notice_kind);
        self.complete_initial_attach_if_ready(ctx);
        // If bootstrap only completed now — a fresh open that dropped
        // mid-handshake and finished it from this reconnect's replay — the
        // live-output path never saw the flip, so report the boundary here
        // too (a no-op for adopted sessions and once already reported).
        self.maybe_report_bootstrap_boundary(ctx);
        self.maybe_dispatch_startup_command(ctx);
        // Transport is back and we're re-attached. This buffer contains only
        // protocol/control traffic; ordinary user bytes are never replayed.
        self.flush_pending_input(ctx);
        self.drive_agent_binding(ctx);
    }

    fn complete_initial_attach_if_ready(&mut self, ctx: &mut ModelContext<Self>) {
        if self.awaiting_attach_snapshot {
            return;
        }
        let model = self.terminal_model.lock();
        // InitShell already enables raw input to interactive rc-file prompts.
        // Such a prompt may legitimately postpone Bootstrapped indefinitely.
        let ready = model.block_list().is_bootstrapped() || model.pending_session_id().is_some();
        drop(model);
        if ready {
            self.initial_attach_pending = false;
            self.set_input_phase(RemoteInputPhase::Ready, ctx);
            if let Some((pty_session_id, generation)) = self.managed_open_identity.take() {
                self.report_managed_launch_opened(&pty_session_id, generation, ctx);
            }
            self.publish_ready_notice(ctx);
        }
    }

    fn stage_ready_notice(&mut self, kind: ReadyNoticeKind) {
        let Some(pty_session_id) = self.pty_session_id.clone() else {
            return;
        };
        self.pending_ready_notice = Some(PendingReadyNotice {
            connection_session_id: self.connection_session_id,
            pty_session_id,
            generation: self.pty_generation,
            kind,
        });
    }

    fn publish_ready_notice(&mut self, ctx: &mut ModelContext<Self>) {
        let Some(pending) = self.pending_ready_notice.take() else {
            return;
        };
        if pending.connection_session_id != self.connection_session_id
            || self.pty_session_id.as_deref() != Some(pending.pty_session_id.as_str())
            || self.pty_generation != pending.generation
        {
            return;
        }
        let host = self.host_label.clone();
        let message = match pending.kind {
            ReadyNoticeKind::PersistentSessionActive => {
                crate::t!("terminal-daemon-persistent-session-active", host = host)
            }
            ReadyNoticeKind::Attached if self.replay_truncated => {
                crate::t!("terminal-daemon-restored-truncated", host = host)
            }
            ReadyNoticeKind::Attached if self.welcomed => {
                crate::t!("terminal-daemon-reconnected", host = host)
            }
            ReadyNoticeKind::Attached => crate::t!("terminal-daemon-reattached", host = host),
        };
        self.welcomed = true;
        self.replay_truncated = false;
        self.show_session_notice(message, ctx);
    }

    /// Hands a connection/restore status to the terminal view, which shows it
    /// outside the terminal grid. Session output stays untouched (#470).
    fn show_session_notice(&mut self, message: String, ctx: &mut ModelContext<Self>) {
        #[cfg(test)]
        self.published_ready_notices.push(message.clone());
        let Some(terminal_view) = self
            .terminal_view
            .as_ref()
            .and_then(|terminal_view| terminal_view.upgrade(ctx))
        else {
            return;
        };
        let connection_session_id = self.connection_session_id;
        terminal_view.update(ctx, |view, ctx| {
            view.show_remote_session_notice(message, Some(connection_session_id), ctx);
        });
    }

    fn arm_initial_attach_timeout(&mut self, ctx: &mut ModelContext<Self>) {
        if self.initial_attach_pending {
            return;
        }
        self.initial_attach_pending = true;
        ctx.spawn(
            async {
                warpui::r#async::Timer::after(INITIAL_ATTACH_TIMEOUT).await;
            },
            |me, (), ctx| me.on_initial_attach_timeout(ctx),
        );
    }

    fn on_initial_attach_timeout(&mut self, ctx: &mut ModelContext<Self>) {
        if self.terminated || !self.initial_attach_pending {
            return;
        }
        self.write_notice(&crate::t!(
            "terminal-daemon-initial-attach-timeout",
            seconds = INITIAL_ATTACH_TIMEOUT.as_secs()
        ));
        self.abandon_failed_attach(ctx);
    }

    /// Finish a failed initial shell so its hidden bootstrap output becomes visible.
    /// An already bootstrapped session keeps its existing disconnect/reconnect UI.
    fn finish_failed_startup(&mut self, ctx: &mut ModelContext<Self>) {
        self.report_managed_launch_failed(
            crate::t!("terminal-daemon-managed-launch-failed").to_string(),
            ctx,
        );
        self.terminated = true;
        self.initial_attach_pending = false;
        self.pending_open = None;
        self.pending_input.clear();
        self.pending_output.clear();
        self.pending_attach_replay = None;
        self.pending_ready_notice = None;
        self.attach_in_flight = None;
        self.awaiting_managed_agent_binding = false;
        self.managed_open_identity = None;
        self.set_input_phase(RemoteInputPhase::Failed, ctx);
        let mut model = self.terminal_model.lock();
        if !model.block_list().is_bootstrapped() {
            model.exit(ExitReason::PtyDisconnected);
        }
    }

    fn abandon_failed_attach(&mut self, ctx: &mut ModelContext<Self>) {
        self.finish_failed_startup(ctx);
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

    fn open_client(&self, ctx: &mut ModelContext<Self>) -> Option<OpenSessionClient> {
        let session_id = self.connection_session_id;
        let manager = RemoteServerManager::handle(ctx);
        manager.read(ctx, |manager, _ctx| {
            manager
                .client_for_session(session_id)
                .cloned()
                .map(|client| {
                    let supports_account_routing = manager
                        .session_supports_feature(session_id, FEATURE_AGENT_ACCOUNT_ROUTING_V1);
                    let supports_logical_open_id =
                        manager.session_supports_feature(session_id, FEATURE_LOGICAL_OPEN_ID_V1);
                    let supports_logical_open_attempt = manager
                        .session_supports_feature(session_id, FEATURE_LOGICAL_OPEN_ATTEMPT_V1);
                    let supports_managed_fleet = manager
                        .session_supports_feature(session_id, FEATURE_MANAGED_AGENT_FLEET_V1)
                        && manager
                            .session_supports_feature(session_id, FEATURE_MANAGED_OPEN_ATTACH_V1)
                        && supports_logical_open_id
                        && supports_logical_open_attempt
                        && manager
                            .session_supports_feature(session_id, FEATURE_AGENT_PTY_BINDING_V2);
                    OpenSessionClient {
                        client,
                        supports_account_routing,
                        supports_managed_fleet,
                        supports_logical_open_id,
                        supports_logical_open_attempt,
                    }
                })
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
        let Some(open_client) = self.open_client(ctx) else {
            return; // Not connected yet; wait for `SessionConnected`.
        };
        if !(open_client.supports_logical_open_id && open_client.supports_logical_open_attempt)
            && self
                .pending_open
                .as_ref()
                .is_some_and(|pending| pending.next_attempt > 0)
        {
            self.write_notice(&crate::t!(
                "terminal-daemon-open-ack-unknown-upgrade-required"
            ));
            self.report_managed_launch_failed(
                crate::t!("terminal-daemon-managed-open-unconfirmed").to_string(),
                ctx,
            );
            self.finish_failed_startup(ctx);
            return;
        }
        let Some((logical_open_id, open_params, size_info, attempt)) = self
            .pending_open
            .as_mut()
            .expect("pending_open is Some (checked above)")
            .begin_attempt()
        else {
            return;
        };
        self.open_session(
            open_client,
            logical_open_id,
            attempt,
            open_params,
            size_info,
            ctx,
        );
    }

    /// Issues the `OpenSession` request over a connected client. The initial
    /// size is taken from the terminal model so the daemon-side PTY matches
    /// what the user sees.
    fn open_session(
        &mut self,
        open_client: OpenSessionClient,
        logical_open_id: String,
        attempt: u64,
        open_params: OpenSessionParams,
        size_info: SizeInfo,
        ctx: &mut ModelContext<Self>,
    ) {
        let OpenSessionClient {
            client,
            supports_account_routing,
            supports_managed_fleet,
            supports_logical_open_id,
            supports_logical_open_attempt,
        } = open_client;
        let cwd = open_params.cwd;
        let shell = open_params.shell;
        let env = open_params.env;
        let ring_ceiling_bytes = open_params.ring_ceiling_bytes;
        let agent_launch_route = open_params.agent_launch_route;
        let managed_launch = open_params.managed_launch;
        let requested_min_available_bytes = open_params.requested_min_available_bytes;
        if !account_route_is_compatible(agent_launch_route.as_ref(), supports_account_routing) {
            self.write_notice(&crate::t!(
                "terminal-daemon-managed-account-route-unsupported"
            ));
            self.pending_open = None;
            self.report_managed_launch_failed(
                crate::t!("terminal-daemon-managed-account-route-unsupported").to_string(),
                ctx,
            );
            self.finish_failed_startup(ctx);
            return;
        }
        if managed_launch.is_some() && !supports_managed_fleet {
            self.write_notice(&crate::t!("terminal-daemon-managed-host-unsupported"));
            self.pending_open = None;
            self.report_managed_launch_failed(
                crate::t!("terminal-daemon-managed-host-unsupported").to_string(),
                ctx,
            );
            self.finish_failed_startup(ctx);
            return;
        }
        if managed_launch.is_some()
            && (agent_launch_route.is_none()
                || cwd.as_deref().is_none_or(|path| path.trim().is_empty()))
        {
            self.write_notice(&crate::t!("terminal-daemon-managed-route-incomplete"));
            self.pending_open = None;
            self.report_managed_launch_failed(
                crate::t!("terminal-daemon-managed-route-incomplete").to_string(),
                ctx,
            );
            self.finish_failed_startup(ctx);
            return;
        }
        let rows = size_info.rows as u32;
        let cols = size_info.columns as u32;
        log::info!("daemon_tty: issuing OpenSession (cwd={cwd:?}, shell={shell:?}, {rows}x{cols}, ring_ceiling={ring_ceiling_bytes:?})");
        let callback_logical_open_id = logical_open_id.clone();
        let future = async move {
            let logical_open_attempt = if supports_logical_open_attempt {
                attempt
            } else {
                0
            };
            match (agent_launch_route, managed_launch) {
                (Some(route), Some(launch)) => {
                    client
                        .open_managed_agent_session(
                            logical_open_id,
                            logical_open_attempt,
                            cwd.expect("managed cwd was validated above"),
                            shell,
                            env,
                            rows,
                            cols,
                            ring_ceiling_bytes,
                            route,
                            launch,
                            requested_min_available_bytes,
                        )
                        .await
                }
                (Some(route), None) => {
                    client
                        .open_session_for_agent_account(
                            logical_open_id,
                            logical_open_attempt,
                            cwd,
                            shell,
                            env,
                            rows,
                            cols,
                            ring_ceiling_bytes,
                            route,
                        )
                        .await
                }
                (None, None) => {
                    client
                        .open_session(
                            logical_open_id,
                            logical_open_attempt,
                            cwd,
                            shell,
                            env,
                            rows,
                            cols,
                            ring_ceiling_bytes,
                        )
                        .await
                }
                (None, Some(_)) => unreachable!("managed route was validated above"),
            }
        };
        ctx.spawn(future, move |me, result, ctx| {
            let Some(pending) = me.pending_open.as_mut() else {
                return;
            };
            if !pending.finish_attempt(&callback_logical_open_id, attempt) {
                return;
            }
            match result {
                Ok(opened) => {
                    let authoritative_attach_required =
                        open_ack_requires_authoritative_attach(attempt, &opened);
                    me.pending_open = None;
                    me.on_session_opened(
                        opened.session_id,
                        opened.generation,
                        authoritative_attach_required,
                        opened.expected_agent_binding,
                        ctx,
                    );
                }
                Err(ClientError::Disconnected | ClientError::ResponseChannelClosed) => {
                    if pending.can_retry_ambiguous_open(
                        supports_logical_open_id && supports_logical_open_attempt,
                    ) {
                        log::warn!(
                            "daemon_tty: OpenSession transport dropped; retaining logical open {} for reconnect",
                            callback_logical_open_id
                        );
                        me.set_input_phase(RemoteInputPhase::Attach, ctx);
                    } else {
                        me.write_notice(&crate::t!(
                            "terminal-daemon-open-ack-unknown-upgrade-required"
                        ));
                        me.report_managed_launch_failed(
                            crate::t!("terminal-daemon-managed-open-unconfirmed").to_string(),
                            ctx,
                        );
                        me.finish_failed_startup(ctx);
                    }
                }
                Err(ClientError::Timeout(timeout)) => {
                    if pending.can_retry_ambiguous_open(
                        supports_logical_open_id && supports_logical_open_attempt,
                    ) {
                        log::warn!(
                            "daemon_tty: OpenSession acknowledgement timed out after {timeout:?}; retrying logical open {} on the live transport",
                            callback_logical_open_id
                        );
                        me.try_open(ctx);
                    } else {
                        log::warn!(
                            "daemon_tty: OpenSession acknowledgement remained ambiguous for logical open {}",
                            callback_logical_open_id
                        );
                        me.write_notice(&crate::t!(
                            "terminal-daemon-open-ack-unknown-upgrade-required"
                        ));
                        me.report_managed_launch_failed(
                            crate::t!("terminal-daemon-managed-open-unconfirmed").to_string(),
                            ctx,
                        );
                        me.finish_failed_startup(ctx);
                    }
                }
                Err(err) => {
                    // The transport is up (so the connect-failure path never fired),
                    // but the daemon refused to open the session (bad cwd, unspawnable
                    // shell, fd exhaustion, …). Surface it instead of leaving a blank,
                    // hung tab; drop the pending open so a later event can't reopen it.
                    log::error!("daemon_tty: OpenSession failed: {err:?}");
                    me.write_notice(&crate::t!(
                        "terminal-daemon-open-failed",
                        detail = err.to_string()
                    ));
                    me.report_managed_launch_failed(
                        crate::t!("terminal-daemon-managed-open-rejected").to_string(),
                        ctx,
                    );
                    me.finish_failed_startup(ctx);
                }
            }
        });
    }

    fn report_managed_launch_failed(&mut self, error: String, ctx: &mut ModelContext<Self>) {
        let Some(launch_id) = self.managed_launch_id.take() else {
            return;
        };
        RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.report_managed_launch_failed(launch_id, error, ctx);
        });
    }

    pub(super) fn relinquish_managed_launch(&mut self, launch_id: &str) -> bool {
        if self.managed_launch_id.as_deref() != Some(launch_id) {
            return false;
        }
        self.managed_launch_id = None;
        self.managed_open_identity = None;
        true
    }

    fn report_managed_launch_opened(
        &mut self,
        pty_session_id: &str,
        generation: u64,
        ctx: &mut ModelContext<Self>,
    ) {
        let Some(launch_id) = self.managed_launch_id.take() else {
            return;
        };
        RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.report_managed_launch_opened(
                launch_id,
                pty_session_id.to_string(),
                generation,
                ctx,
            );
        });
    }

    fn on_connect_failed(&mut self, phase: &str, error: &str, ctx: &mut ModelContext<Self>) {
        log::error!(
            "daemon connect failed for {:?} at {phase}: {error}",
            self.connection_session_id
        );
        // Surface the failure in the tab so the user sees *why* instead of a
        // blank/hung view (the connection never produced any PTY output).
        self.write_notice(&crate::t!(
            "terminal-daemon-connection-failed",
            phase = phase,
            detail = error
        ));
        // Drop the pending open so a later spurious event can't reopen it.
        self.report_managed_launch_failed(
            crate::t!("terminal-daemon-managed-connection-failed").to_string(),
            ctx,
        );
        self.finish_failed_startup(ctx);
    }

    fn on_session_opened(
        &mut self,
        pty_session_id: String,
        generation: u64,
        authoritative_attach_required: bool,
        expected_agent_binding: Option<AgentSessionIdentity>,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.terminated {
            return;
        }
        let claim = self.claim_opened_daemon_session(&pty_session_id, generation, ctx);
        self.on_session_opened_with_claim(
            pty_session_id,
            generation,
            authoritative_attach_required,
            expected_agent_binding,
            claim,
            ctx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn on_session_opened_with_claim(
        &mut self,
        pty_session_id: String,
        generation: u64,
        authoritative_attach_required: bool,
        expected_agent_binding: Option<AgentSessionIdentity>,
        claim: OpenedDaemonClaim,
        ctx: &mut ModelContext<Self>,
    ) {
        if claim == OpenedDaemonClaim::Unavailable {
            log::error!(
                "daemon_tty: refusing opened PTY {pty_session_id} because ownership could not be claimed"
            );
            let error = crate::t!("terminal-daemon-managed-claim-unavailable").to_string();
            self.write_notice(&error);
            self.report_managed_launch_failed(error, ctx);
            self.abandon_failed_attach(ctx);
            return;
        }
        let claim_conflicted = claim == OpenedDaemonClaim::Conflict;
        log::info!("daemon_tty: session opened, pty_session_id={pty_session_id}");
        self.pty_session_id = Some(pty_session_id.clone());
        self.pty_generation = (generation != 0).then_some(generation);
        RemoteServerManager::handle(ctx).update(ctx, |manager, ctx| {
            manager.report_session_opened(
                self.connection_session_id,
                self.terminal_view_id,
                pty_session_id.clone(),
                generation,
                ctx,
            );
        });
        if claim_conflicted {
            // The SessionOpened event above lets the workspace discard this
            // provisional surface and focus the existing owner. Do not attach:
            // an attach from the losing connection could otherwise race the
            // deferred workspace event and steal the daemon-side PTY owner.
            self.report_managed_launch_failed(
                crate::t!("terminal-daemon-managed-claim-conflict").to_string(),
                ctx,
            );
            self.finish_failed_startup(ctx);
            return;
        }
        if self.managed_launch_id.is_some() {
            if generation == 0 {
                self.report_managed_launch_failed(
                    crate::t!("terminal-daemon-managed-generation-invalid").to_string(),
                    ctx,
                );
                self.finish_failed_startup(ctx);
                return;
            }
            self.managed_open_identity = Some((pty_session_id.clone(), generation));
        }
        if let (Some(terminal_view_id), Some(generation)) =
            (self.terminal_view_id, self.pty_generation)
        {
            let host_id = RemoteServerManager::handle(ctx).read(ctx, |manager, _ctx| {
                manager
                    .host_id_for_session(self.connection_session_id)
                    .map(|host_id| host_id.as_str().to_string())
            });
            if let Some(host_id) = host_id {
                crate::cockpit::launch_registry::attach_remote_terminal(
                    terminal_view_id,
                    &host_id,
                    &pty_session_id,
                    generation,
                );
            }
        }
        self.first_ready_notice_kind = ReadyNoticeKind::PersistentSessionActive;
        let managed_attach_required = self.managed_launch_id.is_some();
        if authoritative_attach_required || managed_attach_required {
            self.awaiting_attach_snapshot = true;
            self.expected_attach_agent_binding = expected_agent_binding;
        }
        let replay_required = self.drain_pending_output(ctx);
        if replay_required || authoritative_attach_required || managed_attach_required {
            self.awaiting_attach_snapshot = true;
            if managed_attach_required && self.expected_attach_agent_binding.is_none() {
                // A lost first Ack can leave the new PTY attached and already
                // streaming before the retry response arrives. The buffered
                // lifecycle bytes reveal the exact provider session; bind that
                // identity authoritatively before issuing the mandatory attach.
                self.awaiting_managed_agent_binding = true;
                self.drive_agent_binding(ctx);
                return;
            }
            self.reattach(ctx);
            return;
        }
        self.awaiting_attach_snapshot = false;
        self.stage_ready_notice(ReadyNoticeKind::PersistentSessionActive);
        // The pre-OpenSession output burst may already have supplied InitShell
        // or Bootstrapped. Evaluate that real model evidence now; the Ack and
        // replay bytes alone are not readiness.
        self.complete_initial_attach_if_ready(ctx);
        self.drive_agent_binding(ctx);
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
        // Flush protocol/control traffic that arrived before the session was
        // addressable. Ordinary user bytes were rejected and cannot execute
        // later as a side effect of this acknowledgement.
        self.flush_pending_input(ctx);
    }

    fn claim_opened_daemon_session(
        &self,
        pty_session_id: &str,
        generation: u64,
        ctx: &mut ModelContext<Self>,
    ) -> OpenedDaemonClaim {
        let Some(terminal_view) = self
            .terminal_view
            .as_ref()
            .and_then(|terminal_view| terminal_view.upgrade(ctx))
        else {
            return OpenedDaemonClaim::Unavailable;
        };
        let descriptor = RemoteServerManager::handle(ctx).read(ctx, |manager, _| {
            manager.connected_session_descriptor(self.connection_session_id)
        });
        let Some(descriptor) = descriptor else {
            return OpenedDaemonClaim::Unavailable;
        };
        let identity = crate::app_state::DaemonPtyIdentity {
            daemon_host_id: descriptor.host_id.as_str().to_string(),
            runtime_filename: descriptor.daemon_runtime.runtime_filename().to_string(),
            server_version: descriptor.daemon_runtime.server_version().to_string(),
            pty_session_id: pty_session_id.to_string(),
            pty_generation: generation,
        };
        let owner = crate::app_state::DaemonPtyClaimOwner {
            terminal_view_id: Some(terminal_view.id()),
            connection_session_id: self.connection_session_id,
            terminal_view: Some(terminal_view.downgrade()),
        };
        match crate::app_state::claim_daemon_pty(identity, owner.clone(), ctx) {
            crate::app_state::DaemonPtyClaimOutcome::Claimed => OpenedDaemonClaim::Owned,
            crate::app_state::DaemonPtyClaimOutcome::Existing(existing)
                if existing.owns_same_terminal_surface(&owner) =>
            {
                OpenedDaemonClaim::Owned
            }
            crate::app_state::DaemonPtyClaimOutcome::Existing(_) => OpenedDaemonClaim::Conflict,
        }
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
    fn drain_pending_output(&mut self, ctx: &mut ModelContext<Self>) -> bool {
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
            self.process_live_pty_bytes(&bytes[offset..], ctx);
            self.last_seq = end_seq;
        }
        replay_required
    }

    /// Flush buffered protocol/control traffic once the exact PTY is attached.
    /// Ordinary user bytes never enter this queue.
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
        if self.terminated {
            return;
        }
        match message {
            EventLoopMessage::Input(bytes) => self.dispatch_user_input(bytes, ctx),
            EventLoopMessage::Resize(size) => {
                self.dispatch_or_buffer_control(EventLoopMessage::Resize(size), ctx)
            }
            EventLoopMessage::Shutdown => {
                self.dispatch_or_buffer_control(EventLoopMessage::Shutdown, ctx)
            }
            EventLoopMessage::ChildExited => {
                self.dispatch_or_buffer_control(EventLoopMessage::ChildExited, ctx)
            }
        }
    }

    /// Buffer internal protocol input or control traffic while the exact session
    /// is not addressable. Resizes coalesce; protocol replies remain bounded.
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
        if total > MAX_PENDING_PROTOCOL_INPUT_BYTES {
            log::warn!(
                "daemon_tty: buffered protocol replies exceeded \
                 {MAX_PENDING_PROTOCOL_INPUT_BYTES} bytes during an outage — dropping oldest replies"
            );
            let mut i = 0;
            while total > MAX_PENDING_PROTOCOL_INPUT_BYTES && i < self.pending_input.len() {
                if let EventLoopMessage::Input(b) = &self.pending_input[i] {
                    total -= b.len();
                    self.pending_input.remove(i);
                } else {
                    i += 1;
                }
            }
        }
    }

    /// Delivers user bytes only while this exact PTY generation is attached and
    /// the shell has supplied real input-readiness evidence. This helper is the
    /// deterministic daemon-side seam; the TerminalView still needs to project
    /// the same state into a visibly disabled editor while preserving its draft.
    fn try_deliver_user_input_with<E>(
        &self,
        bytes: Cow<'static, [u8]>,
        dispatch: impl FnOnce(&str, Vec<u8>) -> Result<(), E>,
    ) -> Result<bool, E> {
        if !self.is_user_input_ready() {
            return Ok(false);
        }
        let Some(pty_session_id) = self.pty_session_id.as_deref() else {
            return Ok(false);
        };
        dispatch(pty_session_id, bytes.into_owned())?;
        Ok(true)
    }

    fn dispatch_user_input(&mut self, bytes: Cow<'static, [u8]>, ctx: &mut ModelContext<Self>) {
        let Some(client) = self.client(ctx) else {
            log::debug!("daemon_tty: rejected user input while the transport is unavailable");
            return;
        };
        match self.try_deliver_user_input_with(bytes, |pty_session_id, bytes| {
            client.send_session_input(pty_session_id.to_string(), bytes)
        }) {
            Ok(true) => {}
            Ok(false) => {
                log::debug!("daemon_tty: rejected user input before attach/replay/input readiness");
            }
            Err(err) => {
                log::error!("Failed to send user input to daemon session: {err:?}");
            }
        }
    }

    fn dispatch_or_buffer_control(
        &mut self,
        message: EventLoopMessage,
        ctx: &mut ModelContext<Self>,
    ) {
        if self.awaiting_attach_snapshot {
            self.buffer_pending(message);
            return;
        }
        match self.pty_session_id.clone() {
            Some(pty_session_id) => self.dispatch_message(&pty_session_id, message, ctx),
            None => self.buffer_pending(message),
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
            // Transport is down. Only protocol/control traffic reaches this
            // path; ordinary user input is rejected by `dispatch_user_input`.
            log::debug!("daemon_tty: buffering control message {message:?} for {pty_session_id}");
            self.buffer_pending(message);
            return;
        };
        let (result, retry_resize) = match message {
            EventLoopMessage::Input(bytes) => (
                client.send_session_input(pty_session_id.to_string(), bytes.into_owned()),
                None,
            ),
            EventLoopMessage::Resize(size_info) => (
                client.send_resize_session(
                    pty_session_id.to_string(),
                    size_info.rows as u32,
                    size_info.columns as u32,
                ),
                Some(size_info),
            ),
            // The daemon owns the PTY lifecycle; a client-side shutdown simply
            // detaches this view — the session keeps running for reattachment.
            EventLoopMessage::Shutdown | EventLoopMessage::ChildExited => {
                (client.send_detach_session(pty_session_id.to_string()), None)
            }
        };
        if let Err(err) = result {
            log::error!("Failed to send message to daemon session {pty_session_id}: {err:?}");
            if let Some(size_info) = retry_resize {
                self.buffer_pending(EventLoopMessage::Resize(size_info));
            }
        }
    }

    fn on_session_exited(&mut self, exit_code: Option<i32>, ctx: &mut ModelContext<Self>) {
        log::info!(
            "Daemon session {:?} exited (code {exit_code:?})",
            self.pty_session_id
        );
        // A clean exit is a terminal state: latch it so that if the transport
        // later drops (a `SessionDisconnected` reaching this still-open tab) we
        // don't append a contradictory "connection lost" line under this one.
        self.terminated = true;
        let notice = match exit_code {
            Some(code) => crate::t!("terminal-daemon-session-ended-with-code", code = code),
            None => crate::t!("terminal-daemon-session-ended"),
        };
        self.write_notice(&notice);
        self.finish_failed_startup(ctx);
    }

    /// A terminal transport loss with no auto-reconnect left (spontaneous drop
    /// or reconnect exhausted). Tell the user once instead of leaving a frozen
    /// grid that quietly eats keystrokes — and be honest about the payoff: the
    /// daemon owns the PTY, so a persistent session is very likely still running
    /// on the host and reopening it reattaches. No-op if a terminal state was
    /// already surfaced (a clean `SessionExited`, or a prior disconnect).
    fn on_transport_lost(&mut self, ctx: &mut ModelContext<Self>) {
        if self.terminated {
            return;
        }
        self.terminated = true;
        self.write_notice(&crate::t!(
            "terminal-daemon-connection-lost",
            host = self.host_label.clone()
        ));
        self.finish_failed_startup(ctx);
    }

    /// Writes a Zaplex notice line (e.g. a connection error or session-ended
    /// message) into the terminal via the normal ANSI path, so the user sees it
    /// in the tab rather than a blank/hung view. Rendered in bold red.
    fn write_notice(&mut self, text: &str) {
        let line = format!("\r\n\x1b[1;31m[zaplex] {text}\x1b[0m\r\n");
        self.process_historical_pty_bytes(line.as_bytes());
    }

    /// Neutral (non-error) status line, used for install/setup progress. Dim
    /// cyan instead of the red error styling of [`Self::write_notice`].
    fn write_progress(&mut self, text: &str) {
        let line = format!("\r\n\x1b[2;36m[zaplex] {text}\x1b[0m\r\n");
        self.process_historical_pty_bytes(line.as_bytes());
    }

    /// Advisory (non-fatal) warning line — yellow, between the dim-cyan
    /// progress and the red error notices.
    fn write_warning(&mut self, text: &str) {
        let line = format!("\r\n\x1b[1;33m[zaplex] {text}\x1b[0m\r\n");
        self.process_historical_pty_bytes(line.as_bytes());
    }

    /// Processes replayed or synthetic bytes without answering terminal
    /// queries. Replaying old output must never write a second reply to the PTY.
    fn process_historical_pty_bytes(&mut self, bytes: &[u8]) {
        let mut terminal_model = self.terminal_model.lock();
        self.parser
            .parse_bytes(&mut *terminal_model, bytes, &mut io::sink());
        self.channel_event_listener.send_wakeup_event();
    }

    /// Processes newly observed PTY bytes and routes ANSI query replies back to
    /// the daemon only after releasing the terminal-model guard.
    fn process_live_pty_bytes(&mut self, bytes: &[u8], ctx: &mut ModelContext<Self>) {
        let mut reply = TerminalReplyBuffer::default();
        {
            let mut terminal_model = self.terminal_model.lock();
            self.parser
                .parse_bytes(&mut *terminal_model, bytes, &mut reply);
        }
        self.channel_event_listener.send_wakeup_event();
        if reply.truncated {
            log::warn!(
                "daemon_tty: terminal replies exceeded {MAX_TERMINAL_REPLY_BYTES} bytes and were truncated"
            );
        }
        if !reply.bytes.is_empty() {
            self.dispatch_or_buffer_control(EventLoopMessage::Input(Cow::Owned(reply.bytes)), ctx);
        }
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
            self.process_historical_pty_bytes(bootstrap_preamble);
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
            self.process_historical_pty_bytes(b"\x1b[H\x1b[2J\x1b[3J");
            self.write_notice(&crate::t!("terminal-daemon-scrollback-truncated"));
            self.replay_truncated = true;
        }
        if !replay.is_empty() {
            self.process_historical_pty_bytes(replay);
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
                self.write_notice(&crate::t!(
                    "terminal-daemon-startup-helper-upgrade-required"
                ));
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
                    me.write_notice(&crate::t!("terminal-daemon-startup-command-rejected"));
                }
                Ok(ack) => {
                    me.startup_command_in_flight = None;
                    me.startup_retry_requires_reconnect = true;
                    log::error!(
                        "daemon_tty: mismatched startup Ack: session={}, command_id={}",
                        ack.session_id,
                        ack.startup_command_id
                    );
                    me.write_notice(&crate::t!("terminal-daemon-startup-ack-invalid"));
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
                    if matches!(err, ClientError::Timeout(_))
                        && attempt < MAX_STARTUP_COMMAND_DELIVERY_ATTEMPTS
                    {
                        me.maybe_dispatch_startup_command(ctx);
                    } else if matches!(err, ClientError::Timeout(_)) {
                        me.startup_retry_requires_reconnect = true;
                        me.write_notice(&crate::t!(
                            "terminal-daemon-startup-ack-unconfirmed",
                            attempts = MAX_STARTUP_COMMAND_DELIVERY_ATTEMPTS
                        ));
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
            || self.next_startup_command_attempt >= MAX_STARTUP_COMMAND_DELIVERY_ATTEMPTS
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
    fn begin_transport_reconnect(&mut self, ctx: &mut ModelContext<Self>) {
        self.prepare_transport_reconnect();
        self.set_input_phase(RemoteInputPhase::Transport, ctx);
    }

    fn prepare_transport_reconnect(&mut self) {
        if let Some(pending_open) = self.pending_open.as_mut() {
            pending_open.allow_retry();
        }
        self.allow_startup_command_retry();
        self.allow_agent_binding_retry();
        self.allow_attach_retry();
        if self.pending_attach_replay.is_some() {
            self.reattach_after_replay = true;
        }
        self.pending_ready_notice = None;
        self.awaiting_attach_snapshot = true;
    }

    #[cfg(test)]
    fn begin_transport_reconnect_for_test(&mut self) {
        self.prepare_transport_reconnect();
        self.user_input_ready = false;
        self.input_phase = RemoteInputPhase::Attach;
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
