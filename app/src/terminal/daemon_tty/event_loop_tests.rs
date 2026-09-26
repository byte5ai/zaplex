use super::*;
use std::borrow::Cow;
use std::time::Duration;
use warp_core::HostId;
use warpui::r#async::FutureExt;
use warpui::{App, ModelHandle};

const OUR_PTY: &str = "pty-ours";
const HOST: &str = "test-host";

#[test]
fn first_ack_for_existing_managed_session_requires_generation_checked_attach() {
    let existing = remote_server::proto::SessionOpened {
        session_id: "managed-existing".to_string(),
        generation: 17,
        requires_attach: true,
        expected_agent_binding: Some(AgentSessionIdentity {
            session_id: "agent-existing".to_string(),
            provider: "claude".to_string(),
            account_id: "account-existing".to_string(),
            ..Default::default()
        }),
    };
    let fresh = remote_server::proto::SessionOpened {
        requires_attach: false,
        expected_agent_binding: None,
        ..existing.clone()
    };

    assert!(open_ack_requires_authoritative_attach(1, &existing));
    assert!(!open_ack_requires_authoritative_attach(1, &fresh));
    assert!(open_ack_requires_authoritative_attach(2, &fresh));
}

#[test]
fn terminal_daemon_visible_errors_use_localized_messages() {
    let source = include_str!("event_loop.rs");
    let english = include_str!("../../../i18n/en/warp.ftl");
    let german = include_str!("../../../i18n/de/warp.ftl");
    let keys = [
        "terminal-daemon-multiplexer-nested",
        "terminal-daemon-attach-generation-invalid",
        "terminal-daemon-attach-agent-routing-unsupported",
        "terminal-daemon-attach-failed",
        "terminal-daemon-attach-generation-mismatch",
        "terminal-daemon-scrollback-truncated",
        "terminal-daemon-final-output-truncated",
        "terminal-daemon-reconnected",
        "terminal-daemon-reattached",
        "terminal-daemon-restored-truncated",
        "terminal-daemon-managed-open-unconfirmed",
        "terminal-daemon-managed-launch-failed",
        "terminal-daemon-managed-account-route-unsupported",
        "terminal-daemon-managed-host-unsupported",
        "terminal-daemon-managed-route-incomplete",
        "terminal-daemon-managed-open-rejected",
        "terminal-daemon-managed-connection-failed",
        "terminal-daemon-managed-claim-conflict",
        "terminal-daemon-managed-claim-unavailable",
        "terminal-daemon-managed-generation-invalid",
        "terminal-daemon-connection-failed",
        "terminal-daemon-persistent-session-active",
        "terminal-daemon-session-ended-with-code",
        "terminal-daemon-session-ended",
        "terminal-daemon-connection-lost",
        "terminal-daemon-startup-helper-upgrade-required",
        "terminal-daemon-startup-command-rejected",
        "terminal-daemon-startup-ack-invalid",
    ];

    for key in keys {
        assert!(
            source.contains(key),
            "managed launch path does not use {key}"
        );
        assert!(
            english.contains(&format!("\n{key} =")),
            "English catalog is missing {key}"
        );
        assert!(
            german.contains(&format!("\n{key} =")),
            "German catalog is missing {key}"
        );
    }
    assert!(!source.contains("The remote open result could not be confirmed safely."));
    for stale_literal in [
        "could not re-attach session: the daemon returned an invalid PTY generation",
        "could not re-attach agent: the host does not support validated agent routing",
        "this host daemon does not support managed agents; update it and reconnect",
        "managed agents require an exact remote account and project",
        "the requested startup command was not accepted and remains pending",
    ] {
        assert!(
            !source.contains(stale_literal),
            "visible terminal copy is still hard-coded: {stale_literal}"
        );
    }
}

#[test]
fn opened_pty_claim_is_resolved_before_any_authoritative_attach() {
    let source = include_str!("event_loop.rs");
    let start = source.find("fn on_session_opened(").unwrap();
    let end = source[start..]
        .find("fn on_session_opened_with_claim(")
        .map(|offset| start + offset)
        .unwrap();
    let wrapper = &source[start..end];
    let claim = wrapper.find("claim_opened_daemon_session(").unwrap();
    let continue_after_claim = wrapper.find("on_session_opened_with_claim(").unwrap();
    assert!(claim < continue_after_claim);

    let start = end;
    let end = source[start..]
        .find("fn buffer_pending_output(")
        .map(|offset| start + offset)
        .unwrap();
    let body = &source[start..end];

    let unavailable = body.find("OpenedDaemonClaim::Unavailable").unwrap();
    let unavailable_return = body[unavailable..]
        .find("return;")
        .map(|offset| unavailable + offset)
        .unwrap();
    let publish = body.find("report_session_opened(").unwrap();
    let collision_return = body.find("if claim_conflicted {").unwrap();
    let collision_failure = body.find("terminal-daemon-managed-claim-conflict").unwrap();
    let failed_startup = body[collision_failure..]
        .find("self.finish_failed_startup(ctx);")
        .map(|offset| collision_failure + offset)
        .unwrap();
    let attach = body.find("self.reattach(ctx);").unwrap();
    assert!(unavailable < unavailable_return);
    assert!(unavailable_return < publish);
    assert!(publish < collision_return);
    assert!(collision_return < collision_failure);
    assert!(collision_failure < failed_startup);
    assert!(failed_startup < attach);
}

#[test]
fn remote_phase_notification_upgrades_the_view_with_its_app_context() {
    let source = include_str!("event_loop.rs");
    assert!(source.contains(".and_then(|terminal_view| terminal_view.upgrade(ctx))"));
    assert!(!source.contains(".and_then(WeakViewHandle::upgrade)"));
}

#[test]
fn logical_open_retry_retains_parameters_and_rejects_stale_callbacks() {
    let mut pending = PendingOpen::new(
        OpenSessionParams {
            cwd: Some("/srv/project".to_string()),
            ..Default::default()
        },
        SizeInfo::new_without_font_metrics(24, 80),
    );
    let (first_id, first_params, _, first_attempt) = pending.begin_attempt().unwrap();
    assert!(pending.finish_attempt(&first_id, first_attempt));
    let (retry_id, retry_params, _, retry_attempt) = pending.begin_attempt().unwrap();

    assert_eq!(retry_id, first_id);
    assert_eq!(retry_params.cwd, first_params.cwd);
    assert!(!pending.finish_attempt(&first_id, first_attempt));
    assert!(pending.finish_attempt(&retry_id, retry_attempt));
    assert!(!pending.can_retry());
    assert!(pending.begin_attempt().is_none());
}

