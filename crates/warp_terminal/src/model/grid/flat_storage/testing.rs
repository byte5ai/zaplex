use unicode_segmentation::UnicodeSegmentation as _;

use crate::model::grid::{cell::Flags, row::Row};

use super::{grapheme::Grapheme, FlatStorage};

/// Extra functions on [`FlatStorage`] that are useful for testing purposes.
impl FlatStorage {
    pub fn push_rows_from_string(&mut self, string: &str) {
        let rows = string.to_rows(self.columns);
        self.push_rows(&rows);
    }

    pub fn from_content_using_rows(
        content: &str,
        columns: usize,
        initial_capacity: Option<usize>,
    ) -> Self {
        let mut storage = Self::new(columns, None, initial_capacity);
        storage.push_rows_from_string(content);
        storage
    }
}

/// Helper trait for converting something to a set of [`Row`]s.
///
/// Ultimately, we want to be testing equivalence of behavior between grid and
/// flat storage at a higher level, but this is still useful for quick-to-run
/// unit tests.
///
/// TODO(vorporeal): Eliminate this and use existing string-to-row machinery
/// (e.g.: session restoration logic) once that logic is in this crate.
pub trait ToRows {
    fn to_rows(&self, columns: usize) -> Vec<Row>;
}

impl ToRows for &str {
    #[allow(unused_assignments)]
    fn to_rows(&self, columns: usize) -> Vec<Row> {
        let mut rows = vec![Row::new(columns)];

        let mut needs_new_row = false;

        for grapheme in self.graphemes(true).map(Grapheme::new_from_str) {
            let mut cell_idx = rows.last().unwrap().occ;

            macro_rules! new_row {
                () => {
                    rows.push(Row::new(columns));
                    cell_idx = 0;
                };
            }

            if needs_new_row {
                needs_new_row = false;
                new_row!();
            }

            if grapheme.starts_new_row() {
                // Don't immediately start a new row - if this is the last
                // grapheme, we shouldn't append an extra empty row.
                needs_new_row = true;
                continue;
            }

            let cell_width = grapheme.cell_width();
            if cell_width == 0 {
                continue;
            }

            if cell_idx + cell_width as usize > columns {
                // If the row is full and we have another character to add to
                // it, set the WRAPLINE flag on the final cell and then start
                // a new row.
                let mut flags = Flags::WRAPLINE;
                if cell_width > 1 && cell_idx < columns {
                    cell_idx += cell_width as usize - 1;
                    // If this is a wide character and there isn't enough space
                    // for it, also add the appropriate flag.
                    flags |= Flags::LEADING_WIDE_CHAR_SPACER;
                }

                rows.last_mut().unwrap()[columns - 1].flags.insert(flags);
                new_row!();
            }

            let row = rows.last_mut().unwrap();
            let cell = &mut row[cell_idx];

            let mut chars = grapheme.chars();
            cell.c = chars.next().unwrap();
            // Add any remaining chars in the grapheme to the cell as zero-width
            // characters.
            chars.for_each(|c| cell.push_zerowidth(c, /* log_long_grapheme_warnings */ true));

            // If the grapheme takes up two cells, mark the following cell as
            // a spacer.
            if cell_width == 2 {
                row[cell_idx].flags.insert(Flags::WIDE_CHAR);
                row[cell_idx + 1].flags.insert(Flags::WIDE_CHAR_SPACER);
            }
        }

        // If the last row didn't end in a newline character, assert that it
        // was a full row, and add the WRAPLINE (soft wrap) flag.
        let last_row = rows.last_mut().unwrap();
        let occupied_cells = last_row.occ;
        if !needs_new_row {
            assert!(
                occupied_cells == last_row.len(),
                "All non-filled rows must explicitly end in a newline to avoid surprises and incorrect tests."
            );
            last_row
                .last_mut()
                .unwrap()
                .flags_mut()
                .insert(Flags::WRAPLINE);
        }

        rows
    }
}
