use super::*;
use pathfinder_geometry::vector::vec2f;
use std::cell::RefCell;
use std::rc::Rc;
use warpui::platform::WindowStyle;
use warpui::{App, Event, Presenter, WindowInvalidation};

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
