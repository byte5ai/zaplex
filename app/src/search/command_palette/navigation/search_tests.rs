use super::full_text_searcher::byte_indices_to_char_indices;
use super::{SearchableSessionStringRanges, SessionHighlightIndices};

// ── End-to-end: highlight pipeline with multi-byte prompt ────────────

/// Simulates the same range construction that `searchable_session_string_and_ranges`
/// performs, then verifies that char-converted Tantivy byte indices produce
/// correct per-element highlights.
#[test]
fn highlight_indices_correct_after_byte_to_char_conversion() {
    // Prompt with multi-byte chars: "→⇒≠" = 3 chars, 9 bytes.
    let prompt = "→⇒≠";
    let command = "ls";
    let hint = "Running...";

    // Build the searchable string the same way the production code does.
    let mut searchable = prompt.to_string();
    let prompt_end = prompt.chars().count(); // 3

    searchable.push(' ');
    searchable.push_str(command);
    let cmd_start = prompt_end + 1; // 4
    let cmd_end = cmd_start + command.chars().count(); // 6
    let command_range = Some(cmd_start..cmd_end);

    searchable.push(' ');
    searchable.push_str(hint);
    let hint_start = cmd_end + 1; // 7
    let hint_end = hint_start + hint.chars().count(); // 17
    let hint_text_range = hint_start..hint_end;

    // Simulate Tantivy returning byte offsets for "ls" in the searchable
    // string.  "→⇒≠ ls Running..." — 'l' is at byte 10, 's' at byte 11.
    let byte_of_l = searchable.find('l').unwrap();
    let byte_of_s = byte_of_l + 1;
    assert_eq!(byte_of_l, 10, "precondition: 'l' should be at byte 10");

    // Without conversion these byte offsets (10, 11) would NOT fall in the
    // char-based command_range (4..6), so highlights would be lost.
    let char_indices = byte_indices_to_char_indices(&searchable, vec![byte_of_l, byte_of_s]);

    let ranges = SearchableSessionStringRanges {
        command_range,
        hint_text_range,
    };
    let highlights = SessionHighlightIndices::new(char_indices, ranges);

    // 'l' and 's' should map to command-relative indices 0 and 1.
    assert_eq!(highlights.command_indices, Some(vec![0, 1]));
    assert!(highlights.hint_text_indices.is_empty());
}

/// Same scenario but without the conversion — demonstrates the bug.
#[test]
fn raw_byte_indices_produce_wrong_highlights() {
    let prompt = "→⇒≠";
    let command = "ls";
    let hint = "Running...";

    let mut searchable = prompt.to_string();
    let prompt_end = prompt.chars().count(); // 3

    searchable.push(' ');
    searchable.push_str(command);
    let cmd_start = prompt_end + 1;
    let cmd_end = cmd_start + command.chars().count();
    let command_range = Some(cmd_start..cmd_end);

    searchable.push(' ');
    searchable.push_str(hint);
    let hint_start = cmd_end + 1;
    let hint_end = hint_start + hint.chars().count();
    let hint_text_range = hint_start..hint_end;

    // Feed raw byte offsets (10, 11) directly — the bug path.
    let byte_of_l = searchable.find('l').unwrap(); // 10
    let byte_of_s = byte_of_l + 1; // 11

    let ranges = SearchableSessionStringRanges {
        command_range,
        hint_text_range,
    };
    let highlights = SessionHighlightIndices::new(vec![byte_of_l, byte_of_s], ranges);

    // Byte 10 and 11 fall in the char-based hint_text_range (7..17), NOT the
    // command_range (4..6), so command highlights are lost and hint highlights
    // land on wrong characters.
    assert_eq!(highlights.command_indices, Some(vec![]));
    assert_eq!(highlights.hint_text_indices, vec![3, 4]); // wrong!
}
