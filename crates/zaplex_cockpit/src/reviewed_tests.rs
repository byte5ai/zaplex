use super::*;

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).expect("valid timestamp")
}

#[test]
fn a_mark_toggles_on_and_off() {
    let mut r = ReviewedSessions::default();
    assert!(!r.contains("s1"));
    assert!(r.toggle("s1", at(100)));
    assert!(r.contains("s1"));
    assert!(!r.toggle("s1", at(200)), "toggling again clears it");
    assert!(!r.contains("s1"));
}

#[test]
fn marks_are_independent() {
    let mut r = ReviewedSessions::default();
    r.toggle("s1", at(100));
    r.toggle("s2", at(200));
    r.toggle("s1", at(300));
    assert!(!r.contains("s1"));
    assert!(r.contains("s2"), "clearing one leaves the other alone");
}

/// The whole point of F7: a mark has to survive the app being closed. The store
/// is what gets written to disk, so it must round-trip exactly.
#[test]
fn marks_survive_a_round_trip_through_json() {
    let mut r = ReviewedSessions::default();
    r.toggle("a9b3a0e6-9067-41a0-b9fd-dcbee7ad5c01", at(100));
    r.toggle("019f135f-7fcc-7d93-8a28-4835d98f8f0a", at(200));

    let json = serde_json::to_string(&r).unwrap();
    let back: ReviewedSessions = serde_json::from_str(&json).unwrap();
    assert_eq!(back, r);
    assert!(back.contains("a9b3a0e6-9067-41a0-b9fd-dcbee7ad5c01"));
}

/// A file written before this field existed, or hand-emptied, must load rather
/// than throw away the user's marks wholesale.
#[test]
fn an_empty_or_older_file_loads_as_no_marks() {
    let empty: ReviewedSessions = serde_json::from_str("{}").unwrap();
    assert!(empty.is_empty());
}

#[test]
fn the_cap_keeps_the_most_recent_marks() {
    let mut r = ReviewedSessions::default();
    for i in 0..10 {
        r.toggle(&format!("s{i}"), at(i as i64 * 100));
    }
    r.prune(3);

    assert_eq!(r.len(), 3);
    assert!(r.contains("s9") && r.contains("s8") && r.contains("s7"));
    assert!(!r.contains("s0"), "the oldest marks go first");
}

/// Marking is the one place that grows the set, so it is the one place that has
/// to keep it bounded — otherwise the file only stops growing when someone
/// remembers to prune it.
#[test]
fn marking_never_grows_the_set_past_the_cap() {
    let mut r = ReviewedSessions::default();
    for i in 0..(REVIEWED_LIMIT + 50) {
        r.toggle(&format!("s{i}"), at(i as i64));
    }
    assert_eq!(r.len(), REVIEWED_LIMIT);
    // The newest survived, the very first did not.
    assert!(r.contains(&format!("s{}", REVIEWED_LIMIT + 49)));
    assert!(!r.contains("s0"));
}

#[test]
fn pruning_below_the_count_changes_nothing() {
    let mut r = ReviewedSessions::default();
    r.toggle("s1", at(100));
    r.prune(10);
    assert!(r.contains("s1"));
}
