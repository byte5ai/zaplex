use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use super::GuardSlot;

struct FakeGuard {
    stop_count: Arc<AtomicUsize>,
}

impl Drop for FakeGuard {
    fn drop(&mut self) {
        self.stop_count.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn minidump_backend_repeated_initialization_starts_once_and_uninitialization_stops_once() {
    let start_count = Arc::new(AtomicUsize::new(0));
    let stop_count = Arc::new(AtomicUsize::new(0));
    let mut slot = GuardSlot::default();

    for expected_started in [true, false] {
        let start_count = start_count.clone();
        let stop_count = stop_count.clone();
        let started = slot
            .initialize(|| {
                start_count.fetch_add(1, Ordering::SeqCst);
                Ok::<_, ()>(FakeGuard { stop_count })
            })
            .expect("fake minidump backend should initialize");
        assert_eq!(started, expected_started);
    }

    assert_eq!(start_count.load(Ordering::SeqCst), 1);
    assert_eq!(stop_count.load(Ordering::SeqCst), 0);

    drop(slot.take());
    drop(slot.take());

    assert_eq!(stop_count.load(Ordering::SeqCst), 1);
}

#[test]
fn minidump_backend_disabled_default_starts_no_backend() {
    let slot = GuardSlot::<FakeGuard>::default();

    assert_eq!(slot.as_ref().is_none(), true);
}
