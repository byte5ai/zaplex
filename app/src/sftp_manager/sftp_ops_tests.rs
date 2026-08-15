use super::*;

#[test]
fn user_message_is_localized_without_exposing_raw_diagnostic() {
    crate::i18n::init(Some("en"));
    let error = SftpOpsError::Connection("socket reset by peer".to_string());

    let message = error.user_message();

    assert!(message.contains("connection") || message.contains("Connection"));
    assert!(!message.contains("socket reset by peer"));
    assert_eq!(
        error.to_string(),
        "Connection error: socket reset by peer",
        "the diagnostic remains available for logging"
    );
}

#[cfg(unix)]
#[test]
fn local_transfer_temp_creation_never_follows_existing_symlink() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let protected = temp.path().join("protected");
    let candidate = temp.path().join("partial");
    fs::write(&protected, b"must survive").unwrap();
    symlink(&protected, &candidate).unwrap();

    let result = open_new_local_transfer_file(&candidate);

    assert!(
        matches!(result, Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists),
        "an existing temporary path must be rejected"
    );
    assert_eq!(fs::read(&protected).unwrap(), b"must survive");
}

/// Test SftpOpsError::Connection Display output
#[test]
fn test_sftp_ops_error_display_connection() {
    assert_eq!(
        SftpOpsError::Connection("refused".into()).to_string(),
        "Connection error: refused"
    );
}

/// Test SftpOpsError::Operation Display output
#[test]
fn test_sftp_ops_error_display_operation() {
    assert_eq!(
        SftpOpsError::Operation("not found".into()).to_string(),
        "Operation error: not found"
    );
}

#[test]
fn secure_transfer_capability_error_has_actionable_localized_message() {
    crate::i18n::init(Some("en"));
    let error = SftpOpsError::CapabilityRequired("not negotiated".to_string());

    assert_eq!(
        error.user_message(),
        "Reconnect this server before starting a secure transfer."
    );
    assert!(!error.user_message().contains("not negotiated"));
}

/// Test SftpOpsError::LocalIo Display output
#[test]
fn test_sftp_ops_error_display_local_io() {
    assert_eq!(
        SftpOpsError::LocalIo("disk full".into()).to_string(),
        "Local I/O error: disk full"
    );
}

/// Test SftpOpsError::NoCredentials Display output
#[test]
fn test_sftp_ops_error_display_no_credentials() {
    assert_eq!(
        SftpOpsError::NoCredentials("no key".into()).to_string(),
        "Credentials not found: no key"
    );
}

/// Test SftpOpsError::Cancelled Display output
#[test]
fn test_sftp_ops_error_display_cancelled() {
    assert_eq!(SftpOpsError::Cancelled.to_string(), "Transfer cancelled");
}

/// Test conversion from std::io::Error to SftpOpsError
#[test]
fn test_sftp_ops_error_from_io_error() {
    let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
    let ops_err: SftpOpsError = io_err.into();
    assert!(matches!(ops_err, SftpOpsError::NotFound(_)));
}

/// Test conversion from zap_sftp::SftpError to SftpOpsError
#[test]
fn test_sftp_ops_error_from_sftp_error() {
    let sftp_err = zap_sftp::SftpError::General("test error".into());
    let ops_err: SftpOpsError = sftp_err.into();
    assert!(matches!(ops_err, SftpOpsError::Operation(_)));
}

/// Test shellexpand_path expanding ~/ path
#[test]
fn test_shellexpand_path_home() {
    let home = dirs::home_dir().unwrap_or_default();
    let result = shellexpand_path("~/test");
    if !home.as_os_str().is_empty() {
        assert!(!result.starts_with('~'));
        assert!(result.contains("test"));
    }
}

/// Test shellexpand_path preserving absolute path
#[test]
fn test_shellexpand_path_absolute() {
    let result = shellexpand_path("/absolute/path");
    assert_eq!(result, "/absolute/path");
}

/// Test shellexpand_path preserving relative path
#[test]
fn test_shellexpand_path_relative() {
    let result = shellexpand_path("relative/path");
    assert_eq!(result, "relative/path");
}

/// Test shellexpand_path with tilde only (no expansion)
#[test]
fn test_shellexpand_path_tilde_only() {
    let result = shellexpand_path("~");
    assert_eq!(result, "~");
}

