//! Native acceptance steps for the workspace. Fixtures use isolated local data;
//! they do not claim live SSH, daemon, or provider acceptance.

use std::sync::{Arc, Mutex};

use pathfinder_geometry::{rect::RectF, vector::vec2f};
use warpui::{
    event::Event,
    integration::{AssertionOutcome, StepData, TestStep},
    windowing::WindowManager,
    App, EntityId, SingletonEntity,
};

use crate::{
    cockpit::favorites::FavoritesStore,
    integration_testing::{
        ssh_manager::create_server_via_db,
        view_getters::{pane_group_view, workspace_view},
    },
    ssh_manager::{SshTreeChangedEvent, SshTreeChangedNotifier},
    terminal::resizable_data::{ModalType, ResizableData},
    workspace::WorkspaceAction,
};

pub const LONG_HOST: &str = "production-europe-west-development-0123456789";
const HOST_IDS: &str = "native_workspace_host_ids";
const HOST_GEOMETRY: &str = "native_workspace_host_geometry";
const MENU_FOCUS: &str = "native_workspace_menu_focus";

pub fn bounds(app: &App, window_id: warpui::WindowId, id: &str) -> Option<RectF> {
    app.presenter(window_id)?
        .borrow()
        .position_cache()
        .get_position(id)
}

fn contains(outer: RectF, inner: RectF) -> bool {
    inner.width() > 0.
        && inner.height() > 0.
        && inner.min_x() >= outer.min_x() - 1.
        && inner.max_x() <= outer.max_x() + 1.
        && inner.min_y() >= outer.min_y() - 1.
        && inner.max_y() <= outer.max_y() + 1.
}

pub fn resize(width: f32, height: f32) -> TestStep {
    TestStep::new(&format!("Resize native workspace to {width}x{height}"))
        .with_action(move |app, window_id, _| {
            let origin = app
                .window_bounds(&window_id)
                .expect("window bounds")
                .origin();
            app.read(|ctx| {
                WindowManager::as_ref(ctx)
                    .set_window_bounds(window_id, RectF::new(origin, vec2f(width, height)))
            });
        })
        .add_named_assertion(
            "Native window reached requested geometry",
            move |app, window_id| {
                let rect = app.window_bounds(&window_id).expect("window bounds");
                if (rect.width() - width).abs() > 1. || (rect.height() - height).abs() > 1. {
                    return AssertionOutcome::failure(format!(
                        "Expected {width}x{height}, got {rect:?}"
                    ));
                }
                AssertionOutcome::Success
            },
        )
}

pub fn seed_connections() -> TestStep {
    TestStep::new("Seed two saved hosts without connecting to either endpoint").with_action(
        |app, window_id, data| {
            let ids = vec![
                create_server_via_db(LONG_HOST, None),
                create_server_via_db("ci-short-host", None),
            ];
            app.update(|ctx| {
                SshTreeChangedNotifier::handle(ctx)
                    .update(ctx, |_, ctx| ctx.emit(SshTreeChangedEvent::TreeChanged));
            });
            let workspace = workspace_view(app, window_id);
            app.dispatch_typed_action(
                window_id,
                &[workspace.id()],
                &WorkspaceAction::OpenSshManager,
            );
            app.dispatch_typed_action(
                window_id,
                &[workspace.id()],
                &WorkspaceAction::ToggleFavorite {
                    kind: zaplex_cockpit::FavoriteKind::Host,
                    target: ids[0].clone(),
                    label: LONG_HOST.to_string(),
                },
            );
            data.insert(HOST_IDS, ids);
        },
    )
}

