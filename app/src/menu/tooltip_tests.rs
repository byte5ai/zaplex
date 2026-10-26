use std::{cell::RefCell, rc::Rc};

use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use warpui::{
    elements::{ChildView, Container, Element, Flex, ParentElement, Stack},
    event::KeyEventDetails,
    keymap::Keystroke,
    platform::WindowStyle,
    App, AppContext, Entity, Event, Presenter, Scene, TypedActionView, View, ViewContext,
    ViewHandle, WindowId, WindowInvalidation,
};

use super::{Menu, MenuItem, MenuItemFields, SubMenu};
use crate::appearance::Appearance;

#[derive(Clone, Debug, PartialEq)]
enum Click {
    Primary,
    Child,
}

struct Root {
    menu: ViewHandle<Menu<Click>>,
    offset: Vector2F,
    unbounded_parent: bool,
    actions: Vec<Click>,
}

impl Entity for Root {
    type Event = ();
}

impl TypedActionView for Root {
    type Action = Click;

    fn handle_action(&mut self, action: &Click, _: &mut ViewContext<Self>) {
        self.actions.push(action.clone());
    }
}

impl View for Root {
    fn ui_name() -> &'static str {
        "MenuTooltipTestRoot"
    }

    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        let menu = Container::new(ChildView::new(&self.menu).finish())
            .with_padding_left(self.offset.x())
            .with_padding_top(self.offset.y())
            .finish();
        let child = if self.unbounded_parent {
            // A non-flexible row child receives an unbounded horizontal constraint.
            Flex::row().with_child(menu).finish()
        } else {
            menu
        };
        Stack::new().with_child(child).finish()
    }
}

fn render(app: &mut App, window: WindowId, presenter: &Rc<RefCell<Presenter>>) -> Rc<Scene> {
    let updated = app.read(|ctx| ctx.view_ids_for_window(window).into_iter().collect());
    app.update(|ctx| {
        let mut presenter = presenter.borrow_mut();
        presenter.invalidate(
            WindowInvalidation {
                updated,
                ..Default::default()
            },
            ctx,
        );
        presenter.build_scene(vec2f(1024., 768.), 1., None, ctx)
    })
}

fn send(app: &mut App, window: WindowId, presenter: &Rc<RefCell<Presenter>>, event: Event) {
    app.update(|ctx| {
        ctx.simulate_window_event(event, window, presenter.clone());
    });
}

fn move_to(
    app: &mut App,
    window: WindowId,
    presenter: &Rc<RefCell<Presenter>>,
    position: Vector2F,
) {
    send(
        app,
        window,
        presenter,
        Event::MouseMoved {
            position,
            cmd: false,
            shift: false,
            is_synthetic: false,
        },
    );
}

fn click(app: &mut App, window: WindowId, presenter: &Rc<RefCell<Presenter>>, position: Vector2F) {
    move_to(app, window, presenter, position);
    render(app, window, presenter);
    send(
        app,
        window,
        presenter,
        Event::LeftMouseDown {
            position,
            modifiers: Default::default(),
            click_count: 1,
            is_first_mouse: false,
        },
    );
    send(
        app,
        window,
        presenter,
        Event::LeftMouseUp {
            position,
            modifiers: Default::default(),
        },
    );
}

fn position(presenter: &Rc<RefCell<Presenter>>, id: String) -> RectF {
    presenter
        .borrow()
        .position_cache()
        .get_position(&id)
        .unwrap_or_else(|| panic!("missing menu position: {id}"))
}

