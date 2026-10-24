use pathfinder_color::ColorU;
use warp_core::ui::theme::{mock_terminal_colors, Details, WarpTheme};
use warpui::{App, SingletonEntity};

use crate::appearance::Appearance;
use crate::context_chips::{ChipAvailability, ChipDisabledReason, ChipValue, ContextChipKind};
use crate::test_util::settings::initialize_settings_for_tests;

use super::Renderer;

#[test]
fn test_constructor_availability_updates_disabled_state_and_tooltip_override() {
    crate::i18n::init(Some("en"));
    let kind = ContextChipKind::ShellGitBranch;
    let chip = kind.to_chip().expect("chip definition should exist");
    let renderer = Renderer::new(
        kind,
        chip,
        ChipValue::Text("main".to_string()),
        true,
        ChipAvailability::Disabled(ChipDisabledReason::RequiresExecutable {
            command: "gh".to_string(),
        }),
    );

    assert!(renderer.is_disabled);
    assert_eq!(
        renderer.tooltip_override_text.as_deref(),
        Some("Requires the GitHub CLI")
    );
}

#[test]
fn existing_agent_chip_follows_dark_light_dark_without_losing_its_value() {
    App::test((), |mut app| async move {
        crate::i18n::init(Some("en"));
        initialize_settings_for_tests(&mut app);
        app.add_singleton_model(|_| Appearance::mock());
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
        let kind = ContextChipKind::ShellGitBranch;
        let renderer = Renderer::new(
            kind.clone(),
            kind.to_chip().unwrap(),
            ChipValue::Text("unsaved-branch".to_string()),
            true,
            ChipAvailability::Enabled,
        );
        let mut disabled = renderer.clone();
        disabled.is_disabled = true;
        // Keep the same renderer alive across both theme changes, as the settings editor does.
        for (theme, is_light) in [(dark.clone(), false), (light, true), (dark, false)] {
            appearance.update(&mut app, |appearance, ctx| appearance.set_theme(theme, ctx));
            appearance.read(&app, |appearance, _| {
                let color = renderer.value_color(appearance);
                for channel in [color.r, color.g, color.b] {
                    assert_eq!(
                        channel < 128,
                        is_light,
                        "chip must contrast with its background: {color:?}"
                    );
                }
                assert_eq!(color.a, 153);
                let disabled_color = disabled.value_color(appearance);
                assert_eq!(
                    (disabled_color.r, disabled_color.g, disabled_color.b),
                    (color.r, color.g, color.b)
                );
                assert_eq!(disabled_color.a, 76);
            });
            assert_eq!(renderer.kind, ContextChipKind::ShellGitBranch);
            assert_eq!(
                renderer.kind.display_value(&renderer.value),
                "git:(unsaved-branch)"
            );
            assert!(!renderer.is_disabled);
        }
    });
}