#[test]
fn ambiguous_open_retry_requires_negotiated_logical_open_capability_and_is_bounded() {
    let mut pending = PendingOpen::new(
        OpenSessionParams::default(),
        SizeInfo::new_without_font_metrics(24, 80),
    );
    let (logical_open_id, _, _, first_attempt) = pending.begin_attempt().unwrap();
    assert!(pending.finish_attempt(&logical_open_id, first_attempt));

    assert!(!pending.can_retry_ambiguous_open(false));
    assert!(pending.can_retry_ambiguous_open(true));

    let (retry_id, _, _, retry_attempt) = pending.begin_attempt().unwrap();
    assert_eq!(retry_id, logical_open_id);
    assert!(pending.finish_attempt(&retry_id, retry_attempt));
    assert!(!pending.can_retry_ambiguous_open(true));
    assert!(pending.begin_attempt().is_none());
}

#[test]
fn connected_and_reconnected_events_share_one_attach_phase() {
    let mut guard = AttachPhaseGuard::default();
    let first_transport = Arc::new(());

    assert!(guard.begin(&first_transport, OUR_PTY, Some(7)));
    assert!(
        !guard.begin(&first_transport, OUR_PTY, Some(7)),
        "SessionReconnected must not start a second attach after SessionConnected"
    );
}

#[test]
fn later_reconnect_starts_one_new_attach_phase() {
    let mut guard = AttachPhaseGuard::default();
    let first_transport = Arc::new(());
    let second_transport = Arc::new(());

    assert!(guard.begin(&first_transport, OUR_PTY, Some(7)));
    assert!(!guard.begin(&first_transport, OUR_PTY, Some(7)));
    assert!(
        guard.begin(&second_transport, OUR_PTY, Some(7)),
        "a replacement transport must start one new attach"
    );
    assert!(!guard.begin(&second_transport, OUR_PTY, Some(7)));
}

#[test]
fn reconnect_transition_closes_input_before_replay() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(44u64);
        let (manager, event_loop, _model, _wakeups) = start_adopted_loop(&mut app, conn);
        event_loop.update(&mut app, |me, _| {
            me.awaiting_attach_snapshot = false;
            me.user_input_ready = true;
            me.input_phase = RemoteInputPhase::Ready;
        });

        manager.update(&mut app, |_, ctx| {
            ctx.emit(RemoteServerManagerEvent::SessionConnecting {
                session_id: conn,
                reconnecting: true,
            });
        });

        event_loop.read(&app, |me, _| {
            assert_eq!(me.input_phase(), RemoteInputPhase::Transport);
            assert!(!me.is_user_input_ready());
            assert!(me.awaiting_attach_snapshot);
        });

        event_loop.update(&mut app, |me, ctx| me.on_transport_connected(ctx));
        event_loop.read(&app, |me, _| {
            assert_eq!(
                me.input_phase(),
                RemoteInputPhase::Transport,
                "without a registered replacement client, reconnect must remain transport-gated"
            );
            assert!(!me.is_user_input_ready());
        });
    });
}

#[test]
fn attach_phase_identity_includes_pty_and_generation() {
    let mut guard = AttachPhaseGuard::default();
    let transport = Arc::new(());

    assert!(guard.begin(&transport, OUR_PTY, Some(7)));
    assert!(guard.begin(&transport, "pty-other", Some(7)));
    assert!(guard.begin(&transport, "pty-other", Some(8)));
}

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

/// A [`ChannelEventListener`] whose wakeup channel we keep. Historical parsing
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

fn terminal_contents(model: &Arc<FairMutex<TerminalModel>>) -> String {
    model
        .lock()
        .block_list()
        .blocks()
        .iter()
        .map(|block| block.contents_to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn terminal_message_count(model: &Arc<FairMutex<TerminalModel>>, message: &str) -> usize {
    let contents = terminal_contents(model)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let message = message
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    contents.matches(message.as_str()).count()
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
            None,
            "test-host".to_string(),
            ctx,
        )
    });
    (manager, event_loop, model, wakeups_rx)
}

/// Replay callbacks run on the app executor after a background task completes.
/// Wakeups speed up the wait, but an unready shell can finish replay silently.
/// Periodic state checks also cover that case without assuming a yield count.
async fn wait_for_attach_replay(
    event_loop: &ModelHandle<EventLoop>,
    app: &App,
    wakeups: &async_channel::Receiver<()>,
) {
    async {
        while event_loop.read(app, |me, _| me.pending_attach_replay.is_some()) {
            if let Ok(result) = wakeups
                .recv()
                .with_timeout(Duration::from_millis(10))
                .await
            {
                result.expect("terminal wakeup channel closed");
            }
        }
    }
    .with_timeout(Duration::from_secs(5))
    .await
    .expect("attach replay did not complete");
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
fn ordinary_session_open_ack_emits_exact_surface_without_managed_launch() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(36u64);
        let terminal_view_id = EntityId::new();
        let manager = app.add_singleton_model(RemoteServerManager::new);
        let (events_tx, events_rx) = async_channel::unbounded();
        app.update(|ctx| {
            ctx.subscribe_to_model(&manager, move |_manager, event, _ctx| {
                events_tx.try_send(event.clone()).unwrap();
            });
        });
        let (listener, _wakeups_rx) = test_listener();
        let model = Arc::new(FairMutex::new(TerminalModel::mock(
            None,
            Some(listener.clone()),
        )));
        let (_event_loop_tx, event_loop_rx) = async_channel::unbounded::<EventLoopMessage>();
        let event_loop = app.add_model(|ctx| {
            EventLoop::start(
                model,
                event_loop_rx,
                listener,
                SizeInfo::new_without_font_metrics(24, 80),
                conn,
                OpenSessionParams::default(),
                None,
                None,
                None,
                None,
                None,
                HOST.to_string(),
                ctx,
            )
        });
        assert!(events_rx.is_empty());
        assert!(event_loop.read(&app, |me, _| me.initial_attach_pending));

        event_loop.update(&mut app, |me, ctx| {
            assert!(me.managed_launch_id.is_none());
            assert!(me.pty_session_id.is_none());
            me.terminal_view_id = Some(terminal_view_id);
            me.on_session_opened_with_claim(
                "pty-new".to_string(),
                9,
                false,
                None,
                OpenedDaemonClaim::Owned,
                ctx,
            );
            assert!(!me.initial_attach_pending);
        });

        let event = events_rx
            .try_recv()
            .expect("OpenSession acknowledgement event");
        assert_eq!(event.session_id(), Some(conn));
        assert!(matches!(
            event,
            RemoteServerManagerEvent::SessionOpened {
                session_id,
                terminal_view_id: Some(event_terminal_view_id),
                pty_session_id,
                generation: 9,
            } if session_id == conn
                && event_terminal_view_id == terminal_view_id
                && pty_session_id == "pty-new"
        ));
        assert!(events_rx.is_empty());
    });
}

#[test]
fn fresh_open_success_notice_waits_for_input_readiness_and_is_emitted_once() {
    assert_fresh_open_notice_after_readiness(false);
}

#[test]
fn fresh_open_transport_drop_before_ready_preserves_first_welcome() {
    assert_fresh_open_notice_after_readiness(true);
}

