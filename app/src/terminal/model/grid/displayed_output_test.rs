use super::*;
use bimap::BiMap;
use itertools::Itertools;

/// Converts a vector of ranges into DisplayedRows objects with source FilterMatch.
fn make_displayed_rows_from_ranges(ranges: Vec<RangeInclusive<usize>>) -> Vec<DisplayedRows> {
    ranges
        .into_iter()
        .map(|range| DisplayedRows {
            range,
            source: DisplaySource::FilterMatch,
        })
        .collect_vec()
}

#[test]
pub fn test_truncate_to_row() {
    let mut displayed_output =
        DisplayedOutput::new_for_test(vec![6..=11, 15..=17, 20..=25, 27..=30]);

    displayed_output.truncate_to_row(19);
    assert_eq!(
        displayed_output.displayed_rows,
        make_displayed_rows_from_ranges(vec![6..=11, 15..=17])
    );
    assert_eq!(displayed_output.height, 9);
}

#[test]
pub fn test_truncate_to_row_truncate_before_start() {
    let mut displayed_output = DisplayedOutput::new_for_test(vec![6..=11, 12..=14]);

    displayed_output.truncate_to_row(5);
    assert_eq!(displayed_output.displayed_rows, vec![]);
    assert_eq!(displayed_output.height, 0);
}

#[test]
pub fn test_truncate_to_row_truncate_after_end() {
    let mut displayed_output = DisplayedOutput::new_for_test(vec![6..=11, 12..=14]);

    displayed_output.truncate_to_row(15);
    assert_eq!(
        displayed_output.displayed_rows,
        make_displayed_rows_from_ranges(vec![6..=11, 12..=14])
    );
    assert_eq!(displayed_output.height, 9);
}

#[test]
pub fn test_truncate_to_row_in_range() {
    let mut displayed_output =
        DisplayedOutput::new_for_test(vec![6..=11, 15..=17, 20..=25, 27..=30]);

    displayed_output.truncate_to_row(16);
    assert_eq!(
        displayed_output.displayed_rows,
        make_displayed_rows_from_ranges(vec![6..=11, 15..=16])
    );
    assert_eq!(displayed_output.height, 8);

    let mut displayed_output =
        DisplayedOutput::new_for_test(vec![6..=11, 15..=17, 20..=25, 27..=30]);

    displayed_output.truncate_to_row(15);
    assert_eq!(
        displayed_output.displayed_rows,
        make_displayed_rows_from_ranges(vec![6..=11, 15..=15])
    );
    assert_eq!(displayed_output.height, 7);

    let mut displayed_output =
        DisplayedOutput::new_for_test(vec![6..=11, 15..=17, 20..=25, 27..=30]);

    displayed_output.truncate_to_row(17);
    assert_eq!(
        displayed_output.displayed_rows,
        make_displayed_rows_from_ranges(vec![6..=11, 15..=17])
    );
    assert_eq!(displayed_output.height, 9);
}

#[test]
pub fn test_truncate_to_row_in_last_range() {
    let mut displayed_output =
        DisplayedOutput::new_for_test(vec![6..=11, 15..=17, 20..=25, 27..=30]);

    displayed_output.truncate_to_row(28);
    assert_eq!(
        displayed_output.displayed_rows,
        make_displayed_rows_from_ranges(vec![6..=11, 15..=17, 20..=25, 27..=28])
    );
    assert_eq!(displayed_output.height, 17);
}

fn create_bimap_with_insertions(insertion_list: &[(usize, usize)]) -> BiMap<usize, usize> {
    let mut bimap = BiMap::new();
    for (row, offset_row) in insertion_list {
        bimap.insert(*row, *offset_row);
    }
    bimap
}

#[test]
fn test_get_exact_or_next_displayed_row() {
    // Start with displayed_rows [2..=3, 6..=8, 14..=15].
    let bimap =
        create_bimap_with_insertions(&[(2, 0), (3, 1), (6, 2), (7, 3), (8, 4), (14, 5), (15, 6)]);
    let displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![2..=3, 6..=8, 14..=15]),
        height: 0,
        row_translation_map: bimap.into(),
    };

    let expected_input_output_pairs = vec![
        (0, Some(0)),
        (1, Some(0)),
        (2, Some(0)),
        (3, Some(1)),
        (4, Some(2)),
        (5, Some(2)),
        (6, Some(2)),
        (7, Some(3)),
        (8, Some(4)),
        (9, Some(5)),
        (10, Some(5)),
        (11, Some(5)),
        (12, Some(5)),
        (13, Some(5)),
        (14, Some(5)),
        (15, Some(6)),
    ];
    for (input, expected_output) in expected_input_output_pairs {
        assert_eq!(
            displayed_output.get_exact_or_next_displayed_row(input),
            expected_output
        );
    }
}

#[test]
fn test_get_exact_or_next_displayed_row_no_next_closest() {
    // Start with displayed_rows [2..=3, 6..=8, 14..=15].
    let bimap =
        create_bimap_with_insertions(&[(2, 0), (3, 1), (6, 2), (7, 3), (8, 4), (14, 5), (15, 6)]);
    let displayed_output = DisplayedOutput {
        displayed_rows: make_displayed_rows_from_ranges(vec![2..=3, 6..=8, 14..=15]),
        height: 0,
        row_translation_map: bimap.into(),
    };

    assert_eq!(displayed_output.get_exact_or_next_displayed_row(16), None);
}

#[test]
fn test_get_exact_or_next_displayed_row_empty() {
    let displayed_output = DisplayedOutput {
        displayed_rows: Vec::new(),
        height: 0,
        row_translation_map: BiMap::new().into(),
    };

    assert!(displayed_output
        .get_exact_or_next_displayed_row(5)
        .is_none());
}
