use super::*;
use crate::fonts::{collect_glyph_indices, init_fonts, Properties};
use crate::platform::FontDB as _;
use crate::{
    elements::DEFAULT_UI_LINE_HEIGHT_RATIO,
    text_layout::{TextStyle, DEFAULT_TOP_BOTTOM_RATIO},
};
use anyhow::Result;

const FONT_SIZE: f32 = 16.;
const FRAME_WIDTH: f32 = 80.;
const FRAME_HEIGHT: f32 = f32::MAX;

#[test]
fn test_layout_text_first_line_indent_large() -> Result<()> {
    let (font_db, roboto) = init_fonts();

    let text = "Let's lay out s𐍈me Roboto text.";
    //          0123456789012345678901234567890
    let line_style = LineStyle {
        font_size: FONT_SIZE,
        line_height_ratio: DEFAULT_UI_LINE_HEIGHT_RATIO,
        baseline_ratio: DEFAULT_TOP_BOTTOM_RATIO,
        fixed_width_tab_size: None,
    };
    let style_runs = [(
        0..text.encode_utf16().count(),
        StyleAndFont::new(roboto, Properties::default(), TextStyle::new()),
    )];

    // First, lay out the text with no head indent.
    let no_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(0.),
    );

    // The text should contain multiple lines.
    // The first line has about the same amount of content as the others,
    // since there's no head indent.
    assert_eq!(no_indent_frame.lines().len(), 4);
    assert_eq!(
        collect_glyph_indices(&no_indent_frame),
        vec![
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
            vec![10, 11, 12, 13, 14, 15, 16, 17],
            vec![19, 20, 21, 22, 23, 24],
            vec![26, 27, 28, 29, 30],
        ]
    );
    assert!(first_line_bounded(&no_indent_frame, 0., FRAME_WIDTH));
    assert!(all_lines_bounded(&no_indent_frame, FRAME_WIDTH));

    // Lay out the text with a head indent that's 5px bigger than the width of the frame.
    let overflow_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(FRAME_WIDTH + 5.),
    );

    // The first line is left entirely blank since no glyphs fit on it.
    assert_eq!(
        collect_glyph_indices(&overflow_indent_frame),
        vec![
            vec![], // No glyphs fit on this line.
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
            vec![10, 11, 12, 13, 14, 15, 16, 17],
            vec![19, 20, 21, 22, 23, 24],
            vec![26, 27, 28, 29, 30],
        ]
    );
    assert!(first_line_bounded(
        &overflow_indent_frame,
        FRAME_WIDTH + 5.,
        FRAME_WIDTH,
    ));
    assert!(all_lines_bounded(&overflow_indent_frame, FRAME_WIDTH));

    // Lay out the text with a 79px head indent,
    // which spans almost the entire width of the frame.
    let big_indent_frame = font_db.text_layout_system().layout_text(
        text,
        line_style,
        &style_runs,
        FRAME_WIDTH,
        FRAME_HEIGHT,
        Default::default(),
        Some(FRAME_WIDTH - 0.1),
    );

    // The first line is left entirely blank since no glyphs fit on it.
    assert_eq!(big_indent_frame.lines().len(), 5);
    assert_eq!(
        collect_glyph_indices(&big_indent_frame),
        vec![
            vec![], // No glyphs fit on this line.
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8],
            vec![10, 11, 12, 13, 14, 15, 16, 17],
            vec![19, 20, 21, 22, 23, 24],
            vec![26, 27, 28, 29, 30],
        ]
    );
    assert!(first_line_bounded(
        &big_indent_frame,
        FRAME_WIDTH - 0.1,
        FRAME_WIDTH,
    ));
    assert!(all_lines_bounded(&big_indent_frame, FRAME_WIDTH));

    Ok(())
}

/// Checks that the head indent and first line's width don't exceed the frame's width.
fn first_line_bounded(frame: &TextFrame, first_line_indent: f32, frame_width: f32) -> bool {
    let first_line_width = frame.lines().first().unwrap().width;
    first_line_width + first_line_indent.min(frame_width) <= frame_width
}

fn all_lines_bounded(frame: &TextFrame, frame_width: f32) -> bool {
    frame.lines().iter().fold(true, |all_bounded, line| {
        let current_bounded = line.width <= frame_width;
        all_bounded && current_bounded
    })
}
