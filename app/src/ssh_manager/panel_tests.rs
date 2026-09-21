//! Unit tests for panel.rs — covers pure logic like tree building, parent resolution, and display sorting.
//!
//! Author: logic

use super::*;
use std::cell::RefCell;
use std::rc::Rc;

use chrono::NaiveDateTime;
use diesel::Connection;
use diesel_migrations::MigrationHarness;
use pathfinder_geometry::vector::vec2f;
use remote_server::proto::SessionList;
use warp_core::ui::appearance::Appearance;
use warp_ssh_manager::{NodeKind, OneKeyCredentialKind, SshNode};
use warpui::platform::WindowStyle;
use warpui::units::IntoPixels;
use warpui::{App, Event, Presenter, WindowInvalidation};

use crate::cockpit::favorites::FavoritesStore;
use crate::test_util::settings::initialize_settings_for_tests;

#[test]
fn connection_rows_reserve_leading_icons_for_folders_only() {
    assert_eq!(
        tree_row_leading_icon(NodeKind::Folder),
        Some(crate::ui_components::icons::Icon::Folder)
    );
    assert_eq!(tree_row_leading_icon(NodeKind::Server), None);
}

#[test]
fn connection_server_rows_expose_favorite_connect_disconnect_and_management_actions() {
    assert_eq!(
        connection_row_capabilities(NodeKind::Server, false, false),
        ConnectionRowCapabilities {
            favorite: true,
            connect: true,
            disconnect: false,
            management: true,
        }
    );
    assert_eq!(
        connection_row_capabilities(NodeKind::Server, false, true),
        ConnectionRowCapabilities {
            favorite: true,
            connect: false,
            disconnect: true,
            management: true,
        }
    );
}

#[derive(Clone, Debug)]
enum ConnectionRowTestAction {
    Primary,
    Favorite,
}

struct ConnectionRowTestView {
    primary_state: MouseStateHandle,
    favorite_action: CompactRowAction,
    use_session_layout: bool,
    primary_clicks: usize,
    favorite_clicks: usize,
}

impl ConnectionRowTestView {
    fn new(ctx: &mut ViewContext<Self>) -> Self {
        Self {
            primary_state: MouseStateHandle::default(),
            favorite_action: CompactRowAction::new(
                crate::ui_components::icons::Icon::Star,
                "Favorite",
                ConnectionRowTestAction::Favorite,
                ctx,
            ),
            use_session_layout: false,
            primary_clicks: 0,
            favorite_clicks: 0,
        }
    }
}

impl Entity for ConnectionRowTestView {
    type Event = ();
}

impl TypedActionView for ConnectionRowTestView {
    type Action = ConnectionRowTestAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ConnectionRowTestAction::Primary => self.primary_clicks += 1,
            ConnectionRowTestAction::Favorite => self.favorite_clicks += 1,
        }
        ctx.notify();
    }
}

impl View for ConnectionRowTestView {
    fn ui_name() -> &'static str {
        "ConnectionRowTestView"
    }

    fn render(&self, _app: &AppContext) -> Box<dyn Element> {
        let primary_target = Hoverable::new(self.primary_state.clone(), |_| {
            ConstrainedBox::new(Empty::new().finish())
                .with_width(240.0)
                .with_height(32.0)
                .finish()
        })
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(ConnectionRowTestAction::Primary))
        .finish();
        let favorite_action = SavePosition::new(
            self.favorite_action.render(),
            "connection_row_test:favorite",
        )
        .finish();

        let row = if self.use_session_layout {
            compose_session_row_targets(primary_target, Some(favorite_action))
        } else {
            compose_connection_row_targets(primary_target, Some(favorite_action), None, None, None)
        };
        Stack::new().with_child(row).finish()
    }
}

#[test]
fn compact_connection_secondary_action_survives_rerender_without_triggering_primary_row_click() {
    for use_session_layout in [false, true] {
        App::test((), |mut app| async move {
            app.add_singleton_model(|_| Appearance::mock());
            let (window_id, view) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
                let mut view = ConnectionRowTestView::new(ctx);
                view.use_session_layout = use_session_layout;
                view
            });
            let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
            let invalidation = WindowInvalidation {
                updated: app.read(|ctx| ctx.view_ids_for_window(window_id).into_iter().collect()),
                ..Default::default()
            };

            let click_position = app.update({
                let presenter = presenter.clone();
                let invalidation = invalidation.clone();
                move |ctx| {
                    presenter.borrow_mut().invalidate(invalidation, ctx);
                    presenter
                        .borrow_mut()
                        .build_scene(vec2f(320.0, 60.0), 1.0, None, ctx);
                    presenter
                        .borrow()
                        .position_cache()
                        .get_position("connection_row_test:favorite")
                        .expect("favorite action should be positioned")
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
                        .build_scene(vec2f(320.0, 60.0), 1.0, None, ctx);
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
                assert_eq!(view.favorite_clicks, 1);
                assert_eq!(view.primary_clicks, 0);
            });
            for event in [
                Event::LeftMouseDown {
                    position: vec2f(30.0, 15.0),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                Event::LeftMouseUp {
                    position: vec2f(30.0, 15.0),
                    modifiers: Default::default(),
                },
            ] {
                app.update(|ctx| {
                    ctx.simulate_window_event(event, window_id, presenter.clone());
                });
            }
            view.read(&app, |view, _| {
                assert_eq!(
                    view.primary_clicks, 1,
                    "the identity must open exactly once"
                );
                assert_eq!(
                    view.favorite_clicks, 1,
                    "the primary must not trigger the sibling icon"
                );
            });
        });
    }
}

