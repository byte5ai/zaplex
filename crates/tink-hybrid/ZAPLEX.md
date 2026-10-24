# Tink-Hybrid in Zaplex

Dieses Modul stammt unverändert bis auf die unten genannten Anpassungen aus
[`warpdotdev/tink-rust`, Commit `0141035f04a5e262b955c450857b689cff877469`](https://github.com/warpdotdev/tink-rust/tree/0141035f04a5e262b955c450857b689cff877469/hybrid).
Die Apache-2.0-Lizenz und die ursprünglichen Copyright-Hinweise bleiben erhalten.

Die lokale Übernahme behebt [RUSTSEC-2026-0387](https://rustsec.org/advisories/RUSTSEC-2026-0387)
für [Zaplex #668](https://github.com/byte5ai/zaplex/issues/668).
Der bisherige Fork und sein Upstream verlangen noch P-256 0.13.2. Zaplex nutzt
P-256 0.14 mit `crypto-bigint` mindestens 0.7.6; `Cargo.lock` bindet die konkreten
Versionen. Tink-Core und Tink-Proto bleiben auf der bisherigen Fork-Revision.

Angepasst sind das Cargo-Manifest und die P-256-API in
`src/subtle/elliptic_curves.rs`: Array-/SEC1-Typen, Zufallsschlüsselerzeugung und
fehlbare Byte-Konvertierung. Es gibt keine eigene Kurvenarithmetik.
Die bestehenden ECIES-Templates, Schlüsseltypen, Präfixe, HKDF-/AEAD-Verfahren
und öffentlichen Tink-Funktionen bleiben erhalten. Für WASM aktiviert die neue
Zufallsquelle den vorhandenen JavaScript-Unterbau; der lokale Nachweis umfasst
Linux, nicht WASM.

`src/compatibility_tests.rs` prüft 216 veröffentlichte Wycheproof-ECDH-Fälle,
beide bestehenden ECIES-Templates, alte Ciphertexte, Manipulationen und
Kontextbindung. Die Fixture-Herkunft steht in `fixtures/README.md`.
Die bestehenden HPKE-Tests in `warp_managed_secrets` prüfen die Einbindung des
separaten X25519-HPKE-Managers. Der manuelle CI-Batch führt beide Pakete aus.

Als einmaliger Gegencheck wurden außerdem mit dem neuen Backend erzeugte
Ciphertexte beider Templates durch das ursprüngliche Backend mit P-256 0.13.2
entschlüsselt. Beide Richtungen bleiben damit für die geprüften Formate kompatibel.
