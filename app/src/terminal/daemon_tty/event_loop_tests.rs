use super::*;
use std::borrow::Cow;
use warp_core::HostId;
use warpui::{App, ModelHandle};

const OUR_PTY: &str = "pty-ours";
const HOST: &str = "test-host";

#[test]
fn account_route_requires_the_negotiated_capability() {
    let route = AgentLaunchRoute {
        schema_version: 1,
        provider: "claude".to_string(),
        account_id: "opaque-account".to_string(),
    };

    assert!(!account_route_is_compatible(Some(&route), false));
    assert!(account_route_is_compatible(Some(&route), true));
    assert!(account_route_is_compatible(None, false));
}

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

fn output_event(conn: SessionId, pty: &str, seq: u64, bytes: &[u8]) -> RemoteServerManagerEvent {
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
        let model = Arc::new(FairMutex::new(TerminalModel::mock(
            None,
            Some(listener.clone()),
        )));
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
            assert_eq!(
                me.pending_input.len(),
                2,
                "input must be buffered before open"
            );
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
fn startup_command_waits_for_bootstrap_and_runs_exactly_once() {
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
fn startup_command_does_not_run_on_session_opened_or_init_shell() {
    let mut event_loop = unbootstrapped_event_loop_with_startup("codex resume not-ready-session");

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
fn startup_command_survives_replay_then_live_bootstrap() {
    let mut event_loop = unbootstrapped_event_loop_with_startup("claude --resume replay-session");
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

    event_loop.try_dispatch_startup_command_with(|_pty_session_id, id, _bytes| -> Result<(), ()> {
        command_id = Some(id.to_string());
        Ok(())
    });

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

/// If the transport disconnects before the daemon processes the request,
/// reconnect retries the same logical delivery id exactly once.
#[test]
fn disconnect_before_daemon_processing_retries_same_command_id_once() {
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
        "after the reconnect retry is acknowledged, no third delivery is dispatched"
    );
}

#[test]
fn retained_startup_command_runs_exactly_once_after_reconnect() {
    let mut event_loop = ready_event_loop_with_startup("codex resume session-reconnect");
    let mut accepted_ids = std::collections::HashSet::new();
    let mut executions = 0;
    let mut acknowledged_id = None;

    event_loop.try_dispatch_startup_command_with(
        |_pty_session_id, command_id, _bytes| -> Result<(), ()> {
            if accepted_ids.insert(command_id.to_string()) {
                executions += 1;
            }
            acknowledged_id = Some(command_id.to_string());
            Ok(())
        },
    );
    event_loop.begin_transport_reconnect();
    event_loop.try_dispatch_startup_command_with(
        |_pty_session_id, command_id, _bytes| -> Result<(), ()> {
            if accepted_ids.insert(command_id.to_string()) {
                executions += 1;
            }
            acknowledged_id = Some(command_id.to_string());
            Ok(())
        },
    );

    assert_eq!(
        executions, 1,
        "the stable id deduplicates the reconnect retry"
    );
    event_loop.acknowledge_startup_command(
        acknowledged_id
            .as_deref()
            .expect("the daemon returned the cached acknowledgement"),
    );
    assert!(event_loop.startup_command.is_none());
}

#[test]
fn second_bootstrap_after_reconnect_does_not_resend_startup_command() {
    let mut event_loop = ready_event_loop_with_startup("codex resume session-bootstrapped");
    let mut attempts = 0;
    let mut command_id = None;
    event_loop.try_dispatch_startup_command_with(|_pty_session_id, id, _bytes| -> Result<(), ()> {
        attempts += 1;
        command_id = Some(id.to_string());
        Ok(())
    });
    event_loop.acknowledge_startup_command(
        command_id
            .as_deref()
            .expect("the first bootstrap dispatches a command"),
    );

    event_loop.begin_transport_reconnect();
    event_loop.process_pty_bytes(&bootstrapped_dcs());
    event_loop.try_dispatch_startup_command_with(
        |_pty_session_id, _id, _bytes| -> Result<(), ()> {
            attempts += 1;
            Ok(())
        },
    );

    assert_eq!(attempts, 1, "an acknowledged command is never recreated");
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
fn pending_buffer_never_evicts_unacknowledged_startup_command() {
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
        account_id: String::new(),
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
        account_id: String::new(),
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
            CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
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
            account_id: String::new(),
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
                    account_id: String::new(),
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
        account_id: String::new(),
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
            account_id: String::new(),
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
fn sidebar_attach_binds_authoritative_antigravity_account_identity() {
    App::test((), |mut app| async move {
        let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        let conn = SessionId::from(26u64);
        let (listener, _wakeups_rx) = test_listener();
        let model = Arc::new(FairMutex::new(TerminalModel::mock(
            None,
            Some(listener.clone()),
        )));
        let terminal_view_id = EntityId::new();
        let identity = AgentSessionIdentity {
            session_id: "antigravity-sidebar".to_string(),
            provider: "antigravity".to_string(),
            account_email: "antigravity@example.com".to_string(),
            config_dir: "/home/agent/.gemini/antigravity".to_string(),
            account_id: String::new(),
        };
        let event_loop = app.add_model(|_ctx| EventLoop::new(model, listener, conn));

        event_loop.update(&mut app, |me, ctx| {
            me.terminal_view_id = Some(terminal_view_id);
            me.apply_authoritative_agent_binding(Some(identity), ctx);
        });

        sessions.read(&app, |sessions, _ctx| {
            let account = sessions
                .account_identity(terminal_view_id)
                .expect("authoritative attach must bind the Antigravity account");
            assert_eq!(account.agent(), CLIAgent::Antigravity);
            assert_eq!(
                account.account_email.as_deref(),
                Some("antigravity@example.com")
            );
            assert_eq!(
                account.config_dir.as_deref(),
                Some("/home/agent/.gemini/antigravity")
            );
        });
    });
}

#[test]
fn unverifiable_lifecycle_providers_do_not_produce_daemon_bindings() {
    App::test((), |mut app| async move {
        use crate::terminal::cli_agent_sessions::{
            CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
        };

        let sessions = app.add_singleton_model(|_| CLIAgentSessionsModel::new());
        for (index, agent) in [CLIAgent::Grok, CLIAgent::Antigravity]
            .into_iter()
            .enumerate()
        {
            let conn = SessionId::from(28u64 + index as u64);
            let (listener, _wakeups_rx) = test_listener();
            let model = Arc::new(FairMutex::new(TerminalModel::mock(
                None,
                Some(listener.clone()),
            )));
            let terminal_view_id = EntityId::new();
            let event_loop = app.add_model(|_ctx| EventLoop::new(model, listener, conn));
            sessions.update(&mut app, |sessions, ctx| {
                sessions.bind_account_identity(
                    terminal_view_id,
                    agent,
                    Some("/home/agent/.config".to_string()),
                    Some("agent@example.com".to_string()),
                );
                sessions.set_session(
                    terminal_view_id,
                    CLIAgentSession {
                        agent,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext {
                            session_id: Some(format!("unsupported-{index}")),
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

            event_loop.update(&mut app, |me, ctx| {
                me.terminal_view_id = Some(terminal_view_id);
                me.awaiting_attach_snapshot = true;
                me.refresh_desired_agent_binding(ctx);
            });

            event_loop.read(&app, |me, _ctx| {
                assert!(me.desired_agent_binding.is_none());
                assert!(me.desired_agent_binding_from_lifecycle);
            });
        }
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
        account_id: String::new(),
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
        let model = Arc::new(FairMutex::new(TerminalModel::mock(
            None,
            Some(listener.clone()),
        )));
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
        event_loop.update(&mut app, |me, ctx| {
            me.on_connect_failed("Connect", "ssh: connect timed out", ctx)
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
