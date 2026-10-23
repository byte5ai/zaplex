use super::*;

#[cfg(unix)]
fn resolve_fixture_alias(
    config: &Path,
    server: &SshServerInfo,
    username: &str,
) -> Result<ResolvedSftpEndpoint, SftpOpsError> {
    let mut command = command::r#async::Command::new("ssh");
    command.arg("-F").arg(config);
    resolve_sftp_endpoint_with_command(server, username, command, Duration::from_secs(5))
}

#[cfg(unix)]
#[test]
fn sftp_alias_uses_openssh_include_and_match_without_changing_saved_host() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    let included = directory.path().join("included");
    fs::write(&config, format!("Include {}\n", included.display())).unwrap();
    fs::write(
        included,
        "Host zaplex-sftp-alias.invalid\n  HostName 127.0.0.1\n  Port 2207\n\
         Match originalhost zaplex-sftp-alias.invalid\n  User fixture-user\n",
    )
    .unwrap();
    let mut server = SshServerInfo::new_default("fixture".into());
    server.host = "zaplex-sftp-alias.invalid".into();

    // No network is used: this is OpenSSH's actual configuration evaluation,
    // including constructs the one-time import parser intentionally omits.
    let endpoint = resolve_fixture_alias(&config, &server, "").unwrap();
    assert_eq!(endpoint.host, "127.0.0.1");
    assert_eq!(endpoint.port, 2207);
    assert_eq!(endpoint.username, "fixture-user");
    assert_eq!(server.host, "zaplex-sftp-alias.invalid");
}

#[cfg(unix)]
#[test]
fn sftp_alias_keeps_credential_user_and_explicit_port_precedence() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    fs::write(
        &config,
        "Host fixture.invalid\n  HostName 127.0.0.1\n  Port 2207\n  User config-user\n",
    )
    .unwrap();
    let mut server = SshServerInfo::new_default("fixture".into());
    server.host = "fixture.invalid".into();
    server.port = 2208;
    server.username = "saved-user".into();
    let endpoint = resolve_fixture_alias(&config, &server, "credential-user").unwrap();
    assert_eq!(endpoint.username, "credential-user");
    assert_eq!(endpoint.port, 2208);
    assert_eq!(server.username, "saved-user");
}

#[cfg(unix)]
#[test]
fn sftp_alias_invalid_config_does_not_fall_back_to_direct_dns() {
    let directory = tempfile::tempdir().unwrap();
    let config = directory.path().join("config");
    fs::write(&config, "Host fixture.invalid\n  UnknownZaplexOption yes\n").unwrap();
    let mut server = SshServerInfo::new_default("fixture".into());
    server.host = "fixture.invalid".into();
    assert!(matches!(
        resolve_fixture_alias(&config, &server, "fixture-user"),
        Err(SftpOpsError::Connection(_))
    ));
}

#[test]
fn sftp_alias_never_bypasses_proxy_routes_or_alternate_trust_names() {
    let direct = "hostname destination.example\nport 22\nuser alice\n";
    for option in [
        "proxyjump jump.example",
        "proxycommand ssh -W %h:%p jump",
        "hostkeyalias pinned-name",
    ] {
        assert!(matches!(
            parse_sftp_endpoint(&format!("{direct}{option}\n")),
            Err(SftpOpsError::Connection(_))
        ));
    }
    assert_eq!(
        parse_sftp_endpoint(&format!("{direct}proxycommand none\n")).unwrap(),
        ResolvedSftpEndpoint {
            host: "destination.example".into(),
            port: 22,
            username: "alice".into(),
        }
    );
}

#[test]
fn sftp_alias_rejects_incomplete_or_invalid_config_output() {
    for output in [
        "",
        "hostname destination.example\nport 22\n",
        "hostname destination.example\nport 0\nuser alice\n",
        "hostname -oProxyCommand=bad\nport 22\nuser alice\n",
    ] {
        assert!(matches!(
            parse_sftp_endpoint(output),
            Err(SftpOpsError::Connection(_))
        ));
    }
}

#[cfg(unix)]
#[test]
fn sftp_alias_lookup_has_a_bounded_timeout() {
    let mut server = SshServerInfo::new_default("fixture".into());
    server.host = "fixture.invalid".into();
    let mut command = command::r#async::Command::new("sh");
    command.args(["-c", "exec sleep 30"]);
    let started = std::time::Instant::now();
    assert!(matches!(
        resolve_sftp_endpoint_with_command(&server, "alice", command, Duration::from_millis(20)),
        Err(SftpOpsError::Connection(message)) if message.contains("timed out")
    ));
    assert!(started.elapsed() < Duration::from_secs(5));
}

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
