use warpui::fonts::Weight;

use super::{
    BufferBlockItem, BufferTextStyle, MarkdownStyle, TextStyles,
};

#[test]
fn test_text_style_xor() {
    // This test makes sure that the `TextStyles` XOR implementations are updated as we add new styles.
    for style in enum_iterator::all::<BufferTextStyle>() {
        let mut with_style = TextStyles::default();

        match style {
            BufferTextStyle::Weight(weight) => {
                with_style.set_weight(Weight::from_custom_weight(Some(weight)));
            }
            style => {
                if let Some(style_mut) = with_style.style_mut(&style) {
                    *style_mut = true;
                } else {
                    panic!("Impossible code path -- style {style:?} not handled");
                }
            }
        }

        assert!(
            (with_style ^ TextStyles::default()).colliding_style(&style),
            "Set ^ Unset = Set failed for {style:?}"
        );
        assert!(
            (TextStyles::default() ^ with_style).colliding_style(&style),
            "Unset ^ Set = Set failed for {style:?}"
        );
        assert!(
            !(with_style ^ with_style).colliding_style(&style),
            "Set ^ Set = Unset failed for {style:?}"
        );
        assert!(
            !(TextStyles::default() ^ TextStyles::default()).colliding_style(&style),
            "Unset ^ Unset = Unset failed for {style:?}"
        );

        let mut editable = with_style;

        editable ^= with_style;
        assert!(
            !editable.colliding_style(&style),
            "Set ^= Set -> Unset failed for {style:?}"
        );

        editable ^= TextStyles::default();
        assert!(
            !editable.colliding_style(&style),
            "Unset ^= Unset -> Unset failed for {style:?}"
        );

        editable ^= with_style;
        assert!(
            editable.colliding_style(&style),
            "Unset ^= Set -> Set failed for {style:?}"
        );

        editable ^= TextStyles::default();
        assert!(
            editable.colliding_style(&style),
            "Set ^= Unset -> Set failed for {style:?}"
        );
    }
}

#[test]
fn buffer_block_image_as_markdown_preserves_title() {
    let untitled = BufferBlockItem::Image {
        alt_text: "A dog".to_string(),
        source: "dog.png".to_string(),
        title: None,
    };
    assert_eq!(
        &*untitled.as_markdown(MarkdownStyle::Internal),
        "![A dog](dog.png)"
    );

    let titled = BufferBlockItem::Image {
        alt_text: "A dog".to_string(),
        source: "dog.png".to_string(),
        title: Some("Rex, my dog".to_string()),
    };
    assert_eq!(
        &*titled.as_markdown(MarkdownStyle::Internal),
        "![A dog](dog.png \"Rex, my dog\")"
    );
}

#[test]
fn buffer_block_image_partial_eq_considers_title() {
    let untitled = BufferBlockItem::Image {
        alt_text: "A dog".to_string(),
        source: "dog.png".to_string(),
        title: None,
    };
    let titled = BufferBlockItem::Image {
        alt_text: "A dog".to_string(),
        source: "dog.png".to_string(),
        title: Some("Rex".to_string()),
    };
    assert_ne!(untitled, titled);

    let titled_again = BufferBlockItem::Image {
        alt_text: "A dog".to_string(),
        source: "dog.png".to_string(),
        title: Some("Rex".to_string()),
    };
    assert_eq!(titled, titled_again);
}
