use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Barrier;
use std::thread;

use base64::Engine as _;

use super::{
    authenticate_after_host_key_check, check_known_host_key, enforce_host_key_policy,
    host_key_fingerprint_sha256, load_known_hosts, matching_known_host_keys, persist_host_key,
    preferred_host_key_algorithms, replace_host_key, HostKeyConfirmation, HostKeyPolicyAction,
};
use crate::SftpError;
use ssh2::{CheckResult, HostKeyType, KnownHostFileKind, MethodType, Session};

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

// These fixtures use SSH's length-prefixed algorithm field and a synthetic key payload.
// No private keys or live servers are needed for known_hosts matching.
fn synthetic_host_key(algorithm: &str, marker: u8) -> Vec<u8> {
    let mut key = (algorithm.len() as u32).to_be_bytes().to_vec();
    key.extend_from_slice(algorithm.as_bytes());
    key.extend_from_slice(&32_u32.to_be_bytes());
    key.extend_from_slice(&[marker; 32]);
    key
}

fn known_hosts_line(hosts: &str, algorithm: &str, key: &[u8]) -> String {
    let encoded_key = base64::engine::general_purpose::STANDARD.encode(key);
    format!("{hosts} {algorithm} {encoded_key} fixture\n")
}

// OpenSSH HMAC-SHA1 host hashes with a deterministic, non-secret 20-byte salt.
const HASHED_TARGET: &str = "|1|AAECAwQFBgcICQoLDA0ODxAREhM=|SiswnR5Nm1AN9bI2Sx08M5CihcE=";
const HASHED_OTHER: &str = "|1|AAECAwQFBgcICQoLDA0ODxAREhM=|P8bgQeODILYY3OTgSIVr03wmvxA=";

#[test]
fn trusted_ed25519_key_is_preferred_over_default_ecdsa() {
    let session = Session::new().unwrap();
    let mut known_hosts = session.known_hosts().unwrap();
    let key = synthetic_host_key("ssh-ed25519", 1);
    known_hosts
        .add("sftp.example", &key, "test", HostKeyType::Ed25519.into())
        .unwrap();

    let preferred = preferred_host_key_algorithms(
        &session,
        &known_hosts_text(&known_hosts),
        "sftp.example",
        22,
    )
    .unwrap();
    let ed25519 = preferred
        .iter()
        .position(|method| *method == "ssh-ed25519")
        .unwrap();
    let ecdsa = preferred
        .iter()
        .position(|method| *method == "ecdsa-sha2-nistp256")
        .unwrap();
    assert!(
        ed25519 < ecdsa,
        "trusted key must precede unpinned algorithms: {preferred:?}"
    );
    assert!(matches!(
        check_known_host_key(
            &session,
            &known_hosts_text(&known_hosts),
            "sftp.example",
            22,
            &key
        )
        .unwrap(),
        CheckResult::Match
    ));
}

#[test]
fn trusted_rsa_key_preserves_sha2_signature_preference() {
    let session = Session::new().unwrap();
    let mut known_hosts = session.known_hosts().unwrap();
    known_hosts
        .add(
            "sftp.example",
            &synthetic_host_key("ssh-rsa", 1),
            "test",
            HostKeyType::Rsa.into(),
        )
        .unwrap();
    let expected: Vec<_> = session
        .supported_algs(MethodType::HostKey)
        .unwrap()
        .into_iter()
        .filter(|method| matches!(*method, "rsa-sha2-512" | "rsa-sha2-256" | "ssh-rsa"))
        .collect();
    assert_eq!(expected.first(), Some(&"rsa-sha2-512"));

    let preferred = preferred_host_key_algorithms(
        &session,
        &known_hosts_text(&known_hosts),
        "sftp.example",
        22,
    )
    .unwrap();
    assert_eq!(&preferred[..expected.len()], expected.as_slice());
}

#[test]
fn unrelated_hosts_do_not_change_algorithm_preferences() {
    let session = Session::new().unwrap();
    let mut known_hosts = session.known_hosts().unwrap();
    known_hosts
        .add(
            "other.example",
            &synthetic_host_key("ssh-ed25519", 1),
            "test",
            HostKeyType::Ed25519.into(),
        )
        .unwrap();
    assert_eq!(
        preferred_host_key_algorithms(
            &session,
            &known_hosts_text(&known_hosts),
            "sftp.example",
            22
        )
        .unwrap(),
        session.supported_algs(MethodType::HostKey).unwrap()
    );
}

