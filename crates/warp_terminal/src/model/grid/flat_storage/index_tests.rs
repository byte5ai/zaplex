use crate::model::grid::FlatStorage;

use super::*;

#[test]
fn test_cell_type() {
    // 1: 😀😃
    // 2: 😄\n
    // 3: a😄\n
    let storage = FlatStorage::from_content_using_rows("😀😃😄\na😄\n", 5, Some(2));
    assert_eq!(storage.index.rows.len(), 3);

    assert_eq!(storage.cell_type(0, 0), Some(CellType::WideChar));
    assert_eq!(storage.cell_type(0, 1), Some(CellType::WideCharSpacer));

    assert_eq!(
        storage.cell_type(0, 4),
        Some(CellType::LeadingWideCharSpacer)
    );

    // Empty cells at the end of a hard-wrapped line are narrow.
    // We test both the first empty cell (to check off-by-one errors) and
    // a later cell (for completeness).
    assert_eq!(storage.cell_type(1, 2), Some(CellType::RegularChar));
    assert_eq!(storage.cell_type(1, 4), Some(CellType::RegularChar));

    // Make sure we properly handle rows with non-uniform grapheme sizing.
    assert_eq!(storage.cell_type(2, 0), Some(CellType::RegularChar));
    assert_eq!(storage.cell_type(2, 1), Some(CellType::WideChar));
    assert_eq!(storage.cell_type(2, 2), Some(CellType::WideCharSpacer));
}

mod offset_point_conversion {
    use super::*;

    #[test]
    #[ignore = "does not work properly; will re-enable once content offset/point conversion uses a custom type"]
    fn test_wide_char_spacer() {
        // 1: 😀😃
        // 2: 😄\n
        // 3: a😄\n
        let storage = FlatStorage::from_content_using_rows("😀😃😄\na😄\n", 5, Some(2));

        let original_point = Point::new(0, 3);

        let offset = storage
            .content_offset_at_point(original_point)
            .expect("should be able to convert point to offset");
        assert_eq!(offset, ByteOffset::from(4));

        let point = storage
            .content_offset_to_point(offset)
            .expect("should be able to convert offset back to point");
        assert_eq!(point, original_point);
    }
}
