use super::*;

#[test]
fn sha256_manifest_returns_platform_digest() {
    let digest = "a".repeat(64);
    let manifest = format!(
        "{}  zap-remote-server-linux-x86_64.tar.gz\n{}  zap-remote-server-macos-aarch64.tar.gz\n",
        "b".repeat(64),
        digest
    );

    assert_eq!(
        parse_sha256_manifest(&manifest, "zap-remote-server-macos-aarch64.tar.gz").unwrap(),
        digest
    );
}

#[test]
fn sha256_manifest_fails_closed_for_missing_platform() {
    let manifest = format!(
        "{}  zap-remote-server-linux-x86_64.tar.gz\n",
        "a".repeat(64)
    );

    let error =
        parse_sha256_manifest(&manifest, "zap-remote-server-linux-aarch64.tar.gz").unwrap_err();

    assert!(error
        .to_string()
        .contains("authenticated digest is missing"));
}

#[test]
fn sha256_manifest_rejects_malformed_or_duplicate_entries() {
    assert!(parse_sha256_manifest(
        "not-a-digest  zap-remote-server-linux-x86_64.tar.gz\n",
        "zap-remote-server-linux-x86_64.tar.gz"
    )
    .is_err());

    let digest = "a".repeat(64);
    let duplicate =
        format!("{digest}  zap-remote-server-linux-x86_64.tar.gz\n{digest}  zap-remote-server-linux-x86_64.tar.gz\n");
    assert!(parse_sha256_manifest(&duplicate, "zap-remote-server-linux-x86_64.tar.gz").is_err());
}