/// Test shellexpand_path with empty path
#[test]
fn test_shellexpand_path_empty() {
    let result = shellexpand_path("");
    assert_eq!(result, "");
}

// ==================== bool_to_rwx tests ====================

/// Test full permissions rwx
#[test]
fn test_bool_to_rwx_all_true() {
    assert_eq!(bool_to_rwx(true, true, true), "rwx");
}

/// Test no permissions
#[test]
fn test_bool_to_rwx_all_false() {
    assert_eq!(bool_to_rwx(false, false, false), "---");
}

/// Test read-only permission
#[test]
fn test_bool_to_rwx_read_only() {
    assert_eq!(bool_to_rwx(true, false, false), "r--");
}

/// Test write-only permission
#[test]
fn test_bool_to_rwx_write_only() {
    assert_eq!(bool_to_rwx(false, true, false), "-w-");
}

/// Test execute-only permission
#[test]
fn test_bool_to_rwx_exec_only() {
    assert_eq!(bool_to_rwx(false, false, true), "--x");
}

/// Test read-write permissions
#[test]
fn test_bool_to_rwx_read_write() {
    assert_eq!(bool_to_rwx(true, true, false), "rw-");
}

/// Test read-execute permissions
#[test]
fn test_bool_to_rwx_read_exec() {
    assert_eq!(bool_to_rwx(true, false, true), "r-x");
}

/// Test write-execute permissions
#[test]
fn test_bool_to_rwx_write_exec() {
    assert_eq!(bool_to_rwx(false, true, true), "-wx");
}

/// Test return value length is always 3
#[test]
fn test_bool_to_rwx_length() {
    for r in [true, false] {
        for w in [true, false] {
            for x in [true, false] {
                assert_eq!(bool_to_rwx(r, w, x).len(), 3);
            }
        }
    }
}

/// Test each character position is only the target character
#[test]
fn test_bool_to_rwx_valid_chars() {
    for r in [true, false] {
        for w in [true, false] {
            for x in [true, false] {
                let s = bool_to_rwx(r, w, x);
                let chars: Vec<char> = s.chars().collect();
                assert!((chars[0] == 'r') || (chars[0] == '-'));
                assert!((chars[1] == 'w') || (chars[1] == '-'));
                assert!((chars[2] == 'x') || (chars[2] == '-'));
            }
        }
    }
}

// ==================== SftpOpsError edge case tests ====================

/// Test SftpOpsError::Connection with empty message
#[test]
fn test_sftp_ops_error_connection_empty() {
    assert_eq!(
        SftpOpsError::Connection(String::new()).to_string(),
        "Connection error: "
    );
}

/// Test SftpOpsError::Operation with empty message
#[test]
fn test_sftp_ops_error_operation_empty() {
    assert_eq!(
        SftpOpsError::Operation(String::new()).to_string(),
        "Operation error: "
    );
}

/// Test SftpOpsError::LocalIo with empty message
#[test]
fn test_sftp_ops_error_local_io_empty() {
    assert_eq!(
        SftpOpsError::LocalIo(String::new()).to_string(),
        "Local I/O error: "
    );
}

/// Test SftpOpsError::NoCredentials with empty message
#[test]
fn test_sftp_ops_error_no_credentials_empty() {
    assert_eq!(
        SftpOpsError::NoCredentials(String::new()).to_string(),
        "Credentials not found: "
    );
}

/// Test SftpOpsError::Cancelled always returns fixed text
#[test]
fn test_sftp_ops_error_cancelled_consistent() {
    let s1 = SftpOpsError::Cancelled.to_string();
    let s2 = SftpOpsError::Cancelled.to_string();
    assert_eq!(s1, s2);
    assert_eq!(s1, "Transfer cancelled");
}

/// Test shellexpand_path expanding nested ~/ path
#[test]
fn test_shellexpand_path_home_nested() {
    let result = shellexpand_path("~/a/b/c");
    assert!(!result.starts_with('~'));
    assert!(result.contains("a/b/c"));
}

/// Test shellexpand_path with tilde followed by slash with no additional path
#[test]
fn test_shellexpand_path_home_root() {
    let result = shellexpand_path("~/");
    let home = dirs::home_dir().unwrap_or_default();
    if !home.as_os_str().is_empty() {
        assert!(!result.starts_with('~'));
    }
}
