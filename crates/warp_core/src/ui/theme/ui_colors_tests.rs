use super::*;

/// Verifies that UiColors deserializes correctly when every field is Option::None (an empty ui_colors block).
#[test]
fn deserialize_empty_ui_colors() {
    let yaml = r##"---
{}
"##;
    let colors: UiColors = serde_yaml::from_str(yaml).expect("deserialization failed");
    assert!(colors.surface_1.is_none());
    assert!(colors.border.is_none());
    assert!(colors.main_text.is_none());
}

/// Verifies that UiColors correctly deserializes colors that include an alpha channel.
#[test]
fn deserialize_ui_colors_with_alpha() {
    let yaml = r##"---
surface_1: "#202122"
surface_2: "#242526"
surface_3: "#2A2B2C"
border: "#333536"
focus_border: "#3994BCB3"
selection: "#3994BC33"
hover: "#FFFFFF0D"
"##;
    let colors: UiColors = serde_yaml::from_str(yaml).expect("deserialization failed");

    assert_eq!(colors.surface_1.unwrap(), ColorU { r: 0x20, g: 0x21, b: 0x22, a: 255 });
    assert_eq!(colors.surface_2.unwrap(), ColorU { r: 0x24, g: 0x25, b: 0x26, a: 255 });
    assert_eq!(colors.focus_border.unwrap(), ColorU { r: 0x39, g: 0x94, b: 0xBC, a: 0xB3 });
    assert_eq!(colors.selection.unwrap(), ColorU { r: 0x39, g: 0x94, b: 0xBC, a: 0x33 });
    assert_eq!(colors.hover.unwrap(), ColorU { r: 0xFF, g: 0xFF, b: 0xFF, a: 0x0D });
    // Fields that were not set should be None
    assert!(colors.main_text.is_none());
}