#[test]
fn session_rows_keep_flexible_identity_fixed_actions_and_only_semantic_metadata() {
    for width in [250.0, 320.0, 480.0] {
        App::test((), |mut app| async move {
            crate::i18n::init(Some("en"));
            initialize_settings_for_tests(&mut app);
            app.add_singleton_model(|_| Appearance::mock());
            app.add_singleton_model(|_| SshTreeChangedNotifier::new());
            app.add_singleton_model(FavoritesStore::new_for_test);
            app.add_singleton_model(RemoteServerManager::new);

            let sessions = [
                SessionInfo {
                    session_id: "12345678-abcdef-0123456789".into(),
                    title: "A session identity that is much wider than the minimum sidebar".into(),
                    ring_bytes: 223_232,
                    ..Default::default()
                },
                SessionInfo {
                    session_id: "87654321-abcdef-0123456789".into(),
                    ring_bytes: 1_048_576,
                    ..Default::default()
                },
            ];
            let multiplexer = MultiplexerSessionInfo {
                name: "A byobu session with a deliberately long descriptive name".into(),
                target: "fixture-session".into(),
                kind: MultiplexerKind::ByobuTmux as i32,
                windows: 12,
                attached_clients: 2,
            };
            let mut keys: Vec<String> = sessions
                .iter()
                .map(|session| session_row_key("fixture-host", session, None))
                .collect();
            let mux_key = multiplexer_row_key("fixture-host", &multiplexer);
            keys.push(mux_key.clone());
            let (window_id, panel) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
                let mut panel = SshManagerPanel::new(ctx);
                panel.set_nodes_for_test(
                    vec![server(
                        "fixture-host",
                        None,
                        "build-node-with-a-long-identity-to-test-the-whole-remaining-width",
                        0,
                    )],
                    ctx,
                );
                panel.resilient_hosts.insert("fixture-host".into());
                panel.sessions_expanded.insert("fixture-host".into());
                let generation = panel.begin_session_fetch("fixture-host").unwrap();
                panel.complete_session_fetch(
                    "fixture-host",
                    generation,
                    Ok(
                        crate::remote_server::session_inventory::HostSessionInventory {
                            daemon: SessionList {
                                sessions: sessions.to_vec(),
                                host_ring_cap_bytes: 268_435_456,
                                ..Default::default()
                            },
                            sessions: sessions
                                .into_iter()
                                .map(|session| {
                                    crate::remote_server::session_inventory::RoutedDaemonSession {
                                        session,
                                        route: None,
                                    }
                                })
                                .collect(),
                            multiplexers: remote_server::proto::MultiplexerSessionList {
                                sessions: vec![multiplexer],
                                ..Default::default()
                            },
                        },
                    ),
                    ctx,
                );
                panel
            });
            let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
            let invalidation = WindowInvalidation {
                updated: app.read(|ctx| ctx.view_ids_for_window(window_id).into_iter().collect()),
                ..Default::default()
            };
            let render = |app: &mut App| {
                app.update(|ctx| {
                    presenter.borrow_mut().invalidate(invalidation.clone(), ctx);
                    presenter
                        .borrow_mut()
                        .build_scene(vec2f(width, 800.0), 1.0, None, ctx);
                });
            };
            render(&mut app);
            let position = |key: &str, part: &str| {
                presenter
                    .borrow()
                    .position_cache()
                    .get_position(&format!("ssh-manager-session:{key}:{part}"))
                    .expect("the actual session row should be positioned")
            };
            let action = position(&mux_key, "open");
            let first_title = position(&keys[0], "title");
            let host_title = presenter
                .borrow()
                .position_cache()
                .get_position("ssh-manager-node:fixture-host:title")
                .unwrap();
            let refresh = presenter
                .borrow()
                .position_cache()
                .get_position("ssh-manager-node:fixture-host:refresh")
                .unwrap();
            let disclosure = presenter
                .borrow()
                .position_cache()
                .get_position("ssh-manager-node:fixture-host:disclosure")
                .unwrap();
            let persistence_mark_width = app.read(|ctx| Appearance::as_ref(ctx).ui_font_body());
            // The test FontDB has zero intrinsic text width. Check the allocated
            // slot against every fixed neighbor, rather than an estimated minimum.
            let expected_title_start = disclosure.max_x() + ITEM_PADDING_HORIZONTAL;
            let expected_title_end = refresh.min_x()
                - ITEM_PADDING_HORIZONTAL
                - persistence_mark_width
                - ITEM_ICON_TEXT_SPACING;
            let expected_title_width = expected_title_end - expected_title_start;
            assert!((host_title.min_x() - expected_title_start).abs() < 0.5);
            assert!(
                (host_title.width() - expected_title_width).abs() < 0.5,
                "host identity must fill the flexible remainder at {width}px: got {}, expected {expected_title_width}",
                host_title.width()
            );
            assert!((host_title.max_x() - expected_title_end).abs() < 0.5);
            assert!((disclosure.min_x() - ITEM_PADDING_HORIZONTAL).abs() < 0.5);
            assert!((disclosure.width() - ROW_ACTION_SIZE).abs() < 0.5);
            assert!((refresh.width() - ROW_ACTION_SIZE).abs() < 0.5);
            assert!(
                (refresh.min_x() + 3.0 * ROW_ACTION_SIZE + ITEM_PADDING_HORIZONTAL - width).abs()
                    < 0.5,
                "refresh, favorite and connection must retain their fixed trailing slots"
            );
            for key in &keys {
                let title = position(key, "title");
                assert!((title.min_x() - first_title.min_x()).abs() < 0.5);
                assert!(title.width() > 100.0, "identity must retain useful width");
                assert!(title.max_x() <= width - ITEM_PADDING_HORIZONTAL + 0.5);
            }
            for key in &keys[..2] {
                assert!(
                    presenter
                        .borrow()
                        .position_cache()
                        .get_position(&format!("ssh-manager-session:{key}:metadata"))
                        .is_none(),
                    "ordinary native session rows must not expose output-buffer metrics"
                );
            }
            let mux_metadata = position(&mux_key, "metadata");
            assert!((mux_metadata.min_x() - position(&mux_key, "title").min_x()).abs() < 0.5);
            assert!(mux_metadata.max_x() <= width - ITEM_PADDING_HORIZONTAL + 0.5);
            assert!(mux_metadata.min_y() >= position(&mux_key, "title").max_y());
            assert!(position(&mux_key, "title").max_x() <= action.min_x());
            assert!(mux_metadata.max_x() <= action.min_x());
            assert!((action.width() - 22.0).abs() < 0.5);
            assert!(action.max_x() <= width - ITEM_PADDING_HORIZONTAL + 0.5);

            app.update(|ctx| {
                ctx.simulate_window_event(
                    Event::MouseMoved {
                        position: action.center(),
                        cmd: false,
                        shift: false,
                        is_synthetic: false,
                    },
                    window_id,
                    presenter.clone(),
                );
            });
            render(&mut app);
            assert_eq!(
                position(&mux_key, "open"),
                action,
                "hover must not shift actions"
            );
            assert_eq!(position(&keys[0], "title"), first_title);

            let generation = panel.update(&mut app, |panel, ctx| {
                let generation = panel.begin_session_fetch("fixture-host").unwrap();
                ctx.notify();
                generation
            });
            render(&mut app);
            for key in &keys {
                assert!(position(key, "title").width() > 100.0);
            }
            assert_eq!(
                position(&keys[0], "title"),
                first_title,
                "refresh must not move existing rows"
            );
            assert_eq!(
                presenter
                    .borrow()
                    .position_cache()
                    .get_position("ssh-manager-node:fixture-host:refresh")
                    .unwrap(),
                refresh
            );
            panel.update(&mut app, |panel, ctx| {
                panel.set_connecting("fixture-host", true, ctx);
            });
            render(&mut app);
            assert_eq!(
                presenter
                    .borrow()
                    .position_cache()
                    .get_position("ssh-manager-node:fixture-host:title")
                    .unwrap(),
                host_title,
                "connecting feedback must not steal identity width"
            );
            panel.update(&mut app, |panel, ctx| {
                panel.complete_session_fetch(
                    "fixture-host",
                    generation,
                    Err("The inventory refresh timed out.".into()),
                    ctx,
                );
            });
            render(&mut app);
            {
                let presenter = presenter.borrow();
                assert!(presenter
                    .position_cache()
                    .get_position("ssh-manager-session-error:fixture-host")
                    .is_some());
                for key in &keys {
                    assert!(
                        presenter
                            .position_cache()
                            .get_position(&format!("ssh-manager-session:{key}:title"))
                            .is_none(),
                        "failed refresh must not present old inventory as current"
                    );
                }
            }
            let (retry_generation, inventory) = panel.update(&mut app, |panel, ctx| {
                // Cover the first-load failure too: there is no cached inventory
                // available to keep the previous error visible during a retry.
                let inventory = panel
                    .host_session_inventories
                    .remove("fixture-host")
                    .unwrap();
                let generation = panel.begin_session_fetch("fixture-host").unwrap();
                assert_eq!(panel.begin_session_fetch("fixture-host"), None);
                assert_eq!(panel.begin_session_fetch("fixture-host"), None);
                assert_eq!(
                    panel.sessions_error["fixture-host"],
                    "The inventory refresh timed out."
                );
                ctx.notify();
                (generation, inventory)
            });
            render(&mut app);
            assert!(
                presenter
                    .borrow()
                    .position_cache()
                    .get_position("ssh-manager-session-error:fixture-host")
                    .is_some(),
                "a queued retry must not replace a first-load error with loading alone"
            );
            panel.update(&mut app, |panel, ctx| {
                assert!(panel.complete_session_fetch(
                    "fixture-host",
                    retry_generation,
                    Ok(inventory),
                    ctx
                ));
                assert!(!panel.sessions_error.contains_key("fixture-host"));
            });
            render(&mut app);
            assert!(presenter
                .borrow()
                .position_cache()
                .get_position("ssh-manager-session-error:fixture-host")
                .is_none());
            for key in &keys {
                assert!(position(key, "title").width() > 100.0);
            }
            let mux_events = Rc::new(RefCell::new(0usize));
            app.update(|ctx| {
                let events = mux_events.clone();
                ctx.subscribe_to_view(&panel, move |_, event, _| {
                    if matches!(
                        event,
                        SshManagerPanelEvent::PersistenceError(_)
                            | SshManagerPanelEvent::OpenMultiplexerSession { .. }
                    ) {
                        *events.borrow_mut() += 1;
                    }
                });
            });
            for (key, part, is_mux) in [
                (&keys[0], "open", false),
                (&mux_key, "open", true),
                (&mux_key, "title", true),
            ] {
                let click = position(key, part).center();
                let before = *mux_events.borrow();
                app.update(|ctx| {
                    ctx.simulate_window_event(
                        Event::LeftMouseDown {
                            position: click,
                            modifiers: Default::default(),
                            click_count: 1,
                            is_first_mouse: false,
                        },
                        window_id,
                        presenter.clone(),
                    );
                });
                let inventory = panel.update(&mut app, |panel, ctx| {
                    let inventory = panel.host_session_inventories["fixture-host"].clone();
                    let generation = panel.begin_session_fetch("fixture-host").unwrap();
                    panel.complete_session_fetch(
                        "fixture-host",
                        generation,
                        Ok(inventory.clone()),
                        ctx,
                    );
                    inventory
                });
                render(&mut app);
                app.update(|ctx| {
                    ctx.simulate_window_event(
                        Event::LeftMouseUp {
                            position: click,
                            modifiers: Default::default(),
                        },
                        window_id,
                        presenter.clone(),
                    );
                });
                if is_mux {
                    assert_eq!(
                        *mux_events.borrow(),
                        before + 1,
                        "refresh must preserve the mux title/icon click exactly once"
                    );
                } else {
                    panel.read(&app, |panel, _| {
                        assert!(
                            panel.sessions_error.contains_key("fixture-host"),
                            "refresh must preserve the daemon open click"
                        )
                    });
                }
                panel.update(&mut app, |panel, ctx| {
                    let generation = panel.begin_session_fetch("fixture-host").unwrap();
                    panel.complete_session_fetch("fixture-host", generation, Ok(inventory), ctx);
                });
                render(&mut app);
            }
            for event in [
                Event::LeftMouseDown {
                    position: disclosure.center(),
                    modifiers: Default::default(),
                    click_count: 1,
                    is_first_mouse: false,
                },
                Event::LeftMouseUp {
                    position: disclosure.center(),
                    modifiers: Default::default(),
                },
            ] {
                app.update(|ctx| {
                    ctx.simulate_window_event(event, window_id, presenter.clone());
                });
            }
            render(&mut app);
            panel.read(&app, |panel, _| {
                assert!(!panel.sessions_expanded.contains("fixture-host"));
                assert!(
                    panel.selected_id.is_none(),
                    "disclosure must not bubble to host selection"
                );
            });
            assert!(presenter
                .borrow()
                .position_cache()
                .get_position(&format!("ssh-manager-session:{}:title", keys[0]))
                .is_none());
            assert_eq!(
                presenter
                    .borrow()
                    .position_cache()
                    .get_position("ssh-manager-node:fixture-host:disclosure")
                    .unwrap(),
                disclosure
            );
        });
    }
}

