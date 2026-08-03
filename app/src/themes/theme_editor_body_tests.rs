use super::*;
use crate::appearance::{self, Appearance};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::themes::default_themes::dark_theme;
use warpui::assets::asset_cache::AssetSource;
use warpui::platform::WindowStyle;
use warpui::{App, SingletonEntity as _, View as _};

#[test]
fn window_close_clears_transient_theme_preview() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);
        appearance::register(&mut app);

        let original_theme = app.update(|ctx| Appearance::as_ref(ctx).theme().clone());
        let (_, editor) = app.add_window(WindowStyle::NotStealFocus, ThemeEditorBody::new);
        editor.update(&mut app, |editor, ctx| {
            let mut preview = light_theme();
            preview.set_name("Transient window-close preview");
            editor.load_theme(preview.clone(), preview.name().to_owned(), true, None, ctx);
            assert_eq!(Appearance::as_ref(ctx).theme().name(), preview.name());

            editor.on_window_closed(ctx);
            assert_eq!(Appearance::as_ref(ctx).theme(), &original_theme);
        });
    });
}

#[test]
fn yaml_color_update_preserves_unrelated_theme_fields() {
    let theme = dark_theme();
    let image = theme.background_image();
    let updated = set_theme_color(
        &theme,
        &["terminal_colors", "normal", "red"],
        ColorU::new(1, 2, 3, 255),
    )
    .unwrap();
    assert_eq!(
        ansi_color(&updated, false, "red"),
        ColorU::new(1, 2, 3, 255)
    );
    assert_eq!(updated.background_image(), image);
    assert_eq!(updated.background(), theme.background());
}

#[test]
fn removing_ui_overrides_restores_derived_colors() {
    let theme = dark_theme();
    let overridden =
        set_theme_color(&theme, &["ui_colors", "border"], ColorU::new(1, 2, 3, 255)).unwrap();
    assert_eq!(overridden.outline().into_solid(), ColorU::new(1, 2, 3, 255));
    assert_eq!(
        remove_theme_key(&overridden, "ui_colors")
            .unwrap()
            .outline(),
        theme.outline()
    );
}

#[test]
fn resetting_ui_overrides_preserves_primary_outline_and_selection() {
    let theme = dark_theme();
    let outline = ColorU::new(1, 2, 3, 255);
    let selection = ColorU::new(4, 5, 6, 255);
    let link = ColorU::new(7, 8, 9, 255);
    let theme = set_theme_color(&theme, &["ui_colors", "border"], outline).unwrap();
    let theme = set_theme_color(&theme, &["ui_colors", "selection"], selection).unwrap();
    let theme = set_theme_color(&theme, &["ui_colors", "link"], link).unwrap();

    let reset = reset_ui_color_overrides(&theme).unwrap();

    assert_eq!(reset.outline().into_solid(), outline);
    assert_eq!(reset.block_selection_color().into_solid(), selection);
    assert_eq!(configured_ui_color(&reset, "link"), None);
}

#[test]
fn theme_filename_preserves_letters_and_separates_words() {
    assert_eq!(
        safe_theme_filename(" Nächtliche Werkstatt "),
        "Nächtliche-Werkstatt"
    );
    assert_eq!(safe_theme_filename("alpha / beta"), "alpha-beta");
}

#[test]
fn image_path_update_preserves_the_rest_of_the_theme() {
    let theme = dark_theme();
    let image_path = std::env::temp_dir().join("theme-background.png");
    let updated = set_theme_image_path(&theme, &image_path).unwrap();
    let image = updated.background_image().unwrap();
    assert_eq!(
        image.source,
        AssetSource::LocalFile {
            path: image_path.to_string_lossy().into_owned(),
        }
    );
    assert_eq!(updated.background(), theme.background());
    assert_eq!(updated.terminal_colors(), theme.terminal_colors());
}

#[test]
fn authored_gradients_survive_yaml_export_and_import() {
    let background_start = ColorU::new(10, 20, 30, 255);
    let background_end = ColorU::new(40, 50, 60, 255);
    let accent_start = ColorU::new(70, 80, 90, 255);
    let accent_end = ColorU::new(100, 110, 120, 255);
    let theme = set_theme_fill(
        &dark_theme(),
        "background",
        ThemeFill::VerticalGradient(VerticalGradient::new(background_start, background_end)),
    )
    .unwrap();
    let theme = set_theme_fill(
        &theme,
        "accent",
        ThemeFill::HorizontalGradient(HorizontalGradient::new(accent_start, accent_end)),
    )
    .unwrap();

    let yaml = serde_yaml::to_string(&theme).unwrap();
    let imported: WarpTheme = serde_yaml::from_str(&yaml).unwrap();

    assert_eq!(
        fill_colors(imported.background()),
        (background_start, background_end)
    );
    assert_eq!(fill_colors(imported.accent()), (accent_start, accent_end));
}

#[test]
fn yaml_round_trip_preserves_ansi_and_ui_color_overrides() {
    let ansi_red = ColorU::new(201, 32, 44, 255);
    let warning = ColorU::new(244, 184, 96, 255);
    let theme = set_theme_color(
        &dark_theme(),
        &["terminal_colors", "normal", "red"],
        ansi_red,
    )
    .unwrap();
    let theme = set_theme_color(&theme, &["ui_colors", "warning"], warning).unwrap();

    let yaml = serde_yaml::to_string(&theme).unwrap();
    let imported: WarpTheme = serde_yaml::from_str(&yaml).unwrap();

    assert_eq!(ansi_color(&imported, false, "red"), ansi_red);
    assert_eq!(configured_ui_color(&imported, "warning"), Some(warning));
}
