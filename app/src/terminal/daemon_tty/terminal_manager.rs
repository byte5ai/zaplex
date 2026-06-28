use crate::ai::blocklist::InputConfig;
use crate::context_chips::prompt_type::PromptType;
use crate::pane_group::TerminalViewResources;
use crate::persistence::ModelEvent;
use crate::terminal::daemon_tty::event_loop::EventLoop;
use crate::terminal::event_listener::ChannelEventListener;
use crate::terminal::model::session::Sessions;
use crate::terminal::model_events::ModelEventDispatcher;
use crate::terminal::shell::{ShellName, ShellType};
use crate::terminal::writeable_pty::terminal_manager_util::{
    init_pty_controller_model, wire_up_pty_controller_with_view,
};
use crate::terminal::writeable_pty::{self, Message};
use crate::terminal::{terminal_manager, ShellLaunchState, SizeInfo, TerminalModel, TerminalView};
use async_channel::{Receiver, Sender};
use parking_lot::FairMutex;
use pathfinder_geometry::vector::Vector2F;
use std::any::Any;
use std::collections::HashMap;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use warp_core::SessionId;
use warpui::{AppContext, ModelHandle, ViewHandle, WindowId};

// Reuses the same `EventLoopSender` impl for `Sender<Message>` defined by
// `remote_tty`; do not re-implement it here (it would violate coherence).
type PtyController = writeable_pty::PtyController<Sender<Message>>;

/// Parameters for opening a fresh daemon-hosted session. The initial size is
/// derived from the terminal model, so it is intentionally not included here.
#[derive(Clone, Debug, Default)]
pub struct OpenSessionParams {
    pub cwd: Option<String>,
    pub shell: Option<String>,
    pub env: HashMap<String, String>,
}

/// A request to back a newly created terminal with a daemon-hosted session on an
/// already-identified (possibly not-yet-connected) remote-server connection.
/// Carried through `NewTerminalOptions` into `create_session`.
#[derive(Clone, Debug)]
pub struct DaemonSessionRequest {
    /// The manager/connection session id the daemon session lives on. Allocated
    /// up front (see `headless_connect::alloc_daemon_session_id`); the terminal
    /// waits for it to reach `Connected` before issuing `OpenSession`.
    pub connection_session_id: SessionId,
    pub open_params: OpenSessionParams,
    /// `None` opens a fresh session; `Some(pty_session_id)` adopts an existing
    /// daemon session (attach + replay instead of open) — the multi-session
    /// "adopt a running session" path (Stage 4).
    pub adopt_pty_session_id: Option<String>,
}

/// A [`crate::terminal::TerminalManager`] whose PTY lives in the remote daemon
/// and survives transport drops. Sibling of [`crate::terminal::remote_tty`];
/// both build on the shared terminal-manager helpers and differ only in the
/// transport their [`EventLoop`] speaks.
pub struct TerminalManager {
    model: Arc<FairMutex<TerminalModel>>,

    // Hold references to the PTYController and EventLoop so the UI framework
    // doesn't deallocate them for lack of strong references.
    _pty_controller: ModelHandle<PtyController>,

    _event_loop: ModelHandle<EventLoop>,

    view: ViewHandle<TerminalView>,
}

