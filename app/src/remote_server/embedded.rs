//! Bundled remote-server tarballs (install ladder rung 3a).
//!
//! Release/test bundles ship the Linux remote-server tarballs inside the app's
//! resources (macOS: `Zaplex.app/Contents/Resources/bundled/remote-server/`),
//! staged by CI with the same `GIT_RELEASE_TAG` as the client — inherently
//! version-matched. This gives the first-connect auto-install a fully offline,
//! instant source for the common host platforms; anything not bundled falls to
//! the client-download relay (rung 3b).

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use remote_server::setup::{tarball_basename, RemotePlatform};

/// Subdirectory (under the bundle's `Resources/bundled/`) holding the
/// remote-server tarballs. Must match the CI staging step in
/// `.github/workflows/test-dmg.yml` (and the release pipeline).
const REMOTE_SERVER_RESOURCE_DIR: &str = "remote-server";
const REMOTE_SERVER_DIGEST_MANIFEST: &str = "SHA256SUMS";

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

/// Returns the release-specific digest authenticated by the client bundle for `platform`.
/// The runtime never downloads this metadata alongside the archive: release CI places it in
/// the resources before the application is signed, so controlling the archive host cannot
/// replace the expected digest.
pub fn expected_server_tarball_sha256(platform: &RemotePlatform) -> Result<String> {
    let manifest_path = embedded_server_dir()
        .ok_or_else(|| anyhow!("bundled remote-server digest directory is unavailable"))?
        .join(REMOTE_SERVER_DIGEST_MANIFEST);
    let manifest = std::fs::read_to_string(&manifest_path).with_context(|| {
        format!(
            "failed to read authenticated remote-server digest manifest at {}",
            manifest_path.display()
        )
    })?;
    parse_sha256_manifest(&manifest, &tarball_basename(platform))
}

fn parse_sha256_manifest(manifest: &str, archive_name: &str) -> Result<String> {
    let mut matched_digest = None;

    for (line_index, line) in manifest.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split_ascii_whitespace();
        let digest = fields.next().ok_or_else(|| {
            anyhow!(
                "invalid remote-server digest manifest line {}",
                line_index + 1
            )
        })?;
        let filename = fields.next().ok_or_else(|| {
            anyhow!(
                "invalid remote-server digest manifest line {}",
                line_index + 1
            )
        })?;
        if fields.next().is_some()
            || digest.len() != 64
            || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(anyhow!(
                "invalid remote-server digest manifest line {}",
                line_index + 1
            ));
        }

        if filename == archive_name {
            if matched_digest.is_some() {
                return Err(anyhow!(
                    "duplicate digest for remote-server archive {archive_name}"
                ));
            }
            matched_digest = Some(digest.to_ascii_lowercase());
        }
    }

    matched_digest.ok_or_else(|| {
        anyhow!("authenticated digest is missing for remote-server archive {archive_name}")
    })
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

#[cfg(test)]
#[path = "embedded_tests.rs"]
mod tests;