fn assert_fresh_open_notice_after_readiness(reconnect_before_ready: bool) {
    crate::i18n::init(Some("en"));
    App::test((), move |mut app| async move {
        let conn = SessionId::from(360u64);
        let manager = app.add_singleton_model(RemoteServerManager::new);
        let (listener, _wakeups_rx) = test_listener();
        let model = Arc::new(FairMutex::new(TerminalModel::mock_not_bootstrapped(Some(
            listener.clone(),
        ))));
        let (_event_loop_tx, event_loop_rx) = async_channel::unbounded::<EventLoopMessage>();
        let model_for_loop = model.clone();
        let event_loop = app.add_model(|ctx| {
            EventLoop::start(
                model_for_loop,
                event_loop_rx,
                listener,
                SizeInfo::new_without_font_metrics(24, 80),
                conn,
                OpenSessionParams::default(),
                None,
                None,
                None,
                None,
                None,
                "test-host".to_string(),
                ctx,
            )
        });
        let success = crate::t!(
            "terminal-daemon-persistent-session-active",
            host = "test-host"
        );

        event_loop.update(&mut app, |me, ctx| {
            me.on_session_opened_with_claim(
                "pty-new".to_string(),
                9,
                false,
                None,
                OpenedDaemonClaim::Owned,
                ctx,
            );
        });
        event_loop.read(&app, |me, _| {
            let pending = me
                .pending_ready_notice
                .as_ref()
                .expect("the exact open route remains pending until Ready");
            assert_eq!(pending.connection_session_id, conn);
            assert_eq!(pending.pty_session_id, "pty-new");
            assert_eq!(pending.generation, Some(9));
            assert_eq!(pending.kind, ReadyNoticeKind::PersistentSessionActive);
            assert!(!me.welcomed);
            assert_ne!(me.input_phase(), RemoteInputPhase::Ready);
        });
        assert_eq!(terminal_message_count(&model, success.as_ref()), 0);

        let pending_output = b"shell is still starting";
        manager.update(&mut app, |_, ctx| {
            ctx.emit(output_event(conn, "pty-new", 0, pending_output));
        });
        event_loop.read(&app, |me, _| {
            assert!(me.pending_ready_notice.is_some());
            assert!(!me.welcomed);
            assert_ne!(me.input_phase(), RemoteInputPhase::Ready);
        });
        assert_eq!(terminal_message_count(&model, success.as_ref()), 0);

        if reconnect_before_ready {
            event_loop.update(&mut app, |me, ctx| {
                me.begin_transport_reconnect_for_test();
                assert!(me.pending_ready_notice.is_none());
                me.on_session_attached(
                    SessionAttached {
                        session_id: "pty-new".to_string(),
                        size: None,
                        base_seq: pending_output.len() as u64,
                        replay: Vec::new(),
                        bootstrap_preamble: Vec::new(),
                        generation: 9,
                        agent_binding: None,
                    },
                    true,
                    ctx,
                );
                assert_eq!(
                    me.pending_ready_notice.as_ref().unwrap().kind,
                    ReadyNoticeKind::PersistentSessionActive,
                );
                assert_ne!(me.input_phase(), RemoteInputPhase::Ready);
            });
            assert_eq!(terminal_message_count(&model, success.as_ref()), 0);
        }

        let ready = init_shell_dcs();
        manager.update(&mut app, |_, ctx| {
            ctx.emit(output_event(
                conn,
                "pty-new",
                pending_output.len() as u64,
                &ready,
            ));
        });
        event_loop.read(&app, |me, _| {
            assert!(me.pending_ready_notice.is_none());
            assert!(me.welcomed);
            assert_eq!(me.input_phase(), RemoteInputPhase::Ready);
        });
        assert_eq!(terminal_message_count(&model, success.as_ref()), 0);
        event_loop.read(&app, |me, _| {
            assert_eq!(me.published_ready_notices, vec![success.clone()]);
        });

        manager.update(&mut app, |_, ctx| {
            ctx.emit(output_event(
                conn,
                "pty-new",
                (pending_output.len() + ready.len()) as u64,
                b"later output chunk",
            ));
        });
        assert_eq!(terminal_message_count(&model, success.as_ref()), 0);
        event_loop.read(&app, |me, _| {
            assert_eq!(me.published_ready_notices, vec![success.clone()]);
        });
        for wrong_notice in [
            crate::t!("terminal-daemon-reattached", host = "test-host"),
            crate::t!("terminal-daemon-reconnected", host = "test-host"),
        ] {
            assert_eq!(terminal_message_count(&model, wrong_notice.as_ref()), 0);
        }
    });
}

#[test]
fn managed_launch_success_waits_for_authoritative_attach_readiness() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(37u64);
        let manager = app.add_singleton_model(RemoteServerManager::new);
        let (events_tx, events_rx) = async_channel::unbounded();
        app.update(|ctx| {
            ctx.subscribe_to_model(&manager, move |_manager, event, _ctx| {
                events_tx.try_send(event.clone()).unwrap();
            });
        });
        let (listener, _wakeups_rx) = test_listener();
        let model = Arc::new(FairMutex::new(TerminalModel::mock(
            None,
            Some(listener.clone()),
        )));
        let (_event_loop_tx, event_loop_rx) = async_channel::unbounded::<EventLoopMessage>();
        let event_loop = app.add_model(|ctx| {
            EventLoop::start(
                model,
                event_loop_rx,
                listener,
                SizeInfo::new_without_font_metrics(24, 80),
                conn,
                OpenSessionParams {
                    managed_launch: Some(remote_server::proto::ManagedLaunch {
                        schema_version: 1,
                        launch_id: "launch-ready".to_string(),
                        provider: "claude".to_string(),
                        project_root: "/srv/project".to_string(),
                        kind: "interactive-agent".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
                None,
                None,
                None,
                None,
                HOST.to_string(),
                ctx,
            )
        });

        event_loop.update(&mut app, |me, ctx| {
            me.on_session_opened_with_claim(
                "managed-pty".to_string(),
                11,
                false,
                None,
                OpenedDaemonClaim::Owned,
                ctx,
            );
        });
        assert!(matches!(
            events_rx.try_recv(),
            Ok(RemoteServerManagerEvent::SessionOpened {
                pty_session_id,
                generation: 11,
                ..
            }) if pty_session_id == "managed-pty"
        ));
        assert!(
            events_rx.is_empty(),
            "the OpenSession Ack is not launch success"
        );
        event_loop.read(&app, |me, _| {
            assert!(me.awaiting_attach_snapshot);
            assert!(me.awaiting_managed_agent_binding);
            assert_eq!(me.managed_launch_id.as_deref(), Some("launch-ready"));
        });

        let binding = AgentSessionIdentity {
            session_id: "agent-ready".to_string(),
            provider: "claude".to_string(),
            account_id: "account-ready".to_string(),
            ..Default::default()
        };
        event_loop.update(&mut app, |me, ctx| {
            // This is the state established by a successful BindAgentPty Ack
            // immediately before the generation- and agent-checked attach.
            me.expected_attach_agent_binding = Some(binding.clone());
            me.awaiting_managed_agent_binding = false;
            me.on_session_attached(
                SessionAttached {
                    session_id: "managed-pty".to_string(),
                    size: None,
                    base_seq: 0,
                    replay: Vec::new(),
                    bootstrap_preamble: Vec::new(),
                    generation: 11,
                    agent_binding: Some(binding),
                },
                true,
                ctx,
            );
        });

        assert!(matches!(
            events_rx.try_recv(),
            Ok(RemoteServerManagerEvent::ManagedLaunchOpened {
                launch_id,
                pty_session_id,
                generation: 11,
            }) if launch_id == "launch-ready" && pty_session_id == "managed-pty"
        ));
        assert!(
            events_rx.is_empty(),
            "managed success must be reported once"
        );
        event_loop.read(&app, |me, _| {
            assert_eq!(me.input_phase(), RemoteInputPhase::Ready);
            assert!(me.managed_launch_id.is_none());
            assert!(me.managed_open_identity.is_none());
        });
    });
}

