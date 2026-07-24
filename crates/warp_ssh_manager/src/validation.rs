use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointUse {
    Save,
    Test,
    Connect,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedSshEndpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum SshEndpointValidationError {
    #[error("SSH host is required")]
    EmptyHost,
    #[error("SSH host is invalid")]
    InvalidHost,
    #[error("SSH port must be an integer from 1 through 65535")]
    InvalidPort,
}

pub fn validate_ssh_endpoint(
    _endpoint_use: EndpointUse,
    host: &str,
    port: &str,
) -> Result<ValidatedSshEndpoint, SshEndpointValidationError> {
    let host = host.trim();
    if host.is_empty() {
        return Err(SshEndpointValidationError::EmptyHost);
    }
    if host.starts_with('-')
        || host
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(SshEndpointValidationError::InvalidHost);
    }
    let port = port
        .trim()
        .parse()
        .map_err(|_| SshEndpointValidationError::InvalidPort)?;
    if port == 0 {
        return Err(SshEndpointValidationError::InvalidPort);
    }
    Ok(ValidatedSshEndpoint {
        host: host.to_string(),
        port,
    })
}

#[cfg(test)]
#[path = "validation_tests.rs"]
mod tests;
