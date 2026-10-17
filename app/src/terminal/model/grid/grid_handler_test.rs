use warp_terminal::model::char_or_str::CharOrStr;
use warp_util::path::LineAndColumnArg;
use warpui::text::words::is_default_word_boundary;

use crate::terminal::model::secrets::{ObfuscateSecrets, SecretLevel};
use crate::terminal::model::{blockgrid::BlockGrid, secrets::IsObfuscated};
use crate::test_util::mock_blockgrid;

use super::*;

const MAX_SCROLL_LIMIT: usize = 1000;

fn has_wide_char_character(blockgrid: &BlockGrid) -> bool {
    let mut point = Point { row: 0, col: 0 };

    while let Some(cell_type) = blockgrid.grid_handler().cell_type(point) {
        if matches!(cell_type, CellType::WideChar) {
            return true;
        }
        point = point.wrapping_add(blockgrid.grid_handler().columns(), 1);
    }

    false
}

#[test]
fn regex_left_on_line_wrap() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("aat\nest\r\n111");

    // Check regex across wrapped and unwrapped lines.
    let dfas = RegexDFAs::new("t.*st").unwrap();
    let start = Point::new(1, 2);
    let end = Point::new(0, 0);
    let match_start = Point::new(0, 2);
    let match_end = Point::new(1, 2);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        Some(match_start..=match_end)
    );
}

#[test]
fn regex_right() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        testing66\r\n\
        Warp\n\
        123\r\n\
        Warp\r\n\
        123\
    ");

    // Check regex across wrapped and unwrapped lines.
    let dfas = RegexDFAs::new("Wa.*123").unwrap();
    let start = Point::new(1, 0);
    let end = Point::new(4, 2);
    let match_start = Point::new(1, 0);
    let match_end = Point::new(2, 2);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(match_start..=match_end)
    );
}

#[test]
fn regex_left() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        testing66\r\n\
        Warp\n\
        123\r\n\
        Warp\r\n\
        123\
    ");

    // Check regex across wrapped and unwrapped lines.
    let dfas = RegexDFAs::new("Wa.*123").unwrap();
    let start = Point::new(4, 2);
    let end = Point::new(1, 0);
    let match_start = Point::new(1, 0);
    let match_end = Point::new(2, 2);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        Some(match_start..=match_end)
    );
}

#[test]
fn nested_regex() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        Wa -> Warp -> rp\r\n\
        rp\
    ");

    // Greedy stopped at linebreak.
    let dfas = RegexDFAs::new("Wa.*rp").unwrap();
    let start = Point::new(0, 0);
    let end = Point::new(0, 15);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(start..=end)
    );

    // Greedy stopped at dead state.
    let dfas = RegexDFAs::new("Wa[^y]*rp").unwrap();
    let start = Point::new(0, 0);
    let end = Point::new(0, 9);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(start..=end)
    );
}

#[test]
fn regex_word_boundary() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        echo foo-bar foobar foo
    ");

    // Greedy stopped at linebreak.
    let dfas = RegexDFAs::new("\\bfoo\\b").unwrap();
    let end = Point::new(0, 22);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, Point::new(0, 0), end),
        Some(Point::new(0, 5)..=Point::new(0, 7))
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, Point::new(0, 8), end),
        Some(Point::new(0, 20)..=Point::new(0, 22))
    );
}

#[test]
fn no_match_right() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        first line\n\
        broken second\r\n\
        third\
    ");

    let dfas = RegexDFAs::new("nothing").unwrap();
    let start = Point::new(2, 0);
    let end = Point::new(0, 4);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        None
    );
}

#[test]
fn no_match_left() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        first line\n\
        broken second\r\n\
        third\
    ");

    let dfas = RegexDFAs::new("nothing").unwrap();
    let start = Point::new(0, 4);
    let end = Point::new(2, 0);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        None
    );
}

#[test]
fn include_linebreak_left() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        testing123\r\n\
        xxx\
    ");

    // Make sure the cell containing the linebreak is not skipped.
    let dfas = RegexDFAs::new("te.*123").unwrap();
    let start = Point::new(1, 0);
    let end = Point::new(0, 0);
    let match_start = Point::new(0, 0);
    let match_end = Point::new(0, 9);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        Some(match_start..=match_end)
    );
}

#[test]
fn include_linebreak_right() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        xxx\r\n\
        testing123\
    ");

    // Make sure the cell containing the linebreak is not skipped.
    let dfas = RegexDFAs::new("te.*123").unwrap();
    let start = Point::new(0, 2);
    let end = Point::new(1, 9);
    let match_start = Point::new(1, 0);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(match_start..=end)
    );
}

#[test]
fn skip_dead_cell() {
    let blockgrid = mock_blockgrid("hellooo");

    // Make sure dead state cell is skipped when reversing.
    let dfas = RegexDFAs::new("hello").unwrap();
    let start = Point::new(0, 0);
    let end = Point::new(0, 4);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(start..=end)
    );
}