#[test]
fn managed_launch_without_claim_context_fails_before_open_or_attach_success() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(371u64);
        let manager = app.add_singleton_model(RemoteServerManager::new);
        let (events_tx, events_rx) = async_channel::unbounded();
        app.update(|ctx| {
            ctx.subscribe_to_model(&manager, move |_manager, event, _ctx| {
                events_tx.try_send(event.clone()).unwrap();
            });
        });
        let (listener, _wakeups_rx) = test_listener();
        let model = Arc::new(FairMutex::new(TerminalModel::mock(
            None,
            Some(listener.clone()),
        )));
        let (_event_loop_tx, event_loop_rx) = async_channel::unbounded::<EventLoopMessage>();
        let event_loop = app.add_model(|ctx| {
            EventLoop::start(
                model,
                event_loop_rx,
                listener,
                SizeInfo::new_without_font_metrics(24, 80),
                conn,
                OpenSessionParams {
                    managed_launch: Some(remote_server::proto::ManagedLaunch {
                        schema_version: 1,
                        launch_id: "launch-without-claim".to_string(),
                        provider: "claude".to_string(),
                        project_root: "/srv/project".to_string(),
                        kind: "interactive-agent".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
                None,
                None,
                None,
                None,
                "test-host".to_string(),
                ctx,
            )
        });

        event_loop.update(&mut app, |me, ctx| {
            me.on_session_opened("unclaimed-pty".to_string(), 11, false, None, ctx);
        });

        event_loop.read(&app, |me, _| {
            assert!(me.terminated);
            assert_eq!(me.input_phase(), RemoteInputPhase::Failed);
            assert!(me.pty_session_id.is_none());
            assert!(me.managed_open_identity.is_none());
        });
        let events = std::iter::from_fn(|| events_rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(
            event,
            RemoteServerManagerEvent::ManagedLaunchFailed { launch_id, .. }
                if launch_id == "launch-without-claim"
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            RemoteServerManagerEvent::SessionOpened { .. }
                | RemoteServerManagerEvent::ManagedLaunchOpened { .. }
        )));
    });
}

#[test]
fn managed_launch_failure_is_reported_exactly_once() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(38u64);
        let manager = app.add_singleton_model(RemoteServerManager::new);
        let (events_tx, events_rx) = async_channel::unbounded();
        app.update(|ctx| {
            ctx.subscribe_to_model(&manager, move |_manager, event, _ctx| {
                if matches!(event, RemoteServerManagerEvent::ManagedLaunchFailed { .. }) {
                    events_tx.try_send(event.clone()).unwrap();
                }
            });
        });
        let (listener, _wakeups_rx) = test_listener();
        let model = Arc::new(FairMutex::new(TerminalModel::mock_not_bootstrapped(Some(
            listener.clone(),
        ))));
        let (_event_loop_tx, event_loop_rx) = async_channel::unbounded::<EventLoopMessage>();
        let event_loop = app.add_model(|ctx| {
            EventLoop::start(
                model,
                event_loop_rx,
                listener,
                SizeInfo::new_without_font_metrics(24, 80),
                conn,
                OpenSessionParams {
                    managed_launch: Some(remote_server::proto::ManagedLaunch {
                        schema_version: 1,
                        launch_id: "launch-failed".to_string(),
                        provider: "claude".to_string(),
                        project_root: "/srv/project".to_string(),
                        kind: "interactive-agent".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
                None,
                None,
                None,
                None,
                HOST.to_string(),
                ctx,
            )
        });

        event_loop.update(&mut app, |me, ctx| {
            assert!(me.initial_attach_pending);
            me.on_initial_attach_timeout(ctx);
        });

        assert!(matches!(
            events_rx.try_recv(),
            Ok(RemoteServerManagerEvent::ManagedLaunchFailed { launch_id, .. })
                if launch_id == "launch-failed"
        ));
        assert!(
            events_rx.is_empty(),
            "failure must consume the launch exactly once"
        );
        event_loop.read(&app, |me, _| {
            assert!(me.terminated);
            assert_eq!(me.input_phase(), RemoteInputPhase::Failed);
            assert!(me.managed_launch_id.is_none());
        });
    });
}

#[test]
fn managed_launch_relinquish_is_exact_and_one_shot() {
    let mut event_loop = ready_event_loop_with_startup("");
    event_loop.managed_launch_id = Some("launch-transfer".to_string());
    event_loop.managed_open_identity = Some(("pty-transfer".to_string(), 9));

    assert!(!event_loop.relinquish_managed_launch("foreign-launch"));
    assert_eq!(
        event_loop.managed_launch_id.as_deref(),
        Some("launch-transfer")
    );
    assert!(event_loop.relinquish_managed_launch("launch-transfer"));
    assert!(event_loop.managed_launch_id.is_none());
    assert!(event_loop.managed_open_identity.is_none());
    assert!(!event_loop.relinquish_managed_launch("launch-transfer"));
}

