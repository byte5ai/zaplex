use super::*;

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pathfinder_geometry::vector::vec2f;
use warpui::elements::Stack;
use warpui::platform::WindowStyle;
use warpui::{
    App, AppContext, Entity, Event, Presenter, SingletonEntity, TypedActionView, View, ViewContext,
    WindowInvalidation,
};

use super::super::transfer_job::TransferProgress;

struct TransferPanelTestView {
    transfers: Vec<TransferTask>,
    cancel_btn_states: std::collections::HashMap<usize, MouseStateHandle>,
    pause_btn_states: std::collections::HashMap<usize, MouseStateHandle>,
    retry_btn_states: std::collections::HashMap<usize, MouseStateHandle>,
    close_btn_state: MouseStateHandle,
    scroll_state: ClippedScrollStateHandle,
    cancelled_ids: Vec<usize>,
}

struct WorkspaceTransferPanelTestView;

struct WorkspaceTransferActivityObserver {
    notifications: Arc<AtomicUsize>,
}

impl TransferPanelTestView {
    fn new() -> Self {
        Self {
            transfers: vec![make_transfer_task(1)],
            cancel_btn_states: std::collections::HashMap::from([(1, MouseStateHandle::default())]),
            pause_btn_states: std::collections::HashMap::from([(1, MouseStateHandle::default())]),
            retry_btn_states: std::collections::HashMap::new(),
            close_btn_state: MouseStateHandle::default(),
            scroll_state: ClippedScrollStateHandle::default(),
            cancelled_ids: Vec::new(),
        }
    }

    fn with_many_transfers() -> Self {
        let mut view = Self::new();
        view.transfers = (0..100).map(make_transfer_task).collect();
        for task in &mut view.transfers {
            task.state = TransferState::Completed;
        }
        view.transfers[0].state = TransferState::InProgress;
        view.transfers[1].state = TransferState::Failed("cleanup".to_string());
        view.transfers[1]
            .recovery_paths
            .push(PathBuf::from("/retained"));
        view
    }
}

impl Entity for TransferPanelTestView {
    type Event = ();
}

impl Entity for WorkspaceTransferPanelTestView {
    type Event = ();
}

impl Entity for WorkspaceTransferActivityObserver {
    type Event = ();
}

impl TypedActionView for WorkspaceTransferActivityObserver {
    type Action = crate::WorkspaceAction;

    fn handle_action(&mut self, _action: &Self::Action, _ctx: &mut ViewContext<Self>) {}
}

impl WorkspaceTransferActivityObserver {
    fn new(notifications: Arc<AtomicUsize>, ctx: &mut ViewContext<Self>) -> Self {
        ctx.observe(
            &super::super::transfer_queue::TransferQueue::handle(ctx),
            |me, _, ctx| {
                me.notifications.fetch_add(1, Ordering::SeqCst);
                ctx.notify();
            },
        );
        Self { notifications }
    }
}

impl TypedActionView for WorkspaceTransferPanelTestView {
    type Action = crate::WorkspaceAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        if let crate::WorkspaceAction::CancelTransfer(target) = action {
            super::super::transfer_queue::TransferQueue::handle(ctx).update(ctx, |queue, ctx| {
                queue.cancel(*target);
                ctx.notify();
            });
        }
    }
}

impl View for WorkspaceTransferPanelTestView {
    fn ui_name() -> &'static str {
        "WorkspaceTransferPanelTestView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        render_workspace_transfer_panel(app).unwrap_or_else(|| Stack::new().finish())
    }
}

impl View for WorkspaceTransferActivityObserver {
    fn ui_name() -> &'static str {
        "WorkspaceTransferActivityObserver"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        render_workspace_transfer_panel(app).unwrap_or_else(|| Stack::new().finish())
    }
}

impl TypedActionView for TransferPanelTestView {
    type Action = SftpBrowserAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        if let SftpBrowserAction::CancelTransfer(id, _) = action {
            self.cancelled_ids.push(*id);
            ctx.notify();
        }
    }
}

impl View for TransferPanelTestView {
    fn ui_name() -> &'static str {
        "TransferPanelTestView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        Stack::new()
            .with_child(render_transfer_panel(
                &self.transfers,
                &self.cancel_btn_states,
                &self.pause_btn_states,
                &self.retry_btn_states,
                appearance,
                self.close_btn_state.clone(),
                self.scroll_state.clone(),
                TransferPanelTarget::Browser,
            ))
            .finish()
    }
}

