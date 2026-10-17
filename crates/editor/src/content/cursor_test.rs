use sum_tree::SumTree;

use crate::content::text::{BufferBlockStyle, BufferText, BufferTextStyle, MarkerDir};
use string_offset::CharOffset;

use super::{BufferCursor, BufferSumTree};

#[test]
fn test_styled_text_before_markers() {
    let mut tree: SumTree<BufferText> = SumTree::new();
    tree.push(BufferText::BlockMarker {
        marker_type: BufferBlockStyle::PlainText,
    });
    tree.append_str("Plain text");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::bold(),
        dir: MarkerDir::Start,
    });
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::Italic,
        dir: MarkerDir::Start,
    });
    tree.append_str("BI");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::bold(),
        dir: MarkerDir::End,
    });
    tree.append_str("Just Italic");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::Italic,
        dir: MarkerDir::End,
    });
    tree.append_str("Plain text");
    assert_eq!(
        tree.debug(),
        "<text>Plain text<b_s><i_s>BI<b_e>Just Italic<i_e>Plain text"
    );

    let cursor = tree.cursor::<CharOffset, CharOffset>();
    let mut text_cursor = BufferCursor::new(cursor);
    text_cursor.seek_to_offset_before_markers(CharOffset::from(11));
    let new_content = text_cursor.slice_to_offset_after_markers(CharOffset::from(13));
    assert_eq!(new_content.debug(), "<b_s><i_s>BI<b_e>");

    let new_content = text_cursor.slice_to_offset_after_markers(CharOffset::from(17));
    assert_eq!(new_content.debug(), "Just");

    let new_content = text_cursor.slice_to_offset_before_markers(CharOffset::from(24));
    assert_eq!(new_content.debug(), " Italic");

    let new_content = text_cursor.suffix();
    assert_eq!(new_content.debug(), "<i_e>Plain text");
}

#[test]
fn test_char_at() {
    let mut tree: SumTree<BufferText> = SumTree::new();
    tree.append_str("Line");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::bold(),
        dir: MarkerDir::Start,
    });
    tree.append_str("String");
    tree.push(BufferText::Marker {
        marker_type: BufferTextStyle::bold(),
        dir: MarkerDir::End,
    });
    tree.push(BufferText::Newline);
    tree.append_str("Next");
    assert_eq!(tree.debug(), "Line<b_s>String<b_e>\\nNext");

    let cursor = tree.cursor::<CharOffset, CharOffset>();
    let mut text_cursor = BufferCursor::new(cursor);
    assert_eq!(text_cursor.char_at(CharOffset::from(1)), Some('i'));
    assert_eq!(text_cursor.char_at(CharOffset::from(3)), Some('e'));
    assert_eq!(text_cursor.char_at(CharOffset::from(4)), Some('S'));
    assert_eq!(text_cursor.char_at(CharOffset::from(10)), Some('\n'));
}
