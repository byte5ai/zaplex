use super::is_provider_credential_environment_variable;

#[test]
fn identifies_provider_credentials_that_must_not_reach_cli_harnesses() {
    for name in [
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "OPENAI_API_KEY",
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "AWS_BEARER_TOKEN_BEDROCK",
        "CLAUDE_CODE_USE_BEDROCK",
    ] {
        assert!(is_provider_credential_environment_variable(name));
    }

    assert!(!is_provider_credential_environment_variable("GITHUB_TOKEN"));
}