#[test]
fn managed_install_handshake_failure_finishes_without_open_success() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(39u64);
        let manager = app.add_singleton_model(RemoteServerManager::new);
        let (events_tx, events_rx) = async_channel::unbounded();
        app.update(|ctx| {
            ctx.subscribe_to_model(&manager, move |_manager, event, _ctx| {
                events_tx.try_send(event.clone()).unwrap();
            });
        });
        let (listener, _wakeups_rx) = test_listener();
        let model = Arc::new(FairMutex::new(TerminalModel::mock_not_bootstrapped(Some(
            listener.clone(),
        ))));
        let (_event_loop_tx, event_loop_rx) = async_channel::unbounded::<EventLoopMessage>();
        let event_loop = app.add_model(|ctx| {
            EventLoop::start(
                model,
                event_loop_rx,
                listener,
                SizeInfo::new_without_font_metrics(24, 80),
                conn,
                OpenSessionParams {
                    managed_launch: Some(remote_server::proto::ManagedLaunch {
                        schema_version: 1,
                        launch_id: "launch-install-handshake".to_string(),
                        provider: "claude".to_string(),
                        project_root: "/srv/project".to_string(),
                        kind: "interactive-agent".to_string(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
                None,
                None,
                None,
                None,
                HOST.to_string(),
                ctx,
            )
        });

        manager.update(&mut app, |_, ctx| {
            ctx.emit(RemoteServerManagerEvent::SessionConnectionFailed {
                session_id: conn,
                phase: crate::remote_server::manager::RemoteServerInitPhase::Initialize,
                error: "initialize rejected".to_string(),
            });
        });

        event_loop.read(&app, |me, _| {
            assert!(me.terminated);
            assert_eq!(me.input_phase(), RemoteInputPhase::Failed);
            assert!(me.managed_launch_id.is_none());
        });
        let events = std::iter::from_fn(|| events_rx.try_recv().ok()).collect::<Vec<_>>();
        assert!(events.iter().any(|event| matches!(
            event,
            RemoteServerManagerEvent::ManagedLaunchFailed { launch_id, .. }
                if launch_id == "launch-install-handshake"
        )));
        assert!(!events.iter().any(|event| matches!(
            event,
            RemoteServerManagerEvent::SessionOpened { .. }
                | RemoteServerManagerEvent::ManagedLaunchOpened { .. }
        )));
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
fn attach_response_with_wrong_pty_fails_closed() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(61u64);
        let (_manager, event_loop, _model, _wakeups) =
            start_adopted_loop_unbootstrapped(&mut app, conn);

        event_loop.update(&mut app, |me, ctx| {
            me.on_session_attached(
                SessionAttached {
                    session_id: "pty-from-another-route".to_string(),
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

        event_loop.read(&app, |me, _| {
            assert!(me.terminated);
            assert_eq!(me.input_phase(), RemoteInputPhase::Failed);
            assert!(me.pending_ready_notice.is_none());
            assert!(!me.welcomed);
        });
    });
}

#[test]
fn attach_response_with_wrong_generation_fails_closed() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(62u64);
        let (_manager, event_loop, _model, _wakeups) =
            start_adopted_loop_unbootstrapped(&mut app, conn);

        event_loop.update(&mut app, |me, ctx| {
            me.on_session_attached(
                SessionAttached {
                    session_id: OUR_PTY.to_string(),
                    size: None,
                    base_seq: 0,
                    replay: Vec::new(),
                    bootstrap_preamble: Vec::new(),
                    generation: 8,
                    agent_binding: None,
                },
                true,
                ctx,
            );
        });

        event_loop.read(&app, |me, _| {
            assert!(me.terminated);
            assert_eq!(me.input_phase(), RemoteInputPhase::Failed);
            assert!(me.pending_ready_notice.is_none());
            assert!(!me.welcomed);
        });
    });
}

#[test]
fn attach_success_notices_follow_real_ready_transitions_once_per_attach() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        let conn = SessionId::from(620u64);
        let (manager, event_loop, model, wakeups) =
            start_adopted_loop_unbootstrapped(&mut app, conn);
        let replay = b"historical output";
        let reattached = crate::t!("terminal-daemon-reattached", host = "test-host");
        let reconnected = crate::t!("terminal-daemon-reconnected", host = "test-host");

        event_loop.update(&mut app, |me, ctx| {
            me.on_session_attached(
                SessionAttached {
                    session_id: OUR_PTY.to_string(),
                    size: None,
                    base_seq: 0,
                    replay: replay.to_vec(),
                    bootstrap_preamble: Vec::new(),
                    generation: 7,
                    agent_binding: None,
                },
                true,
                ctx,
            );
        });
        wait_for_attach_replay(&event_loop, &app, &wakeups).await;
        event_loop.read(&app, |me, _| {
            let pending = me
                .pending_ready_notice
                .as_ref()
                .expect("the exact attach route remains pending until Ready");
            assert_eq!(pending.connection_session_id, conn);
            assert_eq!(pending.pty_session_id, OUR_PTY);
            assert_eq!(pending.generation, Some(7));
            assert_eq!(pending.kind, ReadyNoticeKind::Attached);
            assert!(!me.welcomed);
            assert_eq!(me.input_phase(), RemoteInputPhase::Replay);
        });
        assert_eq!(terminal_message_count(&model, reattached.as_ref()), 0);
        assert_eq!(terminal_message_count(&model, reconnected.as_ref()), 0);

        let pending_output = b"shell still not ready";
        manager.update(&mut app, |_, ctx| {
            ctx.emit(output_event(
                conn,
                OUR_PTY,
                replay.len() as u64,
                pending_output,
            ));
        });
        event_loop.read(&app, |me, _| {
            assert!(me.pending_ready_notice.is_some());
            assert!(!me.welcomed);
            assert_eq!(me.input_phase(), RemoteInputPhase::Replay);
        });
        assert_eq!(terminal_message_count(&model, reattached.as_ref()), 0);

        let ready = init_shell_dcs();
        manager.update(&mut app, |_, ctx| {
            ctx.emit(output_event(
                conn,
                OUR_PTY,
                (replay.len() + pending_output.len()) as u64,
                &ready,
            ));
        });
        event_loop.read(&app, |me, _| {
            assert!(me.pending_ready_notice.is_none());
            assert!(me.welcomed);
            assert_eq!(me.input_phase(), RemoteInputPhase::Ready);
        });
        assert_eq!(terminal_message_count(&model, reattached.as_ref()), 0);
        assert_eq!(terminal_message_count(&model, reconnected.as_ref()), 0);
        event_loop.read(&app, |me, _| {
            assert_eq!(me.published_ready_notices, vec![reattached.clone()]);
        });

        let reconnect_base_seq = event_loop.read(&app, |me, _| me.last_seq);
        event_loop.update(&mut app, |me, _| me.begin_transport_reconnect_for_test());
        event_loop.update(&mut app, |me, ctx| {
            me.on_session_attached(
                SessionAttached {
                    session_id: OUR_PTY.to_string(),
                    size: None,
                    base_seq: reconnect_base_seq,
                    replay: Vec::new(),
                    bootstrap_preamble: Vec::new(),
                    generation: 7,
                    agent_binding: None,
                },
                true,
                ctx,
            );
        });
        event_loop.read(&app, |me, _| {
            assert!(me.pending_ready_notice.is_none());
            assert_eq!(me.input_phase(), RemoteInputPhase::Ready);
        });
        assert_eq!(terminal_message_count(&model, reattached.as_ref()), 0);
        assert_eq!(terminal_message_count(&model, reconnected.as_ref()), 0);
        event_loop.read(&app, |me, _| {
            assert_eq!(
                me.published_ready_notices,
                vec![reattached.clone(), reconnected.clone()]
            );
        });

        manager.update(&mut app, |_, ctx| {
            ctx.emit(output_event(
                conn,
                OUR_PTY,
                reconnect_base_seq,
                b"post-reconnect output chunk",
            ));
        });
        assert_eq!(terminal_message_count(&model, reattached.as_ref()), 0);
        assert_eq!(terminal_message_count(&model, reconnected.as_ref()), 0);
        event_loop.read(&app, |me, _| {
            assert_eq!(
                me.published_ready_notices,
                vec![reattached.clone(), reconnected.clone()]
            );
        });
    });
}

