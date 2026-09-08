use std::sync::atomic::{AtomicUsize, Ordering};

use super::{
    authenticate_after_host_key_check, enforce_host_key_policy, host_key_fingerprint_sha256,
    HostKeyConfirmation,
};
use crate::SftpError;
use ssh2::CheckResult;

#[test]
fn host_key_mismatch_prevents_password_authentication() {
    let authentication_attempts = AtomicUsize::new(0);
    let result = authenticate_after_host_key_check(
        Err(SftpError::HostKeyMismatch {
            fingerprint_sha256: "SHA256:server-key-b".to_string(),
            key_type: "ED25519".to_string(),
        }),
        || {
            authentication_attempts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    );

    assert!(matches!(result, Err(SftpError::HostKeyMismatch { .. })));
    assert_eq!(authentication_attempts.load(Ordering::SeqCst), 0);
}

#[test]
fn sha256_fingerprint_uses_openssh_format_without_padding() {
    assert_eq!(
        host_key_fingerprint_sha256(b"server-host-key"),
        "SHA256:cdYRWX5O9rfSI+w3fDBD3cHZZrl3DK758pQGI+bm9o4"
    );
}

#[test]
fn confirmation_is_bound_to_host_port_and_retry_fingerprint() {
    let confirmation = HostKeyConfirmation::new(
        "sftp.example".to_string(),
        2222,
        "SHA256:approved".to_string(),
    );
    let persisted = AtomicUsize::new(0);

    enforce_host_key_policy(
        CheckResult::NotFound,
        "sftp.example",
        2222,
        "SHA256:approved".to_string(),
        "ED25519".to_string(),
        Some(&confirmation),
        || {
            persisted.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(persisted.load(Ordering::SeqCst), 1);

    let result = enforce_host_key_policy(
        CheckResult::NotFound,
        "sftp.example",
        22,
        "SHA256:approved".to_string(),
        "ED25519".to_string(),
        Some(&confirmation),
        || {
            persisted.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    );
    assert!(matches!(result, Err(SftpError::HostKeyMismatch { .. })));
    assert_eq!(persisted.load(Ordering::SeqCst), 1);
}
