use super::*;

#[test]
fn test_debug_ui_pattern() {
    // Debug the ui/* pattern step by step
    let text = "/src/ui/button.rs";
    let pattern = "ui/*";

    // Test if it should hit the prefix fast path
    let is_prefix_pattern = pattern.ends_with('*')
        && !pattern[..pattern.len() - 1].contains('*')
        && !pattern[..pattern.len() - 1].contains('?');
    assert!(is_prefix_pattern);

    let prefix = &pattern[..pattern.len() - 1];
    assert_eq!(prefix, "ui/");
    assert!(text.contains(prefix)); // Changed from starts_with to contains since text starts with "/src"

    let result = match_wildcard_pattern(text, pattern);
    assert!(result.is_some());
}

#[test]
fn test_wildcard_pattern_star() {
    let result = match_wildcard_pattern("button.rs", "*.rs");
    assert!(result.is_some());
    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_pattern_question_mark() {
    let result = match_wildcard_pattern("button.rs", "butto?.rs");
    assert!(result.is_some());
    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_pattern_no_match() {
    let result = match_wildcard_pattern("button.rs", "*.py");
    assert!(result.is_none());
}

#[test]
fn test_wildcard_pattern_case_insensitive() {
    let result = match_wildcard_pattern_case_insensitive("Button.RS", "*.rs");
    assert!(result.is_some());
    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_pattern_complex() {
    let result = match_wildcard_pattern("/src/ui/button.rs", "*/ui/*.rs");
    assert!(result.is_some());
    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_pattern_exact_match() {
    let result = match_wildcard_pattern("/src/components/button.rs", "/src/components/button.rs");
    assert!(result.is_some());
    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_wildcard_escape_special_chars() {
    // Test that regex special characters are properly escaped
    let result = match_wildcard_pattern("test.file", "test.file");
    assert!(result.is_some());

    let result = match_wildcard_pattern("testXfile", "test.file");
    assert!(result.is_none()); // Should not match because . is literal, not wildcard
}

#[test]
fn test_ui_star_pattern() {
    // Test the specific ui/* pattern that's not working
    let result = match_wildcard_pattern_case_insensitive("/src/ui/button.rs", "ui/*");
    assert!(result.is_some());
    let match_result = result.unwrap();
    assert!(match_result.score > 0);
    assert!(!match_result.matched_indices.is_empty());
}

#[test]
fn test_prefix_star_patterns() {
    // Test various prefix/* patterns
    let result1 = match_wildcard_pattern("src/ui/button.rs", "src/*");
    assert!(result1.is_some());

    let result2 = match_wildcard_pattern("ui/button.rs", "ui/*");
    assert!(result2.is_some());

    let result3 = match_wildcard_pattern("/src/ui/button.rs", "ui/*");
    // This should now match because we do substring matching
    assert!(result3.is_some());
}

#[test]
fn test_partial_suffix_patterns() {
    // Test partial suffix patterns for progressive typing
    let result1 = match_wildcard_pattern("button.rs", "*.r");
    assert!(result1.is_some());

    let result2 = match_wildcard_pattern("component.tsx", "*.t");
    assert!(result2.is_some());

    let result3 = match_wildcard_pattern("test.js", "*.");
    assert!(result3.is_some());

    // Should not match if there's no matching suffix
    let result4 = match_wildcard_pattern("button.rs", "*.py");
    assert!(result4.is_none());
}

#[test]
fn test_ui_star_partial_patterns() {
    // Test the specific cases mentioned in the issue
    let path = "/src/ui/button.rs";

    let result1 = match_wildcard_pattern_case_insensitive(path, "ui/*.rs");
    assert!(result1.is_some());

    let result2 = match_wildcard_pattern_case_insensitive(path, "ui/*.r");
    assert!(result2.is_some());

    let result3 = match_wildcard_pattern_case_insensitive(path, "ui/*.");
    assert!(result3.is_some());

    assert_eq!(result1.unwrap().matched_indices, vec![5, 6, 7, 14, 15, 16]);
}

#[test]
fn multi_star_matches_nested_typescript_path() {
    assert!(match_wildcard_pattern("src/deep/module/file.ts", "**/*.ts").is_some());
    assert!(match_wildcard_pattern("src/deep/module/file.txt", "**/*.ts").is_none());
}
