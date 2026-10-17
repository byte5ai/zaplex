use std::path::PathBuf;
use std::time::Instant;

use notify_debouncer_full::notify::{
    event::{ModifyKind, RenameMode},
    Event, EventKind,
};
use notify_debouncer_full::DebouncedEvent;

use super::deduplicate_and_merge_raw_notifier_events;

fn rename_event(mode: RenameMode, paths: &[&str]) -> DebouncedEvent {
    let event = paths.iter().fold(
        Event::new(EventKind::Modify(ModifyKind::Name(mode))),
        |event, path| event.add_path(PathBuf::from(path)),
    );
    DebouncedEvent::new(event, Instant::now())
}

#[test]
fn correlated_both_is_only_a_move() {
    let update = deduplicate_and_merge_raw_notifier_events(&[rename_event(
        RenameMode::Both,
        &["/watched/from", "/watched/to"],
    )])
    .unwrap();

    assert_eq!(
        update.moved.get(&PathBuf::from("/watched/to")),
        Some(&PathBuf::from("/watched/from"))
    );
    assert!(update.added.is_empty());
    assert!(update.deleted.is_empty());
}