pub fn connection_layout(panel_width: f32, save: bool) -> TestStep {
    TestStep::new(&format!("Assert native connection identities at {panel_width}px"))
        .with_action(move |app, window_id, _| {
            let workspace = workspace_view(app, window_id);
            app.update(|ctx| {
                let handle = ResizableData::as_ref(ctx).get_handle(window_id, ModalType::LeftPanelWidth).expect("panel sizing handle");
                handle.lock().expect("panel sizing lock").set_size(panel_width);
                workspace.update(ctx, |_, ctx| ctx.notify());
            });
        })
        .add_named_assertion_with_data_from_prior_step("Long and short hosts retain the same action gutter", move |app, window_id, data| {
            let ids = data.get::<_, Vec<String>>(HOST_IDS).expect("seeded host ids");
            let Some(panel) = bounds(app, window_id, "ssh_manager_panel_root") else {
                return AssertionOutcome::failure("Connections panel has not painted".into());
            };
            let mut geometry = Vec::new();
            for id in ids {
                let Some(row) = bounds(app, window_id, &format!("ssh-manager-node:{id}")) else {
                    return AssertionOutcome::failure(format!("Host {id} has not painted"));
                };
                let Some(title) = bounds(app, window_id, &format!("ssh-manager-node:{id}:title")) else {
                    return AssertionOutcome::failure(format!("Host {id} has no title bounds"));
                };
                if !contains(panel, row) || !contains(row, title) || title.width() < 80. || row.max_x() - title.max_x() < 40. {
                    return AssertionOutcome::failure(format!("Identity or action gutter clipped: panel={panel:?}, row={row:?}, title={title:?}"));
                }
                geometry.push((row, title));
            }
            if (geometry[0].1.max_x() - geometry[1].1.max_x()).abs() > 1. {
                return AssertionOutcome::failure("Host length changes the action gutter".into());
            }
            if save {
                return AssertionOutcome::SuccessWithData(StepData::new(HOST_GEOMETRY, geometry));
            }
            let prior = data.get::<_, Vec<(RectF, RectF)>>(HOST_GEOMETRY).expect("saved host geometry");
            if prior != &geometry {
                return AssertionOutcome::failure("Hover changed connection row geometry".into());
            }
            AssertionOutcome::Success
        })
}

pub fn hover_first_connection() -> TestStep {
    TestStep::new("Hover the long native host identity").with_event_fn(|app, window_id| {
        let id = app.read(|ctx| FavoritesStore::as_ref(ctx).items()[0].target.clone());
        Event::MouseMoved {
            position: bounds(app, window_id, &format!("ssh-manager-node:{id}:title"))
                .expect("painted host")
                .center(),
            cmd: false,
            shift: false,
            is_synthetic: false,
        }
    })
}

pub fn open_favorites() -> TestStep {
    TestStep::new("Open real workspace favorites menu near the right window edge")
        .with_action(|app, window_id, _| {
            let rect = app.window_bounds(&window_id).expect("window bounds");
            let workspace = workspace_view(app, window_id);
            app.dispatch_typed_action(
                window_id,
                &[workspace.id()],
                &WorkspaceAction::OpenNewSessionMenu {
                    position: vec2f(rect.width() - 24., 80.),
                },
            );
        })
        .add_named_assertion_with_data_from_prior_step(
            "Favorite label and ellipsis are separate visible targets",
            |app, window_id, _| {
                let Some(primary) = bounds(app, window_id, LONG_HOST) else {
                    return AssertionOutcome::failure("Favorite label not painted".into());
                };
                let Some(trigger) = bounds(app, window_id, &favorite_trigger()) else {
                    return AssertionOutcome::failure("Favorite ellipsis not painted".into());
                };
                if primary.max_x() > trigger.min_x() + 1.
                    || (primary.center().y() - trigger.center().y()).abs() > 1.
                {
                    return AssertionOutcome::failure(format!(
                        "Favorite targets overlap or wrap: {primary:?}, {trigger:?}"
                    ));
                }
                AssertionOutcome::SuccessWithData(StepData::new(
                    MENU_FOCUS,
                    app.focused_view_id(window_id).expect("focused menu"),
                ))
            },
        )
}