#[test]
fn reverse_search_dead_recovery() {
    let blockgrid = mock_blockgrid("zooo lense");

    // Make sure the reverse DFA operates the same as a forward DFA.
    let dfas = RegexDFAs::new("zoo").unwrap();
    let start = Point::new(0, 9);
    let end = Point::new(0, 0);
    let match_start = Point::new(0, 0);
    let match_end = Point::new(0, 2);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        Some(match_start..=match_end)
    );
}

#[test]
fn multibyte_unicode() {
    let blockgrid = mock_blockgrid("testвосибing");

    let dfas = RegexDFAs::new("te.*ing").unwrap();
    let start = Point::new(0, 0);
    let end = Point::new(0, 11);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(start..=end)
    );

    let dfas = RegexDFAs::new("te.*ing").unwrap();
    let start = Point::new(0, 11);
    let end = Point::new(0, 0);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        Some(end..=start)
    );
}

#[test]
fn double_width_chars() {
    let blockgrid = mock_blockgrid("大家好😀");

    let mut dfas = RegexDFAs::new("大家好").unwrap();
    let start = Point::new(0, 0);
    let end = Point::new(0, 7);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(Point::new(0, 0)..=Point::new(0, 5))
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, end, start),
        Some(Point::new(0, 0)..=Point::new(0, 5))
    );

    dfas = RegexDFAs::new("😀").unwrap();
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(Point::new(0, 6)..=Point::new(0, 7))
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, end, start),
        Some(Point::new(0, 6)..=Point::new(0, 7))
    );

    dfas = RegexDFAs::new("大").unwrap();
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(Point::new(0, 0)..=Point::new(0, 1))
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, end, start),
        Some(Point::new(0, 0)..=Point::new(0, 1))
    );
}

#[test]
fn wrapping() {
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        xxx\r\n\
        xxx\
    ");

    let dfas = RegexDFAs::new("xxx").unwrap();
    let start = Point::new(0, 2);
    let end = Point::new(1, 2);
    let match_start = Point::new(1, 0);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_rightwards(&dfas, start, end),
        Some(match_start..=end)
    );

    let dfas = RegexDFAs::new("xxx").unwrap();
    let start = Point::new(1, 0);
    let end = Point::new(0, 0);
    let match_end = Point::new(0, 2);
    assert_eq!(
        blockgrid
            .grid_handler
            .regex_search_leftwards(&dfas, start, end),
        Some(end..=match_end)
    );
}

#[test]
fn test_semantic_search() {
    let blockgrid =
        mock_blockgrid("/usr/local/bin\r\nword_with_underscores\r\nвосибing\r\nsome«quotes»");

    // Left search
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_left(Point::new(0, 7), is_default_word_boundary),
        Point::new(0, 5)
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_left(Point::new(1, 7), is_default_word_boundary),
        Point::new(1, 0)
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_left(Point::new(2, 4), is_default_word_boundary),
        Point::new(2, 0)
    );

    // Right search
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_right(Point::new(0, 7), is_default_word_boundary),
        Point::new(0, 9)
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_left(Point::new(1, 20), is_default_word_boundary),
        Point::new(1, 0)
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_left(Point::new(2, 7), is_default_word_boundary),
        Point::new(2, 0)
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .semantic_search_left(Point::new(3, 9), is_default_word_boundary),
        Point::new(3, 5)
    );
}

#[test]
fn test_secrets_serialization() {
    let mut blockgrid = mock_blockgrid("foo zach@warp.dev bar");
    blockgrid.maybe_enable_secret_obfuscation(ObfuscateSecrets::Yes);
    blockgrid.grid_handler_mut().mark_secret_range(
        Point::new(0, 4)..=Point::new(0, 16),
        IsObfuscated::Yes,
        "zach@warp.dev".to_string(),
        SecretLevel::User,
    );

    assert_eq!(
        "foo ************* bar",
        blockgrid.grid_handler.bounds_to_string(
            Point::new(0, 0),
            Point::new(0, 21),
            false,
            RespectObfuscatedSecrets::Yes,
            false, /* force_secrets_obfuscated */
            RespectDisplayedOutput::No
        )
    );

    let (handle, _) = blockgrid
        .grid_handler()
        .secret_at_displayed_point(Point::new(0, 4))
        .expect("handle should be defined");

    blockgrid
        .grid_handler_mut()
        .unobfuscate_secret(handle)
        .expect("should unobfuscate secret");

    assert_eq!(
        "foo zach@warp.dev bar",
        blockgrid.grid_handler.bounds_to_string(
            Point::new(0, 0),
            Point::new(0, 21),
            false,
            RespectObfuscatedSecrets::Yes,
            false, /* force_secrets_obfuscated */
            RespectDisplayedOutput::No
        )
    );
}

