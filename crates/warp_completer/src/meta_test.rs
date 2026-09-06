use super::*;

/*
0 1 2 3
w a r p
-------
0     4  << the span for the string "warp" is (0, 4)

Spanned {
    item: String::new("warp"),  << warp string
    span: Span::new(0, 4)       << span
}

or >> String::new("warp").spanned(Span::new(0, 4))        */
fn warp() -> Spanned<String> {
    String::from("warp").spanned(Span::new(0, 4))
}

fn empty() -> Spanned<String> {
    String::new().spanned_unknown()
}

#[test]
fn knows_distances() {
    assert!(warp().span.distance() == 4);
    assert!(empty().span.distance() == 0);
}

#[test]
fn slice_returns_the_exact_substring_for_a_valid_span() {
    assert_eq!(Span::new(0, 6).slice("zaplex terminal"), "zaplex");
    assert_eq!(Span::new(7, 15).slice("zaplex terminal"), "terminal");
}

#[test]
fn slice_clamps_offsets_inside_a_multibyte_character() {
    let source = "aéz";
    assert_eq!(source.len(), 4);
    assert!(!source.is_char_boundary(2));

    assert_eq!(Span::new(2, 4).slice(source), "éz");
}

#[test]
fn slice_clamps_out_of_bounds_offsets_to_the_end() {
    assert_eq!(Span::new(0, 1000).slice("zaplex"), "zaplex");
    assert_eq!(Span::new(1000, 2000).slice("zaplex"), "");
}

#[test]
fn slice_clamps_an_inverted_raw_span_without_underflowing() {
    let span = Span { start: 10, end: 2 };

    assert_eq!(span.slice("zaplex terminal"), "");
}