#[test]
fn panel_content_can_scroll_when_ssh_list_is_taller_than_panel() {
    App::test((), |mut app| async move {
        crate::i18n::init(Some("en"));
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| SshTreeChangedNotifier::new());
        app.add_singleton_model(FavoritesStore::new_for_test);
        app.add_singleton_model(RemoteServerManager::new);

        let nodes = (0..60)
            .map(|i| {
                let name = if i == 0 {
                    "server-with-a-name-that-is-much-wider-than-the-sidebar".to_owned()
                } else {
                    format!("server-{i}")
                };
                server(&format!("s{i}"), None, &name, i)
            })
            .collect::<Vec<_>>();
        let (window_id, panel) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut panel = SshManagerPanel::new(ctx);
            panel.set_nodes_for_test(nodes, ctx);
            panel
        });
        let mut presenter = Presenter::new(window_id);
        let mut updated = std::collections::HashSet::new();
        updated.insert(app.root_view_id(window_id).unwrap());
        let invalidation = WindowInvalidation {
            updated,
            ..Default::default()
        };

        app.update(|ctx| {
            presenter.invalidate(invalidation.clone(), ctx);
            presenter.build_scene(vec2f(240.0, 120.0), 1.0, None, ctx);
        });
        let scroll_state = panel.read(&app, |panel, _| panel.content_scroll_state.clone());

        scroll_state.scroll_by(10_000_f32.into_pixels());
        app.update(|ctx| {
            presenter.invalidate(invalidation, ctx);
            presenter.build_scene(vec2f(240.0, 120.0), 1.0, None, ctx);
        });

        let scroll_start = scroll_state.scroll_start().as_f32();
        assert!(scroll_start > 0.0);
        assert!(scroll_start < 10_000.0);
    });
}