#[test]
fn changed_key_of_the_same_algorithm_remains_a_mismatch() {
    let session = Session::new().unwrap();
    let mut known_hosts = session.known_hosts().unwrap();
    known_hosts
        .add(
            "sftp.example",
            &synthetic_host_key("ssh-ed25519", 1),
            "test",
            HostKeyType::Ed25519.into(),
        )
        .unwrap();
    assert!(matches!(
        check_known_host_key(
            &session,
            &known_hosts_text(&known_hosts),
            "sftp.example",
            22,
            &synthetic_host_key("ssh-ed25519", 2),
        )
        .unwrap(),
        CheckResult::Mismatch
    ));
}

#[test]
fn unpinned_algorithm_is_unknown_and_cannot_authenticate_without_confirmation() {
    let session = Session::new().unwrap();
    let mut known_hosts = session.known_hosts().unwrap();
    known_hosts
        .add(
            "sftp.example",
            &synthetic_host_key("ssh-ed25519", 1),
            "test",
            HostKeyType::Ed25519.into(),
        )
        .unwrap();
    let presented = synthetic_host_key("ecdsa-sha2-nistp256", 2);
    let checked = check_known_host_key(
        &session,
        &known_hosts_text(&known_hosts),
        "sftp.example",
        22,
        &presented,
    )
    .unwrap();
    assert!(matches!(checked, CheckResult::NotFound));

    let authentication_attempts = AtomicUsize::new(0);
    let policy = enforce_host_key_policy(
        checked,
        "sftp.example",
        22,
        host_key_fingerprint_sha256(&presented),
        "ECDSA P-256".to_string(),
        None,
    );
    let result = authenticate_after_host_key_check(policy.map(|_| ()), || {
        authentication_attempts.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });
    assert!(matches!(result, Err(SftpError::UnknownHostKey { .. })));
    assert_eq!(authentication_attempts.load(Ordering::SeqCst), 0);
}

#[test]
fn custom_port_does_not_reuse_the_default_port_key_or_preferences() {
    let session = Session::new().unwrap();
    let mut known_hosts = session.known_hosts().unwrap();
    let key = synthetic_host_key("ssh-ed25519", 1);
    known_hosts
        .add("sftp.example", &key, "test", HostKeyType::Ed25519.into())
        .unwrap();
    assert!(matches!(
        check_known_host_key(
            &session,
            &known_hosts_text(&known_hosts),
            "sftp.example",
            2222,
            &key
        )
        .unwrap(),
        CheckResult::NotFound
    ));
    assert_eq!(
        preferred_host_key_algorithms(
            &session,
            &known_hosts_text(&known_hosts),
            "sftp.example",
            2222
        )
        .unwrap(),
        session.supported_algs(MethodType::HostKey).unwrap()
    );
}

#[test]
fn configured_custom_port_uses_only_its_own_pin() {
    let session = Session::new().unwrap();
    let mut known_hosts = session.known_hosts().unwrap();
    let default_key = synthetic_host_key("ssh-ed25519", 1);
    let custom_key = synthetic_host_key("ssh-ed25519", 2);
    known_hosts
        .add(
            "sftp.example",
            &default_key,
            "test",
            HostKeyType::Ed25519.into(),
        )
        .unwrap();
    known_hosts
        .add(
            "[sftp.example]:2222",
            &custom_key,
            "test",
            HostKeyType::Ed25519.into(),
        )
        .unwrap();
    assert!(matches!(
        check_known_host_key(
            &session,
            &known_hosts_text(&known_hosts),
            "sftp.example",
            2222,
            &custom_key
        )
        .unwrap(),
        CheckResult::Match
    ));
    assert!(matches!(
        check_known_host_key(
            &session,
            &known_hosts_text(&known_hosts),
            "sftp.example",
            2222,
            &default_key
        )
        .unwrap(),
        CheckResult::Mismatch
    ));
}

