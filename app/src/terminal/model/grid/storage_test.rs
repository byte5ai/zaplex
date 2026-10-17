use crate::terminal::model::cell::Cell;
use crate::terminal::model::grid::row::Row;
use crate::terminal::model::grid::storage::Storage;

// Use a large value for `MAX_CACHE_SIZE` for testing.
const MAX_CACHE_SIZE: usize = 1_000;

#[test]
fn initialize() {
    // Setup storage area.
    let mut storage = Storage {
        inner: vec![
            filled_row('4'),
            filled_row('5'),
            filled_row('0'),
            filled_row('1'),
            filled_row('2'),
            filled_row('3'),
        ],
        bottom_row: 2,
        visible_lines: 0,
        len: 6,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    // Initialize additional lines.
    let init_size = 3;
    storage.initialize(init_size, 1);

    // Generate expected grid.
    let mut expected_inner = vec![
        filled_row('0'),
        filled_row('1'),
        filled_row('2'),
        filled_row('3'),
        filled_row('4'),
        filled_row('5'),
    ];
    let expected_init_size = std::cmp::max(init_size, MAX_CACHE_SIZE);
    expected_inner.append(&mut vec![filled_row('\0'); expected_init_size]);
    let expected_storage = Storage {
        inner: expected_inner,
        bottom_row: 0,
        visible_lines: 0,
        len: 9,
        is_sequential: false,
        max_cache_size: MAX_CACHE_SIZE,
    };

    assert_eq!(storage.len, expected_storage.len);
    assert_eq!(storage.bottom_row, expected_storage.bottom_row);
    assert_eq!(storage.inner, expected_storage.inner);
}

fn filled_row(content: char) -> Row {
    let mut row = Row::new(1);
    let mut cell = Cell::default();
    cell.c = content;
    row[0] = cell;
    row
}

fn partially_filled_row(content: char, columns_to_fill: usize, total_columns: usize) -> Row {
    let mut row = Row::new(total_columns);
    for col in 0..total_columns {
        let mut cell = Cell::default();
        if col <= columns_to_fill {
            cell.c = content;
        }
        row[col] = cell;
    }
    row
}