#[test]
fn right_click_moves_visible_focus_to_the_context_menu_target() {
    App::test((), |mut app| async move {
        crate::i18n::init(Some("en"));
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| SshTreeChangedNotifier::new());
        app.add_singleton_model(FavoritesStore::new_for_test);
        app.add_singleton_model(RemoteServerManager::new);
        let (window_id, panel) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut panel = SshManagerPanel::new(ctx);
            panel.set_nodes_for_test(
                vec![
                    server("host-a", None, "Host A", 0),
                    server("host-b", None, "Host B", 1),
                ],
                ctx,
            );
            panel
        });
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        let render = |app: &mut App| {
            app.update(|ctx| {
                presenter.borrow_mut().invalidate(
                    WindowInvalidation {
                        updated: ctx.view_ids_for_window(window_id).into_iter().collect(),
                        ..Default::default()
                    },
                    ctx,
                );
                presenter
                    .borrow_mut()
                    .build_scene(vec2f(320.0, 500.0), 1.0, None, ctx)
            })
        };
        let position = |id: &str| {
            presenter
                .borrow()
                .position_cache()
                .get_position(&format!("ssh-manager-node:{id}:title"))
                .expect("the real host title must be laid out")
                .center()
        };
        render(&mut app);
        let host_a = position("host-a");
        let host_b = position("host-b");
        for event in [
            Event::LeftMouseDown {
                position: host_a,
                modifiers: Default::default(),
                click_count: 1,
                is_first_mouse: false,
            },
            Event::LeftMouseUp {
                position: host_a,
                modifiers: Default::default(),
            },
        ] {
            app.update(|ctx| {
                ctx.simulate_window_event(event, window_id, presenter.clone());
            });
        }
        let selection_background: ElementFill =
            app.read(|ctx| internal_colors::fg_overlay_3(Appearance::as_ref(ctx).theme()).into());
        let scene = render(&mut app);
        assert!(
            scene.layers().flat_map(|layer| &layer.rects).any(|rect| {
                rect.bounds.contains_point(host_a) && rect.background == selection_background
            }),
            "left click must visibly focus host A before opening the other menu"
        );
        panel.read(&app, |panel, _| {
            assert!(panel.keyboard_focused);
            assert_eq!(panel.focused_row, Some(FocusedRow::Node("host-a".into())));
        });
        app.update(|ctx| {
            ctx.simulate_window_event(
                Event::RightMouseDown {
                    position: host_b,
                    cmd: false,
                    shift: false,
                    click_count: 1,
                },
                window_id,
                presenter.clone(),
            );
        });
        let scene = render(&mut app);
        panel.read(&app, |panel, _| {
            assert!(panel.keyboard_focused);
            assert_eq!(panel.selected_id.as_deref(), Some("host-b"));
            assert_eq!(panel.focused_row, Some(FocusedRow::Node("host-b".into())));
            assert_eq!(panel.context_menu_target.as_deref(), Some("host-b"));
            assert!(panel.context_menu_position.is_some());
        });
        assert!(
            scene.layers().flat_map(|layer| &layer.rects).any(|rect| {
                rect.bounds.contains_point(host_b) && rect.background == selection_background
            }),
            "the highlighted host must be the host affected by context-menu actions"
        );
        assert!(
            !scene.layers().flat_map(|layer| &layer.rects).any(|rect| {
                rect.bounds.contains_point(host_a) && rect.background == selection_background
            }),
            "the previous host must not retain the selection highlight"
        );
        panel.update(&mut app, |panel, ctx| {
            panel.on_open_context_menu(None, vec2f(0.0, 400.0), ctx);
            assert!(panel.selected_id.is_none());
            assert!(panel.focused_row.is_none());
            assert!(panel.context_menu_target.is_none());
        });
    });
}

#[test]
fn keyboard_navigation_reaches_both_session_kinds_and_enter_uses_the_exact_session() {
    App::test((), |mut app| async move {
        crate::i18n::init(Some("en"));
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| SshTreeChangedNotifier::new());
        app.add_singleton_model(FavoritesStore::new_for_test);
        app.add_singleton_model(RemoteServerManager::new);
        app.update(init);
        let daemon = SessionInfo {
            session_id: "keyboard-pty".into(),
            generation: 7,
            ..Default::default()
        };
        let mux = MultiplexerSessionInfo {
            name: "keyboard-mux".into(),
            target: "exact-mux".into(),
            kind: MultiplexerKind::Tmux as i32,
            ..Default::default()
        };
        let daemon_key = session_row_key("keyboard-host", &daemon, None);
        let mux_key = multiplexer_row_key("keyboard-host", &mux);
        let (window_id, panel) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let mut panel = SshManagerPanel::new(ctx);
            panel.set_nodes_for_test(vec![server("keyboard-host", None, "Keyboard host", 0)], ctx);
            panel.resilient_hosts.insert("keyboard-host".into());
            panel.sessions_expanded.insert("keyboard-host".into());
            let generation = panel.begin_session_fetch("keyboard-host").unwrap();
            panel.complete_session_fetch(
                "keyboard-host",
                generation,
                Ok(
                    crate::remote_server::session_inventory::HostSessionInventory {
                        daemon: SessionList::default(),
                        sessions: vec![
                            crate::remote_server::session_inventory::RoutedDaemonSession {
                                session: daemon,
                                route: None,
                            },
                        ],
                        multiplexers: remote_server::proto::MultiplexerSessionList {
                            sessions: vec![mux],
                            ..Default::default()
                        },
                    },
                ),
                ctx,
            );
            ctx.focus_self();
            panel
        });
        let presenter = Rc::new(RefCell::new(Presenter::new(window_id)));
        let render = |app: &mut App| {
            app.update(|ctx| {
                presenter.borrow_mut().invalidate(
                    WindowInvalidation {
                        updated: ctx.view_ids_for_window(window_id).into_iter().collect(),
                        ..Default::default()
                    },
                    ctx,
                );
                presenter
                    .borrow_mut()
                    .build_scene(vec2f(250.0, 300.0), 1.0, None, ctx);
            })
        };
        let key = |app: &mut App, key: &str| {
            app.update(|ctx| {
                ctx.simulate_window_event(
                    Event::KeyDown {
                        keystroke: warpui::keymap::Keystroke::parse(key).unwrap(),
                        chars: key.to_string(),
                        details: warpui::event::KeyEventDetails::default(),
                        is_composing: false,
                    },
                    window_id,
                    presenter.clone(),
                )
            })
        };
        render(&mut app);
        assert!(key(&mut app, "down"));
        render(&mut app);
        panel.read(&app, |panel, _| {
            assert_eq!(
                panel.focused_row,
                Some(FocusedRow::Session(daemon_key.clone()))
            )
        });
        assert!(key(&mut app, "down"));
        render(&mut app);
        panel.read(&app, |panel, _| {
            assert_eq!(panel.focused_row, Some(FocusedRow::Session(mux_key.clone())));
            let rows = panel.navigation_rows();
            assert!(matches!(&rows[2].1, SshManagerPanelAction::OpenMultiplexerSession { session, .. } if session.target == "exact-mux"));
        });
        assert!(key(&mut app, "up"));
        render(&mut app);
        assert!(key(&mut app, "enter"));
        panel.read(&app, |panel, _| {
            // This fixture intentionally has no saved database host. Reaching the
            // daemon's missing-host error proves Enter dispatched adoption, not Connect.
            assert!(panel.sessions_error.contains_key("keyboard-host"));
            assert_eq!(
                panel.focused_row,
                Some(FocusedRow::Node("keyboard-host".into()))
            );
            assert_eq!(panel.focused_node_id().as_deref(), Some("keyboard-host"));
            assert_eq!(
                panel.navigation_rows().len(),
                1,
                "stale/error sessions must not remain keyboard targets"
            );
        });
    });
}

