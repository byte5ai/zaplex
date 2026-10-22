//! Native geometry and interaction checks for the terminal introduction.

use pathfinder_geometry::rect::RectF;
use warpui::{
    integration::{AssertionOutcome, TestStep},
    SingletonEntity,
};

use crate::{
    integration_testing::{native_workspace::bounds, view_getters::pane_group_view},
    terminal::settings::TerminalSettings,
};

pub fn layout() -> TestStep {
    TestStep::new("Keep every introduction hint and dismiss action inside the terminal")
        .add_named_assertion("Complete labels fit their actual pane", |app, window_id| {
            let Some(content) = bounds(app, window_id, "terminal-introduction-content") else {
                return AssertionOutcome::failure("Introduction not painted".into());
            };
            let terminal_pane = pane_group_view(app, window_id, 0).read(app, |group, _| {
                group
                    .visible_pane_ids()
                    .into_iter()
                    .find(|pane| pane.is_terminal_pane())
            });
            let Some(pane) =
                terminal_pane.and_then(|pane| bounds(app, window_id, &pane.position_id()))
            else {
                return AssertionOutcome::failure("Terminal pane not painted".into());
            };
            let fits = |outer: RectF, inner: RectF| {
                inner.width() > 0.
                    && inner.height() > 0.
                    && inner.min_x() >= outer.min_x() - 1.
                    && inner.max_x() <= outer.max_x() + 1.
                    && inner.min_y() >= outer.min_y() - 1.
                    && inner.max_y() <= outer.max_y() + 1.
            };
            if !fits(pane, content) {
                return AssertionOutcome::failure(format!(
                    "Introduction {content:?} exceeds pane {pane:?}"
                ));
            }
            let ids = [
                "terminal-introduction-title-end".to_owned(),
                "terminal-introduction-dismiss".to_owned(),
                format!(
                    "message-bar-text-end:{}",
                    crate::t!("terminal-zero-state-start-agent")
                ),
                format!(
                    "message-bar-text-end:{}",
                    crate::t!("terminal-zero-state-cycle-history")
                ),
                format!(
                    "message-bar-text-end:{}",
                    crate::t!("terminal-zero-state-open-code-review")
                ),
                format!(
                    "message-bar-text-end:{}",
                    crate::t!("terminal-zero-state-autodetect-prompts")
                ),
            ];
            for id in ids {
                let Some(rect) = bounds(app, window_id, &id) else {
                    return AssertionOutcome::failure(format!("Missing rendered hint: {id}"));
                };
                if !fits(content, rect) {
                    return AssertionOutcome::failure(format!(
                        "{id}: {rect:?} exceeds content {content:?}"
                    ));
                }
            }
            AssertionOutcome::Success
        })
}

pub fn dismiss() -> TestStep {
    TestStep::new("Dismiss the introduction using its visible action")
        .with_click_on_saved_position_fn(|_, _| "terminal-introduction-dismiss".into())
        .add_named_assertion(
            "Dismissal persists and removes the introduction",
            |app, window_id| {
                if app.update(|ctx| *TerminalSettings::as_ref(ctx).show_terminal_zero_state_block)
                    || bounds(app, window_id, "terminal-introduction-content").is_some()
                {
                    return AssertionOutcome::failure(
                        "Introduction still visible or enabled".into(),
                    );
                }
                AssertionOutcome::Success
            },
        )
}