fn initialize_app(app: &mut App) {
    app.add_singleton_model(|_| Appearance::mock());
    app.add_singleton_model(super::super::transfer_queue::TransferQueue::new_with_notifications);
}

fn make_transfer_task(id: usize) -> TransferTask {
    let mut task = TransferTask::new(
        id,
        PathBuf::from(format!("/remote/file_{id}.txt")),
        PathBuf::from(format!("/local/file_{id}.txt")),
        TransferDirection::Download,
        1024,
    );
    task.state = TransferState::InProgress;
    task
}

#[test]
fn review14_source_restore_warning_has_a_distinct_localized_label() {
    let restored = completed_warning_label(super::super::transfer_queue::SOURCE_RESTORED_WARNING);
    let cleanup = completed_warning_label("cleanup failed");

    assert_eq!(
        restored,
        crate::t!("fm-transfer-completed-source-restored").to_string()
    );
    assert_eq!(
        cleanup,
        crate::t!("fm-transfer-completed-cleanup-required").to_string()
    );
    assert_ne!(restored, cleanup);
}

#[test]
fn review20_skipped_queue_activity_is_not_rendered_as_completed() {
    crate::i18n::init(Some("en"));
    let state =
        transfer_state_from_queue_state(super::super::transfer_queue::QueuedTransferState::Skipped);

    assert!(
        !matches!(state, TransferState::Completed),
        "a fully skipped transfer needs its own visible state"
    );
    assert_ne!(
        crate::t!("fm-transfer-skipped"),
        crate::t!("fm-transfer-completed")
    );
}

#[test]
fn review9_partial_completion_has_a_distinct_source_kept_label() {
    crate::i18n::init(Some("en"));
    let partial = partial_state_label(3, 4, 2, false);
    let source_kept = partial_state_label(3, 4, 2, true);

    assert_ne!(partial, crate::t!("fm-transfer-completed"));
    assert_ne!(partial, crate::t!("fm-transfer-cancelled"));
    assert_ne!(partial, source_kept);
    assert!(
        partial.contains('4'),
        "published entry count must be visible"
    );
    assert!(
        partial.contains('3'),
        "transferred file count must be visible"
    );
}

#[test]
fn review9_neutral_copy_direction_renders_in_the_existing_transfer_panel() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, view) =
            app.add_window(WindowStyle::NotStealFocus, |_| TransferPanelTestView::new());
        view.update(&mut app, |view, _| {
            view.transfers[0].direction = TransferDirection::Copy;
            view.transfers[0].state = TransferState::PartiallyCompleted {
                transferred: 1,
                published: 2,
                skipped: 1,
                source_kept: true,
            };
        });
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));

        build_scene(&mut app, window_id, presenter, vec2f(320., 120.));
    });
}

fn build_scene(
    app: &mut App,
    window_id: warpui::WindowId,
    presenter: Rc<RefCell<Presenter>>,
    size: pathfinder_geometry::vector::Vector2F,
) {
    let root_view_id = app
        .root_view_id(window_id)
        .expect("test window should contain root view");
    let invalidation = WindowInvalidation {
        updated: HashSet::from([root_view_id]),
        ..Default::default()
    };
    app.update(move |ctx| {
        presenter.borrow_mut().invalidate(invalidation, ctx);
        presenter.borrow_mut().build_scene(size, 1., None, ctx);
    });
}

#[test]
fn clicking_panel_background_does_not_toggle_transfer_panel() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, view) =
            app.add_window(WindowStyle::NotStealFocus, |_| TransferPanelTestView::new());
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        build_scene(&mut app, window_id, presenter.clone(), vec2f(320., 120.));

        app.update({
            let presenter = presenter.clone();
            move |ctx| {
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: vec2f(4., 12.),
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );
                ctx.simulate_window_event(
                    Event::LeftMouseUp {
                        position: vec2f(4., 12.),
                        modifiers: Default::default(),
                    },
                    window_id,
                    presenter,
                );
            }
        });

        view.read(&app, |view, _| {
            assert_eq!(view.transfers.len(), 1);
        });
    });
}