#[test]
fn test_split_submenu_tooltip_paints_above_flyout_without_stealing_pointer() {
    crate::i18n::init(Some("en"));
    App::test((), |mut app| async move {
        app.add_singleton_model(|_| Appearance::mock());
        app.update(super::init);
        for (offset, unbounded_parent) in [
            (vec2f(20., 20.), false),
            (vec2f(780., 700.), false),
            (vec2f(20., 20.), true),
            (vec2f(780., 700.), true),
        ] {
            let (window, root) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
                let menu = ctx.add_typed_action_view(|ctx| {
                    let mut menu = Menu::new().with_ignore_hover_when_covered();
                    menu.set_width(220.);
                    menu.set_origin(Some(offset));
                    menu.set_items(
                        vec![MenuItem::Submenu {
                            fields: MenuItemFields::new_submenu(
                                "production-host-with-a-long-unambiguous-identity",
                            )
                            .with_on_select_action(Click::Primary)
                            .with_split_submenu_trigger(
                                "More actions for production-host-with-a-long-unambiguous-identity",
                            ),
                            menu: SubMenu::new(vec![MenuItemFields::new("Child action")
                                .with_on_select_action(Click::Child)
                                .into_item()]),
                        }],
                        ctx,
                    );
                    menu
                });
                Root {
                    menu,
                    offset,
                    unbounded_parent,
                    actions: Vec::new(),
                }
            });
            app.read(|ctx| ctx.windows().show_window_and_focus_app(window));
            let menu = app.read(|ctx| root.as_ref(ctx).menu.clone());
            menu.update(&mut app, |_, ctx| ctx.focus_self());
            assert_eq!(app.focused_view_id(window), Some(menu.id()));
            let namespace = app.read(|ctx| menu.as_ref(ctx).submenu_position_namespace);
            let presenter = Rc::new(RefCell::new(Presenter::new(window)));
            render(&mut app, window, &presenter);
            let row = position(
                &presenter,
                SubMenu::<Click>::save_position_id(namespace, 0, 0),
            );
            let trigger = vec2f(row.max_x() - 14., row.center().y());
            for _ in 0..3 {
                move_to(&mut app, window, &presenter, trigger);
                let scene = render(&mut app, window, &presenter);
                assert!(app.read(|ctx| menu.as_ref(ctx).menu.has_visible_submenu_at_depth(0)));
                let flyout = position(
                    &presenter,
                    SubMenu::<Click>::panel_position_id(namespace, 1),
                );
                assert_eq!(flyout.min_x() < row.min_x(), offset.x() > 500.);
                let layers: Vec<_> = scene.layers().collect();
                let tooltip_index = layers
                    .iter()
                    .rposition(|layer| layer.click_through && !layer.rects.is_empty())
                    .expect("hovered identity must have a click-through overlay");
                let flyout_index = layers
                    .iter()
                    .rposition(|layer| {
                        !layer.click_through
                            && layer
                                .rects
                                .iter()
                                .any(|rect| rect.bounds.contains_point(flyout.center()))
                    })
                    .expect("flyout must have an interactive painted layer");
                assert!(
                    tooltip_index > flyout_index,
                    "identity must paint above flyout"
                );
            }
            send(
                &mut app,
                window,
                &presenter,
                Event::KeyDown {
                    keystroke: Keystroke::parse("escape").unwrap(),
                    chars: String::new(),
                    details: KeyEventDetails::default(),
                    is_composing: false,
                },
            );
            assert!(!app.read(|ctx| menu.as_ref(ctx).menu.has_visible_submenu_at_depth(0)));
            assert!(app.read(|ctx| root.as_ref(ctx).actions.is_empty()));
            render(&mut app, window, &presenter);
            // Clicking the trailing target reopens the child; it never connects.
            click(&mut app, window, &presenter, trigger);
            render(&mut app, window, &presenter);
            assert!(app.read(|ctx| root.as_ref(ctx).actions.is_empty()));
            let child = position(
                &presenter,
                SubMenu::<Click>::save_position_id(namespace, 1, 0),
            );
            click(&mut app, window, &presenter, child.center());
            assert_eq!(
                app.read(|ctx| root.as_ref(ctx).actions.clone()),
                vec![Click::Child]
            );
            render(&mut app, window, &presenter);
            click(
                &mut app,
                window,
                &presenter,
                vec2f(row.min_x() + 20., row.center().y()),
            );
            assert_eq!(
                app.read(|ctx| root.as_ref(ctx).actions.clone()),
                vec![Click::Child, Click::Primary]
            );
        }
    });
}