#[test]
fn test_finds_url_in_grid() {
    // Test url in one line.
    let blockgrid = mock_blockgrid("https://google.com");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 0 }),
        Some(Link {
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 17 },
            is_empty: false
        })
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 17 }),
        Some(Link {
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 17 },
            is_empty: false
        })
    );

    // Test url with some other texts.
    let blockgrid = mock_blockgrid("abc https://google.com");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 0 }),
        None
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 10 }),
        Some(Link {
            range: Point { row: 0, col: 4 }..=Point { row: 0, col: 21 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_url_line_wrapping() {
    let blockgrid = mock_blockgrid("abc https://goog\nle.com");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 1, col: 0 }),
        Some(Link {
            range: Point { row: 0, col: 4 }..=Point { row: 1, col: 5 },
            is_empty: false
        })
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 15 }),
        Some(Link {
            range: Point { row: 0, col: 4 }..=Point { row: 1, col: 5 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_url_with_delimiter() {
    let blockgrid = mock_blockgrid("https://google.com'");
    // Should not include the trailing delimiter.
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 0 }),
        Some(Link {
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 17 },
            is_empty: false
        })
    );

    let blockgrid = mock_blockgrid("https://google.com/search?q=warp");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 0 }),
        Some(Link {
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 31 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_url_line_breaks() {
    let blockgrid = mock_blockgrid("abc https://goog\r\nle.com");
    let expected_link = Link {
        range: Point { row: 0, col: 4 }..=Point { row: 1, col: 5 },
        is_empty: false,
    };
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 1, col: 0 }),
        Some(expected_link.clone())
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 10 }),
        Some(expected_link.clone())
    );
    let raw_url = blockgrid.grid_handler.bounds_to_string(
        *expected_link.range.start(),
        *expected_link.range.end(),
        false,
        RespectObfuscatedSecrets::Yes,
        false,
        RespectDisplayedOutput::No,
    );
    assert_eq!(raw_url, "https://goog\nle.com");
    assert_eq!(
        normalize_hard_wrapped_http_url(&raw_url),
        Some("https://google.com".to_owned())
    );
    assert_eq!(
        normalize_hard_wrapped_http_url("HTTPS://goog\nle.com"),
        Some("HTTPS://google.com".to_owned())
    );
    assert_eq!(
        normalize_hard_wrapped_http_url(
            "https://x.test/?redirect_uri=http%3A%2F%2Flocalhost%2Fcal\nlback&scope=openid&originator=\ncodex-tui"
        ),
        Some(
            "https://x.test/?redirect_uri=http%3A%2F%2Flocalhost%2Fcallback&scope=openid&originator=codex-tui"
                .to_owned()
        )
    );
}

#[test]
fn test_find_hard_wrapped_url_only_when_filtered_rows_are_contiguous() {
    let mut blockgrid = mock_blockgrid("abc https://goog\r\nle.com");
    blockgrid
        .grid_handler_mut()
        .set_displayed_output(DisplayedOutput::new_for_test(vec![0..=1]));
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 1, col: 0 }),
        Some(Link {
            range: Point { row: 0, col: 4 }..=Point { row: 1, col: 5 },
            is_empty: false,
        })
    );

    blockgrid
        .grid_handler_mut()
        .set_displayed_output(DisplayedOutput::new_for_test(vec![1..=1]));
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 0 }),
        None
    );
}

#[test]
fn test_does_not_join_normal_text_across_hard_line_breaks() {
    let blockgrid = mock_blockgrid("https://example.com\r\nordinary multiline text");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 1, col: 0 }),
        None
    );
    assert_eq!(
        blockgrid.grid_handler.bounds_to_string(
            Point { row: 0, col: 0 },
            Point { row: 1, col: 22 },
            false,
            RespectObfuscatedSecrets::Yes,
            false,
            RespectDisplayedOutput::No,
        ),
        "https://example.com\nordinary multiline text"
    );
    assert_eq!(
        normalize_hard_wrapped_http_url("https://example.com\nordinarytext"),
        None
    );
    assert_eq!(
        normalize_hard_wrapped_http_url("https://example.com?q=x\nordinarytext"),
        None
    );
    assert_eq!(
        normalize_hard_wrapped_http_url("https://example.com/path/\nordinarytext"),
        None
    );
}

#[test]
fn test_does_not_join_multiple_urls_across_hard_line_breaks() {
    let blockgrid = mock_blockgrid("https://one.example\r\nhttps://two.example");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 8 }),
        Some(Link {
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 18 },
            is_empty: false,
        })
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 1, col: 8 }),
        Some(Link {
            range: Point { row: 1, col: 0 }..=Point { row: 1, col: 18 },
            is_empty: false,
        })
    );
    assert_eq!(
        blockgrid.grid_handler.bounds_to_string(
            Point { row: 0, col: 0 },
            Point { row: 1, col: 18 },
            false,
            RespectObfuscatedSecrets::Yes,
            false,
            RespectDisplayedOutput::No,
        ),
        "https://one.example\nhttps://two.example"
    );
    assert_eq!(
        normalize_hard_wrapped_http_url("https://one.example\nhttps://two.example"),
        None
    );
    assert_eq!(
        normalize_hard_wrapped_http_url("https://one.example/?next=\nHTTPS://two.example"),
        None
    );
}

