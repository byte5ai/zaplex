use std::fs;
use std::os::unix::fs::{symlink, PermissionsExt as _};

use tempfile::tempdir;

use super::{
    encrypt_with_key, initialize_once_on_success, make_encryption_key, SecureStorage,
    FALLBACK_KEY_FILE, FALLBACK_KEY_LEN, LEGACY_FALLBACK_KEY,
};
use crate::secure_storage::Error;

#[test]
fn transient_initialization_failure_is_retried() {
    let cell = std::cell::OnceCell::new();
    let attempts = std::cell::Cell::new(0);
    let first = initialize_once_on_success(&cell, || {
        attempts.set(attempts.get() + 1);
        Err::<u8, _>("transient")
    });
    assert_eq!(first, Err("transient"));
    assert!(cell.get().is_none());

    let second = initialize_once_on_success(&cell, || {
        attempts.set(attempts.get() + 1);
        Ok::<u8, &str>(42)
    });
    assert_eq!(second.map(|value| *value), Ok(42));
    assert_eq!(attempts.get(), 2);
}

#[test]
fn fallback_storage_is_private_and_installation_scoped() {
    let root_a = tempdir().unwrap();
    let root_b = tempdir().unwrap();
    let fallback_a = root_a.path().join("fallback");
    let fallback_b = root_b.path().join("fallback");
    fs::create_dir(&fallback_a).unwrap();
    fs::set_permissions(&fallback_a, fs::Permissions::from_mode(0o755)).unwrap();
    let ciphertext_a = fallback_a.join("darmok-token");
    fs::write(&ciphertext_a, b"old-placeholder").unwrap();
    fs::set_permissions(&ciphertext_a, fs::Permissions::from_mode(0o644)).unwrap();

    let storage_a = SecureStorage::new_with_fallback("darmok", fallback_a.clone());
    storage_a
        .write_fallback_value("token", "shaka when the walls fell")
        .unwrap();

    assert_eq!(
        fs::metadata(&fallback_a).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(fallback_a.join(FALLBACK_KEY_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(&ciphertext_a).unwrap().permissions().mode() & 0o777,
        0o600
    );

    let ciphertext = fs::read(&ciphertext_a).unwrap();
    let storage_b = SecureStorage::new_with_fallback("darmok", fallback_b);
    assert!(storage_b.fallback_decrypt(&ciphertext).is_err());

    let restarted = SecureStorage::new_with_fallback("darmok", fallback_a);
    assert_eq!(
        restarted.read_fallback_value("token").unwrap(),
        "shaka when the walls fell"
    );
}

#[test]
fn legacy_ciphertext_is_reencrypted_with_installation_key() {
    let root = tempdir().unwrap();
    let fallback = root.path().join("fallback");
    fs::create_dir(&fallback).unwrap();
    fs::set_permissions(&fallback, fs::Permissions::from_mode(0o755)).unwrap();

    let mut legacy_bytes = LEGACY_FALLBACK_KEY.to_vec();
    legacy_bytes.resize(FALLBACK_KEY_LEN, 0);
    let legacy_key = make_encryption_key(&legacy_bytes).unwrap();
    let legacy_ciphertext = encrypt_with_key(&legacy_key, "legacy secret").unwrap();
    let ciphertext_path = fallback.join("darmok-token");
    fs::write(&ciphertext_path, &legacy_ciphertext).unwrap();
    fs::set_permissions(&ciphertext_path, fs::Permissions::from_mode(0o644)).unwrap();

    let storage = SecureStorage::new_with_fallback("darmok", fallback);
    assert_eq!(
        storage.read_fallback_value("token").unwrap(),
        "legacy secret"
    );

    let migrated = fs::read(ciphertext_path).unwrap();
    assert_ne!(migrated, legacy_ciphertext);
    assert!(storage.legacy_fallback_decrypt(&migrated).is_err());
    assert_eq!(
        storage.fallback_decrypt(&migrated).unwrap(),
        "legacy secret"
    );
}

#[test]
fn fallback_rejects_symbolic_link_ciphertext() {
    let root = tempdir().unwrap();
    let fallback = root.path().join("fallback");
    fs::create_dir(&fallback).unwrap();
    let target = root.path().join("target");
    fs::write(&target, b"must not be replaced").unwrap();
    symlink(&target, fallback.join("darmok-token")).unwrap();
    let storage = SecureStorage::new_with_fallback("darmok", fallback);

    assert!(storage.write_fallback_value("token", "secret").is_err());
    assert_eq!(fs::read(target).unwrap(), b"must not be replaced");
}

#[test]
fn decrypt_fails_on_malformed_data() {
    let root = tempdir().unwrap();
    let storage = SecureStorage::new_with_fallback("darmok", root.path().join("fallback"));
    let bad_datas: [&[u8]; 4] = [&[], &[0; 1], &[0; 11], &[0; 12]];

    for bad_data in bad_datas {
        let result = storage.fallback_decrypt(bad_data);
        assert!(result.is_err());
        let Error::Unknown(error) = result.unwrap_err() else {
            panic!("expected Error::Unknown")
        };
        assert_eq!(
            format!("{error}"),
            "Attempting to decrypt too small value for fallback decryption"
        );
    }
}