#[test]
fn cancel_cancels_inflight_operation_exactly_once() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, view) =
            app.add_window(WindowStyle::NotStealFocus, |_| TransferPanelTestView::new());
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        build_scene(&mut app, window_id, presenter.clone(), vec2f(320., 120.));
        let click_position = presenter
            .borrow()
            .position_cache()
            .get_position("sftp_btn:cancel_transfer:1")
            .expect("cancel button should be positioned")
            .center();

        app.update({
            let presenter = presenter.clone();
            move |ctx| {
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: click_position,
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter,
                );
            }
        });
        build_scene(&mut app, window_id, presenter.clone(), vec2f(320., 120.));
        app.update({
            let presenter = presenter.clone();
            move |ctx| {
                ctx.simulate_window_event(
                    Event::LeftMouseUp {
                        position: click_position,
                        modifiers: Default::default(),
                    },
                    window_id,
                    presenter,
                );
            }
        });

        view.read(&app, |view, _| {
            assert_eq!(view.cancelled_ids, vec![1]);
        });
    });
}

#[test]
fn narrow_panel_bounds_many_history_rows() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| {
            TransferPanelTestView::with_many_transfers()
        });
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        build_scene(&mut app, window_id, presenter.clone(), vec2f(260., 300.));

        let bounds = presenter
            .borrow()
            .position_cache()
            .get_position("sftp_transfer_history_scroll")
            .expect("bounded transfer history should be positioned");
        assert!(
            bounds.size().y() <= 120.0,
            "narrow panes must cap history height, got {}",
            bounds.size().y()
        );
    });
}

#[test]
fn workspace_activity_controls_global_queue_without_an_sftp_browser() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let id = app.update(|ctx| {
            super::super::transfer_queue::TransferQueue::handle(ctx).update(ctx, |queue, ctx| {
                let id = queue
                    .enqueue_job(
                        "workspace",
                        PathBuf::from("/source"),
                        PathBuf::from("/target"),
                        TransferDirection::Upload,
                        1024,
                    )
                    .unwrap();
                ctx.notify();
                id
            })
        });
        let (window_id, _) = app.add_window(WindowStyle::NotStealFocus, |_| {
            WorkspaceTransferPanelTestView
        });
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        build_scene(&mut app, window_id, presenter.clone(), vec2f(640., 360.));
        let click_position = presenter
            .borrow()
            .position_cache()
            .get_position(&format!("sftp_btn:cancel_transfer:{id}"))
            .expect("workspace transfer affordance must expose queue controls")
            .center();

        app.update({
            let presenter = presenter.clone();
            move |ctx| {
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position: click_position,
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter,
                );
            }
        });
        build_scene(&mut app, window_id, presenter.clone(), vec2f(640., 360.));
        app.update({
            let presenter = presenter.clone();
            move |ctx| {
                ctx.simulate_window_event(
                    Event::LeftMouseUp {
                        position: click_position,
                        modifiers: Default::default(),
                    },
                    window_id,
                    presenter,
                );
            }
        });

        let state = app.update(|ctx| {
            super::super::transfer_queue::TransferQueue::as_ref(ctx)
                .activity(id)
                .unwrap()
                .state
        });
        assert_eq!(
            state,
            super::super::transfer_queue::QueuedTransferState::Cancelling
        );
    });
}

#[test]
fn workspace_observes_worker_progress_without_a_live_sftp_browser() {
    App::test((), |mut app| async move {
        initialize_app(&mut app);
        let worker = app.update(|ctx| {
            super::super::transfer_queue::TransferQueue::handle(ctx).update(ctx, |queue, _| {
                let id = queue
                    .enqueue_job(
                        "workspace",
                        PathBuf::from("/source"),
                        PathBuf::from("/target"),
                        TransferDirection::Upload,
                        1024,
                    )
                    .unwrap();
                queue.activity_handle(id).unwrap()
            })
        });
        let notifications = Arc::new(AtomicUsize::new(0));
        let observed = notifications.clone();
        let _ = app.add_window(WindowStyle::NotStealFocus, move |ctx| {
            WorkspaceTransferActivityObserver::new(observed, ctx)
        });

        worker.update_progress(TransferProgress {
            transferred: 1024,
            total: 1024,
            eta: None,
            phase: TransferPhase::Verifying,
        });
        worker.set_state(super::super::transfer_queue::QueuedTransferState::Completed);
        warpui::r#async::Timer::after(Duration::from_millis(250)).await;

        assert!(
            notifications.load(Ordering::SeqCst) > 0,
            "workspace observers must be notified when a worker mutates the global queue"
        );
    });
}
