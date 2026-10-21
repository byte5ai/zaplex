use super::endpoint_validation_message;
use warp_ssh_manager::{validate_ssh_endpoint, EndpointUse};

#[test]
fn empty_and_invalid_hosts_have_distinct_actionable_messages() {
    crate::i18n::init(Some("en"));
    for endpoint_use in [EndpointUse::Save, EndpointUse::Test, EndpointUse::Connect] {
        for (host, expected) in [
            ("", "Host cannot be empty."),
            ("   ", "Host cannot be empty."),
            (
                "bad host name",
                "Enter a host name, IP address or SSH alias without whitespace or control characters; it must not start with '-'.",
            ),
            (
                "-option",
                "Enter a host name, IP address or SSH alias without whitespace or control characters; it must not start with '-'.",
            ),
        ] {
            let error = validate_ssh_endpoint(endpoint_use, host, "22").unwrap_err();
            assert_eq!(endpoint_validation_message(error), expected);
        }
        assert!(validate_ssh_endpoint(endpoint_use, "example.invalid", "22").is_ok());
    }
}
