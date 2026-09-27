use super::*;
use pathfinder_geometry::vector::vec2f;
use std::cell::RefCell;
use std::rc::Rc;
use warpui::platform::WindowStyle;
use warpui::{App, Event, Presenter, WindowInvalidation};
use zaplex_cockpit::TaskItem;

use crate::test_util::settings::initialize_settings_for_tests;

#[test]
fn fleet_total_button_accepts_button_activation_keys_only() {
    for key in ["enter", "numpadenter", " "] {
        assert!(is_fleet_total_activation_keystroke(
            &warpui::keymap::Keystroke {
                key: key.to_string(),
                ..Default::default()
            }
        ));
    }
    assert!(!is_fleet_total_activation_keystroke(
        &warpui::keymap::Keystroke {
            key: "right".to_string(),
            ..Default::default()
        }
    ));
    assert!(!is_fleet_total_activation_keystroke(
        &warpui::keymap::Keystroke {
            ctrl: true,
            key: " ".to_string(),
            ..Default::default()
        }
    ));
}

#[test]
fn focused_fleet_total_button_handles_real_unmodified_keydown_forms_only() {
    App::test((), |mut app| async move {
        crate::i18n::init(Some("en"));
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        let (window_id, button) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            let button = FleetTotalButton::new(ctx);
            ctx.focus_self();
            button
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
                    .build_scene(vec2f(250.0, 80.0), 1.0, None, ctx);
            });
        };
        let press = |app: &mut App, keystroke: warpui::keymap::Keystroke, chars: &str| {
            app.update(|ctx| {
                ctx.simulate_window_event(
                    Event::KeyDown {
                        keystroke,
                        chars: chars.to_string(),
                        details: warpui::event::KeyEventDetails::default(),
                        is_composing: false,
                    },
                    window_id,
                    presenter.clone(),
                )
            })
        };

        render(&mut app);
        assert!(press(
            &mut app,
            warpui::keymap::Keystroke {
                key: "enter".to_string(),
                ..Default::default()
            },
            "\r",
        ));
        render(&mut app);
        assert!(press(
            &mut app,
            warpui::keymap::Keystroke {
                key: "numpadenter".to_string(),
                ..Default::default()
            },
            "\r",
        ));
        render(&mut app);
        assert!(press(
            &mut app,
            warpui::keymap::Keystroke {
                key: " ".to_string(),
                ..Default::default()
            },
            " ",
        ));
        render(&mut app);
        let _ = press(
            &mut app,
            warpui::keymap::Keystroke {
                shift: true,
                key: "enter".to_string(),
                ..Default::default()
            },
            "\r",
        );
        button.read(&app, |button, _| assert_eq!(button.activation_count, 3));
    });
}

#[test]
fn current_task_prefers_in_progress_step() {
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

    assert_eq!(current_task_title(&state), Some("Current"));
}