// --- Test helpers -------------------------------------------------------

fn ts() -> NaiveDateTime {
    chrono::DateTime::from_timestamp(0, 0).unwrap().naive_utc()
}

fn folder(id: &str, parent_id: Option<&str>, name: &str, sort_order: i32) -> SshNode {
    SshNode {
        id: id.to_string(),
        parent_id: parent_id.map(|s| s.to_string()),
        kind: NodeKind::Folder,
        name: name.to_string(),
        sort_order,
        created_at: ts(),
        updated_at: ts(),
        is_collapsed: false,
    }
}

fn server(id: &str, parent_id: Option<&str>, name: &str, sort_order: i32) -> SshNode {
    SshNode {
        id: id.to_string(),
        parent_id: parent_id.map(|s| s.to_string()),
        kind: NodeKind::Server,
        name: name.to_string(),
        sort_order,
        created_at: ts(),
        updated_at: ts(),
        is_collapsed: false,
    }
}

#[cfg(unix)]
#[tokio::test]
async fn discover_tailscale_action_never_calls_blocking_factory() {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;
    use std::sync::Mutex;
    use warp_ssh_manager::WorkspaceCommandFactory;

    struct RecordingCommandFactory {
        script: std::path::PathBuf,
        programs: Mutex<Vec<String>>,
    }

    impl WorkspaceCommandFactory for RecordingCommandFactory {
        fn async_command(&self, program: &str) -> command::r#async::Command {
            self.programs.lock().unwrap().push(program.to_string());
            let mut command = command::r#async::Command::new(&self.script);
            command.arg(program);
            command
        }

        fn blocking_command(&self, program: &str) -> command::blocking::Command {
            panic!("blocking command factory must not be called for {program}")
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("fake-command");
    let mut file = std::fs::File::create(&script).unwrap();
    file.write_all(b"#!/bin/sh\nprintf '%s\\n' \"$@\"\n")
        .unwrap();
    let mut permissions = file.metadata().unwrap().permissions();
    permissions.set_mode(0o700);
    std::fs::set_permissions(&script, permissions).unwrap();
    drop(file);
    let factory = Arc::new(RecordingCommandFactory {
        script,
        programs: Mutex::new(Vec::new()),
    });

    let output = tailscale_status_output(factory.clone()).await.unwrap();

    assert!(output.status.success());
    assert_eq!(factory.programs.lock().unwrap().as_slice(), ["tailscale"]);
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "tailscale\nstatus\n--json\n"
    );
}

#[test]
fn invalid_import_port_is_rejected_without_fallback() {
    let candidate = SshConfigCandidate {
        alias: "example.com".to_string(),
        hostname: None,
        user: None,
        port: None,
        invalid_port: Some("70000".to_string()),
        identity_file: None,
    };

    assert_eq!(
        validate_candidate_endpoint(&candidate),
        Err(SshEndpointValidationError::InvalidPort)
    );
}

#[test]
fn missing_import_port_still_uses_the_explicit_default() {
    let candidate = SshConfigCandidate {
        alias: "example.com".to_string(),
        hostname: None,
        user: None,
        port: None,
        invalid_port: None,
        identity_file: None,
    };

    assert_eq!(validate_candidate_endpoint(&candidate).unwrap().port, 22);
}

// --- resolve_parent_for_new_node tests ----------------------------------------

#[test]
fn parent_no_selection_returns_none() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    assert_eq!(resolve_parent_for_new_node(None, &nodes), None);
}

#[test]
fn parent_folder_selected_returns_folder_id() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    assert_eq!(
        resolve_parent_for_new_node(Some("f1"), &nodes),
        Some("f1".to_string())
    );
}

#[test]
fn parent_server_at_root_selected_returns_none() {
    let nodes = vec![server("s1", None, "srv", 0)];
    assert_eq!(resolve_parent_for_new_node(Some("s1"), &nodes), None);
}

#[test]
fn parent_server_under_folder_selected_returns_folder_id() {
    let nodes = vec![
        folder("f1", None, "Prod", 0),
        server("s1", Some("f1"), "web", 0),
    ];
    assert_eq!(
        resolve_parent_for_new_node(Some("s1"), &nodes),
        Some("f1".to_string())
    );
}

#[test]
fn parent_invalid_selected_id_returns_none() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    assert_eq!(
        resolve_parent_for_new_node(Some("nonexistent"), &nodes),
        None
    );
}

#[test]
fn parent_empty_nodes_with_selection_returns_none() {
    assert_eq!(resolve_parent_for_new_node(Some("any"), &[]), None);
}

#[test]
fn parent_deeply_nested_folder_selected_returns_immediate_parent() {
    // f1(root) → f2(child) → s1(grandchild server)
    let nodes = vec![
        folder("f1", None, "L0", 0),
        folder("f2", Some("f1"), "L1", 0),
        server("s1", Some("f2"), "srv", 0),
    ];
    // Select f2 → new node created under f2
    assert_eq!(
        resolve_parent_for_new_node(Some("f2"), &nodes),
        Some("f2".to_string())
    );
    // Select s1 → new node created under s1's parent (f2) (sibling semantics)
    assert_eq!(
        resolve_parent_for_new_node(Some("s1"), &nodes),
        Some("f2".to_string())
    );
}

#[test]
fn delete_confirmation_lists_all_descendant_hosts() {
    let nodes = vec![
        folder("f1", None, "Production", 0),
        server("s1", Some("f1"), "web", 0),
        folder("f2", Some("f1"), "Databases", 1),
        server("s2", Some("f2"), "db", 0),
        server("s3", None, "unrelated", 1),
    ];
    let credential_labels = HashMap::from([
        ("s1".to_string(), vec!["Deploy key (ops)".to_string()]),
        ("s3".to_string(), vec!["Unrelated key".to_string()]),
    ]);

    let impact = build_delete_impact(&nodes, "f1", &credential_labels).unwrap();

    assert_eq!(impact.node_name, "Production");
    assert_eq!(impact.node_kind, NodeKind::Folder);
    assert_eq!(impact.host_names, vec!["web", "db"]);
    assert_eq!(
        impact.credential_impacts,
        vec![DeleteCredentialImpact {
            host_name: "web".to_string(),
            credential_label: "Deploy key (ops)".to_string(),
        }]
    );
}

#[test]
fn host_delete_impact_never_includes_sibling_hosts() {
    let nodes = vec![
        folder("f1", None, "Production", 0),
        server("s1", Some("f1"), "web", 0),
        server("s2", Some("f1"), "db", 1),
    ];
    let credential_labels = HashMap::from([("s1".to_string(), vec!["Deploy key".to_string()])]);

    let impact = build_delete_impact(&nodes, "s1", &credential_labels).unwrap();

    assert_eq!(impact.node_kind, NodeKind::Server);
    assert_eq!(impact.host_names, vec!["web"]);
    assert_eq!(impact.credential_impacts.len(), 1);
    assert_eq!(impact.credential_impacts[0].host_name, "web");
}

