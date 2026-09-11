use super::is_provider_credential_environment_variable;

#[test]
fn identifies_provider_credentials_that_must_not_reach_cli_harnesses() {
    for name in crate::ai::subscription_agent::CLAUDE_SUBSCRIPTION_PROVIDER_ENVIRONMENT_VARIABLES
        .into_iter()
        .chain(["OPENAI_API_KEY", "GEMINI_API_KEY", "GOOGLE_API_KEY"])
    {
        assert!(is_provider_credential_environment_variable(name));
    }

    assert!(!is_provider_credential_environment_variable("GITHUB_TOKEN"));
}
