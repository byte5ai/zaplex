use cocoa::base::{id, nil};
use cocoa::foundation::NSAutoreleasePool;
use objc::runtime::{NO, YES};
use objc::{class, msg_send, sel, sel_impl};

use super::{to_string, warp_should_dispatch_native_window_chrome_event};

unsafe fn string_from_utf16(code_units: &[u16]) -> id {
    let string: id = msg_send![class!(NSString), alloc];
    msg_send![string, initWithCharacters: code_units.as_ptr() length: code_units.len()]
}

#[test]
fn to_string_handles_unpaired_surrogate() {
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        let string = string_from_utf16(&[0xDDDD]);

        assert_eq!(to_string(string), "");

        let _: () = msg_send![string, release];
        pool.drain();
    }
}

#[test]
fn to_string_preserves_embedded_null() {
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        let string = string_from_utf16(&['a' as u16, 0, 'b' as u16]);

        assert_eq!(to_string(string), "a\0b");

        let _: () = msg_send![string, release];
        pool.drain();
    }
}

#[test]
fn window_button_gestures_stay_native_on_every_supported_macos() {
    unsafe {
        for resize_supported in [NO, YES] {
            for on_resize_edge in [NO, YES] {
                assert_eq!(
                    warp_should_dispatch_native_window_chrome_event(
                        YES,
                        on_resize_edge,
                        resize_supported,
                    ),
                    YES,
                );
            }
        }
    }
}

#[test]
fn content_gestures_keep_the_tab_and_pane_drag_workaround() {
    unsafe {
        for resize_supported in [NO, YES] {
            assert_eq!(
                warp_should_dispatch_native_window_chrome_event(NO, NO, resize_supported),
                NO,
            );
        }
    }
}

#[test]
fn resize_edge_gestures_keep_the_os_compatibility_gate() {
    unsafe {
        assert_eq!(
            warp_should_dispatch_native_window_chrome_event(NO, YES, YES),
            YES
        );
        assert_eq!(
            warp_should_dispatch_native_window_chrome_event(NO, YES, NO),
            NO
        );
    }
}