#[test]
fn changed_descendants_require_a_fresh_delete_confirmation() {
    let mut nodes = vec![
        folder("f1", None, "Production", 0),
        server("s1", Some("f1"), "web", 0),
    ];
    let original = build_delete_impact(&nodes, "f1", &HashMap::new()).unwrap();
    nodes.push(server("s2", Some("f1"), "db", 1));
    let current = build_delete_impact(&nodes, "f1", &HashMap::new()).unwrap();

    assert_ne!(current, original);
    assert_eq!(current.host_names, vec!["web", "db"]);
}

#[test]
fn delete_confirmation_lists_every_credential_impact_for_mixed_auth_hosts() {
    let nodes = vec![
        folder("f1", None, "Mixed", 0),
        server("s1", Some("f1"), "password", 0),
        server("s2", Some("f1"), "key", 1),
        server("s3", Some("f1"), "onekey", 2),
    ];
    let password_impacts = delete_credential_impacts(
        &DeleteHostExpectation {
            node_id: "s1".to_string(),
            auth_type: AuthType::Password,
            key_path: None,
            credential_id: None,
            secret_kinds: vec![SecretKind::Password, SecretKind::RootPassword],
        },
        &HashMap::new(),
    );
    let key_impacts = delete_credential_impacts(
        &DeleteHostExpectation {
            node_id: "s2".to_string(),
            auth_type: AuthType::Key,
            key_path: Some("/keys/id_ed25519".to_string()),
            credential_id: None,
            secret_kinds: vec![SecretKind::Passphrase],
        },
        &HashMap::new(),
    );
    let onekey_impacts = delete_credential_impacts(
        &DeleteHostExpectation {
            node_id: "s3".to_string(),
            auth_type: AuthType::OneKey,
            key_path: None,
            credential_id: Some("credential-1".to_string()),
            secret_kinds: Vec::new(),
        },
        &HashMap::from([("credential-1".to_string(), "Shared credential".to_string())]),
    );
    let credential_impacts = HashMap::from([
        ("s1".to_string(), password_impacts.clone()),
        ("s2".to_string(), key_impacts.clone()),
        ("s3".to_string(), onekey_impacts.clone()),
    ]);

    assert_eq!(
        password_impacts,
        vec![
            crate::t!("workspace-left-panel-ssh-manager-delete-credential-password"),
            crate::t!("workspace-left-panel-ssh-manager-delete-credential-root-password"),
        ]
    );
    assert_eq!(
        key_impacts,
        vec![
            crate::t!(
                "workspace-left-panel-ssh-manager-delete-credential-identity-file",
                path = "/keys/id_ed25519"
            ),
            crate::t!("workspace-left-panel-ssh-manager-delete-credential-passphrase"),
        ]
    );
    assert_eq!(
        onekey_impacts,
        vec![crate::t!(
            "workspace-left-panel-ssh-manager-delete-credential-onekey-assignment",
            label = "Shared credential"
        )]
    );

    let impact = build_delete_impact(&nodes, "f1", &credential_impacts).unwrap();

    assert_eq!(impact.host_names, vec!["password", "key", "onekey"]);
    assert_eq!(
        impact.credential_impacts,
        vec![
            DeleteCredentialImpact {
                host_name: "password".to_string(),
                credential_label: password_impacts[0].clone(),
            },
            DeleteCredentialImpact {
                host_name: "password".to_string(),
                credential_label: password_impacts[1].clone(),
            },
            DeleteCredentialImpact {
                host_name: "key".to_string(),
                credential_label: key_impacts[0].clone(),
            },
            DeleteCredentialImpact {
                host_name: "key".to_string(),
                credential_label: key_impacts[1].clone(),
            },
            DeleteCredentialImpact {
                host_name: "onekey".to_string(),
                credential_label: onekey_impacts[0].clone(),
            },
        ]
    );
}

#[test]
fn large_folder_delete_impact_keeps_every_host_and_credential_impact() {
    let mut nodes = vec![folder("f1", None, "Large fleet", 0)];
    let mut credential_impacts = HashMap::new();
    for index in 0..100 {
        let id = format!("s{index}");
        nodes.push(server(&id, Some("f1"), &format!("host-{index:03}"), index));
        if index % 2 == 0 {
            credential_impacts.insert(id, vec![format!("credential-{index:03}")]);
        }
    }

    let impact = build_delete_impact(&nodes, "f1", &credential_impacts).unwrap();

    assert_eq!(impact.host_names.len(), 100);
    assert_eq!(
        impact.host_names.first().map(String::as_str),
        Some("host-000")
    );
    assert_eq!(
        impact.host_names.last().map(String::as_str),
        Some("host-099")
    );
    assert_eq!(impact.credential_impacts.len(), 50);
}

// --- compute_depths tests -------------------------------------------------

#[test]
fn depths_empty_nodes() {
    let depths = compute_depths(&[]);
    assert!(depths.is_empty());
}

#[test]
fn depths_single_root() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    let depths = compute_depths(&nodes);
    assert_eq!(depths["f1"], 0);
}

#[test]
fn depths_nested_tree() {
    let nodes = vec![
        folder("f1", None, "Root", 0),
        folder("f2", Some("f1"), "Child", 0),
        server("s1", Some("f2"), "Grandchild", 0),
    ];
    let depths = compute_depths(&nodes);
    assert_eq!(depths["f1"], 0);
    assert_eq!(depths["f2"], 1);
    assert_eq!(depths["s1"], 2);
}

#[test]
fn depths_multiple_roots() {
    let nodes = vec![
        folder("f1", None, "Root1", 0),
        folder("f2", None, "Root2", 1),
        server("s1", Some("f1"), "srv", 0),
        server("s2", Some("f2"), "srv", 0),
    ];
    let depths = compute_depths(&nodes);
    assert_eq!(depths["f1"], 0);
    assert_eq!(depths["f2"], 0);
    assert_eq!(depths["s1"], 1);
    assert_eq!(depths["s2"], 1);
}

// --- sort_for_display tests -----------------------------------------------

#[test]
fn sort_empty() {
    let depths = HashMap::new();
    let sorted = sort_for_display(vec![], &depths);
    assert!(sorted.is_empty());
}

#[test]
fn sort_single_root() {
    let nodes = vec![folder("f1", None, "Root", 0)];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    assert_eq!(sorted.len(), 1);
    assert_eq!(sorted[0].id, "f1");
}

