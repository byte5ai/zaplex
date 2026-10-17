use super::*;

#[test]
fn test_merge_overlapping_ranges() {
    let highlight = Highlight::new();

    let range1 = HighlightedRange {
        highlight,
        highlight_indices: vec![1, 2, 3],
    };
    let range2 = HighlightedRange {
        highlight,
        highlight_indices: vec![3, 4, 5],
    };

    let result = HighlightedRange::merge_overlapping_ranges(vec![range1.clone(), range2.clone()]);

    assert_eq!(
        result,
        vec![HighlightedRange {
            highlight,
            highlight_indices: vec![1, 2, 3, 4, 5],
        }]
    );
}