#[test]
fn hashed_hosts_use_only_the_requested_endpoints_algorithm() {
    let session = Session::new().unwrap();
    let mut known_hosts = session.known_hosts().unwrap();
    let target_key = synthetic_host_key("ssh-ed25519", 1);
    let other_key = synthetic_host_key("ecdsa-sha2-nistp256", 2);
    for line in [
        known_hosts_line(HASHED_OTHER, "ecdsa-sha2-nistp256", &other_key),
        known_hosts_line(HASHED_TARGET, "ssh-ed25519", &target_key),
    ] {
        known_hosts
            .read_str(&line, KnownHostFileKind::OpenSSH)
            .unwrap();
    }
    let preferred = preferred_host_key_algorithms(
        &session,
        &known_hosts_text(&known_hosts),
        "sftp.example",
        22,
    )
    .unwrap();
    assert_eq!(preferred.first(), Some(&"ssh-ed25519"));
    assert!(matches!(
        check_known_host_key(
            &session,
            &known_hosts_text(&known_hosts),
            "sftp.example",
            22,
            &target_key
        )
        .unwrap(),
        CheckResult::Match
    ));
    assert!(matches!(
        check_known_host_key(
            &session,
            &known_hosts_text(&known_hosts),
            "sftp.example",
            22,
            &other_key
        )
        .unwrap(),
        CheckResult::NotFound
    ));
}

#[test]
fn replacement_rewrites_only_the_confirmed_endpoint() {
    let session = Session::new().unwrap();
    let mut known_hosts = session.known_hosts().unwrap();
    let old_key = synthetic_host_key("ssh-ed25519", 1);
    let other_key = synthetic_host_key("ssh-ed25519", 2);
    let new_key = synthetic_host_key("ssh-ed25519", 3);
    known_hosts
        .add(
            "sftp.example",
            &old_key,
            "test",
            HostKeyType::Ed25519.into(),
        )
        .unwrap();
    known_hosts
        .add(
            "other.example",
            &other_key,
            "test",
            HostKeyType::Ed25519.into(),
        )
        .unwrap();
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("known_hosts");
    known_hosts
        .write_file(&path, KnownHostFileKind::OpenSSH)
        .unwrap();

    replace_host_key(
        &session,
        &path,
        "sftp.example",
        22,
        &new_key,
        HostKeyType::Ed25519,
    )
    .unwrap();

    let mut persisted = session.known_hosts().unwrap();
    persisted
        .read_file(&path, KnownHostFileKind::OpenSSH)
        .unwrap();
    assert!(matches!(
        persisted.check("sftp.example", &new_key),
        CheckResult::Match
    ));
    assert!(matches!(
        persisted.check("sftp.example", &old_key),
        CheckResult::Mismatch
    ));
    assert!(matches!(
        persisted.check("other.example", &other_key),
        CheckResult::Match
    ));
}

#[test]
fn replacement_preserves_other_hosts_with_the_same_key() {
    let session = Session::new().unwrap();
    let old_key = synthetic_host_key("ssh-ed25519", 1);
    let new_key = synthetic_host_key("ssh-ed25519", 2);
    let unrelated_line = known_hosts_line("other.example", "ssh-ed25519", &old_key);
    let target_line = known_hosts_line("sftp.example", "ssh-ed25519", &old_key);
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("known_hosts");
    fs::write(&path, format!("{unrelated_line}{target_line}")).unwrap();

    replace_host_key(
        &session,
        &path,
        "sftp.example",
        22,
        &new_key,
        HostKeyType::Ed25519,
    )
    .unwrap();

    assert!(fs::read_to_string(&path).unwrap().contains(&unrelated_line));
    let mut persisted = session.known_hosts().unwrap();
    persisted
        .read_file(&path, KnownHostFileKind::OpenSSH)
        .unwrap();
    assert!(matches!(
        persisted.check("other.example", &old_key),
        CheckResult::Match
    ));
    assert!(matches!(
        persisted.check("sftp.example", &new_key),
        CheckResult::Match
    ));
    assert!(matches!(
        persisted.check("sftp.example", &old_key),
        CheckResult::Mismatch
    ));
}

