use super::GitLineChanges;

#[test]
fn test_parse_from_git_output_both_additions_and_deletions() {
    let input = " 3 files changed, 5 insertions(+), 2 deletions(-)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 3);
    assert_eq!(result.lines_added, 5);
    assert_eq!(result.lines_removed, 2);
}

#[test]
fn test_parse_from_git_output_only_additions() {
    let input = " 2 files changed, 10 insertions(+)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 2);
    assert_eq!(result.lines_added, 10);
    assert_eq!(result.lines_removed, 0);
}

#[test]
fn test_parse_from_git_output_only_deletions() {
    let input = " 1 file changed, 7 deletions(-)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 1);
    assert_eq!(result.lines_added, 0);
    assert_eq!(result.lines_removed, 7);
}

#[test]
fn test_parse_from_git_output_single_values() {
    // Test singular forms (1 file, 1 insertion, 1 deletion)
    let input = " 1 file changed, 1 insertion(+), 1 deletion(-)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 1);
    assert_eq!(result.lines_added, 1);
    assert_eq!(result.lines_removed, 1);
}

#[test]
fn test_parse_from_git_output_no_leading_spaces() {
    // Git output sometimes doesn't have leading spaces
    let input = "2 files changed, 3 insertions(+), 1 deletion(-)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 2);
    assert_eq!(result.lines_added, 3);
    assert_eq!(result.lines_removed, 1);
}

#[test]
fn test_parse_from_git_output_extra_whitespace() {
    // Test with extra whitespace and tabs
    let input = "\t 1 file changed,   5 insertions(+),  \t 3 deletions(-)   ";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 1);
    assert_eq!(result.lines_added, 5);
    assert_eq!(result.lines_removed, 3);
}

#[test]
fn test_parse_from_git_output_empty_string() {
    let input = "";
    let result = GitLineChanges::parse_from_git_output(input);

    assert!(result.is_none());
}

#[test]
fn test_parse_from_git_output_whitespace_only() {
    let input = "   \t\n  ";
    let result = GitLineChanges::parse_from_git_output(input);

    assert!(result.is_none());
}

#[test]
fn test_parse_from_git_output_invalid_format() {
    let input = "This is not a valid git diff --shortstat output";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    // Should parse but find no valid numbers
    assert_eq!(result.files_changed, 0);
    assert_eq!(result.lines_added, 0);
    assert_eq!(result.lines_removed, 0);
}

#[test]
fn test_parse_from_git_output_partial_matches() {
    // Test when only some parts match the expected pattern
    let input = " 2 files changed, some insertions, 3 deletions(-)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 2);
    assert_eq!(result.lines_added, 0); // "some" is not a number
    assert_eq!(result.lines_removed, 3);
}

#[test]
fn test_parse_from_git_output_zero_changes() {
    // Edge case: explicit zero values (unlikely from git but good to test)
    let input = " 0 files changed, 0 insertions(+), 0 deletions(-)";
    let result = GitLineChanges::parse_from_git_output(input).unwrap();

    assert_eq!(result.files_changed, 0);
    assert_eq!(result.lines_added, 0);
    assert_eq!(result.lines_removed, 0);
}
