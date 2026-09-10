use std::sync::atomic::{AtomicUsize, Ordering};

use super::{
    authenticate_after_host_key_check, enforce_host_key_policy, host_key_fingerprint_sha256,
    replace_host_key, HostKeyConfirmation, HostKeyPolicyAction,
};
use crate::SftpError;
use ssh2::{CheckResult, HostKeyType, KnownHostFileKind};

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
    let action = enforce_host_key_policy(
        CheckResult::NotFound,
        "sftp.example",
        2222,
        "SHA256:approved".to_string(),
        "ED25519".to_string(),
        Some(&confirmation),
    )
    .unwrap();
    assert_eq!(action, HostKeyPolicyAction::Persist);

    let result = enforce_host_key_policy(
        CheckResult::NotFound,
        "sftp.example",
        22,
        "SHA256:approved".to_string(),
        "ED25519".to_string(),
        Some(&confirmation),
    );
    assert!(matches!(result, Err(SftpError::HostKeyMismatch { .. })));
}

#[test]
fn changed_host_key_requires_replacement_confirmation_for_exact_fingerprint() {
    let ordinary_confirmation =
        HostKeyConfirmation::new("sftp.example".to_string(), 2222, "SHA256:new".to_string());
    let wrong_confirmation = HostKeyConfirmation::replacement(
        "sftp.example".to_string(),
        2222,
        "SHA256:other".to_string(),
    );
    let wrong_endpoint = HostKeyConfirmation::replacement(
        "other.example".to_string(),
        2222,
        "SHA256:new".to_string(),
    );
    let replacement = HostKeyConfirmation::replacement(
        "sftp.example".to_string(),
        2222,
        "SHA256:new".to_string(),
    );

    for confirmation in [
        None,
        Some(&ordinary_confirmation),
        Some(&wrong_confirmation),
        Some(&wrong_endpoint),
    ] {
        let result = enforce_host_key_policy(
            CheckResult::Mismatch,
            "sftp.example",
            2222,
            "SHA256:new".to_string(),
            "ED25519".to_string(),
            confirmation,
        );
        assert!(matches!(result, Err(SftpError::HostKeyMismatch { .. })));
    }

    assert_eq!(
        enforce_host_key_policy(
            CheckResult::Mismatch,
            "sftp.example",
            2222,
            "SHA256:new".to_string(),
            "ED25519".to_string(),
            Some(&replacement),
        )
        .unwrap(),
        HostKeyPolicyAction::Replace
    );
}

#[test]
fn replacement_rewrites_only_the_confirmed_endpoint() {
    let session = ssh2::Session::new().unwrap();
    let mut known_hosts = session.known_hosts().unwrap();
    let old_key = b"old-server-key";
    let other_key = b"other-server-key";
    let new_key = b"new-server-key";
    known_hosts
        .add("sftp.example", old_key, "test", HostKeyType::Ed25519.into())
        .unwrap();
    known_hosts
        .add(
            "other.example",
            other_key,
            "test",
            HostKeyType::Ed25519.into(),
        )
        .unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("known_hosts");

    replace_host_key(
        &mut known_hosts,
        &path,
        "sftp.example",
        22,
        new_key,
        HostKeyType::Ed25519,
    )
    .unwrap();

    let mut persisted = session.known_hosts().unwrap();
    persisted
        .read_file(&path, KnownHostFileKind::OpenSSH)
        .unwrap();
    assert_eq!(
        persisted.check_port("sftp.example", 22, new_key),
        CheckResult::Match
    );
    assert_eq!(
        persisted.check_port("sftp.example", 22, old_key),
        CheckResult::Mismatch
    );
    assert_eq!(
        persisted.check_port("other.example", 22, other_key),
        CheckResult::Match
    );
}
