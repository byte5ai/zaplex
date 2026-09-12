use super::drawable_for_render;

#[test]
fn missing_metal_drawable_skips_the_frame() {
    assert!(drawable_for_render(None).is_none());
}
