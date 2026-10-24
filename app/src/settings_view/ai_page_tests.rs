use pathfinder_color::ColorU;
use warp_core::ui::theme::{mock_terminal_colors, Details, WarpTheme};
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity};

use crate::appearance::Appearance;
use crate::settings_view::keybindings::KeybindingChangedNotifier;
use crate::test_util::settings::initialize_settings_for_tests;

use super::new_autodetection_denylist_editor;

#[test]
fn denylist_editor_follows_theme_without_losing_draft_selection_or_focus() {
    App::test((), |mut app| async move {
        crate::i18n::init(Some("en"));
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
        app.add_singleton_model(|_| KeybindingChangedNotifier::mock());
        let (window, editor) = app.add_window(
            WindowStyle::NotStealFocus,
            new_autodetection_denylist_editor,
        );
        editor.update(&mut app, |editor, ctx| {
            editor.set_buffer_text("draft-zaplex136", ctx);
            ctx.focus_self();
            editor.select_all(ctx);
        });
        let appearance = Appearance::handle(&app);
        let dark = appearance.read(&app, |appearance, _| appearance.theme().clone());
        let light = WarpTheme::new(
            ColorU::white().into(),
            ColorU::black(),
            ColorU::new(18, 123, 156, 255).into(),
            None,
            Some(Details::Lighter),
            mock_terminal_colors(),
            None,
            Some("Light".to_string()),
            None,
        );
        for theme in [light, dark] {
            let expected_text = theme.active_ui_text_color();
            let expected_disabled = theme.disabled_ui_text_color();
            appearance.update(&mut app, |appearance, ctx| appearance.set_theme(theme, ctx));
            editor.read(&app, |editor, ctx| {
                let colors = editor.text_colors(Appearance::as_ref(ctx));
                assert_eq!(colors.default_color, expected_text);
                assert_eq!(colors.disabled_color, expected_disabled);
                assert_eq!(colors.hint_color, expected_disabled);
                assert_eq!(editor.buffer_text(ctx), "draft-zaplex136");
                assert_eq!(editor.selected_text(ctx), "draft-zaplex136");
            });
            assert_eq!(app.focused_view_id(window), Some(editor.id()));
        }
    });
}
