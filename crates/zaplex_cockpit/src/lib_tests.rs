use super::*;
use chrono::Utc;
use std::{fs, path::Path};
use walkdir::WalkDir;

/// A present-but-unreadable `auth.json` makes codex discovery return no account —
/// identical in shape to "Codex was never set up". The snapshot must report this
/// as *degraded* so the UI can say "couldn't read your account" (and offer a
/// retry) instead of the misleading "no accounts".
#[test]
fn a_malformed_codex_auth_json_degrades_the_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let codex_home = tmp.path().join("codex");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&codex_home).unwrap();
    fs::write(codex_home.join("auth.json"), "{ this is not valid json").unwrap();

    let snap = build_snapshot(
        &home,
        &codex_home,
        None,
        Utc::now(),
        0,
        0,
        &PricingTable::default(),
    );
    assert!(
        matches!(snap.health, ScanHealth::Degraded(_)),
        "a present-but-unreadable codex auth.json must degrade, not read as empty: {:?}",
        snap.health,
    );
}

#[test]
fn a_malformed_claude_identity_degrades_the_snapshot() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let codex_home = home.join(".codex");
    fs::create_dir_all(home.join(".claude")).unwrap();
    fs::write(home.join(".claude/.claude.json"), "{not valid json").unwrap();

    let snap = build_snapshot(
        &home,
        &codex_home,
        None,
        Utc::now(),
        0,
        0,
        &PricingTable::default(),
    );
    assert!(
        matches!(snap.health, ScanHealth::Degraded(_)),
        "a malformed Claude identity must degrade, not invent an account: {:?}",
        snap.health,
    );
    assert!(snap.accounts.is_empty());
}

/// A clean setup with genuinely no accounts is authoritative — an empty list that
/// the UI may present as a real "no accounts" (with a sign-in prompt), not a
/// failure.
#[test]
fn a_clean_empty_setup_is_loaded_not_degraded() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let codex_home = tmp.path().join("codex");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&codex_home).unwrap();

    let snap = build_snapshot(
        &home,
        &codex_home,
        None,
        Utc::now(),
        0,
        0,
        &PricingTable::default(),
    );
    assert_eq!(
        snap.health,
        ScanHealth::Loaded,
        "a clean, genuinely-empty setup is authoritative-empty",
    );
}

#[test]
fn non_test_sources_use_file_backed_test_modules() {
    let source_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in WalkDir::new(source_dir) {
        let entry = entry.expect("crate source tree must be readable");
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !entry.file_type().is_file()
            || path.extension().and_then(|extension| extension.to_str()) != Some("rs")
            || file_name.ends_with("_tests.rs")
        {
            continue;
        }

        let source = fs::read_to_string(path).expect("Rust source must be readable");
        let mut after_test_cfg = false;
        for (line_number, line) in source.lines().enumerate() {
            let line = line.trim();
            if line == "#[cfg(test)]" {
                after_test_cfg = true;
                continue;
            }
            if !after_test_cfg || line.is_empty() || line.starts_with("#[") {
                continue;
            }
            assert!(
                !(line.starts_with("mod ") && line.ends_with('{')),
                "inline test module in {}:{}",
                path.display(),
                line_number + 1,
            );
            after_test_cfg = false;
        }
    }
}