#[test]
fn truncated_reconnect_replay_does_not_claim_nothing_was_lost() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        let conn = SessionId::from(621u64);
        let (manager, event_loop, model, wakeups) =
            start_adopted_loop_unbootstrapped(&mut app, conn);
        let reattached = crate::t!("terminal-daemon-reattached", host = "test-host");
        let reconnected = crate::t!("terminal-daemon-reconnected", host = "test-host");
        let truncated = crate::t!("terminal-daemon-restored-truncated", host = "test-host");

        event_loop.update(&mut app, |me, ctx| {
            me.on_session_attached(
                SessionAttached {
                    session_id: OUR_PTY.to_string(),
                    size: None,
                    base_seq: 0,
                    replay: b"historical output".to_vec(),
                    bootstrap_preamble: Vec::new(),
                    generation: 7,
                    agent_binding: None,
                },
                true,
                ctx,
            );
        });
        wait_for_attach_replay(&event_loop, &app, &wakeups).await;
        let ready = init_shell_dcs();
        let ready_seq = event_loop.read(&app, |me, _| me.last_seq);
        manager.update(&mut app, |_, ctx| {
            ctx.emit(output_event(conn, OUR_PTY, ready_seq, &ready));
        });
        event_loop.read(&app, |me, _| {
            assert_eq!(me.input_phase(), RemoteInputPhase::Ready);
            assert_eq!(me.published_ready_notices, vec![reattached.clone()]);
        });

        // The daemon's ring buffer moved past what this client has seen.
        let gap_base_seq = event_loop.read(&app, |me, _| me.last_seq) + 4096;
        event_loop.update(&mut app, |me, _| me.begin_transport_reconnect_for_test());
        event_loop.update(&mut app, |me, ctx| {
            me.on_session_attached(
                SessionAttached {
                    session_id: OUR_PTY.to_string(),
                    size: None,
                    base_seq: gap_base_seq,
                    replay: Vec::new(),
                    bootstrap_preamble: Vec::new(),
                    generation: 7,
                    agent_binding: None,
                },
                true,
                ctx,
            );
        });
        event_loop.read(&app, |me, _| {
            assert_eq!(me.input_phase(), RemoteInputPhase::Ready);
            assert_eq!(
                me.published_ready_notices,
                vec![reattached.clone(), truncated.clone()]
            );
        });

        // A later gap-free reconnect may report a full restore again.
        let clean_base_seq = event_loop.read(&app, |me, _| me.last_seq);
        event_loop.update(&mut app, |me, _| me.begin_transport_reconnect_for_test());
        event_loop.update(&mut app, |me, ctx| {
            me.on_session_attached(
                SessionAttached {
                    session_id: OUR_PTY.to_string(),
                    size: None,
                    base_seq: clean_base_seq,
                    replay: Vec::new(),
                    bootstrap_preamble: Vec::new(),
                    generation: 7,
                    agent_binding: None,
                },
                true,
                ctx,
            );
        });
        event_loop.read(&app, |me, _| {
            assert_eq!(
                me.published_ready_notices,
                vec![reattached.clone(), truncated.clone(), reconnected.clone()]
            );
        });
        for notice in [&reattached, &truncated, &reconnected] {
            assert_eq!(terminal_message_count(&model, notice.as_ref()), 0);
        }
    });
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

        event_loop.update(&mut app, |me, ctx| {
            me.apply_authoritative_agent_binding_state(None);
            me.awaiting_attach_snapshot = false;
            me.drain_pending_output(ctx);
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
        let (manager, event_loop, _model, wakeups_rx) = start_adopted_loop(&mut app, conn);

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
        wait_for_attach_replay(&event_loop, &app, &wakeups_rx).await;
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
        let (_manager, event_loop, _model, wakeups_rx) = start_adopted_loop(&mut app, conn);

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
        wait_for_attach_replay(&event_loop, &app, &wakeups_rx).await;
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

#[test]
fn live_cursor_query_is_routed_back_as_session_input() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(31u64);
        let (manager, event_loop, _model, wakeups_rx) = start_adopted_loop(&mut app, conn);
        let query = b"\x1b[6n";

        event_loop.update(&mut app, |me, ctx| {
            me.on_session_attached(
                SessionAttached {
                    session_id: OUR_PTY.to_string(),
                    size: None,
                    base_seq: 0,
                    replay: query.to_vec(),
                    bootstrap_preamble: Vec::new(),
                    generation: 7,
                    agent_binding: None,
                },
                true,
                ctx,
            );
        });
        wait_for_attach_replay(&event_loop, &app, &wakeups_rx).await;
        event_loop.update(&mut app, |me, _ctx| {
            me.process_historical_pty_bytes(b"\x1b[H");
        });
        event_loop.read(&app, |me, _| {
            assert!(
                me.pending_input.is_empty(),
                "a cursor query from attach replay must not be answered"
            );
        });

        manager.update(&mut app, |_manager, ctx| {
            ctx.emit(output_event(conn, OUR_PTY, query.len() as u64, query));
        });
        event_loop.read(&app, |me, _| {
            assert_eq!(me.pending_input.len(), 1);
            match &me.pending_input[0] {
                EventLoopMessage::Input(bytes) => assert_eq!(&**bytes, b"\x1b[1;1R"),
                EventLoopMessage::Resize(_)
                | EventLoopMessage::Shutdown
                | EventLoopMessage::ChildExited => {
                    panic!("the terminal reply must be queued as session input")
                }
            }
        });

        manager.update(&mut app, |_manager, ctx| {
            ctx.emit(output_event(conn, OUR_PTY, query.len() as u64, query));
        });
        event_loop.read(&app, |me, _| {
            assert_eq!(
                me.pending_input.len(),
                1,
                "a duplicate live output range must not emit a second terminal reply"
            );
        });
    });
}

/// Keystrokes that arrive before `OpenSession` resolves are rejected at the
/// daemon boundary. They must never execute later merely because the open Ack
/// arrived; the visible editor owns draft preservation.
#[test]
fn input_before_session_open_is_not_flushed_later() {
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
        // resolves, so `pty_session_id` stays `None` and input must be rejected.
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
            assert!(
                me.pending_input.is_empty(),
                "ordinary input must not enter the later-flushed control queue"
            );
        });

        // Opening records the id, but cannot resurrect the rejected bytes.
        event_loop.update(&mut app, |me, ctx| {
            me.on_session_opened_with_claim(
                "pty-late".to_string(),
                7,
                false,
                None,
                OpenedDaemonClaim::Owned,
                ctx,
            );
        });
        event_loop.read(&app, |me, _| {
            assert_eq!(me.pty_session_id.as_deref(), Some("pty-late"));
            assert!(
                me.pending_input.is_empty(),
                "the OpenSession acknowledgement must not execute earlier user bytes"
            );
        });
    });
}

