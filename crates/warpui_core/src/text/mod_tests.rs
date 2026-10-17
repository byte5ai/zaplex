use super::{
    count_chars_up_to_byte,
    point::Point,
    {BufferIndex, TextBuffer},
};

/// Test the [`str`] implementation of [`TextBuffer`], which we rely on in other unit tests.
#[test]
fn test_str_buffer() -> anyhow::Result<()> {
    let buf = "Hello\nWorld!";

    assert_eq!(buf.chars_at(2.into())?.collect::<String>(), "llo\nWorld!");
    assert_eq!(buf.chars_rev_at(3.into())?.collect::<String>(), "leH");

    // For simplicity, we do not wrap newlines into new rows.
    assert_eq!(buf.to_point(7.into())?, Point::new(0, 7));
    assert!(Point::new(1, 1).to_char_offset(buf).is_err());
    assert_eq!(Point::new(0, 7).to_char_offset(buf)?, 7.into());

    Ok(())
}

#[test]
fn test_char_counts_up_to_byte() {
    let text = "abc🔥abc☄️abc😬";
    assert_eq!(count_chars_up_to_byte(text, 0.into()), Some(0.into()));
    assert_eq!(
        count_chars_up_to_byte(text, "abc🔥".len().into()),
        Some(4.into())
    );
    assert_eq!(
        count_chars_up_to_byte(text, text.len().into()),
        Some(text.chars().count().into())
    );
}
