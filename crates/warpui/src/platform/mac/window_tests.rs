use cocoa::base::{id, nil};
use cocoa::foundation::NSAutoreleasePool;
use objc::{class, msg_send, sel, sel_impl};

use super::to_string;

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
