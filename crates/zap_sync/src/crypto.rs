//! AES-256-GCM encryption/decryption module
//!
// author: logic
// date: 2026-05-26

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng, Payload, rand_core::RngCore as _};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use zeroize::Zeroizing;

const ENVELOPE_PREFIX: &str = "zaplex-sync:v2:";
const ENVELOPE_VERSION: u8 = 2;
const KEY_LEN: usize = 32;
const SALT_LEN: usize = 16;
const ARGON2_VERSION: u32 = 0x13;
const ARGON2_MEMORY_KIB: u32 = 19_456;
const ARGON2_ITERATIONS: u32 = 2;
const ARGON2_PARALLELISM: u32 = 1;

/// Encryption/decryption error
#[derive(Debug, Error)]
pub enum CryptoError {
    /// Encryption failed
    #[error("Encryption failed: {0}")]
    Encrypt(String),
    /// Decryption failed
    #[error("Decryption failed: {0}")]
    Decrypt(String),
    /// The supplied sync secret cannot authenticate the wrapped data key.
    #[error("Invalid sync secret or authenticated payload metadata")]
    InvalidSyncSecret,
    /// The encrypted payload uses a format this client cannot safely interpret.
    #[error("Unsupported encrypted payload version: {0}")]
    UnsupportedVersion(u8),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KdfMetadata {
    algorithm: String,
    version: u32,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedEnvelope {
    version: u8,
    kdf: KdfMetadata,
    key_wrap_nonce: String,
    wrapped_dek: String,
    payload_nonce: String,
    ciphertext: String,
}

#[derive(Serialize)]
struct AuthenticatedMetadata<'a> {
    version: u8,
    kdf: &'a KdfMetadata,
}

fn authenticated_metadata(version: u8, kdf: &KdfMetadata) -> Result<Vec<u8>, CryptoError> {
    serde_json::to_vec(&AuthenticatedMetadata { version, kdf })
        .map_err(|error| CryptoError::Encrypt(error.to_string()))
}

fn derive_argon2_key(
    sync_secret: &str,
    metadata: &KdfMetadata,
) -> Result<Zeroizing<[u8; KEY_LEN]>, CryptoError> {
    if metadata.algorithm != "argon2id"
        || metadata.version != ARGON2_VERSION
        || metadata.memory_kib != ARGON2_MEMORY_KIB
        || metadata.iterations != ARGON2_ITERATIONS
        || metadata.parallelism != ARGON2_PARALLELISM
    {
        return Err(CryptoError::InvalidSyncSecret);
    }
    let salt = BASE64
        .decode(&metadata.salt)
        .map_err(|_| CryptoError::InvalidSyncSecret)?;
    if salt.len() != SALT_LEN {
        return Err(CryptoError::InvalidSyncSecret);
    }
    let params = Params::new(
        metadata.memory_kib,
        metadata.iterations,
        metadata.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|_| CryptoError::InvalidSyncSecret)?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = Zeroizing::new([0u8; KEY_LEN]);
    argon2
        .hash_password_into(sync_secret.as_bytes(), &salt, &mut key[..])
        .map_err(|_| CryptoError::InvalidSyncSecret)?;
    Ok(key)
}

fn decode_envelope(encoded: &str) -> Result<EncryptedEnvelope, CryptoError> {
    let payload = encoded
        .strip_prefix(ENVELOPE_PREFIX)
        .ok_or_else(|| CryptoError::Decrypt("Legacy encrypted payload".to_string()))?;
    let json = BASE64
        .decode(payload)
        .map_err(|error| CryptoError::Decrypt(error.to_string()))?;
    let envelope: EncryptedEnvelope =
        serde_json::from_slice(&json).map_err(|error| CryptoError::Decrypt(error.to_string()))?;
    if envelope.version != ENVELOPE_VERSION {
        return Err(CryptoError::UnsupportedVersion(envelope.version));
    }
    Ok(envelope)
}

/// Returns whether the value uses the independent sync-secret envelope format.
pub fn is_current_envelope(encoded: &str) -> bool {
    encoded.starts_with(ENVELOPE_PREFIX)
}

/// Encrypt plaintext with a random DEK protected by a user-controlled sync secret.
///
/// The transport credential is deliberately absent from this API. Argon2id parameters and the
/// random salt are serialized in the envelope and authenticated as AEAD associated data.
pub fn encrypt(sync_secret: &str, plaintext: &str) -> Result<String, CryptoError> {
    if sync_secret.is_empty() {
        return Err(CryptoError::Encrypt(
            "A non-empty sync secret is required".to_string(),
        ));
    }

    let mut salt = [0u8; SALT_LEN];
    OsRng.fill_bytes(&mut salt);
    let kdf = KdfMetadata {
        algorithm: "argon2id".to_string(),
        version: ARGON2_VERSION,
        memory_kib: ARGON2_MEMORY_KIB,
        iterations: ARGON2_ITERATIONS,
        parallelism: ARGON2_PARALLELISM,
        salt: BASE64.encode(salt),
    };
    let aad = authenticated_metadata(ENVELOPE_VERSION, &kdf)?;
    let wrapping_key = derive_argon2_key(sync_secret, &kdf)
        .map_err(|error| CryptoError::Encrypt(error.to_string()))?;
    let wrapping_cipher = Aes256Gcm::new_from_slice(&wrapping_key[..])
        .map_err(|error| CryptoError::Encrypt(error.to_string()))?;

    let mut dek = Zeroizing::new([0u8; KEY_LEN]);
    OsRng.fill_bytes(&mut dek[..]);
    let key_wrap_nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let wrapped_dek = wrapping_cipher
        .encrypt(
            &key_wrap_nonce,
            Payload {
                msg: &dek[..],
                aad: &aad,
            },
        )
        .map_err(|error| CryptoError::Encrypt(error.to_string()))?;

    let data_cipher = Aes256Gcm::new_from_slice(&dek[..])
        .map_err(|error| CryptoError::Encrypt(error.to_string()))?;
    let payload_nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = data_cipher
        .encrypt(
            &payload_nonce,
            Payload {
                msg: plaintext.as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|error| CryptoError::Encrypt(error.to_string()))?;

    let envelope = EncryptedEnvelope {
        version: ENVELOPE_VERSION,
        kdf,
        key_wrap_nonce: BASE64.encode(key_wrap_nonce),
        wrapped_dek: BASE64.encode(wrapped_dek),
        payload_nonce: BASE64.encode(payload_nonce),
        ciphertext: BASE64.encode(ciphertext),
    };
    let json =
        serde_json::to_vec(&envelope).map_err(|error| CryptoError::Encrypt(error.to_string()))?;
    Ok(format!("{ENVELOPE_PREFIX}{}", BASE64.encode(json)))
}

/// Decrypt a current envelope with the user-controlled sync secret.
pub fn decrypt(sync_secret: &str, encoded: &str) -> Result<Zeroizing<String>, CryptoError> {
    let envelope = decode_envelope(encoded)?;
    let aad = serde_json::to_vec(&AuthenticatedMetadata {
        version: envelope.version,
        kdf: &envelope.kdf,
    })
    .map_err(|error| CryptoError::Decrypt(error.to_string()))?;
    let wrapping_key = derive_argon2_key(sync_secret, &envelope.kdf)?;
    let wrapping_cipher =
        Aes256Gcm::new_from_slice(&wrapping_key[..]).map_err(|_| CryptoError::InvalidSyncSecret)?;
    let key_wrap_nonce = BASE64
        .decode(&envelope.key_wrap_nonce)
        .map_err(|_| CryptoError::InvalidSyncSecret)?;
    let key_wrap_nonce = Nonce::from_slice(
        key_wrap_nonce
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::InvalidSyncSecret)?,
    );
    let wrapped_dek = BASE64
        .decode(&envelope.wrapped_dek)
        .map_err(|_| CryptoError::InvalidSyncSecret)?;
    let dek = Zeroizing::new(
        wrapping_cipher
            .decrypt(
                key_wrap_nonce,
                Payload {
                    msg: &wrapped_dek,
                    aad: &aad,
                },
            )
            .map_err(|_| CryptoError::InvalidSyncSecret)?,
    );
    if dek.len() != KEY_LEN {
        return Err(CryptoError::InvalidSyncSecret);
    }

    let data_cipher =
        Aes256Gcm::new_from_slice(&dek).map_err(|_| CryptoError::InvalidSyncSecret)?;
    let payload_nonce = BASE64
        .decode(&envelope.payload_nonce)
        .map_err(|_| CryptoError::InvalidSyncSecret)?;
    let payload_nonce = Nonce::from_slice(
        payload_nonce
            .as_slice()
            .try_into()
            .map_err(|_| CryptoError::InvalidSyncSecret)?,
    );
    let ciphertext = BASE64
        .decode(&envelope.ciphertext)
        .map_err(|_| CryptoError::InvalidSyncSecret)?;
    let plaintext = data_cipher
        .decrypt(
            payload_nonce,
            Payload {
                msg: &ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| CryptoError::InvalidSyncSecret)?;
    let plaintext =
        String::from_utf8(plaintext).map_err(|error| CryptoError::Decrypt(error.to_string()))?;
    Ok(Zeroizing::new(plaintext))
}

/// Decrypt the previous token-derived format during its bounded migration cycle.
pub fn decrypt_legacy(
    transport_token: &str,
    encoded: &str,
) -> Result<Zeroizing<String>, CryptoError> {
    let key = derive_legacy_key(transport_token);
    let combined = BASE64
        .decode(encoded)
        .map_err(|error| CryptoError::Decrypt(error.to_string()))?;
    if combined.len() < 12 {
        return Err(CryptoError::Decrypt("Data too short".to_string()));
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    let cipher = Aes256Gcm::new_from_slice(&key[..])
        .map_err(|error| CryptoError::Decrypt(error.to_string()))?;
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| CryptoError::InvalidSyncSecret)?;
    let plaintext =
        String::from_utf8(plaintext).map_err(|error| CryptoError::Decrypt(error.to_string()))?;
    Ok(Zeroizing::new(plaintext))
}

fn derive_legacy_key(transport_token: &str) -> Zeroizing<[u8; KEY_LEN]> {
    let mut hasher = Sha256::new();
    hasher.update(transport_token.as_bytes());
    let intermediate = Zeroizing::new(<[u8; KEY_LEN]>::from(hasher.finalize()));
    let mut hasher2 = Sha256::new();
    hasher2.update(&intermediate[..]);
    Zeroizing::new(hasher2.finalize().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TOKEN: &str = "test_token_for_crypto";

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let plaintext = "my_secret_password";
        let encrypted = encrypt(TEST_TOKEN, plaintext).unwrap();
        let decrypted = decrypt(TEST_TOKEN, &encrypted).unwrap();
        assert_eq!(decrypted.as_str(), plaintext);
    }

    #[test]
    fn test_same_sync_secret_decrypts_payload() {
        let encrypted = encrypt(TEST_TOKEN, "secret").unwrap();
        let decrypted = decrypt(TEST_TOKEN, &encrypted).unwrap();
        assert_eq!(decrypted.as_str(), "secret");
    }

    #[test]
    fn test_empty_string() {
        let encrypted = encrypt(TEST_TOKEN, "").unwrap();
        let decrypted = decrypt(TEST_TOKEN, &encrypted).unwrap();
        assert_eq!(decrypted.as_str(), "");
    }

    #[test]
    fn test_current_decrypt_rejects_legacy_or_invalid_payload() {
        let result = decrypt(TEST_TOKEN, "!!!not-base64!!!");
        assert!(result.is_err());
    }

    #[test]
    fn test_current_decrypt_rejects_legacy_short_payload() {
        // 8 bytes < 12 bytes (nonce size)
        let short = BASE64.encode(&[0u8; 8]);
        let result = decrypt(TEST_TOKEN, &short);
        assert!(result.is_err());
    }

    #[test]
    fn test_current_decrypt_rejects_legacy_ciphertext() {
        // 12 bytes nonce + 1 byte garbage
        let data = vec![0u8; 13];
        let encoded = BASE64.encode(&data);
        let result = decrypt(TEST_TOKEN, &encoded);
        assert!(result.is_err());
    }

    #[test]
    fn test_encrypt_produces_different_ciphertexts() {
        let plaintext = "same_input";
        let e1 = encrypt(TEST_TOKEN, plaintext).unwrap();
        let e2 = encrypt(TEST_TOKEN, plaintext).unwrap();
        // Different nonces should produce different ciphertexts
        assert_ne!(e1, e2);
        // But both should decrypt correctly
        assert_eq!(decrypt(TEST_TOKEN, &e1).unwrap().as_str(), plaintext);
        assert_eq!(decrypt(TEST_TOKEN, &e2).unwrap().as_str(), plaintext);
    }

    #[test]
    fn test_encrypt_unicode() {
        let plaintext = "你好世界🌍";
        let encrypted = encrypt(TEST_TOKEN, plaintext).unwrap();
        let decrypted = decrypt(TEST_TOKEN, &encrypted).unwrap();
        assert_eq!(decrypted.as_str(), plaintext);
    }

    #[test]
    fn test_encrypt_long_string() {
        let plaintext = "a".repeat(10_000);
        let encrypted = encrypt(TEST_TOKEN, &plaintext).unwrap();
        let decrypted = decrypt(TEST_TOKEN, &encrypted).unwrap();
        assert_eq!(decrypted.as_str(), plaintext);
    }

    #[test]
    fn test_wrong_sync_secret_is_reported_explicitly() {
        let plaintext = "secret_data";
        let encrypted = encrypt("sync-secret-alpha", plaintext).unwrap();
        assert!(matches!(
            decrypt("sync-secret-beta", &encrypted),
            Err(CryptoError::InvalidSyncSecret)
        ));
    }

    #[test]
    fn legacy_payload_can_be_recovered_with_its_original_transport_token() {
        let transport_token = "legacy-transport-token";
        let key = derive_legacy_key(transport_token);
        let cipher = Aes256Gcm::new_from_slice(&key[..]).unwrap();
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher.encrypt(&nonce, b"legacy-secret".as_ref()).unwrap();
        let mut combined = Vec::with_capacity(nonce.len() + ciphertext.len());
        combined.extend_from_slice(&nonce);
        combined.extend_from_slice(&ciphertext);
        let encoded = BASE64.encode(combined);

        assert_eq!(
            decrypt_legacy(transport_token, &encoded).unwrap().as_str(),
            "legacy-secret"
        );
    }

    #[test]
    fn test_empty_sync_secret_is_rejected() {
        assert!(encrypt("", "secret_data").is_err());
    }

    #[test]
    fn test_envelope_carries_versioned_argon2id_metadata() {
        let encrypted = encrypt(TEST_TOKEN, "secret").unwrap();
        let payload = encrypted.strip_prefix(ENVELOPE_PREFIX).unwrap();
        let json = BASE64.decode(payload).unwrap();
        let envelope: EncryptedEnvelope = serde_json::from_slice(&json).unwrap();

        assert_eq!(envelope.version, ENVELOPE_VERSION);
        assert_eq!(envelope.kdf.algorithm, "argon2id");
        assert_eq!(envelope.kdf.version, ARGON2_VERSION);
        assert_eq!(envelope.kdf.memory_kib, ARGON2_MEMORY_KIB);
        assert_eq!(envelope.kdf.iterations, ARGON2_ITERATIONS);
        assert_eq!(envelope.kdf.parallelism, ARGON2_PARALLELISM);
        assert_eq!(BASE64.decode(envelope.kdf.salt).unwrap().len(), SALT_LEN);
    }

    #[test]
    fn test_decrypt_exact_nonce_size() {
        // Exactly 12 bytes (nonce only, no ciphertext), AES-GCM decryption should fail
        let data = vec![0u8; 12];
        let encoded = BASE64.encode(&data);
        let result = decrypt(TEST_TOKEN, &encoded);
        assert!(result.is_err());
    }

    #[test]
    fn test_decrypt_tampered_ciphertext() {
        let encrypted = encrypt(TEST_TOKEN, "hello").unwrap();
        let payload = encrypted.strip_prefix(ENVELOPE_PREFIX).unwrap();
        let json = BASE64.decode(payload).unwrap();
        let mut envelope: EncryptedEnvelope = serde_json::from_slice(&json).unwrap();
        envelope.kdf.iterations += 1;
        let tampered = format!(
            "{ENVELOPE_PREFIX}{}",
            BASE64.encode(serde_json::to_vec(&envelope).unwrap())
        );
        let result = decrypt(TEST_TOKEN, &tampered);
        assert!(
            result.is_err(),
            "Decryption of tampered ciphertext should fail"
        );
    }

    #[test]
    fn test_crypto_error_display_encrypt() {
        let err = CryptoError::Encrypt("something went wrong".to_string());
        assert_eq!(format!("{err}"), "Encryption failed: something went wrong");
    }

    #[test]
    fn test_crypto_error_display_decrypt() {
        let err = CryptoError::Decrypt("bad data".to_string());
        assert_eq!(format!("{err}"), "Decryption failed: bad data");
    }

    #[test]
    fn test_encrypt_with_special_char_sync_secret() {
        let sync_secret = "secret\0with\ncontrol\tcharacters";
        let plaintext = "secret";
        let encrypted = encrypt(sync_secret, plaintext).unwrap();
        let decrypted = decrypt(sync_secret, &encrypted).unwrap();
        assert_eq!(decrypted.as_str(), plaintext);
    }

    #[test]
    fn test_encrypt_whitespace_sync_secret() {
        let sync_secret = "   ";
        let plaintext = "data";
        let encrypted = encrypt(sync_secret, plaintext).unwrap();
        let decrypted = decrypt(sync_secret, &encrypted).unwrap();
        assert_eq!(decrypted.as_str(), plaintext);
    }

    #[test]
    fn test_encrypt_very_long_sync_secret() {
        let sync_secret = "x".repeat(10_000);
        let plaintext = "short";
        let encrypted = encrypt(&sync_secret, plaintext).unwrap();
        let decrypted = decrypt(&sync_secret, &encrypted).unwrap();
        assert_eq!(decrypted.as_str(), plaintext);
    }
}
