//! zap_sftp — SFTP protocol layer wrapper
//!
//! Built on the ssh2 crate, wraps the SFTP file transfer protocol, providing
//! session management, remote file read/write, directory operations, and more.
//! author: logic
//! date: 2026-05-31

pub mod dir;
pub mod error;
pub mod file;
pub mod session;
pub mod sftp;
pub mod types;

pub use dir::Dir;
pub use error::{SftpChannelError, SftpError};
pub use file::File;
pub use session::{AuthMethod, SftpSession};
pub use sftp::Sftp;
pub use types::*;
