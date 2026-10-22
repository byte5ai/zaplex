//! Real keyboard routing for pane menus and their split launch destination.

use pathfinder_geometry::{rect::RectF, vector::vec2f};
use warpui::{
    integration::{AssertionOutcome, TestStep},
    App, WindowId,
};

use super::{
    native_workspace::bounds,
    view_getters::{
        pane_group_view, single_terminal_pane_view_for_tab, single_terminal_view, workspace_view,
    },
};

fn header_open(app: &App, window: WindowId) -> bool {
    single_terminal_pane_view_for_tab(app, window, 0)
        .read(app, |pane, ctx| pane.header().as_ref(ctx).is_overlay_open())
}

pub fn open() -> TestStep {
    TestStep::new("Open the visible pane header menu with its real button")
        .with_click_on_saved_position_fn(|app, window| {
            single_terminal_view(app, window).read(app, |terminal, _| {
                format!(
                    "pane_header_overflow_button:{}",
                    terminal.pane_configuration().id()
                )
            })
        })
        .add_named_assertion("The header menu is visible", |app, window| {
            if !header_open(app, window) {
                return AssertionOutcome::failure("Header menu did not open".into());
            }
            AssertionOutcome::Success
        })
}

pub fn cancel() -> TestStep {
    TestStep::new("Escape closes the menu and returns keyboard focus without a new pane")
        .with_keystrokes(&["escape"])
        .add_named_assertion(
            "Original terminal input is focused and the header menu is closed",
            |app, window| {
                let input_focused = single_terminal_view(app, window).read(app, |terminal, ctx| {
                    terminal.input().as_ref(ctx).editor().is_focused(ctx)
                });
                let count = pane_group_view(app, window, 0)
                    .read(app, |group, _| group.visible_pane_count());
                let split_open = workspace_view(app, window)
                    .read(app, |workspace, _| workspace.is_split_launch_menu_open());
                if header_open(app, window) || split_open || !input_focused || count != 1 {
                    return AssertionOutcome::failure(format!(
                        "Unexpected header/split/focus/panes: {}/{split_open}/{input_focused}/{count}",
                        header_open(app, window)
                    ));
                }
                AssertionOutcome::Success
            },
        )
}

pub fn choose_split() -> TestStep {
    TestStep::new("Select Split Right and observe the actual launch menu")
        .with_click_on_saved_position_fn(|_, _| crate::t!("keybinding-desc-pane-group-split-right"))
}

pub fn split_menu_layout() -> TestStep {
    TestStep::new("Check the complete split instruction at its actual pane-relative position")
        .add_named_assertion(
            "Full favorites instruction stays inside the native window",
            |app, window| {
                if !workspace_view(app, window)
                    .read(app, |workspace, _| workspace.is_split_launch_menu_open())
                {
                    return AssertionOutcome::failure("Split launch menu is not open".into());
                }
                let label = crate::t!("workspace-favorites-empty");
                let Some(hint) = bounds(app, window, &label) else {
                    return AssertionOutcome::failure(
                        "Split launch instruction not painted".into(),
                    );
                };
                let Some(end) = bounds(app, window, &format!("menu-label-end:{label}")) else {
                    return AssertionOutcome::failure("Instruction end not painted".into());
                };
                let viewport = RectF::new(
                    vec2f(0., 0.),
                    app.window_bounds(&window).expect("window").size(),
                );
                let fits = |outer: RectF, inner: RectF| {
                    inner.width() > 0.
                        && inner.height() > 0.
                        && inner.min_x() >= outer.min_x() - 1.
                        && inner.min_y() >= outer.min_y() - 1.
                        && inner.max_x() <= outer.max_x() + 1.
                        && inner.max_y() <= outer.max_y() + 1.
                };
                if !fits(viewport, hint) || !fits(hint, end) {
                    return AssertionOutcome::failure(format!(
                        "Split menu clips: hint={hint:?}, end={end:?}, viewport={viewport:?}"
                    ));
                }
                AssertionOutcome::Success
            },
        )
}
