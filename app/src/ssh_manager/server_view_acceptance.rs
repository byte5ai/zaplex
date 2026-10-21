//! Native form acceptance uses the real loaded view and its painted geometry.

use pathfinder_geometry::rect::RectF;
use warpui::integration::{AssertionOutcome, TestStep};
use warpui::SingletonEntity;

use super::{Appearance, StatusBanner};
use crate::integration_testing::{
    native_workspace::{bounds, LONG_HOST},
    ssh_manager::{create_server_via_db, ssh_server_view},
    view_getters::workspace_view,
};

pub fn open_form() -> TestStep {
    TestStep::new("Open a saved host with a long name beside the local shell")
        .with_action(|app, window_id, _| {
            let id = create_server_via_db(LONG_HOST, None);
            workspace_view(app, window_id).update(app, |workspace, ctx| {
                workspace.open_ssh_server(id, ctx);
            });
        })
        .add_named_assertion(
            "The actual server form finished loading",
            |app, window_id| {
                ssh_server_view(app, window_id).read(app, |view, _| {
                    if view.reload_in_flight || view.server.is_none() {
                        return AssertionOutcome::failure("Server form is still loading".into());
                    }
                    AssertionOutcome::Success
                })
            },
        )
}

pub fn draft() -> TestStep {
    TestStep::new("Keep an unsaved host draft selected across theme changes").with_action(
        |app, window_id, _| {
            ssh_server_view(app, window_id).update(app, |view, ctx| {
                view.status = None;
                view.host_editor.update(ctx, |editor, ctx| {
                    editor.set_buffer_text("bad host name", ctx);
                    ctx.focus_self();
                    editor.select_all(ctx);
                });
                ctx.notify();
            });
        },
    )
}

fn contains(outer: RectF, inner: RectF) -> bool {
    inner.width() > 0.
        && inner.height() > 0.
        && inner.min_x() >= outer.min_x() - 1.
        && inner.max_x() <= outer.max_x() + 1.
        && inner.min_y() >= outer.min_y() - 1.
        && inner.max_y() <= outer.max_y() + 1.
}

pub fn form_layout() -> TestStep {
    TestStep::new("Check painted form actions, title and retained editor state")
        .add_named_assertion(
            "Every action and the complete title fit in the pane",
            |app, window_id| {
                let Some(viewport) = bounds(app, window_id, "ssh-form-viewport") else {
                    return AssertionOutcome::failure("Form viewport is not painted".into());
                };
                for id in [
                    "ssh-form-title-end",
                    "ssh-form-test",
                    "ssh-form-connect",
                    "ssh-form-save",
                ] {
                    let Some(rect) = bounds(app, window_id, id) else {
                        return AssertionOutcome::failure(format!("{id} is not painted"));
                    };
                    if !contains(viewport, rect) {
                        return AssertionOutcome::failure(format!(
                            "{id} clips: {rect:?}, viewport={viewport:?}"
                        ));
                    }
                    if id != "ssh-form-title-end" && (rect.width() < 95. || rect.height() < 27.) {
                        return AssertionOutcome::failure(format!(
                            "{id} lost its full click target: {rect:?}"
                        ));
                    }
                }
                AssertionOutcome::Success
            },
        )
        .add_named_assertion(
            "Theme switching retains the draft, selection and editor focus",
            |app, window_id| {
                ssh_server_view(app, window_id).read(app, |view, ctx| {
                    for handle in [
                        &view.name_editor,
                        &view.host_editor,
                        &view.port_editor,
                        &view.user_editor,
                        &view.password_editor,
                    ] {
                        let matches = handle.read(ctx, |editor, ctx| {
                            let appearance = Appearance::as_ref(ctx);
                            editor.text_colors(appearance).default_color
                                == appearance.theme().active_ui_text_color()
                        });
                        if !matches {
                            return AssertionOutcome::failure(
                                "An existing field retains the old theme color".into(),
                            );
                        }
                    }
                    let retained = view.host_editor.read(ctx, |editor, ctx| {
                        editor.buffer_text(ctx) == "bad host name"
                            && editor.selected_text(ctx) == "bad host name"
                    });
                    if !retained || app.focused_view_id(window_id) != Some(view.host_editor.id()) {
                        return AssertionOutcome::failure(
                            "Theme or resize changed the unsaved draft, selection or focus".into(),
                        );
                    }
                    AssertionOutcome::Success
                })
            },
        )
}

pub fn reject_invalid_host(action: &'static str) -> TestStep {
    TestStep::new(&format!(
        "Click {action} and read the invalid-host explanation"
    ))
    .with_click_on_saved_position_fn(move |_, _| format!("ssh-form-{action}"))
    .add_named_assertion(
        "The clicked action explains the nonempty invalid host",
        |app, window_id| {
            ssh_server_view(app, window_id).read(app, |view, _| match &view.status {
                Some(StatusBanner::Error(message))
                    if message
                        == &crate::t!("workspace-left-panel-ssh-manager-error-host-invalid") =>
                {
                    AssertionOutcome::Success
                }
                _ => AssertionOutcome::failure(
                    "Expected the invalid-host explanation after clicking the action".into(),
                ),
            })
        },
    )
}
