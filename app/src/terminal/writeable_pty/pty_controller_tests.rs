use warpui::App;

use super::*;

const GENERATOR: &str = "warp_run_generator_command 1700000000000001 'compgen -c'";

/// A PtyController whose line editor is active, as after a shell prompt.
fn controller_at_prompt(
    app: &mut App,
) -> (
    ModelHandle<PtyController<Sender<Message>>>,
    Receiver<Message>,
    Arc<FairMutex<TerminalModel>>,
) {
    let terminal_model = Arc::new(FairMutex::new(TerminalModel::mock(None, None)));
    let sessions = app.add_model(|_| Sessions::new_for_test());
    let (_model_events_tx, model_events_rx) = async_channel::unbounded();
    let model_events =
        app.add_model(|ctx| ModelEventDispatcher::new(model_events_rx, sessions.clone(), ctx));
    let line_editor_status =
        app.add_model(|ctx| LineEditorStatus::new(model_events.clone(), sessions.clone(), ctx));
    let (_executor_command_tx, executor_command_rx) = async_channel::unbounded();
    let (event_loop_tx, event_loop_rx) = async_channel::unbounded();
    let controller_model = terminal_model.clone();
    let controller = app.add_model(|ctx| {
        PtyController::new(
            event_loop_tx,
            model_events,
            line_editor_status.clone(),
            sessions,
            executor_command_rx,
            controller_model,
            ctx,
        )
    });
    line_editor_status.update(app, |line_editor_status, ctx| {
        line_editor_status.mark_active_for_test(ctx);
    });
    (controller, event_loop_rx, terminal_model)
}

fn queue_generator(
    controller: &ModelHandle<PtyController<Sender<Message>>>,
    cancel_tx: Sender<InBandCommandCancelledEvent>,
    app: &mut App,
) {
    controller.update(app, |controller, ctx| {
        controller.queue_in_band_command(
            GENERATOR,
            ShellType::Bash,
            "1700000000000001".to_owned(),
            cancel_tx,
            ctx,
        );
    });
}

fn is_in_band_block_marked(terminal_model: &Arc<FairMutex<TerminalModel>>) -> bool {
    let terminal_model = terminal_model.lock();
    terminal_model
        .block_list()
        .is_writing_or_executing_in_band_command()
        || terminal_model
            .block_list()
            .active_block()
            .is_in_band_command_block()
}

/// #530: a replayed prompt makes the line editor look active while the daemon
/// event loop still rejects input. The generator must wait in the queue
/// without marking a block, and run once the remote phase is `Ready`.
#[test]
fn in_band_command_stays_queued_until_remote_input_is_ready() {
    App::test((), |mut app| async move {
        let (controller, event_loop_rx, terminal_model) = controller_at_prompt(&mut app);
        let (cancel_tx, _cancel_rx) = async_channel::unbounded();
        controller.update(&mut app, |controller, ctx| {
            controller.set_remote_input_phase(RemoteInputPhase::Transport, ctx);
        });
        queue_generator(&controller, cancel_tx, &mut app);

        for phase in [
            RemoteInputPhase::Transport,
            RemoteInputPhase::Attach,
            RemoteInputPhase::Replay,
        ] {
            controller.update(&mut app, |controller, ctx| {
                controller.set_remote_input_phase(phase, ctx);
            });
            assert!(
                event_loop_rx.try_recv().is_err(),
                "nothing may be written in the {phase:?} phase"
            );
            assert!(
                !is_in_band_block_marked(&terminal_model),
                "no block may be marked in-band in the {phase:?} phase"
            );
        }

        controller.update(&mut app, |controller, ctx| {
            controller.set_remote_input_phase(RemoteInputPhase::Ready, ctx);
        });
        let Ok(Message::InBandCommand(command)) = event_loop_rx.try_recv() else {
            panic!("the held generator must be written as an in-band command once input is ready");
        };
        assert!(
            String::from_utf8_lossy(&command.bytes).contains(GENERATOR),
            "the written bytes run the held generator"
        );
        assert!(
            event_loop_rx.try_recv().is_err(),
            "it is written exactly once"
        );
        assert!(
            !is_in_band_block_marked(&terminal_model),
            "the event loop, not the controller, marks the block on delivery"
        );
    });
}

/// A PTY without a remote input phase keeps writing generators right away and
/// marks their block before the bytes leave the controller.
#[test]
fn in_band_command_without_remote_phase_is_written_and_marked_immediately() {
    App::test((), |mut app| async move {
        let (controller, event_loop_rx, terminal_model) = controller_at_prompt(&mut app);
        let (cancel_tx, _cancel_rx) = async_channel::unbounded();

        queue_generator(&controller, cancel_tx, &mut app);

        let Ok(Message::Input(bytes)) = event_loop_rx.try_recv() else {
            panic!("a PTY that accepts every write receives the generator as plain input");
        };
        assert!(String::from_utf8_lossy(&bytes).contains(GENERATOR));
        assert!(is_in_band_block_marked(&terminal_model));
    });
}
