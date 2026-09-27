use super::{coloru_from_hex_string, ColorU};
use crate::ui::color::{hex_color, hex_color_alpha};

#[derive(serde::Deserialize)]
struct RgbField {
    #[serde(with = "hex_color")]
    color: ColorU,
}

#[derive(serde::Deserialize)]
struct RgbaField {
    #[serde(with = "hex_color_alpha")]
    color: ColorU,
}

#[test]
fn non_ascii_hex_is_rejected_by_direct_and_serialized_color_parsers() {
    for color in ["#aé", "#aé123", "#aé12345", "#🎨12", "#a🎨123"] {
        assert!(coloru_from_hex_string(color).is_err());
        let value = serde_json::json!({"color": color});
        assert!(serde_json::from_value::<RgbField>(value.clone()).is_err());
        assert!(serde_json::from_value::<RgbaField>(value).is_err());
    }
}

#[test]
fn serialized_color_parsers_keep_valid_short_rgb_and_alpha_values() {
    for (color, expected) in [
        ("#AbC", ColorU::new(170, 187, 204, 255)),
        ("#123456", ColorU::new(18, 52, 86, 255)),
    ] {
        let value = serde_json::json!({"color": color});
        assert_eq!(
            serde_json::from_value::<RgbField>(value.clone())
                .unwrap()
                .color,
            expected
        );
        assert_eq!(
            serde_json::from_value::<RgbaField>(value).unwrap().color,
            expected
        );
    }
    let rgba: RgbaField =
        serde_json::from_value(serde_json::json!({"color": "#12345680"})).unwrap();
    assert_eq!(rgba.color, ColorU::new(18, 52, 86, 128));
}
