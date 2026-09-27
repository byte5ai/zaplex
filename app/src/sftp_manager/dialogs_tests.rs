use super::*;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use pathfinder_geometry::vector::vec2f;
use warpui::platform::WindowStyle;
use warpui::{
    App, AppContext, Entity, Event, Presenter, SingletonEntity, TypedActionView, View, ViewContext,
    WindowInvalidation,
};

struct ConflictDialogTestView {
    handles: [MouseStateHandle; 9],
    actions: Vec<SftpBrowserAction>,
}

impl Entity for ConflictDialogTestView {
    type Event = ();
}

impl TypedActionView for ConflictDialogTestView {
    type Action = SftpBrowserAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        self.actions.push(action.clone());
        ctx.notify();
    }
}

impl View for ConflictDialogTestView {
    fn ui_name() -> &'static str {
        "ConflictDialogTestView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        render_copy_move_conflict(
            "conflicting-file.txt",
            2,
            false,
            Appearance::as_ref(app),
            self.handles[0].clone(),
            self.handles[1].clone(),
            self.handles[2].clone(),
            self.handles[3].clone(),
            self.handles[4].clone(),
            self.handles[5].clone(),
            self.handles[6].clone(),
            self.handles[7].clone(),
            self.handles[8].clone(),
        )
    }
}

#[test]
fn conflict_actions_fit_inside_the_dialog_and_the_last_action_is_clickable() {
    App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        for width in [280.0, DIALOG_MAX_WIDTH] {
            let (window_id, view) =
                app.add_window(WindowStyle::NotStealFocus, |_| ConflictDialogTestView {
                    handles: std::array::from_fn(|_| MouseStateHandle::default()),
                    actions: Vec::new(),
                });
            let root_view_id = app.root_view_id(window_id).unwrap();
            let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
            app.update(|ctx| {
                presenter.borrow_mut().invalidate(
                    WindowInvalidation {
                        updated: HashSet::from([root_view_id]),
                        ..Default::default()
                    },
                    ctx,
                );
                presenter
                    .borrow_mut()
                    .build_scene(vec2f(width, DIALOG_MAX_HEIGHT), 1.0, None, ctx);
            });
            let ids = [
                "sftp_btn:dialog_confirm",
                "sftp_btn:dialog_cancel",
                "sftp_btn:dialog_overwrite_all",
                "sftp_btn:dialog_skip_all",
                "sftp_btn:dialog_rename",
                "sftp_btn:dialog_newer_only",
                "sftp_btn:dialog_rename_all",
                "sftp_btn:dialog_newer_only_all",
            ];
            let bounds = ids.map(|id| {
                presenter
                    .borrow()
                    .position_cache()
                    .get_position(id)
                    .unwrap_or_else(|| panic!("missing conflict action {id}"))
            });
            for (index, rect) in bounds.iter().enumerate() {
                assert!(rect.width() > 0.0 && rect.height() > 0.0);
                assert!(
                    rect.min_x() >= 0.0 && rect.max_x() <= width,
                    "{}",
                    ids[index]
                );
                assert!(
                    rect.min_y() >= 0.0 && rect.max_y() <= DIALOG_MAX_HEIGHT,
                    "{}",
                    ids[index]
                );
                for other in &bounds[index + 1..] {
                    assert!(
                        rect.max_x() <= other.min_x()
                            || other.max_x() <= rect.min_x()
                            || rect.max_y() <= other.min_y()
                            || other.max_y() <= rect.min_y(),
                        "conflict actions overlap"
                    );
                }
            }
            let position = bounds[7].center();
            app.update(|ctx| {
                ctx.simulate_window_event(
                    Event::LeftMouseDown {
                        position,
                        modifiers: Default::default(),
                        click_count: 1,
                        is_first_mouse: false,
                    },
                    window_id,
                    presenter.clone(),
                );
                ctx.simulate_window_event(
                    Event::LeftMouseUp {
                        position,
                        modifiers: Default::default(),
                    },
                    window_id,
                    presenter.clone(),
                );
            });
            view.read(&app, |view, _| {
                assert!(matches!(
                    view.actions.as_slice(),
                    [SftpBrowserAction::NewerOnlyConflict { all: true }]
                ));
            });
        }
    });
}