/// During an outage ordinary input is rejected, while resize control state is
/// retained for the eventual exact reattach.
#[test]
fn input_during_transport_outage_is_not_flushed_later_but_resize_is_preserved() {
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

        // Session is open, transport is down (no client): user bytes must not
        // enter the queue, but the last resize remains useful after reattach.
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
                1,
                "only resize/control state may survive the outage"
            );
            assert!(matches!(me.pending_input[0], EventLoopMessage::Resize(_)));
        });
    });
}

#[test]
fn input_after_attach_and_shell_readiness_is_delivered() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(63u64);
        let (_manager, event_loop, _model, wakeups) = start_adopted_loop(&mut app, conn);
        assert_eq!(
            event_loop.read(&app, |me, _| me.input_phase()),
            RemoteInputPhase::Transport
        );
        event_loop.update(&mut app, |me, ctx| {
            me.on_session_attached(
                SessionAttached {
                    session_id: OUR_PTY.to_string(),
                    size: None,
                    base_seq: 0,
                    replay: vec![b'x'; ATTACH_PARSE_CHUNK_BYTES + 1],
                    bootstrap_preamble: Vec::new(),
                    generation: 7,
                    agent_binding: None,
                },
                true,
                ctx,
            );
        });
        event_loop.update(&mut app, |me, _ctx| {
            assert_eq!(me.input_phase(), RemoteInputPhase::Replay);
            assert!(!me.is_user_input_ready());
            let accepted = me
                .try_deliver_user_input_with(Cow::Borrowed(b"must-wait\r"), |_pty, _bytes| {
                    Ok::<(), ()>(())
                })
                .expect("rejection is not a transport error");
            assert!(!accepted, "user input must remain closed during replay");
        });
        wait_for_attach_replay(&event_loop, &app, &wakeups).await;

        let mut delivered = None;
        event_loop.update(&mut app, |me, _ctx| {
            assert!(me.is_user_input_ready());
            assert_eq!(me.input_phase(), RemoteInputPhase::Ready);
            let accepted = me
                .try_deliver_user_input_with(Cow::Borrowed(b"echo ready\r"), |pty, bytes| {
                    delivered = Some((pty.to_string(), bytes));
                    Ok::<(), ()>(())
                })
                .expect("test dispatch succeeds");
            assert!(accepted);
        });

        assert_eq!(
            delivered,
            Some((OUR_PTY.to_string(), b"echo ready\r".to_vec()))
        );
    });
}

#[test]
fn replay_output_without_input_readiness_does_not_deliver_user_bytes() {
    let mut event_loop = unbootstrapped_event_loop_with_startup("");
    event_loop.apply_attach(&[], 0, b"historical output without a shell handshake");
    event_loop.awaiting_attach_snapshot = false;
    let mut dispatches = 0;

    let accepted = event_loop
        .try_deliver_user_input_with(Cow::Borrowed(b"must-not-run\r"), |_pty, _bytes| {
            dispatches += 1;
            Ok::<(), ()>(())
        })
        .expect("rejection is not a transport error");

    assert!(!accepted);
    assert_eq!(dispatches, 0);
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
                None,
                "test-host".to_string(),
                ctx,
            )
        });

        event_loop.update(&mut app, |me, ctx| {
            me.startup_command = Some("tmux attach".to_string());
            me.on_session_opened_with_claim(
                "pty-x".to_string(),
                7,
                false,
                None,
                OpenedDaemonClaim::Owned,
                ctx,
            );
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
    event_loop.process_historical_pty_bytes(&init_shell_dcs());
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
    event_loop.process_historical_pty_bytes(&bootstrapped_dcs());

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
    event_loop.begin_transport_reconnect_for_test();
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
    event_loop.begin_transport_reconnect_for_test();
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

    event_loop.begin_transport_reconnect_for_test();
    event_loop.process_historical_pty_bytes(&bootstrapped_dcs());
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

#[test]
fn startup_command_delivery_attempts_are_bounded() {
    let mut event_loop = ready_event_loop_with_startup("codex resume bounded-session");
    let mut attempts = 0;

    for _ in 0..(MAX_STARTUP_COMMAND_DELIVERY_ATTEMPTS + 2) {
        event_loop.try_dispatch_startup_command_with(
            |_pty_session_id, _command_id, _bytes| -> Result<(), ()> {
                attempts += 1;
                Err(())
            },
        );
    }

    assert_eq!(attempts, MAX_STARTUP_COMMAND_DELIVERY_ATTEMPTS);
    assert!(event_loop.startup_command.is_some());
    assert!(event_loop.startup_command_in_flight.is_none());
}

/// Internal protocol replies remain bounded during an outage, but an
/// unacknowledged startup command is separate control state. Buffer pressure
/// must neither remove it nor mint a different delivery id.
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
        "protocol-reply eviction must never remove pending startup control state"
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
        buffered_input_bytes <= MAX_PENDING_PROTOCOL_INPUT_BYTES,
        "protocol replies remain bounded independently of startup delivery"
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
        event_loop.update(&mut app, |me, _ctx| {
            me.awaiting_attach_snapshot = true;
            me.terminal_view_id = Some(terminal_view_id);
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
        event_loop.update(&mut app, |me, ctx| {
            // This fixture has no TerminalView to install the production model
            // subscription, so invoke the same lifecycle refresh explicitly.
            me.refresh_desired_agent_binding(ctx);
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

/// During a long outage consecutive resizes coalesce and ordinary user input
/// never enters the later-flushed control queue.
#[test]
fn user_input_is_rejected_and_resizes_coalesce_during_outage() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(19u64);
        // Adopted loop: pty id set, no live client → control traffic buffers.
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
            assert!(me
                .pending_input
                .iter()
                .all(|message| matches!(message, EventLoopMessage::Resize(_))));
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
            me.on_session_opened_with_claim(
                "pty-late".to_string(),
                7,
                false,
                None,
                OpenedDaemonClaim::Owned,
                ctx,
            )
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

#[test]
fn large_attach_replay_yields_with_model_unlocked_between_chunks() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(42u64);
        let (manager, event_loop, model, wakeups_rx) = start_adopted_loop(&mut app, conn);
        let replay = vec![b'x'; ATTACH_PARSE_CHUNK_BYTES * 3 + 17];
        let replay_len = replay.len() as u64;
        let live_output = b"live-after-snapshot";

        event_loop.update(&mut app, |me, ctx| {
            me.on_session_attached(
                SessionAttached {
                    session_id: OUR_PTY.to_string(),
                    size: None,
                    base_seq: 0,
                    replay,
                    bootstrap_preamble: Vec::new(),
                    generation: 7,
                    agent_binding: None,
                },
                true,
                ctx,
            );
        });

        event_loop.read(&app, |me, _| {
            assert!(me.pending_attach_replay.is_some());
            assert_eq!(me.last_seq, ATTACH_PARSE_CHUNK_BYTES as u64);
            assert!(me.awaiting_attach_snapshot);
        });
        assert!(
            model.try_lock().is_some(),
            "the terminal model must be unlocked between replay chunks"
        );
        manager.update(&mut app, |_manager, ctx| {
            ctx.emit(output_event(conn, OUR_PTY, replay_len, live_output));
            ctx.emit(RemoteServerManagerEvent::SessionExited {
                session_id: conn,
                host_id: HostId::new(HOST.to_string()),
                pty_session_id: OUR_PTY.to_string(),
                exit_code: Some(0),
            });
        });
        event_loop.read(&app, |me, _| {
            assert_eq!(me.pending_output.len(), 1);
            assert_eq!(me.pending_exit, Some(Some(0)));
            assert!(!me.terminated, "exit stays ordered behind the replay");
        });

        wait_for_attach_replay(&event_loop, &app, &wakeups_rx).await;
        event_loop.read(&app, |me, _| {
            assert!(me.pending_attach_replay.is_none());
            assert_eq!(me.last_seq, replay_len + live_output.len() as u64);
            assert!(!me.awaiting_attach_snapshot);
            assert!(me.terminated, "the buffered exit applies after all output");
        });
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
            me.process_historical_pty_bytes(b"just some output\r\n");
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
            me.process_historical_pty_bytes(&dcs);
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
    event_loop.update(app, |me, _| me.process_historical_pty_bytes(&dcs));

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
                None,
                "test-host".to_string(),
                ctx,
            )
        });
        drain(&events_rx);

        // The live handshake: the InitShell DCS arrives in the normal output
        // stream of the daemon session — no adopt preamble, no armed latch.
        let dcs = init_shell_dcs();
        event_loop.update(&mut app, |me, _| me.process_historical_pty_bytes(&dcs));

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
        event_loop.update(&mut app, |me, _| me.process_historical_pty_bytes(&dcs));

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

#[test]
fn rejected_initial_attach_finishes_the_hidden_bootstrap_block() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(901u64);
        let (_manager, event_loop, model, _wakeups) =
            start_adopted_loop_unbootstrapped(&mut app, conn);
        event_loop.update(&mut app, |me, ctx| {
            me.buffer_pending(EventLoopMessage::Input(Cow::Borrowed(b"must not run")));
            me.write_notice("could not re-attach session: already attached");
            me.abandon_failed_attach(ctx);
            me.on_transport_connected(ctx);
            me.on_session_opened("late-open".to_string(), 7, false, None, ctx);
            me.on_event_loop_message(EventLoopMessage::Input(Cow::Borrowed(b"late input")), ctx);
            assert!(me.terminated);
            assert!(me.pending_input.is_empty());
            assert!(me.pending_open.is_none());
            assert_eq!(me.pty_session_id.as_deref(), Some(OUR_PTY));
        });
        assert!(model.lock().is_read_only());
        assert!(!model.lock().block_list().is_bootstrapped());
    });
}