#[test]
fn replacement_preserves_distinct_hashed_hosts_with_the_same_key() {
    let session = Session::new().unwrap();
    let old_key = synthetic_host_key("ssh-ed25519", 1);
    let new_key = synthetic_host_key("ssh-ed25519", 2);
    let unrelated_line = known_hosts_line(HASHED_OTHER, "ssh-ed25519", &old_key);
    let target_line = known_hosts_line(HASHED_TARGET, "ssh-ed25519", &old_key);
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("known_hosts");
    fs::write(&path, format!("{unrelated_line}{target_line}")).unwrap();

    replace_host_key(
        &session,
        &path,
        "sftp.example",
        22,
        &new_key,
        HostKeyType::Ed25519,
    )
    .unwrap();

    assert!(fs::read_to_string(&path).unwrap().contains(&unrelated_line));
    let mut persisted = session.known_hosts().unwrap();
    persisted
        .read_file(&path, KnownHostFileKind::OpenSSH)
        .unwrap();
    assert!(matches!(
        persisted.check("other.example", &old_key),
        CheckResult::Match
    ));
    assert!(matches!(
        persisted.check("sftp.example", &new_key),
        CheckResult::Match
    ));
    assert!(matches!(
        persisted.check("sftp.example", &old_key),
        CheckResult::Mismatch
    ));
}

#[test]
fn replacement_preserves_aliases_other_algorithms_ports_and_comments() {
    let session = Session::new().unwrap();
    let old_key = synthetic_host_key("ssh-ed25519", 1);
    let new_key = synthetic_host_key("ssh-ed25519", 2);
    let ecdsa_key = synthetic_host_key("ecdsa-sha2-nistp256", 3);
    let aliases_line = known_hosts_line("sftp.example,alias.example", "ssh-ed25519", &old_key);
    let ecdsa_line = known_hosts_line("sftp.example", "ecdsa-sha2-nistp256", &ecdsa_key);
    let other_port_line = known_hosts_line("[sftp.example]:2222", "ssh-ed25519", &old_key);
    let comment = "# Preserve this OpenSSH configuration comment.\n";
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("known_hosts");
    fs::write(
        &path,
        format!("{comment}{aliases_line}{ecdsa_line}{other_port_line}"),
    )
    .unwrap();

    replace_host_key(
        &session,
        &path,
        "sftp.example",
        22,
        &new_key,
        HostKeyType::Ed25519,
    )
    .unwrap();

    let contents = fs::read_to_string(&path).unwrap();
    for preserved in [comment, ecdsa_line.as_str(), other_port_line.as_str()] {
        assert!(
            contents.contains(preserved),
            "missing preserved line: {preserved}"
        );
    }
    let mut persisted = session.known_hosts().unwrap();
    persisted
        .read_file(&path, KnownHostFileKind::OpenSSH)
        .unwrap();
    assert!(matches!(
        persisted.check("alias.example", &old_key),
        CheckResult::Match
    ));
    assert!(matches!(
        persisted.check("sftp.example", &ecdsa_key),
        CheckResult::Match
    ));
    assert!(matches!(
        persisted.check("[sftp.example]:2222", &old_key),
        CheckResult::Match
    ));
    assert!(matches!(
        persisted.check("sftp.example", &new_key),
        CheckResult::Match
    ));
    assert!(matches!(
        persisted.check("sftp.example", &old_key),
        CheckResult::Mismatch
    ));
}

#[test]
fn persist_preserves_existing_openssh_lines_and_adds_only_the_confirmed_key() {
    let session = Session::new().unwrap();
    let ed25519 = synthetic_host_key("ssh-ed25519", 1);
    let ecdsa = synthetic_host_key("ecdsa-sha2-nistp256", 2);
    let alias_line = known_hosts_line("sftp.example,alias.example", "ssh-ed25519", &ed25519);
    let hashed_line = known_hosts_line(HASHED_OTHER, "ssh-ed25519", &ed25519);
    let authority_line = known_hosts_line("@cert-authority *.example", "ssh-ed25519", &ed25519);
    let revoked_line = known_hosts_line("@revoked old.example", "ssh-ed25519", &ed25519);
    let original = format!(
        "# Existing trust configuration.\r\n\n{authority_line}{revoked_line}{alias_line}{hashed_line}"
    );
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("known_hosts");
    fs::write(&path, &original).unwrap();

    persist_host_key(
        &session,
        &path,
        "sftp.example",
        22,
        &ecdsa,
        HostKeyType::Ecdsa256,
    )
    .unwrap();

    let contents = fs::read_to_string(&path).unwrap();
    let appended = contents
        .strip_prefix(&original)
        .expect("existing OpenSSH configuration must be preserved byte-for-byte");
    assert_eq!(appended.lines().count(), 1);
    let mut new_entry = session.known_hosts().unwrap();
    new_entry
        .read_str(appended, KnownHostFileKind::OpenSSH)
        .unwrap();
    assert!(matches!(
        new_entry.check("sftp.example", &ecdsa),
        CheckResult::Match
    ));
}