#[test]
fn test_find_url_wide_characters() {
    let blockgrid = mock_blockgrid("https://google.com/啊啊啊啊");
    assert!(has_wide_char_character(&blockgrid));
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 5 }),
        Some(Link {
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 25 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_url_omits_trailing_periods() {
    // Test that it omits a single trailing period.
    let blockgrid = mock_blockgrid("Visit https://github.com/zerx-lab/warp/issues.");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 10 }),
        Some(Link {
            range: Point { row: 0, col: 6 }..=Point { row: 0, col: 44 },
            is_empty: false
        })
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 45 }),
        None
    );

    // Test that it omits multiple trailing periods.
    let blockgrid = mock_blockgrid("Visit https://github.com/zerx-lab/warp/issues...");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 10 }),
        Some(Link {
            range: Point { row: 0, col: 6 }..=Point { row: 0, col: 44 },
            is_empty: false
        })
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 47 }),
        None
    );

    // Test that it handles a period in the middle of the URL path somewhere.
    let blockgrid = mock_blockgrid("Visit https://github.com/warp.dev/Zaplex/issues.");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 10 }),
        Some(Link {
            range: Point { row: 0, col: 6 }..=Point { row: 0, col: 46 },
            is_empty: false
        })
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 33 }),
        Some(Link {
            range: Point { row: 0, col: 6 }..=Point { row: 0, col: 46 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_url_with_percent() {
    let blockgrid = mock_blockgrid("url with percent https://example.com/search?q=hello%20world");
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 55 }),
        Some(Link {
            range: Point { row: 0, col: 17 }..=Point { row: 0, col: 58 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_long_url() {
    let blockgrid = mock_blockgrid(
        "\
this is long https://very-long-subdomain-name-123456789.another-long-subdomain-p\n\
art-123456789.example.com/this/is/a/very/long/path/with/many/segments/and/we/nee\n\
d/to/make/it/longer/still/so/adding/more/segments/here/and/there/plus/some/query\n\
/parameters?user=john&id=123456789&token=abcdef",
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 3, col: 14 }),
        Some(Link {
            range: Point { row: 0, col: 13 }..=Point { row: 3, col: 46 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_super_long_url() {
    let blockgrid = mock_blockgrid(
        "\
this is long https://very-long-subdomain-name-123456789.another-long-subdomain-p\n\
art-123456789.example.com/this/is/a/very/long/path/with/many/segments/and/we/nee\n\
d/to/make/it/longer/still/so/adding/more/segments/here/and/there/plus/some/query\n\
/parameters?user=john&id=123456789&token=abcdefsdfdsfpiojdsfpojsdfpojsdfpojsdfpo\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsqwertyuioplkj and then it ends and there\n\
s some more content over here\n\
",
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 12, col: 52 }),
        Some(Link {
            range: Point { row: 0, col: 13 }..=Point { row: 12, col: 52 },
            is_empty: false
        })
    );
}

#[test]
fn test_find_super_duper_long_url() {
    let blockgrid = mock_blockgrid(
        "\
this is long https://very-long-subdomain-name-123456789.another-long-subdomain-p\n\
art-123456789.example.com/this/is/a/very/long/path/with/many/segments/and/we/nee\n\
d/to/make/it/longer/still/so/adding/more/segments/here/and/there/plus/some/query\n\
/parameters?user=john&id=123456789&token=abcdefsdfdsfpiojdsfpojsdfpojsdfpojsdfpo\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsddfhsdfouhsdofuhrouvnvoreuenreovundsvoun\n\
dfdsfvcvhoudfhyouhcvjudshvouhdvouhdfouhsqwertyuioplkj and then it ends and there\n\
s some more content over here\n\
",
    );
    assert_eq!(
        blockgrid
            .grid_handler
            .url_at_point(Point { row: 0, col: 40 }),
        None,
    );
}

#[test]
fn test_possible_file_paths() {
    let blockgrid = mock_blockgrid("file link: src/восиб,abcвосиб");
    let valid_path = vec![
        PossiblePath {
            path: CleanPathResult {
                path: "file link: src/восиб,abcвосиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 28 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "file link: src/восиб,".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 20 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "file link: src/восиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 19 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: " link: src/восиб,abcвосиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 4 }..=Point { row: 0, col: 28 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: " link: src/восиб,".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 4 }..=Point { row: 0, col: 20 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: " link: src/восиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 4 }..=Point { row: 0, col: 19 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "link: src/восиб,abcвосиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 5 }..=Point { row: 0, col: 28 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "link: src/восиб,".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 5 }..=Point { row: 0, col: 20 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "link: src/восиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 5 }..=Point { row: 0, col: 19 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: ": src/восиб,abcвосиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 9 }..=Point { row: 0, col: 28 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: ": src/восиб,".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 9 }..=Point { row: 0, col: 20 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: ": src/восиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 9 }..=Point { row: 0, col: 19 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: " src/восиб,abcвосиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 10 }..=Point { row: 0, col: 28 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: " src/восиб,".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 10 }..=Point { row: 0, col: 20 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: " src/восиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 10 }..=Point { row: 0, col: 19 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "src/восиб,abcвосиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 11 }..=Point { row: 0, col: 28 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "src/восиб,".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 11 }..=Point { row: 0, col: 20 },
        },
        PossiblePath {
            path: CleanPathResult {
                path: "src/восиб".into(),
                line_and_column_num: None,
            },
            range: Point { row: 0, col: 11 }..=Point { row: 0, col: 19 },
        },
    ];

    // Hover on "s" in src/восиб.
    assert_eq!(
        blockgrid
            .grid_handler
            .possible_file_paths_at_point(Point { row: 0, col: 11 }),
        valid_path
    );

    // Hover on a single "." in one line.
    let blockgrid = mock_blockgrid(".");
    assert_eq!(
        blockgrid
            .grid_handler
            .possible_file_paths_at_point(Point { row: 0, col: 0 }),
        vec![PossiblePath {
            path: CleanPathResult {
                path: ".".into(),
                line_and_column_num: None
            },
            range: Point { row: 0, col: 0 }..=Point { row: 0, col: 0 }
        }]
    );

    // Hover on a file path with line and column numbers.
    let blockgrid = mock_blockgrid("восиб:100");
    assert_eq!(
        blockgrid
            .grid_handler
            .possible_file_paths_at_point(Point { row: 0, col: 6 }),
        vec![
            PossiblePath {
                path: CleanPathResult {
                    path: "восиб".into(),
                    line_and_column_num: Some(LineAndColumnArg {
                        line_num: 100,
                        column_num: None
                    })
                },
                range: Point { row: 0, col: 0 }..=Point { row: 0, col: 8 }
            },
            PossiblePath {
                path: CleanPathResult {
                    path: "".into(),
                    line_and_column_num: Some(LineAndColumnArg {
                        line_num: 100,
                        column_num: None
                    })
                },
                range: Point { row: 0, col: 5 }..=Point { row: 0, col: 8 }
            },
            PossiblePath {
                path: CleanPathResult {
                    path: "100".into(),
                    line_and_column_num: None
                },
                range: Point { row: 0, col: 6 }..=Point { row: 0, col: 8 }
            },
        ]
    );
}

#[test]
fn test_fragment_boundary_at_point() {
    let assert_fragment_boundary =
        |blockgrid: &BlockGrid,
         (cursor_row, cursor_col),
         expected_fragment: Range<(usize, usize)>| {
            let cursor_point = Point {
                row: cursor_row,
                col: cursor_col,
            };
            let expected_fragment_start = Point {
                row: expected_fragment.start.0,
                col: expected_fragment.start.1,
            };
            let expected_fragment_end = Point {
                row: expected_fragment.end.0,
                col: expected_fragment.end.1,
            };
            let actual_fragment = blockgrid
                .grid_handler
                .fragment_boundary_at_point(&cursor_point)
                .0;
            assert_eq!(
                actual_fragment,
                expected_fragment_start..expected_fragment_end
            );
        };

    let blockgrid = mock_blockgrid("/abc\\text gef");
    assert_fragment_boundary(&blockgrid, (0, 2), (0, 0)..(0, 4)); // Should give boundary of fragment "/abc"
    assert_fragment_boundary(&blockgrid, (0, 4), (0, 4)..(0, 5)); // Should give boundary of fragment "\"
    assert_fragment_boundary(&blockgrid, (0, 7), (0, 5)..(0, 9)); // Should give boundary of fragment "text"
    assert_fragment_boundary(&blockgrid, (0, 10), (0, 10)..(0, 13)); // Should give boundary of fragment "gef"

    // If hovering over the line length boundary, we should return the boundary
    // of the empty fragment from end of line to the max column length.
    assert_fragment_boundary(&blockgrid, (0, 13), (0, 13)..(0, 13));

    let blockgrid = mock_blockgrid("line one\r\nline two");
    assert_fragment_boundary(&blockgrid, (0, 6), (0, 5)..(0, 8)); // Should give boundary of fragment "one"
    assert_fragment_boundary(&blockgrid, (1, 3), (1, 0)..(1, 4)); // Should give boundary of fragment "line"

    // The `ls` command performs indenting via a tab '\t' followed by a series of null characters '\0'
    let blockgrid = mock_blockgrid("migrations\t\0\0\0resources");
    assert_fragment_boundary(&blockgrid, (0, 5), (0, 0)..(0, 10)); // Should give boundary of fragment "migrations"
    assert_fragment_boundary(&blockgrid, (0, 10), (0, 10)..(0, 11)); // Should give boundary of fragment "\t"
    assert_fragment_boundary(&blockgrid, (0, 13), (0, 13)..(0, 14)); // Should give boundary of fragment "\0"
    assert_fragment_boundary(&blockgrid, (0, 16), (0, 14)..(0, 23)); // Should give boundary of fragment "resources"
}

#[test]
fn test_serializing_string() {
    // a
    let a = Cell::from('a');
    assert_eq!(GridHandler::cell_to_string(&a), "a");

    // b, red foreground
    let mut b = Cell::from('b');
    b.fg = Color::Named(NamedColor::Red);
    assert_eq!(GridHandler::cell_to_string(&b), "\x1b[31mb\x1b[0m");

    // c, blue foreground, white background
    let mut c = Cell::from('c');
    c.fg = Color::Named(NamedColor::Blue);
    c.bg = Color::Named(NamedColor::White);
    assert_eq!(GridHandler::cell_to_string(&c), "\x1b[34;47mc\x1b[0m");

    // d, bold
    let mut d = Cell::from('d');
    d.flags = Flags::BOLD;
    assert_eq!(GridHandler::cell_to_string(&d), "\x1b[1md\x1b[0m");

    // e, italic, underline, strike
    let mut e = Cell::from('e');
    e.flags = Flags::ITALIC | Flags::UNDERLINE | Flags::STRIKEOUT;
    assert_eq!(GridHandler::cell_to_string(&e), "\x1b[3;4;9me\x1b[0m");

    // f, italic, green background
    let mut f = Cell::from('f');
    f.bg = Color::Named(NamedColor::Green);
    f.flags = Flags::ITALIC;
    assert_eq!(GridHandler::cell_to_string(&f), "\x1b[42;3mf\x1b[0m");

    // g, yellow background, white foreground, underline, strike
    let mut g = Cell::from('g');
    g.bg = Color::Named(NamedColor::Yellow);
    g.fg = Color::Named(NamedColor::White);
    g.flags = Flags::UNDERLINE | Flags::STRIKEOUT;
    assert_eq!(GridHandler::cell_to_string(&g), "\x1b[37;43;4;9mg\x1b[0m");

    // h, named fg, 256-color bg
    let mut h = Cell::from('h');
    h.bg = Color::Indexed(196);
    h.fg = Color::Named(NamedColor::Cyan);
    assert_eq!(GridHandler::cell_to_string(&h), "\x1b[36;48;5;196mh\x1b[0m");

    // i, true-color fg, named bg
    let mut i = Cell::from('i');
    i.bg = Color::Named(NamedColor::Magenta);
    i.fg = Color::Spec(ColorU::new(24, 143, 3, 255));
    assert_eq!(
        GridHandler::cell_to_string(&i),
        "\x1b[38;2;24;143;3;45mi\x1b[0m"
    );

    // j, 256-color fg, true-color bg
    let mut j = Cell::from('j');
    j.bg = Color::Spec(ColorU::new(130, 32, 10, 255));
    j.fg = Color::Indexed(67);
    assert_eq!(
        GridHandler::cell_to_string(&j),
        "\x1b[38;5;67;48;2;130;32;10mj\x1b[0m"
    );

    // k, default fg and bg
    let mut k = Cell::from('k');
    k.bg = Color::Named(NamedColor::Background);
    k.fg = Color::Named(NamedColor::Foreground);
    assert_eq!(GridHandler::cell_to_string(&k), "k");

    // l, bright colors
    let mut l = Cell::from('l');
    l.bg = Color::Named(NamedColor::BrightBlue);
    l.fg = Color::Named(NamedColor::BrightRed);
    assert_eq!(GridHandler::cell_to_string(&l), "\x1b[91;104ml\x1b[0m");

    // m, handles an exceptional case with impossible colors
    let mut m = Cell::from('m');
    // background color should never be NamedColor::Foreground, and vice versa
    m.bg = Color::Named(NamedColor::Foreground);
    m.fg = Color::Named(NamedColor::Background);
    // should not crash. should fall back to default codes for fg/bg
    assert_eq!(GridHandler::cell_to_string(&m), "\x1b[39;49mm\x1b[0m");

    // n, dim colors preserved
    let mut n = Cell::from('n');
    n.fg = Color::Named(NamedColor::Red);
    n.flags = Flags::DIM;
    assert_eq!(GridHandler::cell_to_string(&n), "\x1b[31;2mn\x1b[0m");
}

#[test]
fn test_split_block_grid() {
    // Sanity check splitting a block grid works correctly.
    // Note that we have more comprehensive tests at the Grid level for splitting as well.
    #[rustfmt::skip]
    let blockgrid = mock_blockgrid("\
        aaaa\r\n\
        bbbb\n\
        cccccc\r\n\
        dd\r\n\
        eeeeeeee\
    ");

    let (top_grid, bottom_grid) =
        blockgrid.split(NonZeroUsize::new(2).expect("should not be zero"));
    let bottom_grid = bottom_grid.expect("Expected Some() response from splitting blockgrid!");

    let g1 = top_grid.grid_storage();
    let g2 = bottom_grid.grid_storage();

    // Check row counts
    assert_eq!(g1.total_rows(), 2);
    assert_eq!(g1.visible_rows(), 2);
    assert_eq!(g1.raw.len(), 2);

    assert_eq!(g2.total_rows(), 3);
    assert_eq!(g2.visible_rows(), 3);
    assert_eq!(g2.raw.len(), 3);

    // Verify character content for grid1 and grid2
    assert_eq!(g1[0][0].c, 'a');
    assert_eq!(g1[0][1].c, 'a');
    assert_eq!(g1[1][0].c, 'b');

    assert_eq!(g2[0][0].c, 'c');
    assert_eq!(g2[0][1].c, 'c');
    assert_eq!(g2[1][0].c, 'd');
    assert_eq!(g2[2][0].c, 'e');

    // Check if scroll_region got adjusted correctly
    // Note that these ranges are EXCLUSIVE of the last index (hence # of rows)
    assert_eq!(
        *top_grid.grid_handler.scroll_region(),
        VisibleRow(0)..VisibleRow(2)
    );
    assert_eq!(
        *bottom_grid.grid_handler.scroll_region(),
        VisibleRow(0)..VisibleRow(3)
    );
}

/// Tests whether we correctly handle emoji variation selectors (to turn 1-width character
/// into a 2-width character).
#[test]
fn test_emoji_variation_selector() {
    // Manually create BlockGrid so we can use input() to insert characters.
    let size = SizeInfo::new_without_font_metrics(2, 10);

    let mut blockgrid = BlockGrid::new(
        size,
        MAX_SCROLL_LIMIT,
        ChannelEventListener::new_for_test(),
        ObfuscateSecrets::No,
        PerformResetGridChecks::default(),
    );

    blockgrid.start();
    blockgrid.input('a');
    // ☁️ should be a wide character - it is \0x2601\0xFE0F (uses emoji variation selector from Unicode).
    // See https://www.unicode.org/reports/tr51/#def_emoji_presentation_selector.
    blockgrid.input('\u{2601}');
    blockgrid.input('\u{FE0F}');
    blockgrid.input('b');
    blockgrid.input('c');

    assert!(has_wide_char_character(&blockgrid));
    let grid = blockgrid.grid_storage();
    assert_eq!(grid[0][0].c, 'a');
    // Not this is only \0x2601, without the variation selector.
    assert_eq!(grid[0][1].c, '☁');
    // Assert that it is a String (has zerowidth characters).
    assert!(matches!(
        grid[0][1].content_for_display(),
        CharOrStr::Str(_)
    ));
    assert!(grid[0][1].flags.intersects(Flags::WIDE_CHAR));
    assert_eq!(grid[0][2].c, '\0');
    assert!(grid[0][2].flags.intersects(Flags::WIDE_CHAR_SPACER));
    assert_eq!(grid[0][3].c, 'b');
    assert_eq!(grid[0][4].c, 'c');
}

#[test]
pub fn test_grid_agnostic_point() {
    let mut grid = mock_blockgrid(
        "\
        This is a wrap.\r\n\
        Short line\r\n\
        Another long line.\r\n",
    );
    grid.resize(SizeInfo::new_without_font_metrics(10, 10));

    let grid_handler = grid.grid_handler();

    let grid_agnostic_point = grid_handler.grid_agnostic_point(Point { row: 0, col: 0 });
    assert_eq!(grid_agnostic_point, Point { row: 0, col: 0 });

    let grid_agnostic_point = grid_handler.grid_agnostic_point(Point { row: 0, col: 9 });
    assert_eq!(grid_agnostic_point, Point { row: 0, col: 9 });

    let grid_agnostic_point = grid_handler.grid_agnostic_point(Point { row: 1, col: 0 });
    assert_eq!(grid_agnostic_point, Point { row: 0, col: 10 });

    let grid_agnostic_point = grid_handler.grid_agnostic_point(Point { row: 4, col: 0 });
    assert_eq!(grid_agnostic_point, Point { row: 2, col: 10 });

    let grid_agnostic_point = grid_handler.grid_agnostic_point(Point { row: 4, col: 7 });
    assert_eq!(grid_agnostic_point, Point { row: 2, col: 17 });
}

#[test]
pub fn test_compatible_point() {
    let mut grid = mock_blockgrid(
        "\
        This is a wrap.\r\n\
        Short line\r\n\
        Another long line.\r\n",
    );
    grid.resize(SizeInfo::new_without_font_metrics(10, 10));

    let grid_handler = grid.grid_handler();

    let compatible_point = grid_handler.compatible_point(Point { row: 0, col: 0 });
    assert_eq!(compatible_point, Point { row: 0, col: 0 });

    let compatible_point = grid_handler.compatible_point(Point { row: 0, col: 11 });
    assert_eq!(compatible_point, Point { row: 1, col: 1 });

    let compatible_point = grid_handler.compatible_point(Point { row: 2, col: 10 });
    assert_eq!(compatible_point, Point { row: 4, col: 0 });
}

/// Asserts that no orphaned WIDE_CHAR or WIDE_CHAR_SPACER flags exist in
/// the visible row of a GridHandler.
fn assert_no_orphaned_wide_chars(grid: &GridHandler, visible_row: VisibleRow) {
    let num_cols = grid.columns();
    let row = &grid.grid_storage()[visible_row];
    for col in 0..num_cols {
        if row[col].flags.contains(Flags::WIDE_CHAR) {
            assert!(
                col + 1 < num_cols && row[col + 1].flags.contains(Flags::WIDE_CHAR_SPACER),
                "Orphaned WIDE_CHAR at column {col}: next cell is not a WIDE_CHAR_SPACER."
            );
        }
        if row[col].flags.contains(Flags::WIDE_CHAR_SPACER) {
            assert!(
                col > 0 && row[col - 1].flags.contains(Flags::WIDE_CHAR),
                "Orphaned WIDE_CHAR_SPACER at column {col}: previous cell is not a WIDE_CHAR."
            );
        }
    }
}

#[test]
fn test_clear_screen_below_at_wide_char_spacer() {
    // Input a wide char at col 0-1, move cursor to col 1 (spacer), and
    // clear screen below.  The WIDE_CHAR at col 0 must be cleared.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('\u{1F600}');
    grid.input('a');
    grid.goto(VisibleRow(0), 1);
    grid.clear_screen(ansi::ClearMode::Below);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    assert!(!grid.grid_storage()[VisibleRow(0)][0]
        .flags
        .contains(Flags::WIDE_CHAR));
}

#[test]
fn test_clear_screen_above_at_wide_char() {
    // Input narrow chars then a wide char at col 2-3, move cursor to col 2
    // (the WIDE_CHAR), and clear screen above.  The spacer at col 3 must
    // be cleared.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('a');
    grid.input('b');
    grid.input('\u{1F600}'); // Cols 2 (WIDE_CHAR), 3 (SPACER).
    grid.input('c');
    grid.goto(VisibleRow(0), 2);
    grid.clear_screen(ansi::ClearMode::Above);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    assert!(!grid.grid_storage()[VisibleRow(0)][3]
        .flags
        .contains(Flags::WIDE_CHAR_SPACER));
}

#[test]
fn test_clear_screen_above_preserves_adjacent_wide_char() {
    // Wide char at cols 2-3, cursor at col 1.  Clearing above should only
    // affect cols 0-1 on the cursor row and must not touch cols 2-3.
    let mut grid = GridHandler::new_for_test(5, 10);
    grid.input('a');
    grid.input('b');
    grid.input('\u{1F600}'); // Cols 2 (WIDE_CHAR), 3 (SPACER).
    grid.input('c');
    grid.goto(VisibleRow(0), 1);
    grid.clear_screen(ansi::ClearMode::Above);

    assert_no_orphaned_wide_chars(&grid, VisibleRow(0));
    // The wide char at cols 2-3 must be preserved.
    assert!(
        grid.grid_storage()[VisibleRow(0)][2]
            .flags
            .contains(Flags::WIDE_CHAR),
        "WIDE_CHAR at col 2 was incorrectly cleared."
    );
    assert!(
        grid.grid_storage()[VisibleRow(0)][3]
            .flags
            .contains(Flags::WIDE_CHAR_SPACER),
        "WIDE_CHAR_SPACER at col 3 was incorrectly cleared."
    );
}

// ─── content_len / trailing blank row trimming ───────────────────────

#[test]
fn content_len_cursor_below_content() {
    // Content on rows 0-2, cursor moved to row 5 via goto → content_len() == 3
    let mut grid = GridHandler::new_for_test(10, 20);
    grid.set_track_content_length(true);
    grid.input_at_cursor("row0");
    grid.carriage_return();
    grid.linefeed();
    grid.input_at_cursor("row1");
    grid.carriage_return();
    grid.linefeed();
    grid.input_at_cursor("row2");
    // Move cursor below content via CUP (as real TUI apps do)
    grid.goto(VisibleRow(5), 0);
    grid.on_finish_byte_processing(&ansi::ProcessorInput::new(&[]));
    assert_eq!(grid.content_len(), 3);
}
