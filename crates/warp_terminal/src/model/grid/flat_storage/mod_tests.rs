use crate::model::{
    char_or_str::CharOrStr,
    grid::cell::{Cell, Flags},
};

use super::*;

/// This test validates our handling of complex emoji sequences.
///
/// The three graphemes here are comprised of a number of Unicode characters.
/// Below are the individual characters that comprise the test string, with
/// "---" denoting how the string gets segmented into graphemes.
///
///  1. 🧑  1F9D1   ADULT
///  2.     1F3FF   EMOJI MODIFIER FITZPATRICK TYPE-6
///  3. ‍    200D    ZERO WIDTH JOINER
///  4. 🦰  1F9B0   EMOJI COMPONENT RED HAIR
///  ---
///  1. 👩  1F469   WOMAN
///  2. ‍    200D    ZERO WIDTH JOINER
///  3. 🦲  1F9B2   EMOJI COMPONENT BALD
///  ---
///  1. 🧔  1F9D4   BEARDED PERSON
///  2. 🏿   1F3FF   EMOJI MODIFIER FITZPATRICK TYPE-6
///  3. ‍    200D    ZERO WIDTH JOINER
///  4. ♂   2642    MALE SIGN
///  5. ️    FE0F    VARIATION SELECTOR-16
#[test]
#[ignore = "will not pass until using a version of unicode-width that includes commit afab363"]
fn test_row_with_complex_emoji() {
    let storage = FlatStorage::from_content_using_rows("🧑🏿‍🦰👩‍🦲🧔🏿‍♂️", 6, Some(1));

    let mut rows = storage.rows_from(0);
    let row1 = rows
        .next()
        .expect("should be able to get first row from storage");
    assert_eq!(row1.occ, 6);

    assert_eq!(row1[0].c, '🧑');
    assert!(matches!(
        row1[0].content_for_display(),
        CharOrStr::Str("🧑🏿‍🦰")
    ));

    assert!(row1[1].flags().contains(Flags::WIDE_CHAR_SPACER));
}

#[test]
fn test_push_rows_with_color() {
    let mut storage = FlatStorage::new(5, None, Some(2));

    let mut fg_cell = Cell::default();
    fg_cell.c = 'f';

    let mut red_cell = Cell::default();
    red_cell.c = 'r';
    red_cell.fg = ansi::Color::Named(ansi::NamedColor::Red);

    let row = Row::from_vec(
        vec![
            Cell::default(),
            Cell::default(),
            red_cell.clone(),
            red_cell,
            Cell::default(),
        ],
        5,
    );
    storage.push_rows([&row]);

    assert_eq!(storage.rows_from(0).next().unwrap().as_ref(), &row);
}