pub fn favorite_trigger() -> String {
    crate::t!("workspace-favorite-more-actions", host = LONG_HOST)
}

pub fn favorite_flyout() -> TestStep {
    TestStep::new("Open the actual favorite flyout, retaining its parent")
        .with_click_on_saved_position_fn(|_, _| favorite_trigger())
        .add_named_assertion("Parent and child actions stay visible within the native viewport", |app, window_id| {
            let Some(parent) = bounds(app, window_id, LONG_HOST) else {
                return AssertionOutcome::failure("Favorite parent missing".into());
            };
            let Some(child) = bounds(app, window_id, &crate::t!("cockpit-tt-favorite-remove")) else {
                return AssertionOutcome::failure("Favorite child missing".into());
            };
            let viewport = RectF::new(vec2f(0., 0.), app.window_bounds(&window_id).expect("window bounds").size());
            if !contains(viewport, parent) || !contains(viewport, child) || (child.min_x() < parent.max_x() && parent.min_x() < child.max_x()) {
                return AssertionOutcome::failure(format!("Flyout clips or covers its parent: {parent:?}, {child:?}, viewport={viewport:?}"));
            }
            let tabs = workspace_view(app, window_id).read(app, |view, _| view.tab_count());
            if tabs != 1 { return AssertionOutcome::failure("Ellipsis started a terminal session".into()); }
            AssertionOutcome::Success
        })
}

pub fn dismiss_flyout() -> TestStep {
    TestStep::new("Escape returns focus to the parent favorites menu")
        .with_keystrokes(&["escape"])
        .add_named_assertion_with_data_from_prior_step(
            "Parent menu retains keyboard focus and no session was opened",
            |app, window_id, data| {
                let expected = data.get::<_, EntityId>(MENU_FOCUS).expect("parent focus");
                if app.focused_view_id(window_id).as_ref() != Some(expected) {
                    return AssertionOutcome::failure(
                        "Escape did not retain the parent menu focus".into(),
                    );
                }
                if workspace_view(app, window_id).read(app, |view, _| view.tab_count()) != 1 {
                    return AssertionOutcome::failure(
                        "Favorite secondary interaction opened a session".into(),
                    );
                }
                AssertionOutcome::Success
            },
        )
}

fn terminal_at(
    app: &App,
    window_id: warpui::WindowId,
    index: usize,
) -> warpui::ViewHandle<crate::terminal::TerminalView> {
    pane_group_view(app, window_id, 0).read(app, |group, ctx| {
        group
            .terminal_view_at_pane_index(index, ctx)
            .expect("terminal pane")
    })
}

fn overflow_id(app: &App, window_id: warpui::WindowId, index: usize) -> String {
    terminal_at(app, window_id, index).read(app, |view, _| {
        format!(
            "pane_header_overflow_button:{}",
            view.pane_configuration().id()
        )
    })
}

