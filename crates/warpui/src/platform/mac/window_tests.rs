use objc::runtime::{NO, YES};

use super::warp_should_dispatch_native_window_chrome_event;

#[test]
fn native_chrome_dispatch_requires_supported_os_and_matching_mouse_down() {
    unsafe {
        assert_eq!(
            warp_should_dispatch_native_window_chrome_event(YES, YES),
            YES
        );
        assert_eq!(warp_should_dispatch_native_window_chrome_event(YES, NO), NO);
        assert_eq!(warp_should_dispatch_native_window_chrome_event(NO, YES), NO);
        assert_eq!(warp_should_dispatch_native_window_chrome_event(NO, NO), NO);
    }
}
