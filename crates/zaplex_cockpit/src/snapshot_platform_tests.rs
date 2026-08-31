#![cfg(not(target_os = "linux"))]

use super::*;
use std::fs;

#[test]
fn non_linux_loaded_snapshot_remains_routable() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join(".claude.json"),
        r#"{"oauthAccount":{"emailAddress":"default@example.com"}}"#,
    )
    .unwrap();

    let snapshot = build_snapshot(
        &home,
        &home.join(".codex"),
        None,
        Utc::now(),
        0,
        0,
        &PricingTable::default(),
    );

    assert_eq!(snapshot.health, ScanHealth::Loaded);
    assert_eq!(
        pick_freest_checked(Provider::Claude, &snapshot).map(|usage| usage.account.key.as_str()),
        Some("claude:default")
    );
}