#[test]
fn current_task_falls_back_to_first_pending_step() {
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
    assert_eq!(current_task_title(&state), Some("Next"));
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
fn removed_host_is_marked_and_cannot_attach_agents() {
    let host = HostNode {
        host: "devhost".to_string(),
        is_local: false,
        host_id: Some("daemon-dev".to_string()),
        availability: HostAvailability::Removed,
        inventory_status: zaplex_cockpit::AgentInventoryStatus::Ready,
        // Deliberately retain a stale id in this presentation-level test: the
        // explicit state, not incidental id clearing, must close every route.
        registry_node_id: Some("node-dev".to_string()),
        needs_me: 1,
        projects: Vec::new(),
    };
    assert_eq!(
        host_display_label(
            &host,
            "removed from Connections",
            "Connections registry unavailable"
        ),
        "devhost — removed from Connections"
    );
    assert!(
        !host.is_available(),
        "removed daemon data is visible but cannot seed session click routes"
    );
}

#[test]
fn unverified_host_is_labeled_and_not_routable() {
    let host = HostNode {
        host: "devhost".to_string(),
        is_local: false,
        host_id: Some("daemon-dev".to_string()),
        availability: HostAvailability::Unverified,
        inventory_status: zaplex_cockpit::AgentInventoryStatus::Ready,
        registry_node_id: Some("node-dev".to_string()),
        needs_me: 1,
        projects: Vec::new(),
    };
    assert_eq!(
        host_display_label(
            &host,
            "removed from Connections",
            "Connections registry unavailable"
        ),
        "devhost — Connections registry unavailable"
    );
    assert!(!host.is_available());
}

#[test]
fn expanded_containers_hide_counts() {
    assert_eq!(container_count_presentation(true, 7, 2), None);
}

#[test]
fn collapsed_counts_carry_hidden_attention_only() {
    assert_eq!(
        container_count_presentation(false, 7, 2),
        Some(ContainerCountPresentation {
            count: 7,
            attention: true,
        })
    );
    assert_eq!(
        container_count_presentation(false, 7, 0),
        Some(ContainerCountPresentation {
            count: 7,
            attention: false,
        })
    );
}

#[test]
fn agent_leaf_separates_provider_from_the_exact_optional_model() {
    assert_eq!(
        agent_leaf_presentation(Provider::Claude, "claude-opus-4-8-20260901"),
        AgentLeafPresentation {
            provider: "Claude",
            model: Some("claude-opus-4-8-20260901"),
        }
    );
    assert_eq!(
        agent_leaf_presentation(Provider::Codex, "  "),
        AgentLeafPresentation {
            provider: "Codex",
            model: None,
        }
    );
}

#[test]
fn session_identity_avoids_only_exact_project_directory_duplication() {
    assert_eq!(project_directory_label("zaplex", "/work/zaplex"), "zaplex");
    assert_eq!(
        project_directory_label("zaplex", "/work/Zaplex"),
        "zaplex — Zaplex"
    );
}

#[test]
fn tree_status_is_glyph_only() {
    let cases = [
        (
            SessionState::Waiting,
            crate::t!("cockpit-task-peek-state-waiting"),
        ),
        (
            SessionState::Active,
            crate::t!("cockpit-task-peek-state-working"),
        ),
        (
            SessionState::Monitor,
            crate::t!("cockpit-task-peek-state-working"),
        ),
        (
            SessionState::Idle,
            crate::t!("cockpit-task-peek-state-idle"),
        ),
    ];

    for (state, expected_semantic_label) in cases {
        let presentation = session_glyph_presentation(state);
        assert_eq!(presentation.visible_label, session_glyph(state));
        assert!(
            !presentation.visible_label.chars().any(char::is_alphabetic),
            "the visible tree state must remain glyph-only"
        );
        assert_eq!(presentation.semantic_label, expected_semantic_label);
        assert!(
            !presentation.semantic_label.trim().is_empty(),
            "every state glyph needs a localized semantic description"
        );
    }
}

#[test]
fn waiting_pulse_is_fixed_and_capped_at_twice_the_core() {
    let start = waiting_pulse_frame(Duration::ZERO, true);
    let near_end = waiting_pulse_frame(Duration::from_millis(1599), true);

    assert!(start.repaint);
    assert!(near_end.repaint);
    assert!((88..=100).contains(&start.core_opacity));
    assert!((88..=100).contains(&near_end.core_opacity));
    assert!(near_end.ring_diameter <= WAITING_GLYPH_CORE_DIAMETER * 2.0);
    assert!(near_end.ring_diameter > WAITING_GLYPH_CORE_DIAMETER * 1.99);
    assert_eq!(WAITING_GLYPH_FOOTPRINT, GLYPH_COL_WIDTH);
}

#[test]
fn reduced_motion_uses_static_waiting_emphasis() {
    let frame = waiting_pulse_frame(Duration::from_secs(30), false);

    assert!(!frame.repaint);
    assert_eq!(frame.core_opacity, 100);
    assert_eq!(frame.ring_diameter, WAITING_GLYPH_CORE_DIAMETER * 1.45);
    assert!(frame.ring_opacity > 0);
}

#[test]
fn waiting_glyph_motion_respects_reduced_motion() {
    let animated = waiting_pulse_frame(Duration::from_millis(1599), true);
    assert!(animated.repaint);
    assert!(animated.ring_diameter <= WAITING_GLYPH_CORE_DIAMETER * 2.0);

    let reduced = waiting_pulse_frame(Duration::from_millis(1599), false);
    assert!(!reduced.repaint);
    assert_eq!(reduced.core_opacity, 100);
    assert_eq!(reduced.ring_diameter, WAITING_GLYPH_CORE_DIAMETER * 1.45);
}

#[test]
fn waiting_pulse_repaint_interval_is_battery_bounded() {
    assert!(WAITING_PULSE_REPAINT >= Duration::from_millis(100));
}

#[test]
fn waiting_pulse_animates_only_in_a_focused_window_without_reduced_motion() {
    assert!(waiting_pulse_should_animate(false, true));
    assert!(!waiting_pulse_should_animate(false, false));
    assert!(!waiting_pulse_should_animate(true, true));
}

#[test]
fn account_scan_error_is_not_rendered_as_zero_accounts() {
    assert_eq!(
        account_count_presentation(&zaplex_cockpit::ScanHealth::Pending, 0),
        None
    );
    assert_eq!(
        account_count_presentation(
            &zaplex_cockpit::ScanHealth::Degraded("account source unreadable".into()),
            0,
        ),
        None
    );
    assert_eq!(
        account_count_presentation(&zaplex_cockpit::ScanHealth::Loaded, 0),
        Some(0)
    );
    assert_eq!(
        account_count_presentation(
            &zaplex_cockpit::ScanHealth::Degraded("one source failed".into()),
            1,
        ),
        Some(1)
    );
}
