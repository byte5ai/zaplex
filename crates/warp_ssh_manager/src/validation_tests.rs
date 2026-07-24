use super::*;

#[test]
fn empty_host_is_rejected_everywhere() {
    for endpoint_use in [EndpointUse::Save, EndpointUse::Test, EndpointUse::Connect] {
        assert_eq!(
            validate_ssh_endpoint(endpoint_use, "   ", "22"),
            Err(SshEndpointValidationError::EmptyHost)
        );
    }
}

#[test]
fn invalid_port_never_falls_back_to_22() {
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
