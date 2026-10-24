use std::io::Cursor;

use tink_core::keyset::{insecure, BinaryReader, Handle};
use tink_proto::{EcPointFormat, EllipticCurveType};

use crate::subtle::{compute_shared_secret, point_decode, EcPrivateKey};

#[test]
fn p256_matches_wycheproof_ecdh_vectors_and_rejects_invalid_points() {
    let vectors: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/ecdh_secp256r1_ecpoint_test.json")).unwrap();
    let mut counts = [0; 3];
    for group in vectors["testGroups"].as_array().unwrap() {
        assert_eq!(group["curve"], "secp256r1");
        for case in group["tests"].as_array().unwrap() {
            let public = hex::decode(case["public"].as_str().unwrap()).unwrap();
            let private = hex::decode(case["private"].as_str().unwrap()).unwrap();
            let format = match public.first() {
                Some(2 | 3) => EcPointFormat::Compressed,
                _ => EcPointFormat::Uncompressed,
            };
            let result =
                point_decode(EllipticCurveType::NistP256, format, &public).and_then(|public| {
                    EcPrivateKey::new(EllipticCurveType::NistP256, &private)
                        .and_then(|private| compute_shared_secret(&public, &private))
                });
            match case["result"].as_str().unwrap() {
                "valid" | "acceptable" => {
                    let index = usize::from(case["result"] == "acceptable");
                    counts[index] += 1;
                    let expected = hex::decode(case["shared"].as_str().unwrap()).unwrap();
                    assert_eq!(result.unwrap(), expected, "case {}", case["tcId"]);
                }
                "invalid" => {
                    counts[2] += 1;
                    assert!(result.is_err(), "case {} accepted", case["tcId"]);
                }
                other => panic!("unknown vector outcome: {}", other),
            }
        }
    }
    assert_eq!(counts, [191, 1, 24]);
}

#[test]
fn ecies_decrypts_legacy_ciphertexts_and_authenticates_context() {
    crate::init();
    let fixtures: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../fixtures/legacy-ecies.json")).unwrap();
    assert_eq!(fixtures.len(), 2);
    for fixture in fixtures {
        let keyset = hex::decode(fixture["keyset_hex"].as_str().unwrap()).unwrap();
        let private = insecure::read(&mut BinaryReader::new(Cursor::new(keyset))).unwrap();
        let ciphertext = hex::decode(fixture["ciphertext_hex"].as_str().unwrap()).unwrap();
        let context = fixture["context"].as_str().unwrap().as_bytes();
        let plaintext = fixture["plaintext"].as_str().unwrap().as_bytes();
        let decrypt = crate::new_decrypt(&private).unwrap();
        assert_eq!(decrypt.decrypt(&ciphertext, context).unwrap(), plaintext);
        assert!(decrypt.decrypt(&ciphertext, b"wrong context").is_err());
        for offset in 0..ciphertext.len() {
            let mut altered = ciphertext.clone();
            altered[offset] ^= 1;
            assert!(decrypt.decrypt(&altered, context).is_err());
        }
        let encrypt = crate::new_encrypt(&private.public().unwrap()).unwrap();
        let fresh = encrypt.encrypt(plaintext, context).unwrap();
        assert_eq!(&fresh[..5], &ciphertext[..5]);
        assert_eq!(fresh.len(), ciphertext.len());
        assert_eq!(decrypt.decrypt(&fresh, context).unwrap(), plaintext);
    }
}

#[test]
fn ecies_generates_fresh_keysets_for_both_existing_templates() {
    crate::init();
    for template in [
        crate::ecies_hkdf_aes128_gcm_key_template(),
        crate::ecies_hkdf_aes128_ctr_hmac_sha256_key_template(),
    ] {
        let private = Handle::new(&template).unwrap();
        let public = private.public().unwrap();
        let encrypted = crate::new_encrypt(&public)
            .unwrap()
            .encrypt(b"fresh key material", b"context")
            .unwrap();
        assert_eq!(
            crate::new_decrypt(&private)
                .unwrap()
                .decrypt(&encrypted, b"context")
                .unwrap(),
            b"fresh key material"
        );
    }
}
