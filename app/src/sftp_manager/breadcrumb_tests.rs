use super::*;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use pathfinder_geometry::vector::vec2f;
use warpui::elements::{CrossAxisAlignment, Flex, ParentElement, Stack};
use warpui::platform::WindowStyle;
use warpui::{
    App, AppContext, Entity, Event, Presenter, SingletonEntity, TypedActionView, View, ViewContext,
    WindowInvalidation,
};

fn labels_and_targets(path: &str) -> Vec<(String, PathBuf)> {
    breadcrumb_segments(Path::new(path))
        .into_iter()
        .map(|segment| (segment.label, segment.target))
        .collect()
}

#[test]
fn absolute_path_segments_keep_absolute_navigation_targets() {
    assert_eq!(
        labels_and_targets("/srv/app"),
        vec![
            ("/".to_string(), PathBuf::from("/")),
            ("srv".to_string(), PathBuf::from("/srv")),
            ("app".to_string(), PathBuf::from("/srv/app")),
        ]
    );
}

#[test]
fn relative_path_segments_keep_relative_navigation_targets() {
    assert_eq!(
        labels_and_targets("srv/app"),
        vec![
            ("srv".to_string(), PathBuf::from("srv")),
            ("app".to_string(), PathBuf::from("srv/app")),
        ]
    );
}

#[test]
fn root_path_has_exactly_one_root_segment() {
    assert_eq!(
        labels_and_targets("/"),
        vec![("/".to_string(), PathBuf::from("/"))]
    );
}

struct BreadcrumbTestView {
    path: PathBuf,
    mouse_handles: HashMap<PathBuf, MouseStateHandle>,
    navigations: Vec<PathBuf>,
}

impl Entity for BreadcrumbTestView {
    type Event = ();
}

impl TypedActionView for BreadcrumbTestView {
    type Action = SftpBrowserAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        if let SftpBrowserAction::NavigateTo(path) = action {
            self.navigations.push(path.clone());
            ctx.notify();
        }
    }
}

impl View for BreadcrumbTestView {
    fn ui_name() -> &'static str {
        "BreadcrumbTestView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        for element in render_breadcrumb(&self.path, &self.mouse_handles, Appearance::as_ref(app)) {
            row.add_child(element);
        }
        Stack::new().with_child(row.finish()).finish()
    }
}

#[test]
fn breadcrumb_click_survives_rerender_between_mouse_down_and_up() {
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        let (window_id, view) =
            app.add_window(WindowStyle::NotStealFocus, |_| BreadcrumbTestView {
                path: PathBuf::from("/alpha/beta"),
                mouse_handles: HashMap::from([
                    (PathBuf::from("/"), MouseStateHandle::default()),
                    (PathBuf::from("/alpha"), MouseStateHandle::default()),
                ]),
                navigations: Vec::new(),
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
                    .get_position("sftp_breadcrumb:/alpha")
                    .expect("clickable breadcrumb segment should be positioned")
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
                view.navigations,
                vec![PathBuf::from("/alpha")],
                "breadcrumb navigation should fire exactly once across a rerender"
            );
        });
    });
}
