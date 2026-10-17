use warp_core::ui::Icon;
use warpui::assets::asset_cache::AssetSource;
use warpui::elements::MouseStateHandle;

use super::{ChipHorizontalAlignment, MessageItem};

#[test]
fn set_color_sets_icon_color_override() {
    let color = pathfinder_color::ColorU {
        r: 1,
        g: 2,
        b: 3,
        a: 255,
    };
    let mut item = MessageItem::icon(Icon::MessagePlusSquare);
    item.set_color(color);

    match item {
        MessageItem::Icon {
            color: Some(item_color),
            ..
        } => assert_eq!(item_color, color),
        _ => panic!("expected icon variant with color override"),
    }
}

#[test]
fn image_constructor_creates_image_with_correct_dimensions() {
    let item = MessageItem::image(
        AssetSource::Bundled {
            path: "bundled/svg/test.svg",
        },
        16.,
        16.,
    );

    match item {
        MessageItem::Image {
            source,
            width,
            height,
        } => {
            assert!(matches!(
                source,
                AssetSource::Bundled {
                    path: "bundled/svg/test.svg"
                }
            ));
            assert_eq!(width, 16.);
            assert_eq!(height, 16.);
        }
        _ => panic!("expected image variant"),
    }
}

#[test]
fn with_horizontal_alignment_sets_right_alignment() {
    let item = MessageItem::chip(
        vec![MessageItem::text("hello")],
        |_ctx| {},
        MouseStateHandle::default(),
    )
    .with_horizontal_alignment(ChipHorizontalAlignment::Right);

    match item {
        MessageItem::Chip {
            horizontal_alignment,
            ..
        } => assert_eq!(horizontal_alignment, ChipHorizontalAlignment::Right),
        _ => panic!("expected chip variant"),
    }
}