#[test]
fn persist_separates_an_existing_final_line_without_a_newline() {
    let session = Session::new().unwrap();
    let old_key = synthetic_host_key("ssh-ed25519", 1);
    let new_key = synthetic_host_key("ssh-ed25519", 2);
    let original = known_hosts_line("other.example", "ssh-ed25519", &old_key);
    let original = original.trim_end_matches('\n');
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("known_hosts");
    fs::write(&path, original).unwrap();

    persist_host_key(
        &session,
        &path,
        "sftp.example",
        22,
        &new_key,
        HostKeyType::Ed25519,
    )
    .unwrap();

    assert!(fs::read_to_string(&path)
        .unwrap()
        .starts_with(&format!("{original}\n")));
    let mut persisted = session.known_hosts().unwrap();
    persisted
        .read_file(&path, KnownHostFileKind::OpenSSH)
        .unwrap();
    assert!(matches!(
        persisted.check("other.example", &old_key),
        CheckResult::Match
    ));
    assert!(matches!(
        persisted.check("sftp.example", &new_key),
        CheckResult::Match
    ));
}

#[test]
fn persist_creates_missing_parent_directory_and_known_hosts_file() {
    let session = Session::new().unwrap();
    let key = synthetic_host_key("ssh-ed25519", 1);
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("ssh").join("known_hosts");
    assert!(!path.parent().unwrap().exists());

    persist_host_key(
        &session,
        &path,
        "sftp.example",
        2222,
        &key,
        HostKeyType::Ed25519,
    )
    .unwrap();

    #[cfg(unix)]
    assert_eq!(
        fs::metadata(path.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );

    let mut persisted = session.known_hosts().unwrap();
    persisted
        .read_file(&path, KnownHostFileKind::OpenSSH)
        .unwrap();
    assert!(matches!(
        persisted.check("[sftp.example]:2222", &key),
        CheckResult::Match
    ));
    assert!(matches!(
        persisted.check("sftp.example", &key),
        CheckResult::NotFound
    ));
}

#[cfg(unix)]
#[test]
fn persist_and_replace_preserve_existing_file_permissions() {
    let session = Session::new().unwrap();
    let old_key = synthetic_host_key("ssh-ed25519", 1);
    let new_key = synthetic_host_key("ssh-ed25519", 2);
    let ecdsa = synthetic_host_key("ecdsa-sha2-nistp256", 3);
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("known_hosts");
    fs::write(
        &path,
        known_hosts_line("sftp.example", "ssh-ed25519", &old_key),
    )
    .unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();

    persist_host_key(
        &session,
        &path,
        "sftp.example",
        22,
        &ecdsa,
        HostKeyType::Ecdsa256,
    )
    .unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );

    replace_host_key(
        &session,
        &path,
        "sftp.example",
        22,
        &new_key,
        HostKeyType::Ed25519,
    )
    .unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
}

