//! SFTP session management module
//!
//! Encapsulates SSH2 connection establishment, authentication, and SFTP subsystem channel creation.
//! author: logic
//! date: 2026-05-31

use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::error::SftpError;
use crate::sftp::Sftp;

/// Default connection timeout (10 seconds)
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);

/// Authentication method
#[derive(Debug, Clone)]
pub enum AuthMethod {
    Password { password: String },
    PublicKey { key_path: PathBuf, passphrase: Option<String> },
}

/// SFTP session, wraps ssh2 connection
pub struct SftpSession {
    session: Arc<ssh2::Session>,
    _tcp: TcpStream,
    /// Marks whether connection was explicitly disconnected, prevents double disconnect in Drop
    disconnected: Arc<AtomicBool>,
}

impl SftpSession {
    /// Establish SSH connection with specified parameters
    ///
    /// # Parameters
    /// - `host`: server address
    /// - `port`: server port
    /// - `username`: username
    /// - `auth`: authentication method
    /// - `timeout`: optional timeout duration; None uses default 10 seconds
    pub fn connect(
        host: &str,
        port: u16,
        username: &str,
        auth: AuthMethod,
        timeout: Option<Duration>,
    ) -> Result<Self, SftpError> {
        let effective_timeout = timeout.unwrap_or(DEFAULT_TIMEOUT);
        let addr = format!("{host}:{port}");

        // Resolve DNS via ToSocketAddrs; supports hostnames and IP addresses
        let socket_addr = addr.to_socket_addrs()
            .map_err(|e| SftpError::ConnectionFailed(format!("Address resolution failed: {e}")))?
            .next()
            .ok_or_else(|| SftpError::ConnectionFailed(format!("DNS resolution returned no results: {addr}")))?;

        // Use TCP connection with timeout
        let tcp = TcpStream::connect_timeout(&socket_addr, effective_timeout)
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::TimedOut {
                    SftpError::Timeout
                } else {
                    SftpError::ConnectionFailed(format!("Failed to connect to {addr}: {e}"))
                }
            })?;

        let mut session = ssh2::Session::new()
            .map_err(|e| SftpError::ConnectionFailed(format!("Failed to create SSH session: {e}")))?;

        let tcp_for_session = tcp.try_clone()
            .map_err(|e| SftpError::ConnectionFailed(format!("Failed to clone TCP stream: {e}")))?;
        session.set_tcp_stream(tcp_for_session);

        // Set SSH session timeout (milliseconds); affects handshake and all subsequent blocking operations
        session.set_timeout(effective_timeout.as_millis() as u32);

        session.handshake()
            .map_err(|e| {
                if is_timeout_error(&e) {
                    SftpError::Timeout
                } else {
                    SftpError::ConnectionFailed(format!("SSH handshake failed: {e}"))
                }
            })?;

        match &auth {
            AuthMethod::Password { password } => {
                session.userauth_password(username, password)
                    .map_err(|e| {
                        if is_timeout_error(&e) {
                            SftpError::Timeout
                        } else {
                            SftpError::AuthFailed(format!("Password authentication failed: {e}"))
                        }
                    })?;
            }
            AuthMethod::PublicKey { key_path, passphrase } => {
                let pass = passphrase.as_deref();
                session.userauth_pubkey_file(username, None, key_path, pass)
                    .map_err(|e| {
                        if is_timeout_error(&e) {
                            SftpError::Timeout
                        } else {
                            SftpError::AuthFailed(format!("Public key authentication failed: {e}"))
                        }
                    })?;
            }
        }

        if !session.authenticated() {
            return Err(SftpError::AuthFailed("Authentication failed".into()));
        }

        // Set operation timeout (30 seconds), prevents indefinite blocking on network issues
        session.set_timeout(30_000);

        Ok(Self {
            session: Arc::new(session),
            _tcp: tcp,
            disconnected: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Get SFTP channel
    pub fn sftp(&self) -> Result<Sftp, SftpError> {
        let sftp = self.session.sftp()?;
        Ok(Sftp::new(sftp))
    }

    /// Disconnect
    pub fn disconnect(&self) -> Result<(), SftpError> {
        if self.disconnected.swap(true, Ordering::SeqCst) {
            // Already disconnected, skip
            return Ok(());
        }
        self.session.disconnect(None, "bye", None)?;
        Ok(())
    }

    /// Check if connection is alive
    pub fn is_authenticated(&self) -> bool {
        self.session.authenticated()
    }
}

impl Drop for SftpSession {
    fn drop(&mut self) {
        if !self.disconnected.swap(true, Ordering::SeqCst) {
            let _ = self.session.disconnect(None, "bye", None);
        }
    }
}

/// Check if ssh2 error is a timeout error
fn is_timeout_error(error: &ssh2::Error) -> bool {
    // ssh2 error code Session(-37) corresponds to LIBSSH2_ERROR_SOCKET_TIMEOUT
    error.code() == ssh2::ErrorCode::Session(-37)
}