impl TerminalManager {
    /// Creates a terminal manager backed by a daemon-hosted PTY session.
    ///
    /// `connection_session_id` identifies an already-connected remote-server
    /// session (the manager resolves the live client from it); the session is
    /// opened on that connection via `OpenSession`.
    #[allow(clippy::too_many_arguments)]
    pub fn create_model(
        resources: TerminalViewResources,
        initial_size: Vector2F,
        model_event_sender: Option<SyncSender<ModelEvent>>,
        window_id: WindowId,
        initial_input_config: Option<InputConfig>,
        connection_session_id: SessionId,
        open_params: OpenSessionParams,
        adopt_pty_session_id: Option<String>,
        ctx: &mut AppContext,
    ) -> ModelHandle<Box<dyn crate::terminal::TerminalManager>> {
        // Create all the channels we need for communication.
        let (wakeups_tx, wakeups_rx) = async_channel::unbounded();
        let (events_tx, events_rx) = async_channel::unbounded();
        let (executor_command_tx, executor_command_rx) = async_channel::unbounded();

        // Empty PTY-reads broadcaster: nothing consumes raw PTY-read broadcasts
        // for a protocol-backed PTY. Capacity is 1 (not 0) because
        // `async_broadcast` asserts a minimum capacity of 1.
        let (pty_reads_tx, _pty_reads_rx) = async_broadcast::broadcast(1);

        let channel_event_proxy = ChannelEventListener::new(wakeups_tx, events_tx, pty_reads_tx);

        // Initialize the sessions model.
        let sessions: ModelHandle<Sessions> =
            ctx.add_model(|ctx| Sessions::new(executor_command_tx, ctx));

        let model_events =
            ctx.add_model(|ctx| ModelEventDispatcher::new(events_rx, sessions.clone(), ctx));

        // Create the terminal model.
        let model = terminal_manager::create_terminal_model(
            None, /* startup_directory */
            None, /* restored_blocks */
            initial_size,
            channel_event_proxy.clone(),
            // TODO: thread the real shell type through once the daemon reports it.
            ShellLaunchState::ShellSpawned {
                available_shell: None,
                display_name: ShellName::blank(),
                shell_type: ShellType::Zsh,
            },
            ctx,
        );

        let size_info = *model.block_list().size();
        let colors = model.colors();
        let model = Arc::new(FairMutex::new(model));

        let (event_loop_tx, event_loop_rx) = async_channel::unbounded();

        let event_loop = Self::create_and_start_event_loop(
            model.clone(),
            channel_event_proxy.clone(),
            event_loop_rx,
            size_info,
            connection_session_id,
            open_params,
            adopt_pty_session_id,
            ctx,
        );

        // Initialize the PtyController.
        let pty_controller = init_pty_controller_model(
            event_loop_tx.clone(),
            executor_command_rx,
            model_events.clone(),
            sessions.clone(),
            model.clone(),
            ctx,
        );

        let cloned_model = model.clone();
        let prompt_type =
            ctx.add_model(|ctx| PromptType::new_dynamic_from_sessions(sessions.clone(), ctx));
        let view = ctx.add_typed_action_view(window_id, |ctx| {
            TerminalView::new(
                resources,
                wakeups_rx,
                model_events.clone(),
                cloned_model,
                sessions.clone(),
                size_info,
                colors,
                model_event_sender.clone(),
                prompt_type,
                initial_input_config,
                None, // conversation_restoration - not used for daemon sessions
                None, // inactive_pty_reads_rx
                ctx,
            )
        });

        wire_up_pty_controller_with_view(
            &pty_controller,
            &view,
            model.clone(),
            sessions,
            model_event_sender,
            ctx,
        );

        // Create the terminal manager itself.
        let terminal_manager = Self {
            model,
            view,
            _pty_controller: pty_controller,
            _event_loop: event_loop,
        };

        ctx.add_model(|_ctx| {
            let manager: Box<dyn crate::terminal::TerminalManager> = Box::new(terminal_manager);
            manager
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn create_and_start_event_loop(
        terminal_model: Arc<FairMutex<TerminalModel>>,
        channel_event_listener: ChannelEventListener,
        event_loop_rx: Receiver<Message>,
        size_info: SizeInfo,
        connection_session_id: SessionId,
        open_params: OpenSessionParams,
        adopt_pty_session_id: Option<String>,
        ctx: &mut AppContext,
    ) -> ModelHandle<EventLoop> {
        ctx.add_model(|ctx| {
            EventLoop::start(
                terminal_model,
                event_loop_rx,
                channel_event_listener,
                size_info,
                connection_session_id,
                open_params,
                adopt_pty_session_id,
                ctx,
            )
        })
    }
}

impl crate::terminal::TerminalManager for TerminalManager {
    fn model(&self) -> Arc<FairMutex<TerminalModel>> {
        self.model.clone()
    }

    fn view(&self) -> ViewHandle<TerminalView> {
        self.view.clone()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
