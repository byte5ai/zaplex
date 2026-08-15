//! Breadcrumb navigation rendering component
//!
//! Renders a clickable breadcrumb navigation based on the current path, supporting
//! step-by-step navigation to parent directories.
//! author: logic
//! date: 2026-05-26

use std::collections::HashMap;
use std::path::{Component, PathBuf};

use warp_core::ui::appearance::Appearance;
use warpui::elements::{
    ConstrainedBox, Container, Hoverable, MouseStateHandle, SavePosition, Text,
};
use warpui::platform::Cursor;
use warpui::Element;

use crate::sftp_manager::browser::SftpBrowserAction;
use crate::ui_components::icons::Icon;

/// Render the path breadcrumb navigation
///
/// Traverse each component of the path; each segment is clickable and triggers a NavigateTo action.
/// Segments are separated by ChevronRight icons; empty paths display "/".
pub fn render_breadcrumb(
    current_path: &PathBuf,
    mouse_handles: &HashMap<PathBuf, MouseStateHandle>,
    appearance: &Appearance,
) -> Vec<Box<dyn Element>> {
    let theme = appearance.theme();
    let text_color = theme.active_ui_text_color();
    let sub_color = theme.sub_text_color(theme.background());
    let ui_font = appearance.ui_font_family();
    let ui_font_size = appearance.ui_font_size();

    let components: Vec<_> = current_path
        .components()
        .filter(|c| !matches!(c, Component::RootDir))
        .collect();

    // When path is empty or only has root, display only "/"
    if components.is_empty() {
        let root_el = Text::new_inline(String::from("/"), ui_font, ui_font_size)
            .with_color(text_color.into())
            .finish();
        return vec![Container::new(root_el).finish()];
    }

    let mut elements: Vec<Box<dyn Element>> = Vec::new();
    let mut accumulated = PathBuf::new();

    for (i, comp) in components.iter().enumerate() {
        accumulated.push(comp);
        let is_last = i == components.len() - 1;

        // Separator (added after the first segment)
        if i > 0 {
            let sep_icon =
                ConstrainedBox::new(Icon::ChevronRight.to_warpui_icon(sub_color.into()).finish())
                    .with_width(12.0)
                    .with_height(12.0)
                    .finish();
            elements.push(
                Container::new(sep_icon)
                    .with_padding_left(2.0)
                    .with_padding_right(2.0)
                    .finish(),
            );
        }

        let segment_label = comp.as_os_str().to_string_lossy().to_string();
        let target_path = accumulated.clone();

        if is_last {
            // Last segment uses highlight color, not clickable
            let text_el = Text::new_inline(segment_label, ui_font, ui_font_size)
                .with_color(text_color.into())
                .finish();
            elements.push(Container::new(text_el).finish());
        } else {
            // Non-last segments are clickable for navigation
            let label_for_closure = segment_label.clone();
            let path = accumulated.display();
            let position_id = format!("sftp_breadcrumb:{path}");
            let Some(mouse_handle) = mouse_handles.get(&accumulated).cloned() else {
                continue;
            };
            let hoverable = Hoverable::new(mouse_handle, move |_| {
                let text_el = Text::new_inline(label_for_closure.clone(), ui_font, ui_font_size)
                    .with_color(sub_color.into())
                    .finish();
                Container::new(text_el).finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(SftpBrowserAction::NavigateTo(target_path.clone()));
            })
            .finish();
            let hit_target = ConstrainedBox::new(hoverable)
                .with_min_width(1.0)
                .with_min_height(ui_font_size)
                .finish();
            elements.push(SavePosition::new(hit_target, &position_id).finish());
        }
    }

    elements
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;

    use pathfinder_geometry::vector::vec2f;
    use warpui::elements::{CrossAxisAlignment, Flex, ParentElement, Stack};
    use warpui::platform::WindowStyle;
    use warpui::{
        App, AppContext, Entity, Event, Presenter, SingletonEntity, TypedActionView, View,
        ViewContext, WindowInvalidation,
    };

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
            for element in
                render_breadcrumb(&self.path, &self.mouse_handles, Appearance::as_ref(app))
            {
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
                    mouse_handles: HashMap::from([(
                        PathBuf::from("alpha"),
                        MouseStateHandle::default(),
                    )]),
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
                        .get_position("sftp_breadcrumb:alpha")
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
                    view.navigations.len(),
                    1,
                    "breadcrumb navigation should fire exactly once across a rerender"
                );
            });
        });
    }
}
