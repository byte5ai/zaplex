//! Pure keyboard-navigation logic for the file-manager pane (MC-style cursor).
//!
//! The file manager is keyboard-driven like Midnight Commander: a single
//! highlighted *cursor* row moves with the arrow keys, distinct from the
//! multi-selection set. This module holds the cursor arithmetic as pure,
//! unit-tested functions so the view layer stays thin.

/// A cursor movement over a list of visible rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorMove {
    Up,
    Down,
    First,
    Last,
    /// Move up by a page; the argument is the page size (rows per screen).
    PageUp(usize),
    /// Move down by a page; the argument is the page size (rows per screen).
    PageDown(usize),
}

/// New cursor position after `mv`, clamped to `[0, len)`.
///
/// Movement does not wrap — MC behaviour: Up at the top stays at the top,
/// Down at the bottom stays at the bottom. An empty list pins the cursor at 0.
pub fn apply_cursor_move(current: usize, len: usize, mv: CursorMove) -> usize {
    if len == 0 {
        return 0;
    }
    let last = len - 1;
    let cur = current.min(last);
    match mv {
        CursorMove::Up => cur.saturating_sub(1),
        CursorMove::Down => (cur + 1).min(last),
        CursorMove::First => 0,
        CursorMove::Last => last,
        // A page of 0 would be a no-op footgun; treat it as at least 1.
        CursorMove::PageUp(page) => cur.saturating_sub(page.max(1)),
        CursorMove::PageDown(page) => (cur + page.max(1)).min(last),
    }
}

/// Clamp a cursor to a (possibly shrunk) list — used after refresh/filter/navigation.
pub fn clamp_cursor(current: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        current.min(len - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn down_and_up_do_not_wrap() {
        assert_eq!(apply_cursor_move(0, 3, CursorMove::Down), 1);
        assert_eq!(apply_cursor_move(1, 3, CursorMove::Down), 2);
        // At the bottom, Down stays.
        assert_eq!(apply_cursor_move(2, 3, CursorMove::Down), 2);
        assert_eq!(apply_cursor_move(2, 3, CursorMove::Up), 1);
        // At the top, Up stays.
        assert_eq!(apply_cursor_move(0, 3, CursorMove::Up), 0);
    }

    #[test]
    fn first_and_last_jump_to_the_ends() {
        assert_eq!(apply_cursor_move(1, 5, CursorMove::First), 0);
        assert_eq!(apply_cursor_move(1, 5, CursorMove::Last), 4);
    }

    #[test]
    fn paging_is_clamped_and_never_zero_step() {
        assert_eq!(apply_cursor_move(10, 100, CursorMove::PageUp(20)), 0);
        assert_eq!(apply_cursor_move(90, 100, CursorMove::PageDown(20)), 99);
        assert_eq!(apply_cursor_move(5, 100, CursorMove::PageDown(20)), 25);
        // Degenerate page size still advances by one.
        assert_eq!(apply_cursor_move(5, 100, CursorMove::PageDown(0)), 6);
        assert_eq!(apply_cursor_move(5, 100, CursorMove::PageUp(0)), 4);
    }

    #[test]
    fn empty_list_pins_cursor_at_zero() {
        assert_eq!(apply_cursor_move(3, 0, CursorMove::Down), 0);
        assert_eq!(apply_cursor_move(3, 0, CursorMove::Up), 0);
        assert_eq!(apply_cursor_move(0, 0, CursorMove::Last), 0);
    }

    #[test]
    fn stale_cursor_beyond_len_is_treated_as_last_before_moving() {
        // Cursor was 9 but the list shrank to 3 rows; Up should land on 1
        // (from the clamped position 2), not underflow.
        assert_eq!(apply_cursor_move(9, 3, CursorMove::Up), 1);
        assert_eq!(apply_cursor_move(9, 3, CursorMove::Down), 2);
    }

    #[test]
    fn clamp_cursor_shrinks_into_range() {
        assert_eq!(clamp_cursor(9, 3), 2);
        assert_eq!(clamp_cursor(1, 3), 1);
        assert_eq!(clamp_cursor(5, 0), 0);
    }
}