#[test]
fn sort_respects_parent_child_order() {
    let nodes = vec![
        server("s1", Some("f1"), "web", 0),
        folder("f1", None, "Prod", 0),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    // f1 comes first, s1 comes second
    assert_eq!(sorted[0].id, "f1");
    assert_eq!(sorted[1].id, "s1");
}

#[test]
fn sort_preserves_existing_folder_children_in_tree_order() {
    let nodes = vec![
        server("s2", Some("f2"), "db", 1),
        folder("f2", None, "Stage", 1),
        server("s1", Some("f1"), "web", 0),
        folder("f1", None, "Prod", 0),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();

    assert_eq!(ids, &["f1", "s1", "f2", "s2"]);
    assert_eq!(depths["s1"], 1);
    assert_eq!(depths["s2"], 1);
}

#[test]
fn sort_multiple_roots_by_sort_order() {
    let nodes = vec![folder("f2", None, "B", 1), folder("f1", None, "A", 0)];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    assert_eq!(sorted[0].id, "f1");
    assert_eq!(sorted[1].id, "f2");
}

#[test]
fn sort_deeply_nested() {
    let nodes = vec![
        folder("f1", None, "Root", 0),
        server("s2", Some("f2"), "deep", 1),
        folder("f2", Some("f1"), "Child", 0),
        server("s1", Some("f1"), "shallow", 1),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, &["f1", "f2", "s2", "s1"]);
}

#[test]
fn sort_multiple_roots_with_children() {
    let nodes = vec![
        folder("f2", None, "Stage", 1),
        folder("f1", None, "Prod", 0),
        server("s1", Some("f1"), "web", 0),
        server("s2", Some("f2"), "app", 0),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();
    // f1 (Prod) and its children come first, f2 (Stage) and its children come later
    assert_eq!(ids, &["f1", "s1", "f2", "s2"]);
}

#[test]
fn sort_keeps_orphaned_existing_nodes_visible_as_roots() {
    let nodes = vec![
        server("s1", Some("missing-folder"), "legacy", 0),
        folder("f1", None, "New folder", 1),
    ];
    let depths = compute_depths(&nodes);
    let sorted = sort_for_display(nodes, &depths);
    let ids: Vec<&str> = sorted.iter().map(|n| n.id.as_str()).collect();

    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&"s1"));
    assert!(ids.contains(&"f1"));
    assert_eq!(depths["s1"], 0);
    assert_eq!(depths["f1"], 0);
}

#[test]
fn daemon_session_rows_keep_the_same_mouse_state_across_renders() {
    let mut states = HashMap::new();
    let inventory = vec![
        crate::remote_server::session_inventory::RoutedDaemonSession {
            session: remote_server::proto::SessionInfo {
                session_id: "pty-1".to_string(),
                ..Default::default()
            },
            route: None,
        },
    ];
    let key = session_row_key("devhost", &inventory[0].session, None);

    sync_session_row_states(&mut states, "devhost", &inventory, &[]);
    let original = states[&key].clone();
    sync_session_row_states(&mut states, "devhost", &inventory, &[]);

    assert!(Arc::ptr_eq(&original, &states[&key]));

    let replacement = vec![
        crate::remote_server::session_inventory::RoutedDaemonSession {
            session: remote_server::proto::SessionInfo {
                session_id: "pty-2".to_string(),
                ..Default::default()
            },
            route: None,
        },
    ];
    let replacement_key = session_row_key("devhost", &replacement[0].session, None);
    sync_session_row_states(&mut states, "devhost", &replacement, &[]);
    assert!(!states.contains_key(&key));
    assert!(states.contains_key(&replacement_key));
}

#[test]
fn identical_pty_identities_on_two_daemons_have_distinct_row_state() {
    let session = remote_server::proto::SessionInfo {
        session_id: "pty-1".to_string(),
        generation: 7,
        ..Default::default()
    };
    let old_route = remote_server::transport::DaemonRuntimeRoute::new(
        "server-v1.0.28.sock".to_string(),
        "v1.0.28".to_string(),
    )
    .unwrap();
    let current_route = remote_server::transport::DaemonRuntimeRoute::new(
        "server-v1.0.29.sock".to_string(),
        "v1.0.29".to_string(),
    )
    .unwrap();

    assert_ne!(
        session_row_key("devhost", &session, Some(&old_route)),
        session_row_key("devhost", &session, Some(&current_route))
    );
}

#[test]
fn one_registry_node_keeps_every_connected_daemon_identity() {
    let grouped = connected_hosts_by_registry_node([
        (
            "node-dev".to_string(),
            HostId::new("daemon-current".to_string()),
        ),
        (
            "node-dev".to_string(),
            HostId::new("daemon-old".to_string()),
        ),
    ]);

    assert_eq!(
        grouped["node-dev"],
        vec!["daemon-current".to_string(), "daemon-old".to_string()]
    );
}

fn with_session_panel(
    test: impl FnOnce(&mut SshManagerPanel, &mut ViewContext<SshManagerPanel>) + 'static,
) {
    App::test((), |mut app| async move {
        crate::i18n::init(Some("en"));
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| SshTreeChangedNotifier::new());
        app.add_singleton_model(FavoritesStore::new_for_test);
        app.add_singleton_model(RemoteServerManager::new);
        let (_, panel) = app.add_window(WindowStyle::NotStealFocus, SshManagerPanel::new);
        panel.update(&mut app, |panel, ctx| {
            panel.set_nodes_for_test(vec![server("devhost", None, "Development", 0)], ctx);
            test(panel, ctx);
        });
    });
}

#[test]
fn inventory_refresh_preserves_control_views_and_mouse_state_but_uses_fresh_mux_metadata() {
    with_session_panel(|panel, ctx| {
        panel.sessions_expanded.insert("devhost".into());
        let mux = MultiplexerSessionInfo {
            target: "same-screen".into(),
            kind: MultiplexerKind::ByobuScreen as i32,
            attached_clients: 0,
            ..Default::default()
        };
        let key = multiplexer_row_key("devhost", &mux);
        let mut inventory = crate::remote_server::session_inventory::HostSessionInventory {
            multiplexers: remote_server::proto::MultiplexerSessionList {
                sessions: vec![mux],
                ..Default::default()
            },
            ..Default::default()
        };
        let generation = panel.begin_session_fetch("devhost").unwrap();
        panel.complete_session_fetch("devhost", generation, Ok(inventory.clone()), ctx);
        let mouse = panel.session_row_states[&key].clone();
        let views: std::collections::HashSet<_> = ctx
            .view_ids_for_window(ctx.window_id())
            .into_iter()
            .collect();
        inventory.multiplexers.sessions[0].attached_clients = 2;
        let generation = panel.begin_session_fetch("devhost").unwrap();
        panel.complete_session_fetch("devhost", generation, Ok(inventory), ctx);
        assert!(Arc::ptr_eq(&mouse, &panel.session_row_states[&key]));
        assert_eq!(
            views,
            ctx.view_ids_for_window(ctx.window_id())
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
        );
        assert!(matches!(
            panel.session_row_action(&key),
            Some(SshManagerPanelAction::OpenMultiplexerSession {
                session,
                ..
            }) if session.attached_clients == 2
        ));
        panel.focused_row = Some(FocusedRow::Session(key));
        let generation = panel.begin_session_fetch("devhost").unwrap();
        panel.complete_session_fetch("devhost", generation, Err("timeout".into()), ctx);
        assert_eq!(panel.focused_row, Some(FocusedRow::Node("devhost".into())));
        assert_eq!(panel.focused_node_id().as_deref(), Some("devhost"));
        panel.handle_action(&SshManagerPanelAction::RefreshFocused, ctx);
        assert!(panel.sessions_expanded.contains("devhost"));
    });
}

