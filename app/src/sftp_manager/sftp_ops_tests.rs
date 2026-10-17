use super::*;

#[cfg(unix)]
fn live_sftp_configuration() -> (String, u16, String, PathBuf, PathBuf) {
    let host = std::env::var("ZAPLEX_LIVE_SFTP_HOST")
        .expect("ZAPLEX_LIVE_SFTP_HOST is required for the ignored live SFTP tests");
    let port = std::env::var("ZAPLEX_LIVE_SFTP_PORT")
        .expect("ZAPLEX_LIVE_SFTP_PORT is required for the ignored live SFTP tests")
        .parse()
        .expect("ZAPLEX_LIVE_SFTP_PORT must be a valid u16");
    let username = std::env::var("ZAPLEX_LIVE_SFTP_USERNAME")
        .expect("ZAPLEX_LIVE_SFTP_USERNAME is required for the ignored live SFTP tests");
    let key_path = PathBuf::from(
        std::env::var("ZAPLEX_LIVE_SFTP_KEY_PATH")
            .expect("ZAPLEX_LIVE_SFTP_KEY_PATH is required for the ignored live SFTP tests"),
    );
    let root = PathBuf::from(
        std::env::var("ZAPLEX_LIVE_SFTP_ROOT")
            .expect("ZAPLEX_LIVE_SFTP_ROOT is required for the ignored live SFTP tests"),
    );
    (host, port, username, key_path, root)
}

#[cfg(unix)]
#[test]
#[ignore = "requires the isolated OpenSSH fixture from live-sftp-safety.yml"]
fn overwrite_existing_file_on_openssh_v3() {
    let (host, port, username, key_path, root) = live_sftp_configuration();
    let case_root = root.join(format!("zaplex-live-sftp-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&case_root).unwrap();
    let remote_target = case_root.join("target.txt");
    fs::write(&remote_target, b"old").unwrap();
    let local = tempfile::tempdir().unwrap();
    let local_source = local.path().join("source.txt");
    fs::write(&local_source, b"new").unwrap();

    let session = SftpSession::connect(
        &host,
        port,
        &username,
        AuthMethod::PublicKey {
            key_path,
            passphrase: None,
        },
        Some(Duration::from_secs(10)),
    )
    .expect("the CI OpenSSH fixture must accept the configured key");
    let sftp = session.sftp().expect("the live SFTP subsystem must open");

    upload_file_streaming(
        &sftp,
        &local_source,
        &remote_target,
        None,
        &AtomicBool::new(false),
    )
    .expect("an existing file on OpenSSH SFTP v3 must be replaced");

    assert_eq!(fs::read(&remote_target).unwrap(), b"new");
    assert!(fs::read_dir(&case_root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".sftp_partial-")
    }));
    drop(sftp);
    drop(session);
    fs::remove_dir_all(case_root).unwrap();
}

#[cfg(unix)]
#[test]
#[ignore = "requires the isolated OpenSSH fixture from live-sftp-safety.yml"]
fn no_replace_existing_file_on_openssh_v3_keeps_target() {
    let (host, port, username, key_path, root) = live_sftp_configuration();
    let case_root = root.join(format!("zaplex-live-sftp-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&case_root).unwrap();
    let remote_target = case_root.join("target.txt");
    fs::write(&remote_target, b"old").unwrap();
    let local = tempfile::tempdir().unwrap();
    let local_source = local.path().join("source.txt");
    fs::write(&local_source, b"new").unwrap();

    let session = SftpSession::connect(
        &host,
        port,
        &username,
        AuthMethod::PublicKey {
            key_path,
            passphrase: None,
        },
        Some(Duration::from_secs(10)),
    )
    .expect("the CI OpenSSH fixture must accept the configured key");
    let sftp = session.sftp().expect("the live SFTP subsystem must open");

    let error = upload_file_streaming_no_replace(
        &sftp,
        &local_source,
        &remote_target,
        None,
        &AtomicBool::new(false),
    )
    .expect_err("a no-replace upload must reject an existing destination");

    assert!(matches!(error, SftpOpsError::Operation(_)));
    assert_eq!(fs::read(&remote_target).unwrap(), b"old");
    assert!(fs::read_dir(&case_root).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".sftp_partial-")
    }));
    drop(sftp);
    drop(session);
    fs::remove_dir_all(case_root).unwrap();
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

#[test]
fn secure_transfer_capability_error_has_actionable_localized_message() {
    crate::i18n::init(Some("en"));
    let error = SftpOpsError::CapabilityRequired("not negotiated".to_string());

    assert_eq!(
        error.user_message(),
        "Reconnect this server before making secure file changes."
    );
    assert!(!error.user_message().contains("not negotiated"));
}

/// Test SftpOpsError::NoCredentials Display output
#[test]
fn test_sftp_ops_error_display_no_credentials() {
    assert_eq!(
        SftpOpsError::NoCredentials("no key".into()).to_string(),
        "Credentials not found: no key"
    );
}

/// Test conversion from zap_sftp::SftpError to SftpOpsError
#[test]
fn test_sftp_ops_error_from_sftp_error() {
    let sftp_err = zap_sftp::SftpError::General("test error".into());
    let ops_err: SftpOpsError = sftp_err.into();
    assert!(matches!(ops_err, SftpOpsError::Operation(_)));
}

/// Test shellexpand_path with tilde only (no expansion)
#[test]
fn test_shellexpand_path_tilde_only() {
    let result = shellexpand_path("~");
    assert_eq!(result, "~");
}

// ==================== bool_to_rwx tests ====================

/// Test full permissions rwx
#[test]
fn test_bool_to_rwx_all_true() {
    assert_eq!(bool_to_rwx(true, true, true), "rwx");
}

// ==================== SftpOpsError edge case tests ====================

/// Test SftpOpsError::NoCredentials with empty message
#[test]
fn test_sftp_ops_error_no_credentials_empty() {
    assert_eq!(
        SftpOpsError::NoCredentials(String::new()).to_string(),
        "Credentials not found: "
    );
}