pub fn split_local_right() -> TestStep {
    TestStep::new("Split the real pane through its overflow menu and choose local")
        .with_click_on_saved_position_fn(|app, window_id| overflow_id(app, window_id, 0))
        .with_click_on_saved_position_fn(|_, _| crate::t!("keybinding-desc-pane-group-split-right"))
        .with_click_on_saved_position_fn(|_, _| crate::t!("workspace-new-session-terminal"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TerminalIdentity {
    pane: crate::pane_group::pane::PaneId,
    terminal: EntityId,
    session: String,
    draft: String,
}

fn terminal_identities(app: &App, window_id: warpui::WindowId) -> Vec<TerminalIdentity> {
    pane_group_view(app, window_id, 0).read(app, |group, ctx| {
        group
            .visible_pane_ids()
            .into_iter()
            .filter(|pane| pane.is_terminal_pane())
            .map(|pane| {
                let terminal = group
                    .terminal_view_from_pane_id(pane, ctx)
                    .expect("terminal view");
                terminal.read(ctx, |view, ctx| TerminalIdentity {
                    pane,
                    terminal: terminal.id(),
                    session: format!("{:?}", view.active_block_session_id()),
                    draft: view.input().read(ctx, |input, ctx| input.buffer_text(ctx)),
                })
            })
            .collect()
    })
}

pub fn assert_split_and_save_identity(expected_draft: &'static str) -> TestStep {
    TestStep::new("Verify two real panes and save their live session identities")
        .add_named_assertion(
            "Local split is side by side with equal full content height",
            move |app, window_id| {
                let identities = terminal_identities(app, window_id);
                if identities.len() != 2
                    || identities.iter().any(|identity| identity.session == "None")
                {
                    return AssertionOutcome::failure(format!(
                        "Expected two terminals, got {}",
                        identities.len()
                    ));
                }
                if !identities
                    .iter()
                    .any(|identity| identity.draft == expected_draft)
                {
                    return AssertionOutcome::failure(format!(
                        "The entered draft was not present before the interaction: {identities:?}"
                    ));
                }
                let Some(left) = bounds(app, window_id, &identities[0].pane.position_id()) else {
                    return AssertionOutcome::failure("Left pane not painted".into());
                };
                let Some(right) = bounds(app, window_id, &identities[1].pane.position_id()) else {
                    return AssertionOutcome::failure("Right pane not painted".into());
                };
                if left.max_x() > right.min_x() + 1.
                    || (left.min_y() - right.min_y()).abs() > 1.
                    || (left.height() - right.height()).abs() > 1.
                    || left.height() < 300.
                {
                    return AssertionOutcome::failure(format!(
                        "Invalid native split geometry: {left:?}, {right:?}"
                    ));
                }
                AssertionOutcome::SuccessWithData(warpui::integration::StepData::new(
                    "native_terminal_identities",
                    identities,
                ))
            },
        )
}

pub fn assert_focused_directory_title(directory: &'static str) -> TestStep {
    TestStep::new("Verify actual shell directory propagates to pane and tab titles")
        .add_named_assertion("Focused pane title and automatic tab title follow the real cd", move |app, window_id| {
            pane_group_view(app, window_id, 0).read(app, |group, ctx| {
                let terminal = group.focused_session_view(ctx).expect("focused terminal");
                let title = terminal.as_ref(ctx).pane_configuration().as_ref(ctx).title();
                if !title.contains(directory) || group.display_title(ctx) != title {
                    return AssertionOutcome::failure(format!("Expected cwd {directory} in focused pane and tab; pane={title:?}, tab={:?}", group.display_title(ctx)));
                }
                AssertionOutcome::Success
            })
        })
}

pub fn drag_first_pane_below_second() -> Vec<TestStep> {
    let points = Arc::new(Mutex::new((vec2f(0., 0.), vec2f(0., 0.))));
    let setup_points = points.clone();
    let mut drag = TestStep::new("Drag the painted pane header below the other pane");
    for phase in 0..4 {
        let points = points.clone();
        drag = drag.with_event_fn(move |_, _| {
            let (start, target) = *points.lock().expect("drag coordinates");
            match phase {
                0 => Event::LeftMouseDown {
                    position: start,
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                1 => Event::LeftMouseDragged {
                    position: start + vec2f(12., 12.),
                    modifiers: Default::default(),
                },
                2 => Event::LeftMouseDragged {
                    position: target,
                    modifiers: Default::default(),
                },
                3 => Event::LeftMouseUp {
                    position: target,
                    modifiers: Default::default(),
                },
                _ => unreachable!("four mouse phases"),
            }
        });
    }
    vec![
        TestStep::new("Measure real header and pane drop positions").with_action(move |app, window_id, _| {
            let identities = terminal_identities(app, window_id);
            let source = bounds(app, window_id, &identities[0].pane.position_id()).expect("source pane bounds");
            let header = bounds(app, window_id, &overflow_id(app, window_id, 0)).expect("native header bounds");
            let target = bounds(app, window_id, &identities[1].pane.position_id()).expect("target pane bounds");
            *setup_points.lock().expect("drag coordinates") = (vec2f(source.center().x(), header.center().y()), vec2f(target.center().x(), target.max_y() - 35.));
        }),
        drag.add_named_assertion_with_data_from_prior_step("Mouse drop changes geometry while keeping both sessions and drafts", |app, window_id, data| {
            let before = data.get::<_, Vec<TerminalIdentity>>("native_terminal_identities").expect("original identities");
            let after = terminal_identities(app, window_id);
            if after.len() != before.len() || before.iter().any(|identity| !after.contains(identity)) {
                return AssertionOutcome::failure(format!("Drag replaced a session or lost its draft: before={before:?}, after={after:?}"));
            }
            let Some(moved) = bounds(app, window_id, &before[0].pane.position_id()) else {
                return AssertionOutcome::failure("Moved pane missing".into());
            };
            let Some(target) = bounds(app, window_id, &before[1].pane.position_id()) else {
                return AssertionOutcome::failure("Target pane missing".into());
            };
            if moved.min_y() < target.max_y() - 1. || (moved.min_x() - target.min_x()).abs() > 1. || (moved.width() - target.width()).abs() > 1. {
                return AssertionOutcome::failure(format!("Mouse drop did not arrange panes vertically: {moved:?}, {target:?}"));
            }
            AssertionOutcome::Success
        })
    ]
}

pub fn open_local_file_manager(index: usize) -> TestStep {
    TestStep::new(&format!(
        "Switch terminal {index} to its own local file manager"
    ))
    .with_action(move |app, window_id, data| {
        let directory = tempfile::Builder::new()
            .prefix("zaplex native FM ' ")
            .tempdir()
            .expect("isolated file-manager directory");
        std::fs::write(
            directory.path().join("acceptance-file.txt"),
            "native file-manager fixture\n",
        )
        .expect("fixture file");
        std::fs::create_dir(directory.path().join("projects")).expect("fixture directory");
        let group = pane_group_view(app, window_id, 0);
        group.update(app, |group, ctx| {
            let pane = group.pane_id_from_index(index).expect("invoking pane");
            group.focus_pane_by_id(pane, ctx);
        });
        let workspace = workspace_view(app, window_id);
        app.dispatch_typed_action(
            window_id,
            &[workspace.id()],
            &WorkspaceAction::OpenLocalFileManager {
                start_path: directory.path().to_path_buf(),
            },
        );
        data.insert(format!("native_fm_directory_{index}"), directory);
    })
    .add_named_assertion(
        "File-manager mode replaced exactly the invoking pane",
        move |app, window_id| {
            let panes = app.read(|ctx| {
                crate::sftp_manager::fm_registry::FileManagerRegistry::as_ref(ctx)
                    .panes()
                    .len()
            });
            let visible =
                pane_group_view(app, window_id, 0).read(app, |group, _| group.visible_pane_count());
            if panes != index + 1 || visible != 2 {
                return AssertionOutcome::failure(format!(
                    "Expected {} file managers and two visible panes; got {panes}/{visible}",
                    index + 1
                ));
            }
            AssertionOutcome::Success
        },
    )
}

fn fm_position(app: &App, index: usize, part: &str) -> String {
    app.read(|ctx| {
        let id =
            crate::sftp_manager::fm_registry::FileManagerRegistry::as_ref(ctx).panes()[index].id;
        format!("sftp_layout:{id}:{part}")
    })
}

pub fn file_manager_layout() -> TestStep {
    TestStep::new("Check both native file-manager legends and file rows")
        .add_named_assertion("Every function key stays on one line within its own pane", |app, window_id| {
            for index in 0..2 {
                let Some(root) = bounds(app, window_id, &fm_position(app, index, "pane-root")) else {
                return AssertionOutcome::failure("File manager has not painted".into());
            };
                let compact = bounds(app, window_id, &fm_position(app, index, "legend-compact"));
                let legend = compact
                    .or_else(|| bounds(app, window_id, &fm_position(app, index, "legend-full")));
                let Some(legend) = legend else {
                return AssertionOutcome::failure("Function legend has not painted".into());
            };
                let mut previous: Option<RectF> = None;
                for key in ["F2", "F3", "F4", "F5", "F6", "F7", "F8", "F10"] {
                    let Some(cell) = bounds(app, window_id, &fm_position(app, index, &format!("function-{key}"))) else {
                return AssertionOutcome::failure(format!("Missing {key} in pane {index}"));
            };
                    if !contains(root, legend) || !contains(legend, cell) || previous.is_some_and(|prior| prior.max_x() > cell.min_x() + 1. || (prior.center().y() - cell.center().y()).abs() > 1.) {
                        let window = app.window_bounds(&window_id);
                        return AssertionOutcome::failure(format!("Function bar overlaps or wraps: index={index}, key={key}, compact={}, window={window:?}, pane={root:?}, legend={legend:?}, cell={cell:?}, previous={previous:?}", compact.is_some()));
                    }
                    previous = Some(cell);
                }
            }
            let browsers = app.views_of_type::<crate::sftp_manager::browser::SftpBrowserView>(window_id).expect("file-manager views");
            if browsers.len() != 2 || browsers.iter().any(|browser| !browser.read(app, |view, _| view.entries().iter().any(|entry| entry.name == "acceptance-file.txt"))) {
                return AssertionOutcome::failure("Actual local fixture files are not visible in both panes".into());
            }
            AssertionOutcome::Success
        })
}

pub fn inactive_file_manager_action() -> TestStep {
    TestStep::new("An inactive pane's function legend cannot create a folder")
        .with_click_on_saved_position_fn(|app, _| fm_position(app, 0, "function-F7"))
        .add_named_assertion("Inactive F7 opens no dialog", |app, window_id| {
            let browsers = app
                .views_of_type::<crate::sftp_manager::browser::SftpBrowserView>(window_id)
                .expect("file-manager views");
            if browsers
                .iter()
                .any(|browser| browser.read(app, |view, _| view.dialog().is_some()))
            {
                return AssertionOutcome::failure(
                    "Inactive function legend executed Create Folder".into(),
                );
            }
            AssertionOutcome::Success
        })
}

pub fn focused_file_manager_action() -> TestStep {
    TestStep::new("Focus the first file manager and invoke F7 through the keyboard")
        .with_click_on_saved_position_fn(|app, _| fm_position(app, 0, "body"))
        .with_keystrokes(&["f7"])
        .add_named_assertion(
            "Focused F7 opens exactly one Create Folder dialog",
            |app, window_id| {
                let browsers = app
                    .views_of_type::<crate::sftp_manager::browser::SftpBrowserView>(window_id)
                    .expect("file-manager views");
                let count = browsers
                    .iter()
                    .filter(|browser| {
                        browser.read(app, |view, _| {
                            matches!(
                                view.dialog(),
                                Some(crate::sftp_manager::types::Dialog::CreateFolder { .. })
                            )
                        })
                    })
                    .count();
                if count != 1 {
                    return AssertionOutcome::failure(format!(
                        "Expected one Create Folder dialog, got {count}"
                    ));
                }
                AssertionOutcome::Success
            },
        )
}

pub fn close_file_manager(remaining: usize) -> TestStep {
    TestStep::new("F10 returns to the retained terminal without recreating it")
        .with_keystrokes(&["f10"])
        .add_named_assertion(
            "File-manager registry and pane count reflect the mode switch",
            move |app, window_id| {
                let count = app.read(|ctx| {
                    crate::sftp_manager::fm_registry::FileManagerRegistry::as_ref(ctx)
                        .panes()
                        .len()
                });
                let visible = pane_group_view(app, window_id, 0)
                    .read(app, |group, _| group.visible_pane_count());
                if count != remaining || visible != 2 {
                    return AssertionOutcome::failure(format!(
                        "Expected {remaining} managers / two panes, got {count}/{visible}"
                    ));
                }
                AssertionOutcome::Success
            },
        )
}

pub fn focus_remaining_file_manager() -> TestStep {
    TestStep::new("Focus the remaining file manager")
        .with_click_on_saved_position_fn(|app, _| fm_position(app, 0, "body"))
}

pub fn assert_retained_terminals() -> TestStep {
    TestStep::new("Both file-manager modes returned to their original sessions and drafts")
        .add_named_assertion_with_data_from_prior_step(
            "Pane, terminal, shell session and draft identities are preserved",
            |app, window_id, data| {
                let before = data
                    .get::<_, Vec<TerminalIdentity>>("native_terminal_identities")
                    .expect("terminal identities");
                let after = terminal_identities(app, window_id);
                if before.len() != after.len()
                    || before.iter().any(|identity| !after.contains(identity))
                {
                    return AssertionOutcome::failure(format!(
                        "Mode switch replaced a terminal or lost a draft: {before:?} -> {after:?}"
                    ));
                }
                for (index, identity) in before.iter().enumerate() {
                    let directory = data
                        .get::<_, tempfile::TempDir>(format!("native_fm_directory_{index}"))
                        .expect("retained file-manager directory");
                    let expected = directory.path().canonicalize().expect("fixture directory");
                    let terminal = pane_group_view(app, window_id, 0).read(app, |group, ctx| {
                        group
                            .terminal_view_from_pane_id(identity.pane, ctx)
                            .expect("retained terminal")
                    });
                    let actual = terminal.read(app, |view, ctx| {
                        view.active_session_cwd(ctx)
                            .and_then(|path| path.canonicalize().ok())
                    });
                    if actual.as_ref() != Some(&expected) {
                        return AssertionOutcome::failure(format!(
                            "F10 did not change the original shell directory: pane={:?}, expected={expected:?}, actual={actual:?}",
                            identity.pane
                        ));
                    }
                }
                AssertionOutcome::Success
            },
        )
}

pub fn seed_session_inventory() -> TestStep {
    TestStep::new("Apply labelled deterministic Zaplex, tmux and byobu layout inventory")
        .with_action(|app, window_id, data| {
            use crate::remote_server::session_inventory::{
                HostSessionInventory, RoutedDaemonSession,
            };
            use remote_server::proto::{
                MultiplexerKind, MultiplexerSessionInfo, MultiplexerSessionList, SessionInfo,
            };
            let node_id = data.get::<_, Vec<String>>(HOST_IDS).expect("host ids")[0].clone();
            let sessions = [
                SessionInfo {
                    session_id: "fixture-long-title".into(),
                    title:
                        "Acceptance fixture: extremely long session title for the minimum sidebar"
                            .into(),
                    ring_bytes: 223_232,
                    ..Default::default()
                },
                SessionInfo {
                    session_id: "fixture-no-title".into(),
                    cwd: "/srv/projects/acceptance-with-long-directory-identity".into(),
                    ring_bytes: 1_048_576,
                    ..Default::default()
                },
                SessionInfo {
                    session_id: "fixture-last-opened".into(),
                    cwd: "/srv/projects/detached-worker".into(),
                    last_attached_epoch_millis: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |elapsed| elapsed.as_millis() as u64)
                        .saturating_sub(3 * 3_600_000),
                    ..Default::default()
                },
            ]
            .into_iter()
            .map(|session| RoutedDaemonSession {
                session,
                route: None,
                host_id: None,
                daemon_runtime: None,
            })
            .collect();
            let multiplexers = [MultiplexerKind::Tmux, MultiplexerKind::ByobuTmux]
                .into_iter()
                .enumerate()
                .map(|(index, kind)| MultiplexerSessionInfo {
                    name: format!("Acceptance fixture {kind:?}: long persistent shell session"),
                    target: format!("fixture-mux-{index}"),
                    kind: kind as i32,
                    windows: 12,
                    attached_clients: 2,
                })
                .collect();
            let panel = app
                .views_of_type::<crate::ssh_manager::panel::SshManagerPanel>(window_id)
                .expect("panel views")
                .into_iter()
                .next()
                .expect("connections panel");
            let keys = panel.update(app, |panel, ctx| {
                panel.apply_layout_inventory(
                    &node_id,
                    HostSessionInventory {
                        sessions,
                        multiplexers: MultiplexerSessionList {
                            sessions: multiplexers,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    ctx,
                )
            });
            data.insert("native_session_keys", keys);
        })
}

pub fn session_layout() -> TestStep {
    TestStep::new("Validate native session identity, action slots and semantic metadata")
        .add_named_assertion_with_data_from_prior_step(
            "Fixture session identities and semantic metadata remain visible",
            |app, window_id, data| {
                let keys = data.get::<_, Vec<String>>("native_session_keys")
                    .expect("inventory keys");
                let Some(panel) = bounds(app, window_id, "ssh_manager_panel_root") else {
                    return AssertionOutcome::failure("Panel not painted".into());
                };
                for (index, key) in keys.iter().enumerate() {
                    let position = |part| {
                        bounds(app, window_id, &format!("ssh-manager-session:{key}:{part}"))
                    };
                    let positions = (position("row"), position("title"), position("open"));
                    let (Some(row), Some(title), Some(open)) = positions else {
                        return AssertionOutcome::failure(format!(
                            "Session fixture {index} has missing identity/action bounds"
                        ));
                    };
                    if !contains(panel, row)
                        || !contains(row, title)
                        || !contains(row, open)
                        || title.max_x() > open.min_x() + 1.
                        || title.width() < 60.
                    {
                        return AssertionOutcome::failure(format!(
                            "Session {index} identity/action collision: {row:?}, {title:?}, {open:?}"
                        ));
                    }
                    // Rows 0 and 1 were never attached; row 2 shows its last
                    // attach, and the multiplexer rows their window/client counts.
                    if (index < 2) == position("metadata").is_some() {
                        return AssertionOutcome::failure(format!(
                            "Session {index}: unexpected metadata line presence"
                        ));
                    }
                }
                AssertionOutcome::Success
            },
        )
}

pub fn theme(light: bool) -> TestStep {
    TestStep::new(if light {
        "Render the production Light theme"
    } else {
        "Render the production Zaplex Dark theme"
    })
    .with_action(move |app, _, _| {
        let theme = if light {
            crate::themes::theme::ThemeKind::Light
        } else {
            crate::themes::theme::ThemeKind::ZaplexDark
        };
        crate::appearance::AppearanceManager::handle(app)
            .update(app, |manager, ctx| manager.set_transient_theme(theme, ctx));
    })
}

pub fn custom_tab_title() -> TestStep {
    TestStep::new("An explicit tab title survives a focus change between real panes")
        .with_action(|app, window_id, _| {
            let workspace = workspace_view(app, window_id);
            app.dispatch_typed_action(
                window_id,
                &[workspace.id()],
                &WorkspaceAction::SetActiveTabName("Acceptance custom title".to_string()),
            );
            pane_group_view(app, window_id, 0).update(app, |group, ctx| {
                let pane = group.pane_id_from_index(0).expect("first pane");
                group.focus_pane_by_id(pane, ctx);
            });
        })
        .add_named_assertion(
            "Explicit tab title is retained when another pane receives focus",
            |app, window_id| {
                pane_group_view(app, window_id, 0).read(app, |group, ctx| {
                    if group.display_title(ctx) != "Acceptance custom title" {
                        return AssertionOutcome::failure(
                            "Changing focus replaced the explicit tab title".into(),
                        );
                    }
                    AssertionOutcome::Success
                })
            },
        )
}
