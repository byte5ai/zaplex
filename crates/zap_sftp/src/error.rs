//! SFTP protocol layer error type definitions
//!
//! Defines SftpError and SftpChannelError enums covering connection, authentication,
//! timeout, permission and other error scenarios.
//! author: logic
//! date: 2026-05-31

use thiserror::Error;

/// SFTP protocol-level errors
#[derive(Debug, Error)]
pub enum SftpError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("SSH2 error: {0}")]
    Ssh2(#[from] ssh2::Error),

    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Authentication failed: {0}")]
    AuthFailed(String),

    #[error("Unknown {key_type} host key ({fingerprint_sha256})")]
    UnknownHostKey {
        fingerprint_sha256: String,
        key_type: String,
    },

    #[error("{key_type} host key mismatch ({fingerprint_sha256})")]
    HostKeyMismatch {
        fingerprint_sha256: String,
        key_type: String,
    },

    #[error("Operation timed out")]
    Timeout,

    #[error("File not found: {0}")]
    NoSuchFile(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Operation failed: {0}")]
    General(String),
}

impl SftpError {
    /// Whether the remote operation failed specifically because the path does
    /// not exist. Callers must not treat permission or connection errors as
    /// evidence that a destination is absent.
    pub fn is_not_found(&self) -> bool {
        match self {
            SftpError::Io(error) => error.kind() == std::io::ErrorKind::NotFound,
            SftpError::Ssh2(error) => {
                matches!(error.code(), ssh2::ErrorCode::SFTP(2))
            }
            SftpError::NoSuchFile(_) => true,
            SftpError::ConnectionFailed(_)
            | SftpError::AuthFailed(_)
            | SftpError::UnknownHostKey { .. }
            | SftpError::HostKeyMismatch { .. }
            | SftpError::Timeout
            | SftpError::PermissionDenied(_)
            | SftpError::General(_) => false,
        }
    }
}

/// SFTP channel errors
#[derive(Debug, Error)]
pub enum SftpChannelError {
    #[error("SFTP error: {0}")]
    Sftp(#[from] SftpError),

    #[error("Failed to send request: {0}")]
    SendFailed(String),

    #[error("Failed to receive response: {0}")]
    RecvFailed(String),
}

impl From<ssh2::Error> for SftpChannelError {
    fn from(e: ssh2::Error) -> Self {
        SftpChannelError::Sftp(SftpError::Ssh2(e))
    }
}
