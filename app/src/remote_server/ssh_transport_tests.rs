use super::*;
use warpui::r#async::BoxFuture;

fn static_auth_context() -> Arc<RemoteServerAuthContext> {
    Arc::new(RemoteServerAuthContext::new(
        || -> BoxFuture<'static, Option<String>> { Box::pin(async { None }) },
        || "user id/with spaces".to_string(),
    ))
}

#[test]
fn remote_proxy_command_quotes_identity_key() {
    let transport = SshTransport::new(
        PathBuf::from("/tmp/control-master.sock"),
        static_auth_context(),
    );

    let command = transport.remote_proxy_command();

    assert!(command.contains("remote-server-proxy --identity-key"));
    assert!(command.contains("'user id/with spaces'"));
}

#[test]
fn downloaded_remote_server_tarball_rejects_digest_mismatch() {
    let tempdir = tempfile::tempdir().unwrap();
    let archive_path = tempdir.path().join("zap-remote-server-linux-x86_64.tar.gz");
    std::fs::write(&archive_path, b"manipulated archive bytes").unwrap();
    let expected_sha256 = format!("{:x}", Sha256::digest(b"different archive bytes"));

    let error = verify_remote_server_tarball(&archive_path, &expected_sha256).unwrap_err();

    assert!(error.to_string().contains("SHA-256 mismatch"));
}

#[test]
fn downloaded_remote_server_tarball_accepts_matching_digest() {
    let tempdir = tempfile::tempdir().unwrap();
    let archive_path = tempdir.path().join("zap-remote-server-linux-x86_64.tar.gz");
    let archive_bytes = b"authenticated archive bytes";
    std::fs::write(&archive_path, archive_bytes).unwrap();
    let expected_sha256 = format!("{:x}", Sha256::digest(archive_bytes));

    verify_remote_server_tarball(&archive_path, &expected_sha256).unwrap();
}
