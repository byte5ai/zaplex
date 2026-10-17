use super::*;

#[test]
fn test_escaped_token_spans() {
    let source = r"\\\||\&&\\&&||";
    let spans: Vec<_> = Lexer::new(source, EscapeChar::Backslash, false)
        .map(|t| t.span)
        .collect();

    assert_eq!(
        spans,
        [
            Span::new(0, 1),   // \
            Span::new(1, 2),   // \
            Span::new(2, 3),   // \
            Span::new(3, 4),   // |
            Span::new(4, 5),   // |
            Span::new(5, 6),   // \
            Span::new(6, 7),   // &
            Span::new(7, 8),   // &
            Span::new(8, 9),   // \
            Span::new(9, 10),  // \
            Span::new(10, 12), // &&
            Span::new(12, 14), // ||
        ]
    );
}