#[cfg(unix)]
#[test]
fn persist_and_replace_retain_relative_symlink_and_update_its_target() {
    let session = Session::new().unwrap();
    let old_key = synthetic_host_key("ssh-ed25519", 1);
    let new_key = synthetic_host_key("ssh-ed25519", 2);
    let ecdsa = synthetic_host_key("ecdsa-sha2-nistp256", 3);
    let temp_dir = tempfile::tempdir().unwrap();
    let target = temp_dir.path().join("shared_known_hosts");
    let path = temp_dir.path().join("ssh").join("known_hosts");
    fs::create_dir(path.parent().unwrap()).unwrap();
    fs::write(
        &target,
        known_hosts_line("sftp.example", "ssh-ed25519", &old_key),
    )
    .unwrap();
    symlink("../shared_known_hosts", &path).unwrap();

    persist_host_key(
        &session,
        &path,
        "sftp.example",
        22,
        &ecdsa,
        HostKeyType::Ecdsa256,
    )
    .unwrap();
    assert!(fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_link(&path).unwrap().to_str().unwrap(),
        "../shared_known_hosts"
    );
    let mut persisted = session.known_hosts().unwrap();
    persisted
        .read_file(&target, KnownHostFileKind::OpenSSH)
        .unwrap();
    assert!(matches!(
        persisted.check("sftp.example", &ecdsa),
        CheckResult::Match
    ));
    assert!(matches!(
        persisted.check("sftp.example", &old_key),
        CheckResult::Match
    ));

    replace_host_key(
        &session,
        &path,
        "sftp.example",
        22,
        &new_key,
        HostKeyType::Ed25519,
    )
    .unwrap();
    assert!(fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_link(&path).unwrap().to_str().unwrap(),
        "../shared_known_hosts"
    );
    let mut replaced = session.known_hosts().unwrap();
    replaced
        .read_file(&target, KnownHostFileKind::OpenSSH)
        .unwrap();
    assert!(matches!(
        replaced.check("sftp.example", &new_key),
        CheckResult::Match
    ));
    assert!(matches!(
        replaced.check("sftp.example", &old_key),
        CheckResult::Mismatch
    ));
    assert!(matches!(
        replaced.check("sftp.example", &ecdsa),
        CheckResult::Match
    ));
}

#[cfg(unix)]
#[test]
fn persist_rejects_a_dangling_symlink_without_replacing_it() {
    let session = Session::new().unwrap();
    let key = synthetic_host_key("ssh-ed25519", 1);
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("known_hosts");
    let missing_target = temp_dir.path().join("missing_known_hosts");
    symlink("missing_known_hosts", &path).unwrap();

    let result = persist_host_key(
        &session,
        &path,
        "sftp.example",
        22,
        &key,
        HostKeyType::Ed25519,
    );

    assert!(result.is_err());
    assert!(fs::symlink_metadata(&path)
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_link(&path).unwrap().to_str().unwrap(),
        "missing_known_hosts"
    );
    assert!(!missing_target.exists());
}

#[test]
fn concurrent_persist_retains_all_confirmed_host_keys() {
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("known_hosts");
    let original = "# Existing trust configuration.\n";
    fs::write(&path, original).unwrap();
    let barrier = Barrier::new(8);

    thread::scope(|scope| {
        for index in 0..8 {
            let path = &path;
            let barrier = &barrier;
            scope.spawn(move || {
                let session = Session::new().unwrap();
                let key = synthetic_host_key("ssh-ed25519", index);
                let host = format!("host-{index}.example");
                barrier.wait();
                persist_host_key(&session, path, &host, 22, &key, HostKeyType::Ed25519).unwrap();
            });
        }
    });

    assert!(fs::read_to_string(&path).unwrap().starts_with(original));
    let session = Session::new().unwrap();
    let mut persisted = session.known_hosts().unwrap();
    persisted
        .read_file(&path, KnownHostFileKind::OpenSSH)
        .unwrap();
    for index in 0..8 {
        let key = synthetic_host_key("ssh-ed25519", index);
        let host = format!("host-{index}.example");
        assert!(
            matches!(persisted.check(&host, &key), CheckResult::Match),
            "concurrent persist lost the key for {host}"
        );
    }
}

