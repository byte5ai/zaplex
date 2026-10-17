use crate::search::slash_command_menu::fuzzy_match::SlashCommandFuzzyMatchResult;

use super::prefix_match_bonus;

#[test]
fn short_prefix_match_ranks_above_longer_fuzzy_match() {
    // Simulates the reported issue: query "new" should give /new a much
    // higher combined score than /figma-create-new-file.
    let short_match = SlashCommandFuzzyMatchResult::try_match("new", "/new", None).unwrap();
    let long_match =
        SlashCommandFuzzyMatchResult::try_match("new", "/figma-create-new-file", None).unwrap();

    const SCORE_MULTIPLIER: f64 = 1000.0;

    let short_score = short_match.score() * SCORE_MULTIPLIER
        + prefix_match_bonus("new", "/new") * SCORE_MULTIPLIER
        + 1.0 / "/new".len() as f64;
    let long_score = long_match.score() * SCORE_MULTIPLIER
        + prefix_match_bonus("new", "/figma-create-new-file") * SCORE_MULTIPLIER
        + 1.0 / "/figma-create-new-file".len() as f64;

    assert!(
        short_score > long_score,
        "/new score ({short_score}) should be greater than /figma-create-new-file score ({long_score})"
    );
}
