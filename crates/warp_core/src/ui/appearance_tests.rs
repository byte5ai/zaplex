use super::*;
use crate::ui::theme::mock_terminal_colors;
use warpui::color::ColorU;

fn mock_appearance() -> Appearance {
    use crate::ui::theme::Details;
    use super::super::theme::Fill;

    let theme = WarpTheme::new(
        Fill::Solid(ColorU::from_u32(0x000000ff)),
        ColorU::from_u32(0xffffffff),
        Fill::Solid(ColorU::new(18, 123, 156, 255)),
        None,
        Some(Details::Darker),
        mock_terminal_colors(),
        None,
        Some("Dark".to_string()),
        None,
    );
    Appearance::new(
        theme,
        FamilyId(0),
        13.0,
        Weight::Normal,
        FamilyId(1),
        1.4,
        FamilyId(0),
        FamilyId(0),
        DEFAULT_UI_FONT_SIZE,
        Default::default(),
    )
}

/// author: logic
/// Verify that header_font_size and overline_font_size scale with ui_font_size
#[test]
fn test_header_and_overline_font_size_scaling() {
    let appearance = mock_appearance();
    assert_eq!(appearance.header_font_size(), 18.0);
    assert_eq!(appearance.overline_font_size(), 10.0);
}

/// author: logic
/// Verify default value constants
#[test]
fn test_default_constants() {
    assert_eq!(DEFAULT_UI_FONT_SIZE, 12.0);
}

/// author: logic
/// Verify that semantic methods maintain strict size hierarchy
#[test]
fn test_semantic_font_size_ordering() {
    let mut appearance = mock_appearance();
    for size in [8.0, 12.0, 20.0] {
        appearance.set_ui_font_size_test(size);

        let overline = appearance.ui_font_overline();
        let footnote = appearance.ui_font_footnote();
        let body = appearance.ui_font_body();
        let body_large = appearance.ui_font_body_large();
        let subheading = appearance.ui_font_subheading();
        let h3 = appearance.ui_font_heading_3();
        let h2 = appearance.ui_font_heading_2();
        let h1 = appearance.ui_font_heading_1();
        let display = appearance.ui_font_display();
        let hero = appearance.ui_font_hero();

        assert!(overline <= footnote, "overline <= footnote at base={}", size);
        assert!(footnote <= body, "footnote <= body at base={}", size);
        assert!(body <= body_large, "body <= body_large at base={}", size);
        assert!(body_large <= subheading, "body_large <= subheading at base={}", size);
        assert!(subheading <= h3, "subheading <= h3 at base={}", size);
        assert!(h3 <= h2, "h3 <= h2 at base={}", size);
        assert!(h2 <= h1, "h2 <= h1 at base={}", size);
        assert!(h1 <= display, "h1 <= display at base={}", size);
        assert!(display <= hero, "display <= hero at base={}", size);
    }
}
