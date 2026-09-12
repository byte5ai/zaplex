use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

static CLOSE_CALLS: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn record_close(_handle: HPCON) {
    CLOSE_CALLS.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn owned_pseudoconsole_closes_exactly_once_on_drop() {
    CLOSE_CALLS.store(0, Ordering::SeqCst);
    let handle = HPCON(1_isize as *mut _);

    drop(OwnedPseudoConsole::new(handle, record_close));

    assert_eq!(CLOSE_CALLS.load(Ordering::SeqCst), 1);
}
