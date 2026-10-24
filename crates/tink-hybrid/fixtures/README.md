# Öffentliche Kryptographie-Testdaten

`ecdh_secp256r1_ecpoint_test.json` ist bytegenau aus
[Wycheproof 2196000605e45d91097147c9c71f26b72af58003](https://github.com/google/wycheproof/blob/2196000605e45d91097147c9c71f26b72af58003/testvectors/ecdh_secp256r1_ecpoint_test.json),
Apache-2.0. SHA256: `14d4ac9ec0b18f960f32d53ca81c7f70403672747f964f48f9a7bfcbd516b690`.
Alle 216 Fälle werden geprüft: 191 gültige, ein zulässiger komprimierter Punkt
und 24 ungültige Punkte. Die erwarteten Shared Secrets stammen aus den
veröffentlichten Vektoren, nicht aus der neuen Implementierung.

`legacy-ecies.json` enthält ausschließlich neu erzeugte öffentliche Testschlüssel
und Testtexte, niemals Produktionsdaten. Sie wurde einmalig mit dem bisherigen
Tink-Hybrid-Commit `0141035f04a5e262b955c450857b689cff877469` und P-256 0.13.2
erzeugt. Pro bestehendem Template (`ecies_hkdf_aes128_gcm` und
`ecies_hkdf_aes128_ctr_hmac_sha256`) wurden mit `Handle::new` ein Testschlüsselsatz
und mit `new_encrypt` ein Ciphertext erzeugt. `keyset_hex` ist das binäre
Tink-Keyset, `ciphertext_hex` der vollständige Ciphertext einschließlich
Tink-Präfix; Klartext und Kontext stehen daneben.

Die Tests entschlüsseln diese festen alten Ciphertexte mit dem neuen Backend,
prüfen alle einzelnen Byte-Manipulationen und falschen Kontext, vergleichen
Präfix und Länge frisch erzeugter Ciphertexte und prüfen neue Schlüsselsätze.

SHA256 der alten ECIES-Fixture: `9d3ed7e5aab98b3179ca4888145ae8830b47cfcd0b34e4986ee294d14011d0db`.
