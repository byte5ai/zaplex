use std::slice;

use super::has_malformed_terminal_correction_signal;
use ai::diff_validation::{DiffDelta, DiffType};

#[test]
fn integration_metric_signal_true_for_shifted_multi_hunk_diff() {
    let diff = DiffType::update(
        vec![
            DiffDelta {
                replacement_line_range: 2..3,
                insertion: "alpha\nbeta\ngamma\n".to_string(),
            },
            DiffDelta {
                replacement_line_range: 5..6,
                insertion: "let value = \"unterminated".to_string(),
            },
        ],
        None,
    );
    assert!(has_malformed_terminal_correction_signal(
        &diff,
        slice::from_ref(&(6..7)),
    ));
}
