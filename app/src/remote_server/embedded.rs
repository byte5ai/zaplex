//! Bundled remote-server tarballs (install ladder rung 3a).
//!
//! Release/test bundles ship the Linux remote-server tarballs inside the app's
//! resources (macOS: `Zaplex.app/Contents/Resources/bundled/remote-server/`),
//! staged by CI with the same `GIT_RELEASE_TAG` as the client — inherently
//! version-matched. This gives the first-connect auto-install a fully offline,
//! instant source for the common host platforms; anything not bundled falls to
//! the client-download relay (rung 3b).

use std::path::PathBuf;

use remote_server::setup::{tarball_basename, RemotePlatform};

/// Subdirectory (under the bundle's `Resources/bundled/`) holding the
/// remote-server tarballs. Must match the CI staging step in
/// `.github/workflows/test-dmg.yml` (and the release pipeline).
const REMOTE_SERVER_RESOURCE_DIR: &str = "remote-server";

/// Directory holding the bundled remote-server tarballs, if this build has one.
fn embedded_server_dir() -> Option<PathBuf> {
    let dir = warp_core::paths::bundled_resources_dir()?
        .join("bundled")
        .join(REMOTE_SERVER_RESOURCE_DIR);
    dir.is_dir().then_some(dir)
}

/// Returns the bundled tarball for `platform`, if this build ships one.
/// The tarball uses the exact release-asset name (`tarball_basename`), so the
/// install path is byte-identical to the download path from here on.
pub fn embedded_server_tarball(platform: &RemotePlatform) -> Option<PathBuf> {
    let path = embedded_server_dir()?.join(tarball_basename(platform));
    path.is_file().then_some(path)
}

/// Whether this build bundles at least one remote-server tarball. Used at
/// startup (feature-flag gating) where the remote platform isn't known yet.
pub fn any_embedded_server_available() -> bool {
    let Some(dir) = embedded_server_dir() else {
        return false;
    };
    std::fs::read_dir(dir)
        .map(|mut entries| {
            entries.any(|e| {
                e.map(|e| e.file_name().to_string_lossy().ends_with(".tar.gz"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}
