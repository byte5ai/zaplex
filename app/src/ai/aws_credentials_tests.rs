use std::time::Duration;

use ai::api_keys::AwsCredentialsRefreshStrategy;
use aws_credential_types::provider::error::CredentialsError;

use super::{
    is_current_refresh_configuration, user_facing_aws_credentials_error_message,
    AwsCredentialsRefreshRequest,
};

#[test]
fn maps_credentials_not_loaded_to_user_message() {
    let message = user_facing_aws_credentials_error_message(
        &CredentialsError::not_loaded_no_source(),
        "sandbox",
    );

    assert_eq!(
        message,
        "AWS credentials were not found for the AWS profile `sandbox`. Log in with the AWS CLI or update your AWS credentials configuration, then refresh."
    );
}

#[test]
fn maps_invalid_configuration_to_user_message() {
    let message = user_facing_aws_credentials_error_message(
        &CredentialsError::invalid_configuration(std::io::Error::other("bad config")),
        "readonly",
    );

    assert_eq!(
        message,
        "The AWS profile `readonly` is invalid or incomplete in your local AWS configuration. Update your AWS profile settings and credentials, then refresh."
    );
}

#[test]
fn maps_provider_timeout_to_user_message() {
    let message = user_facing_aws_credentials_error_message(
        &CredentialsError::provider_timed_out(Duration::from_secs(5)),
        "sandbox",
    );

    assert_eq!(
        message,
        "Timed out while loading AWS credentials. Refresh and try again."
    );
}

#[test]
fn maps_provider_error_to_user_message() {
    let message = user_facing_aws_credentials_error_message(
        &CredentialsError::provider_error(std::io::Error::other("provider error")),
        "sandbox",
    );

    assert_eq!(
        message,
        "Unable to load AWS credentials from your configured provider. Refresh your AWS login and try again."
    );
}

#[test]
fn maps_unhandled_error_to_user_message() {
    let message = user_facing_aws_credentials_error_message(
        &CredentialsError::unhandled(std::io::Error::other("unexpected")),
        "sandbox",
    );

    assert_eq!(
        message,
        "Unexpected error while loading AWS credentials. Refresh your AWS login and try again."
    );
}

fn request(
    generation: u64,
    profile: &str,
    strategy: AwsCredentialsRefreshStrategy,
) -> AwsCredentialsRefreshRequest {
    AwsCredentialsRefreshRequest {
        generation,
        profile: profile.to_string(),
        strategy,
        credentials_enabled: true,
    }
}

#[test]
fn newer_profile_refresh_rejects_late_success_or_error_from_older_refresh() {
    let older = request(1, "account-a", AwsCredentialsRefreshStrategy::LocalChain);
    let newer = request(2, "account-b", AwsCredentialsRefreshStrategy::LocalChain);

    assert_eq!(
        is_current_refresh_configuration(
            &newer,
            true,
            "account-b",
            &AwsCredentialsRefreshStrategy::LocalChain,
            true,
        ),
        true
    );
    assert_eq!(
        is_current_refresh_configuration(
            &older,
            false,
            "account-b",
            &AwsCredentialsRefreshStrategy::LocalChain,
            true,
        ),
        false
    );
}

#[test]
fn local_chain_completion_cannot_overwrite_newer_oidc_refresh() {
    let local = request(3, "default", AwsCredentialsRefreshStrategy::LocalChain);
    let oidc_strategy = AwsCredentialsRefreshStrategy::OidcManaged {
        task_id: Some("task-b".to_string()),
        role_arn: "arn:aws:iam::123456789012:role/new".to_string(),
    };
    let oidc = request(4, "default", oidc_strategy.clone());

    assert_eq!(
        is_current_refresh_configuration(&oidc, true, "default", &oidc_strategy, true),
        true
    );
    assert_eq!(
        is_current_refresh_configuration(&local, false, "default", &oidc_strategy, true),
        false
    );
}
