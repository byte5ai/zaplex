use super::*;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use pathfinder_geometry::vector::vec2f;
use warpui::platform::WindowStyle;
use warpui::{App, Event, Presenter, WindowInvalidation};

#[test]
fn task_glance_preserves_order_and_prefers_in_progress_step() {
    let state = TaskState {
        tasks: vec![
            TaskItem {
                id: "one".into(),
                title: "Finished".into(),
                status: TaskStatus::Completed,
            },
            TaskItem {
                id: "two".into(),
                title: "Current".into(),
                status: TaskStatus::InProgress,
            },
            TaskItem {
                id: "three".into(),
                title: "Later".into(),
                status: TaskStatus::Pending,
            },
        ],
    };

    let glance = task_glance_from_state(&state);
    assert_eq!(glance.completed, 1);
    assert_eq!(glance.total, 3);
    assert_eq!(glance.current, Some("Current"));
    assert_eq!(glance.tasks[0].title, "Finished");
    assert_eq!(glance.tasks[2].title, "Later");
}

#[test]
fn task_glance_falls_back_to_first_pending_step() {
    let state = TaskState {
        tasks: vec![
            TaskItem {
                id: "one".into(),
                title: "Next".into(),
                status: TaskStatus::Pending,
            },
            TaskItem {
                id: "two".into(),
                title: "After".into(),
                status: TaskStatus::Pending,
            },
        ],
    };
    assert_eq!(task_glance_from_state(&state).current, Some("Next"));
}

#[test]
fn task_activity_uses_the_current_step_without_losing_recency() {
    let state = TaskState {
        tasks: vec![TaskItem {
            id: "one".into(),
            title: "Verify release gates".into(),
            status: TaskStatus::InProgress,
        }],
    };

    assert_eq!(
        task_activity_label(Some(&state), "2m ago"),
        "Verify release gates · 2m ago"
    );
    assert_eq!(task_activity_label(None, "2m ago"), "2m ago");
}

#[test]
fn host_node_exposes_manage_terminal_agent_and_files_actions() {
    assert!(matches!(
        open_registered_host_action("node-dev"),
        WorkspaceAction::OpenSshTerminalByNode { node_id } if node_id == "node-dev"
    ));
    assert!(matches!(
        toggle_host_favorite_action("node-dev", "devhost"),
        WorkspaceAction::ToggleFavorite {
            kind: FavoriteKind::Host,
            target,
            label,
        } if target == "node-dev" && label == "devhost"
    ));
    assert!(matches!(
        manage_registered_host_action("node-dev"),
        WorkspaceAction::ManageSshHost { node_id } if node_id == "node-dev"
    ));
    assert!(matches!(
        open_registered_host_agent_action("node-dev", "devhost"),
        WorkspaceAction::OpenSpawnCard {
            registry_node_id: Some(node_id),
            host_id: None,
            host: Some(host),
            project: None,
        } if node_id == "node-dev" && host == "devhost"
    ));
    assert!(matches!(
        open_registered_host_files_action("node-dev"),
        WorkspaceAction::OpenSftpPaneByNode { node_id } if node_id == "node-dev"
    ));
}

struct HostRowTestView {
    state: MouseStateHandle,
    opened: usize,
}

impl Entity for HostRowTestView {
    type Event = ();
}

impl TypedActionView for HostRowTestView {
    type Action = WorkspaceAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        if matches!(
            action,
            WorkspaceAction::OpenSshTerminalByNode { node_id } if node_id == "node-dev"
        ) {
            self.opened += 1;
            ctx.notify();
        }
    }
}

impl View for HostRowTestView {
    fn ui_name() -> &'static str {
        "HostRowTestView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let content = Text::new_inline(
            "devhost".to_string(),
            appearance.ui_font_family(),
            appearance.ui_font_body(),
        )
        .with_color(theme.main_text_color(theme.background()).into_solid())
        .finish();
        registered_host_click_target("node-dev", content, self.state.clone(), appearance)
    }
}

#[test]
fn host_row_click_survives_rerender_between_down_and_up() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |_| HostRowTestView {
            state: MouseStateHandle::default(),
            opened: 0,
        });
        let root_view_id = app
            .root_view_id(window_id)
            .expect("test window should contain root view");
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        let invalidation = WindowInvalidation {
            updated: HashSet::from([root_view_id]),
            ..Default::default()
        };

        let click_position = app.update({
            let presenter = presenter.clone();
            let invalidation = invalidation.clone();
            move |ctx| {
                presenter.borrow_mut().invalidate(invalidation, ctx);
                presenter
                    .borrow_mut()
                    .build_scene(vec2f(320., 60.), 1., None, ctx);
                presenter
                    .borrow()
                    .position_cache()
                    .get_position("cockpit_host:node-dev")
                    .expect("registered host row should be positioned")
                    .center()
            }
        });

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
        app.update({
            let presenter = presenter.clone();
            let invalidation = invalidation.clone();
            move |ctx| {
                presenter.borrow_mut().invalidate(invalidation, ctx);
                presenter
                    .borrow_mut()
                    .build_scene(vec2f(320., 60.), 1., None, ctx);
            }
        });
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
            assert_eq!(
                view.opened, 1,
                "registered host click must fire exactly once across a rerender"
            );
        });
    });
}
