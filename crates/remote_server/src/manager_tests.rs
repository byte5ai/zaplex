//! Pure function level unit tests for `manager.rs`.
//!
//! This covers only pure function helpers — does not touch `RemoteServerManager` itself,
//! since it depends on `warpui::Entity` / `ModelContext` and requires a full App context,
//! which is better suited for the integration testing framework.

use super::*;

// ---------------------------------------------------------------------------
// version_is_compatible
// ---------------------------------------------------------------------------

#[test]
fn version_compat_both_tagged_and_equal() {
    assert!(version_is_compatible(
        Some("v0.2026.05.10.stable"),
        "v0.2026.05.10.stable",
    ));
}

#[test]
fn version_compat_both_tagged_and_different() {
    assert!(!version_is_compatible(
        Some("v0.2026.05.10.stable"),
        "v0.2026.05.10.preview",
    ));
}

#[test]
fn version_compat_both_untagged() {
    // Client has no GIT_RELEASE_TAG (cargo run), server also returns empty string
    // (`script/deploy_remote_server` dev deployment): treat as compatible to preserve
    // local development loop unaffected.
    assert!(version_is_compatible(None, ""));
}

#[test]
fn version_compat_client_tagged_server_untagged() {
    // Client is release, server is dev deployment → treat as incompatible,
    // normally trigger reinstall process.
    assert!(!version_is_compatible(Some("v0.2026.05.10.stable"), ""));
}

#[test]
fn version_compat_client_untagged_server_tagged() {
    // **Critical scenario**: Zaplex client has no tag (cargo build),
    // server is a release from official CDN (with tag). Original helper
    // would judge as incompatible, triggering `remove_remote_server_binary` → infinite loop.
    // This test only records that `version_is_compatible` behavior itself does not change;
    // the actual "skip validation" is handled by [`should_enforce_remote_version_check`].
    assert!(!version_is_compatible(None, "v0.2026.05.10.stable"));
}

// ---------------------------------------------------------------------------
// should_enforce_remote_version_check
// ---------------------------------------------------------------------------

#[test]
fn enforce_version_check_skipped_on_oss() {
    // When Zaplex temporarily reuses official release binaries, client and server versions
    // are always inconsistent; strict validation must be skipped.
    assert!(!should_enforce_remote_version_check(Channel::Oss));
}

#[test]
fn enforce_version_check_kept_on_official_channels() {
    // On official channels, client and server either both come from the same release CI
    // or both from local deployment via `script/deploy_remote_server`; strict
    // validation remains necessary — preserve original stale binary self-healing path.
    for channel in [
        Channel::Stable,
        Channel::Preview,
        Channel::Dev,
        Channel::Local,
        Channel::Integration,
    ] {
        assert!(
            should_enforce_remote_version_check(channel),
            "channel {channel:?} should still enforce version check"
        );
    }
}
