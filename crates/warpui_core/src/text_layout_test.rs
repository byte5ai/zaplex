use super::*;

#[test]
fn test_cache_key_includes_fixed_width_tab_size() {
    let text = "abc";
    let style_runs: &[(Range<usize>, StyleAndFont)] = &[];

    let key_4 = CacheKeyRef {
        text,
        font_size: OrderedFloat(12.),
        line_height_ratio: OrderedFloat(1.),
        fixed_width_tab_size: Some(4),
        style_runs,
        max_width: OrderedFloat(100.),
        max_height: None,
        alignment: TextAlignment::Left,
        first_line_head_indent: None,
        clip_config: None,
    };
    let key_8 = CacheKeyRef {
        fixed_width_tab_size: Some(8),
        ..key_4
    };

    assert!(key_4 != key_8);
}
