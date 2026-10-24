use cocoa::base::{id, nil};
use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize};
use objc::runtime::{NO, YES};
use objc::{class, msg_send, sel, sel_impl};

use super::{to_string, warp_should_dispatch_native_window_chrome_event};

extern "C" {
    fn warp_constrain_window_frame(frame: NSRect, minimum_size: NSSize) -> NSRect;
}

#[test]
fn undersized_window_frames_grow_without_moving_the_top_left_corner() {
    let minimum = NSSize::new(480., 320.);
    for (width, height, expected_width, expected_height, expected_y) in [
        (100., 100., 480., 320., -170.),
        (300., 600., 480., 600., 50.),
        (800., 100., 800., 320., -170.),
        (480., 320., 480., 320., 50.),
        (1024., 678., 1024., 678., 50.),
    ] {
        let requested = NSRect::new(NSPoint::new(25., 50.), NSSize::new(width, height));
        let constrained = unsafe { warp_constrain_window_frame(requested, minimum) };
        assert_eq!(constrained.origin.x, 25.);
        assert_eq!(constrained.origin.y, expected_y);
        assert_eq!(constrained.size.width, expected_width);
        assert_eq!(constrained.size.height, expected_height);
    }
}

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
