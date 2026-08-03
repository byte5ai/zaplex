use super::*;

#[test]
fn empty_host_is_rejected_by_save_test_and_connect() {
    for endpoint_use in [EndpointUse::Save, EndpointUse::Test, EndpointUse::Connect] {
        assert_eq!(
            validate_ssh_endpoint(endpoint_use, "   ", "22"),
            Err(SshEndpointValidationError::EmptyHost)
        );
    }
}

#[test]
fn port_zero_and_out_of_range_are_rejected_without_fallback() {
    for port in ["", "0", "65536", "not-a-port"] {
        for endpoint_use in [EndpointUse::Save, EndpointUse::Test, EndpointUse::Connect] {
            assert_eq!(
                validate_ssh_endpoint(endpoint_use, "example.com", port),
                Err(SshEndpointValidationError::InvalidPort),
                "{endpoint_use:?} unexpectedly accepted {port:?}"
            );
        }
    }

    for port in ["1", "65535"] {
        assert!(validate_ssh_endpoint(EndpointUse::Connect, "example.com", port).is_ok());
    }
}

#[test]
fn option_like_host_is_rejected_everywhere() {
    for endpoint_use in [EndpointUse::Save, EndpointUse::Test, EndpointUse::Connect] {
        assert_eq!(
            validate_ssh_endpoint(endpoint_use, "-oProxyCommand=malicious", "22"),
            Err(SshEndpointValidationError::InvalidHost),
        );
    }
}
