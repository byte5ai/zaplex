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
fn lone_rename_to_is_added() {
    let update = deduplicate_and_merge_raw_notifier_events(&[rename_event(
        RenameMode::To,
        &["/watched/in"],
    )])
    .unwrap();

    assert_eq!(update.added, [PathBuf::from("/watched/in")].into());
    assert!(update.deleted.is_empty());
    assert!(update.moved.is_empty());
}

#[test]
fn lone_rename_from_is_deleted() {
    let update = deduplicate_and_merge_raw_notifier_events(&[rename_event(
        RenameMode::From,
        &["/watched/out"],
    )])
    .unwrap();

    assert_eq!(update.deleted, [PathBuf::from("/watched/out")].into());
    assert!(update.added.is_empty());
    assert!(update.moved.is_empty());
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

#[test]
fn unrelated_from_and_to_events_are_not_paired_by_adjacency() {
    let update = deduplicate_and_merge_raw_notifier_events(&[
        rename_event(RenameMode::From, &["/watched/first-out"]),
        rename_event(RenameMode::From, &["/watched/second-out"]),
        rename_event(RenameMode::To, &["/watched/in"]),
    ])
    .unwrap();

    assert_eq!(
        update.deleted,
        [
            PathBuf::from("/watched/first-out"),
            PathBuf::from("/watched/second-out"),
        ]
        .into()
    );
    assert_eq!(update.added, [PathBuf::from("/watched/in")].into());
    assert!(update.moved.is_empty());
}
