use super::fuzzy_match::SlashCommandFuzzyMatchResult;

#[test]
fn test_match_two_commands_with_same_name_but_different_description() {
    let result1 = SlashCommandFuzzyMatchResult::try_match("test", "test", Some("tests"));
    let result2 = SlashCommandFuzzyMatchResult::try_match("test", "test", Some("faketests"));
    assert!(result1.is_some());
    assert!(result2.is_some());
    let result1 = result1.unwrap();
    let result2 = result2.unwrap();
    assert!(result1.name_match_result.is_some());
    assert!(result2.name_match_result.is_some());
    assert!(result1.description_match_result.is_some());
    assert!(result2.description_match_result.is_some());
    assert!(result1.score() > result2.score());
}