#[test]
fn persistent_disclosures_refresh_after_disconnect_without_reopening_collapsed_hosts() {
    with_session_panel(|panel, _ctx| {
        panel.resilient_hosts.insert("devhost".to_string());
        panel
            .connected_host_ids
            .insert("devhost".to_string(), vec!["daemon-current".to_string()]);
        panel.auto_reveal_connected_sessions();
        assert!(panel.sessions_expanded.contains("devhost"));

        panel.connected_host_ids.clear();
        assert_eq!(panel.session_refresh_ids(), ["devhost"]);

        panel.sessions_expanded.remove("devhost");
        panel
            .connected_host_ids
            .insert("devhost".to_string(), vec!["daemon-current".to_string()]);
        panel.auto_reveal_connected_sessions();
        assert!(!panel.sessions_expanded.contains("devhost"));
        assert!(panel.session_refresh_ids().is_empty());
    });
}

#[test]
fn session_lifecycle_events_during_inventory_fetch_coalesce_into_one_followup() {
    with_session_panel(|panel, ctx| {
        let generation = panel.begin_session_fetch("devhost").unwrap();
        assert_eq!(panel.begin_session_fetch("devhost"), None);
        assert_eq!(panel.begin_session_fetch("devhost"), None);
        assert_eq!(panel.sessions_loading.len(), 1);
        assert!(panel.complete_session_fetch("devhost", generation, Ok(Default::default()), ctx));

        let next_generation = panel.begin_session_fetch("devhost").unwrap();
        let inventory = crate::remote_server::session_inventory::HostSessionInventory {
            sessions: vec![
                crate::remote_server::session_inventory::RoutedDaemonSession {
                    session: SessionInfo {
                        session_id: "newly-opened-session".to_string(),
                        ..Default::default()
                    },
                    route: None,
                },
            ],
            ..Default::default()
        };
        assert!(!panel.complete_session_fetch("devhost", next_generation, Ok(inventory), ctx));
        assert_eq!(
            panel.host_session_inventories["devhost"].sessions[0]
                .session
                .session_id,
            "newly-opened-session"
        );
        assert!(panel.sessions_loading.is_empty());
        assert!(panel.sessions_refresh_pending.is_empty());
    });
}

#[test]
fn changed_or_deleted_hosts_reject_old_inventory_results() {
    with_session_panel(|panel, ctx| {
        let old_generation = panel.begin_session_fetch("devhost").unwrap();
        panel.invalidate_session_inventories();
        assert_eq!(panel.begin_session_fetch("devhost"), None);
        assert!(panel.complete_session_fetch(
            "devhost",
            old_generation,
            Ok(Default::default()),
            ctx
        ));
        assert!(!panel.host_session_inventories.contains_key("devhost"));

        let changed_generation = panel.begin_session_fetch("devhost").unwrap();
        panel.set_nodes_for_test(Vec::new(), ctx);
        assert!(!panel.complete_session_fetch(
            "devhost",
            changed_generation,
            Err("old endpoint".to_string()),
            ctx
        ));
        assert!(!panel.sessions_error.contains_key("devhost"));
        assert!(panel.sessions_loading.is_empty());

        panel.set_nodes_for_test(vec![server("devhost", None, "Replacement", 0)], ctx);
        let replacement_generation = panel.begin_session_fetch("devhost").unwrap();
        assert!(!panel.complete_session_fetch(
            "devhost",
            changed_generation,
            Ok(Default::default()),
            ctx
        ));
        assert_eq!(
            panel.sessions_loading.get("devhost"),
            Some(&replacement_generation)
        );
        assert!(!panel.host_session_inventories.contains_key("devhost"));
        assert!(!panel.complete_session_fetch(
            "devhost",
            replacement_generation,
            Ok(Default::default()),
            ctx
        ));
        assert!(panel.host_session_inventories.contains_key("devhost"));
    });
}

#[test]
fn empty_titles_show_a_short_identity_within_the_zaplex_session_group() {
    crate::i18n::init(Some("en"));
    let session = remote_server::proto::SessionInfo {
        session_id: "12345678-abcdef".to_string(),
        cwd: "/srv/project".to_string(),
        ..Default::default()
    };

    assert_eq!(
        daemon_session_title(&session),
        "Session · \u{2068}12345678\u{2069}"
    );
}

#[test]
fn normalized_agent_titles_are_preserved() {
    let session = remote_server::proto::SessionInfo {
        title: "Codex · zaplex".to_string(),
        ..Default::default()
    };

    assert_eq!(daemon_session_title(&session), "Codex · zaplex");
}

#[test]
fn multiplexer_kind_selects_only_non_destructive_attach_modes() {
    assert_eq!(
        multiplexer_attach_mode(MultiplexerKind::Tmux as i32, 0),
        Some(MultiplexerAttachMode::Tmux)
    );
    assert_eq!(
        multiplexer_attach_mode(MultiplexerKind::ByobuTmux as i32, 2),
        Some(MultiplexerAttachMode::Tmux)
    );
    assert_eq!(
        multiplexer_attach_mode(MultiplexerKind::ByobuScreen as i32, 0),
        Some(MultiplexerAttachMode::ScreenDetached)
    );
    assert_eq!(
        multiplexer_attach_mode(MultiplexerKind::ByobuScreen as i32, 1),
        Some(MultiplexerAttachMode::ScreenAttached)
    );
    assert_eq!(
        multiplexer_attach_mode(MultiplexerKind::Unspecified as i32, 0),
        None
    );
}

#[test]
fn multiplexer_connection_resolves_onekey_to_effective_ssh_auth() {
    let mut conn = diesel::sqlite::SqliteConnection::establish(":memory:").unwrap();
    conn.run_pending_migrations(persistence::MIGRATIONS)
        .unwrap();
    let credential = SshRepository::create_onekey_credential(
        &mut conn,
        "shared-key",
        "deploy",
        OneKeyCredentialKind::Key,
        Some("/home/deploy/.ssh/id_ed25519"),
    )
    .unwrap();
    let mut info = SshServerInfo::new_default(String::new());
    info.host = "edge.example.com".into();
    info.auth_type = AuthType::OneKey;
    info.username = "ignored-local-user".into();
    info.credential_id = Some(credential.id);
    let node = SshRepository::create_server(&mut conn, None, "edge", &info).unwrap();

    let resolved = resolve_server_for_node(&mut conn, &node.id)
        .unwrap()
        .unwrap();

    assert_eq!(resolved.username, "deploy");
    assert_eq!(resolved.auth_type, AuthType::Key);
    assert_eq!(
        resolved.key_path.as_deref(),
        Some("/home/deploy/.ssh/id_ed25519")
    );
}
