#[test]
fn ime_position_callback_uses_matching_nsrect_abi() {
    let host_view_source = include_str!("objc/host_view.m");

    assert!(host_view_source.contains("NSRect warp_ime_position(WarpHostView *, NSRect);"));
    assert!(host_view_source.contains("warp_ime_position(self, contentRect)"));
    assert!(!host_view_source.contains("NSRect warp_ime_position(WarpHostView *, NSRect *);"));
    assert!(!host_view_source.contains("warp_ime_position(self, &contentRect)"));
}
