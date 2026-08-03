use crate::util::color::OPAQUE;

use super::*;

// TODO(CORE-3626): figure out why the colors returned on Windows are slightly different.
#[test]
#[cfg(all(not(target_family = "wasm"), not(windows)))]
fn top_colors_jellyfish_test() {
    let jellyfish_bg_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "assets",
        "async",
        "jpg",
        "jellyfish_bg.jpg",
    ]
    .iter()
    .collect();

    let colors = top_colors_for_image(jellyfish_bg_path)
        .expect("should be able to get colors from jellyfish bg");

    assert_eq!(colors[0], ColorU::new(14, 13, 30, OPAQUE));
    assert_eq!(colors[1], ColorU::new(23, 22, 55, OPAQUE));
    assert_eq!(colors[2], ColorU::new(94, 38, 70, OPAQUE));
    assert_eq!(colors[3], ColorU::new(52, 68, 91, OPAQUE));
    assert_eq!(colors[4], ColorU::new(112, 118, 129, OPAQUE));
}

#[test]
#[cfg(not(target_family = "wasm"))]
fn top_colors_invalid_image_test() {
    let invalid_image_path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "assets",
        "async",
        "jpg",
        "this_doesnt_exist.jpg",
    ]
    .iter()
    .collect();

    let colors = top_colors_for_image(invalid_image_path);
    assert!(colors.is_err());
}

#[test]
fn accent_colors_contrast_test() {
    let foreground = ColorU::white();
    let background = ColorU::black();
    let accent_options = [
        ColorU::new(255, 0, 0, OPAQUE),
        ColorU::new(100, 0, 0, OPAQUE),
        ColorU::new(10, 0, 0, OPAQUE),
    ];
    assert_eq!(
        accent_options[1],
        pick_accent_color_from_options(&[background, foreground], &accent_options)
    );
}

#[test]
fn theme_color_input_accepts_hex_and_rgb() {
    let expected = ColorU::new(12, 34, 56, 255);
    assert_eq!(parse_theme_color_input("#0C2238").unwrap(), expected);
    assert_eq!(parse_theme_color_input("0c2238").unwrap(), expected);
    assert_eq!(
        parse_theme_color_input("rgb(12, 34, 56)").unwrap(),
        expected
    );
    assert_eq!(
        parse_theme_color_input("RGB(12, 34, 56)").unwrap(),
        expected
    );
    assert_eq!(
        parse_theme_color_input("#0C223880").unwrap(),
        ColorU::new(12, 34, 56, 128)
    );
    assert_eq!(
        format_theme_color(ColorU::new(12, 34, 56, 128)),
        "#0C223880"
    );
}

#[test]
fn invalid_theme_color_input_fails_without_a_fallback_color() {
    assert!(parse_theme_color_input("rgb(12, 34)").is_err());
    assert!(parse_theme_color_input("#not-a-color").is_err());
}