#[test]
fn replacement_preserves_openssh_markers_and_partial_alias_crlf() {
    let session = Session::new().unwrap();
    let old_key = synthetic_host_key("ssh-ed25519", 1);
    let new_key = synthetic_host_key("ssh-ed25519", 2);
    let authority_line = known_hosts_line("@cert-authority *.example", "ssh-ed25519", &old_key)
        .replace('\n', "\r\n");
    let revoked_line =
        known_hosts_line("@revoked old.example", "ssh-ed25519", &old_key).replace('\n', "\r\n");
    let aliases_line = known_hosts_line("sftp.example,alias.example", "ssh-ed25519", &old_key)
        .replace('\n', "\r\n");
    let retained_alias_line =
        known_hosts_line("alias.example", "ssh-ed25519", &old_key).replace('\n', "\r\n");
    let comment = "# Keep OpenSSH directives and line endings.\r\n\r\n";
    let original = format!("{comment}{authority_line}{revoked_line}{aliases_line}");
    let expected_prefix = format!("{comment}{authority_line}{revoked_line}{retained_alias_line}");
    let temp_dir = tempfile::tempdir().unwrap();
    let path = temp_dir.path().join("known_hosts");
    fs::write(&path, original).unwrap();

    replace_host_key(
        &session,
        &path,
        "sftp.example",
        22,
        &new_key,
        HostKeyType::Ed25519,
    )
    .unwrap();

    let contents = fs::read_to_string(&path).unwrap();
    let appended = contents
        .strip_prefix(&expected_prefix)
        .expect("replacement must preserve markers, CRLF and the unrelated alias");
    let mut replacement = session.known_hosts().unwrap();
    replacement
        .read_str(appended, KnownHostFileKind::OpenSSH)
        .unwrap();
    assert!(matches!(
        replacement.check("sftp.example", &new_key),
        CheckResult::Match
    ));
}

fn known_hosts_text(hosts: &ssh2::KnownHosts) -> String {
    hosts
        .hosts()
        .unwrap()
        .iter()
        .map(|entry| {
            hosts
                .write_string(entry, KnownHostFileKind::OpenSSH)
                .unwrap()
        })
        .collect()
}

#[test]
fn matching_pins_keep_hashed_endpoints_separate_even_with_identical_keys() {
    let session = Session::new().unwrap();
    let key = synthetic_host_key("ssh-ed25519", 1);
    let contents = format!(
        "{}{}",
        known_hosts_line(HASHED_OTHER, "ssh-ed25519", &key),
        known_hosts_line(HASHED_TARGET, "ssh-ed25519", &key)
    );
    assert_eq!(
        matching_known_host_keys(&session, &contents, "sftp.example", 22).unwrap(),
        vec![key]
    );
}

#[test]
fn matching_pins_ignore_unrelated_large_inventory_and_keep_aliases_and_ports() {
    let session = Session::new().unwrap();
    let other = synthetic_host_key("ssh-ed25519", 1);
    let target = synthetic_host_key("ecdsa-sha2-nistp256", 2);
    let mut contents = String::new();
    for index in 0..2048 {
        contents.push_str(&known_hosts_line(
            &format!("other-{index}.example"),
            "ssh-ed25519",
            &other,
        ));
    }
    contents.push_str(&known_hosts_line(
        "alias.example,[sftp.example]:2222",
        "ecdsa-sha2-nistp256",
        &target,
    ));
    assert_eq!(
        matching_known_host_keys(&session, &contents, "sftp.example", 22).unwrap(),
        Vec::<Vec<u8>>::new()
    );
    assert_eq!(
        matching_known_host_keys(&session, &contents, "sftp.example", 2222).unwrap(),
        vec![target.clone()]
    );
    assert_eq!(
        matching_known_host_keys(&session, &contents, "alias.example", 22).unwrap(),
        vec![target]
    );
}

#[test]
fn marked_entries_are_not_promoted_to_positive_host_key_pins() {
    let session = Session::new().unwrap();
    let key = synthetic_host_key("ssh-ed25519", 1);
    let line = known_hosts_line("sftp.example", "ssh-ed25519", &key);
    let contents = format!("@revoked {line}@cert-authority {line}");
    assert!(matches!(
        check_known_host_key(&session, &contents, "sftp.example", 22, &key).unwrap(),
        CheckResult::NotFound
    ));
}

#[test]
fn matching_pins_preserve_short_aliases_and_non_utf8_comments() {
    let session = Session::new().unwrap();
    let key = synthetic_host_key("ssh-ed25519", 1);
    let mut contents = known_hosts_line("db,database.example", "ssh-ed25519", &key).into_bytes();
    contents.extend_from_slice(b"# legacy Latin-1 comment: \xe4\n");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("known_hosts");
    fs::write(&path, &contents).unwrap();
    let loaded = load_known_hosts(&path).unwrap();
    assert_eq!(
        matching_known_host_keys(&session, &loaded, "db", 22).unwrap(),
        vec![key]
    );
    assert_eq!(fs::read(path).unwrap(), contents);
}
