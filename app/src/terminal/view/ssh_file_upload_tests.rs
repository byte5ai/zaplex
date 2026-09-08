use crate::terminal::ssh::util::{transfer_file_sftp_command, SftpUploadError, SftpUploadPlan};
use warp_util::path::ShellFamily;

#[cfg(all(unix, feature = "local_tty"))]
#[test]
fn remote_pwd_is_not_evaluated_by_local_shell() {
    use std::os::unix::fs::PermissionsExt as _;

    let temp_dir = tempfile::tempdir().unwrap();
    let fake_sftp = temp_dir.path().join("sftp");
    let capture = temp_dir.path().join("batch.txt");
    let marker = temp_dir.path().join("injected.txt");
    let local_file = temp_dir.path().join("local file.txt");
    std::fs::write(&local_file, b"payload").unwrap();
    std::fs::write(&fake_sftp, "#!/bin/sh\n/bin/cat > \"$CAPTURE\"\n").unwrap();
    std::fs::set_permissions(&fake_sftp, std::fs::Permissions::from_mode(0o700)).unwrap();

    let remote_pwd = "/remote/$(printf injected > \"$MARKER\")".to_string();
    let plan = transfer_file_sftp_command(
        local_file.to_string_lossy().into_owned(),
        "user@example.test".to_string(),
        None,
        Some(remote_pwd.clone()),
    )
    .unwrap();
    let upload = plan.materialize(ShellFamily::Posix).unwrap();
    let mode = std::fs::metadata(upload.batch_path())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    assert!(!upload.command().contains(&remote_pwd));

    let output = command::blocking::Command::new("/bin/sh")
        .arg("-c")
        .arg(upload.command())
        .env("PATH", temp_dir.path())
        .env("CAPTURE", &capture)
        .env("MARKER", &marker)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fake sftp failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!marker.exists());

    let batch = std::fs::read_to_string(capture).unwrap();
    assert!(batch.contains("$(printf injected > "));
    assert!(batch.contains("\\\"$MARKER\\\""));
}

#[test]
fn sftp_batch_quotes_paths_without_shell_interpolation() {
    let local_path = r#"/local/$(touch marker) `tick` $VAR "quote" \backslash"#.to_string();
    let remote_path = r#"/remote/$HOME `tick` "quote" \backslash"#;
    let plan = SftpUploadPlan::new(
        &[local_path],
        "user@example.test",
        Some("2222"),
        Some(remote_path),
    )
    .unwrap();
    let batch = String::from_utf8(plan.batch().to_vec()).unwrap();
    let upload = plan.materialize(ShellFamily::Posix).unwrap();

    assert_eq!(
        batch,
        "put \"/local/$(touch marker) `tick` $VAR \\\"quote\\\" \\\\backslash\" \"/remote/$HOME `tick` \\\"quote\\\" \\\\backslash\"\n"
    );
    assert!(!upload.command().contains("$(touch marker)"));
    assert!(!upload.command().contains("$HOME"));
}

#[test]
fn sftp_batch_rejects_command_separators() {
    for invalid in ["bad\nnext", "bad\rnext", "bad\0next"] {
        let error = SftpUploadPlan::new(
            &["/local/file".to_string()],
            "user@example.test",
            None,
            Some(invalid),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            SftpUploadError::InvalidBatchPath {
                field: "remote destination"
            }
        ));

        let error = SftpUploadPlan::new(
            &[invalid.to_string()],
            "user@example.test",
            None,
            Some("/remote"),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            SftpUploadError::InvalidBatchPath {
                field: "local path"
            }
        ));
    }
}

#[test]
fn sftp_argv_rejects_option_injection_and_invalid_ports() {
    let paths = ["/local/file".to_string()];
    assert!(matches!(
        SftpUploadPlan::new(&paths, "-oProxyCommand=payload", None, Some("/remote")),
        Err(SftpUploadError::InvalidHost)
    ));
    for port in ["0", "65536", "22; payload"] {
        assert!(matches!(
            SftpUploadPlan::new(&paths, "user@example.test", Some(port), Some("/remote")),
            Err(SftpUploadError::InvalidPort)
        ));
    }
}