#[test]
fn connection_failure_before_bootstrap_finishes_starting_state() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(902u64);
        let (manager, event_loop, model, _wakeups) =
            start_adopted_loop_unbootstrapped(&mut app, conn);
        manager.update(&mut app, |_, ctx| {
            ctx.emit(RemoteServerManagerEvent::SessionConnectionFailed {
                session_id: conn,
                phase: crate::remote_server::manager::RemoteServerInitPhase::Connect,
                error: "connection refused".to_string(),
            });
        });
        assert!(model.lock().is_read_only());
        assert!(event_loop.read(&app, |me, _| me.terminated));
    });
}

#[test]
fn initial_attach_deadline_finishes_missing_handshake() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(903u64);
        let (_manager, event_loop, model, _wakeups) =
            start_adopted_loop_unbootstrapped(&mut app, conn);
        complete_adopted_attach(&event_loop, &mut app);
        // An empty successful attach is insufficient: the shell never bootstrapped.
        event_loop.update(&mut app, |me, ctx| me.on_initial_attach_timeout(ctx));
        assert!(model.lock().is_read_only());
        assert!(event_loop.read(&app, |me, _| me.terminated));
    });
}

#[test]
fn completed_initial_attach_deadline_cannot_cancel_a_later_reconnect() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(904u64);
        let (_manager, event_loop, model, wakeups) = start_adopted_loop(&mut app, conn);
        complete_adopted_attach(&event_loop, &mut app);
        wait_for_attach_replay(&event_loop, &app, &wakeups).await;
        event_loop.update(&mut app, |me, ctx| {
            assert!(!me.initial_attach_pending);
            me.begin_transport_reconnect(ctx);
            me.awaiting_attach_snapshot = true;
            me.on_initial_attach_timeout(ctx);
            assert!(!me.terminated);
        });
        assert!(!model.lock().is_read_only());
    });
}

#[test]
fn initial_attach_deadline_preserves_an_interactive_shell_initialization() {
    App::test((), |mut app| async move {
        let conn = SessionId::from(905u64);
        let (_manager, event_loop, model, wakeups) =
            start_adopted_loop_unbootstrapped(&mut app, conn);
        event_loop.update(&mut app, |me, ctx| {
            let mut replay = init_shell_dcs();
            replay.extend_from_slice(b"Update shell plugins? [y/N] ");
            me.on_session_attached(
                SessionAttached {
                    session_id: OUR_PTY.to_string(),
                    size: None,
                    base_seq: 0,
                    replay,
                    bootstrap_preamble: Vec::new(),
                    generation: 7,
                    agent_binding: None,
                },
                true,
                ctx,
            );
        });
        wait_for_attach_replay(&event_loop, &app, &wakeups).await;
        assert!(model.lock().pending_session_id().is_some());
        assert!(!model.lock().block_list().is_bootstrapped());
        event_loop.update(&mut app, |me, ctx| {
            assert!(!me.initial_attach_pending);
            me.on_initial_attach_timeout(ctx);
            assert!(!me.terminated);
            assert!(
                me.user_input_ready,
                "InitShell must enable explicit interactive initialization prompts"
            );
            let mut delivered = None;
            let accepted = me
                .try_deliver_user_input_with(Cow::Borrowed(b"n"), |pty, bytes| {
                    delivered = Some((pty.to_string(), bytes));
                    Ok::<(), ()>(())
                })
                .expect("test dispatch succeeds");
            assert!(accepted);
            assert_eq!(delivered, Some((OUR_PTY.to_string(), b"n".to_vec())));
        });
        assert!(!model.lock().is_read_only());
    });
}
